//! One-time migration from the retired Serein/SereinExt identifiers to Nivra.
//!
//! This is the ONLY module in the repository allowed to name the retired
//! identifiers (old data dir, old keyring service, old app id, old shortcut,
//! old Run value). Everywhere else uses only the Nivra names. The repository
//! guard test allowlists this file for that reason.
//!
//! Rules enforced here:
//! - Data dir: rename `serein` -> `nivra` once with `fs::rename`. If the
//!   destination already exists, do not touch the source. If the rename fails,
//!   return Err and NEVER open an empty dir over the old data.
//! - Keyring: copy the token from the old service, read it back, verify it is
//!   equal, and only then delete the old entry. Any failure keeps the old entry.
//! - Autostart (Windows): create the new `Nivra` Run value from the old `Serein`
//!   entry (preserving `--start-minimized`), then delete the old value.
//! - Shortcut (Windows): create `Nivra.lnk` with the new AUMID, then delete
//!   `SereinExt.lnk`. If the new link already exists, only clean up the old.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Retired vs current identifiers (only here).
// ---------------------------------------------------------------------------

/// Old data dir under `dirs::data_local_dir()`.
pub const OLD_DATA_DIR_NAME: &str = "serein";
/// New data dir.
pub const NEW_DATA_DIR_NAME: &str = "nivra";

/// Old keyring service.
pub const OLD_CREDENTIAL_SERVICE: &str = "io.github.vitorhubdev.SereinExt";
/// Old app id / AUMID (oldest credential source, upstream). Newer old first.
pub const OLD_APP_ID: &str = "cz.viceverse.serein";
/// Old keyring sources, newer old first: fork SereinExt, then upstream cz.
pub const OLD_SOURCES: [&str; 2] = [OLD_CREDENTIAL_SERVICE, OLD_APP_ID];
/// Old Windows Run value.
pub const OLD_RUN_VALUE: &str = "Serein";
/// Old Start-Menu shortcut.
pub const OLD_SHORTCUT_NAME: &str = "SereinExt.lnk";
/// Old PowerShell shortcut helper class.
pub const OLD_SHORTCUT_CLASS: &str = "SereinExtShortcut";

// New identifiers are duplicated here for migration logic; everywhere else they
// are the only names present.
pub const NEW_RUN_VALUE: &str = "Nivra";
pub const NEW_SHORTCUT_NAME: &str = "Nivra.lnk";
pub const NEW_SHORTCUT_CLASS: &str = "NivraShortcut";

// ---------------------------------------------------------------------------
// Data dir migration (fatal on failure).
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum DataMigrationError {
	NoBaseDir,
	RenameFailed(String),
	CreateFailed(String),
}

impl std::fmt::Display for DataMigrationError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::NoBaseDir => write!(f, "Could not locate the local data directory"),
			Self::RenameFailed(e) => write!(f, "Could not migrate the old data folder: {e}"),
			Self::CreateFailed(e) => write!(f, "Could not create the Nivra data folder: {e}"),
		}
	}
}

/// Rename `old` -> `new` once. Pure filesystem, no `dirs` dependency so tests
/// can use temp dirs.
///
/// - If `new` exists, do nothing (idempotent, never touches `old`).
/// - Else if `old` exists, `rename(old, new)`. On failure return Err and do NOT
///   create an empty `new` (the caller must not open empty over old data).
/// - Else (neither exists), `create_dir_all(new)`.
pub fn migrate_data_dir(old: &Path, new: &Path) -> Result<(), DataMigrationError> {
	if new.exists() {
		return Ok(());
	}
	if old.exists() {
		std::fs::rename(old, new).map_err(|e| DataMigrationError::RenameFailed(e.to_string()))?;
		return Ok(());
	}
	std::fs::create_dir_all(new).map_err(|e| DataMigrationError::CreateFailed(e.to_string()))?;
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		let _ = std::fs::set_permissions(new, std::fs::Permissions::from_mode(0o700));
	}
	Ok(())
}

/// Resolve `dirs::data_local_dir()/nivra`, migrating `serein` -> `nivra` once.
/// Fatal: on Err the caller must show an error and exit without opening.
pub fn ensure_data_dir() -> Result<PathBuf, DataMigrationError> {
	let base = dirs::data_local_dir().ok_or(DataMigrationError::NoBaseDir)?;
	let old = base.join(OLD_DATA_DIR_NAME);
	let new = base.join(NEW_DATA_DIR_NAME);
	migrate_data_dir(&old, &new)?;
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		let _ = std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o700));
	}
	Ok(new)
}

/// Read account IDs the app knows (accounts table in the migrated nivra store).
/// Best effort for keyring migration: if the store cannot be opened/read (fresh
/// install with no DB yet, incompatible version, locked, etc.), return Err and
/// the caller skips per-account migration but still returns Ok overall.
fn read_account_ids(data_dir: &Path) -> Result<Vec<model::Id>, String> {
	let store = local_store::LocalStore::open(&data_dir.join("client.sqlite3"))
		.map_err(|e| format!("{e:?}"))?;
	let accounts = store.accounts().map_err(|e| format!("{e:?}"))?;
	Ok(accounts.into_iter().map(|a| a.id).collect())
}

// ---------------------------------------------------------------------------
// Keyring migration (non-fatal: failure keeps the old entry).
// ---------------------------------------------------------------------------

/// Minimal credential store abstraction so tests use a mock and never touch the
/// OS keyring.
pub trait CredentialStore {
	fn get(&self, service: &str, account: &str) -> Result<Option<String>, String>;
	fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), String>;
	fn delete(&self, service: &str, account: &str) -> Result<(), String>;
}

/// OS-backed store used in production.
pub struct OsStore;

impl CredentialStore for OsStore {
	fn get(&self, service: &str, account: &str) -> Result<Option<String>, String> {
		match keyring::Entry::new(service, account) {
			Ok(entry) => match entry.get_password() {
				Ok(v) => Ok(Some(v)),
				Err(keyring::Error::NoEntry) => Ok(None),
				Err(e) => Err(format!("{e:?}")),
			},
			Err(e) => Err(format!("{e:?}")),
		}
	}
	fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), String> {
		keyring::Entry::new(service, account)
			.map_err(|e| format!("{e:?}"))?
			.set_password(secret)
			.map_err(|e| format!("{e:?}"))
	}
	fn delete(&self, service: &str, account: &str) -> Result<(), String> {
		match keyring::Entry::new(service, account) {
			Ok(entry) => match entry.delete_credential() {
				Ok(()) => Ok(()),
				Err(keyring::Error::NoEntry) => Ok(()),
				Err(e) => Err(format!("{e:?}")),
			},
			Err(e) => Err(format!("{e:?}")),
		}
	}
}

/// In-memory mock for tests.
#[derive(Default)]
pub struct MockStore {
	inner: Mutex<HashMap<(String, String), String>>,
	/// When true, `set` fails (to test "failure keeps the old entry").
	pub fail_set: bool,
}

impl MockStore {
	pub fn with_entry(service: &str, account: &str, secret: &str) -> Self {
		let s = Self::default();
		s.inner
			.lock()
			.unwrap()
			.insert((service.to_owned(), account.to_owned()), secret.to_owned());
		s
	}
}

impl CredentialStore for MockStore {
	fn get(&self, service: &str, account: &str) -> Result<Option<String>, String> {
		Ok(self
			.inner
			.lock()
			.unwrap()
			.get(&(service.to_owned(), account.to_owned()))
			.cloned())
	}
	fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), String> {
		if self.fail_set {
			return Err("mock set failed".to_owned());
		}
		self.inner
			.lock()
			.unwrap()
			.insert((service.to_owned(), account.to_owned()), secret.to_owned());
		Ok(())
	}
	fn delete(&self, service: &str, account: &str) -> Result<(), String> {
		self.inner
			.lock()
			.unwrap()
			.remove(&(service.to_owned(), account.to_owned()));
		Ok(())
	}
}

#[derive(Debug, PartialEq, Eq)]
pub enum CredentialOutcome {
	Migrated,
	AlreadyMigrated,
	NoOldEntry,
}

/// Copy `account` from `old_service` to `new_service`, read back, verify equal,
/// and only then delete the old entry. Any failure keeps the old entry.
pub fn migrate_one_entry(
	store: &impl CredentialStore,
	old_service: &str,
	new_service: &str,
	account: &str,
) -> Result<CredentialOutcome, String> {
	let old = store.get(old_service, account)?;
	let Some(old_secret) = old else {
		return Ok(CredentialOutcome::NoOldEntry);
	};
	match store.get(new_service, account) {
		Ok(Some(existing)) if existing == old_secret => {
			// Already copied (e.g. previous run wrote new but crashed before
			// deleting old). Clean up the old entry now.
			store.delete(old_service, account)?;
			return Ok(CredentialOutcome::AlreadyMigrated);
		}
		Ok(Some(_)) => {
			// New already holds a different token (user re-logged). Do not
			// overwrite the current login and do not delete the old entry.
			return Err("new credential already exists with a different value".to_owned());
		}
		Ok(None) => {}
		Err(e) => return Err(e),
	}
	store.set(new_service, account, &old_secret)?;
	// Read back and verify before deleting the source.
	match store.get(new_service, account)? {
		Some(v) if v == old_secret => {
			store.delete(old_service, account)?;
			Ok(CredentialOutcome::Migrated)
		}
		Some(_) => Err("verification read-back differs".to_owned()),
		None => Err("verification read-back missing".to_owned()),
	}
}

/// Migrate one credential `account` (principal `discord-session` or per-account
/// `discord-session.<id>`) from old sources (newer SereinExt, then oldest cz)
/// to `new_service`. Same rule per source: copy, read back, verify equal,
/// only then delete that source; if `new_service` already holds a different
/// value, do not overwrite it and do not delete the differing old source.
/// After migrating from the newer old source, the oldest source is cleaned
/// only if its value equals the migrated (new) value.
pub fn migrate_entry_from_sources(
	store: &impl CredentialStore,
	new_service: &str,
	account: &str,
) -> Result<CredentialOutcome, String> {
	let mut migrated: Option<CredentialOutcome> = None;
	let mut saw_different_new = false;
	for old_service in OLD_SOURCES {
		match migrate_one_entry(store, old_service, new_service, account) {
			Ok(CredentialOutcome::Migrated) => {
				migrated = Some(CredentialOutcome::Migrated);
				// Continue to clean the older source if it holds the same value.
			}
			Ok(CredentialOutcome::AlreadyMigrated) => {
				// Old source already cleaned (equal case) - continue to older source
				// in case it still holds the same value needing cleanup.
				if migrated.is_none() {
					migrated = Some(CredentialOutcome::AlreadyMigrated);
				}
			}
			Ok(CredentialOutcome::NoOldEntry) => {}
			Err(e) => {
				// `migrate_one_entry` errs when new already holds a different value
				// (do not overwrite, do not delete) or on store failures (keep old).
				// Remember that new differs so older sources must not overwrite it;
				// still try older sources for equal-value cleanup (which will also
				// refuse to overwrite differing new, but may clean equal older).
				// For store failures (keep old), continue to older source? No -
				// if the store itself fails, older sources likely fail too; still
				// try once for equal-value cleanup, but do not overwrite differing new.
				// Fall through to next source (which will see differing new and refuse).
				if !e.contains("different") && !e.contains("already exists") {
					return Err(e);
				}
				saw_different_new = true;
			}
		}
	}
	if let Some(out) = migrated {
		return Ok(out);
	}
	if saw_different_new {
		return Err("new credential already exists with a different value".to_owned());
	}
	Ok(CredentialOutcome::NoOldEntry)
}

/// Migrate the main `discord-session` entry from both old sources. Non-fatal.
pub fn migrate_keyring(
	store: &impl CredentialStore,
	new_service: &str,
) -> Result<CredentialOutcome, String> {
	migrate_entry_from_sources(store, new_service, super::ACCOUNT)
}

// ---------------------------------------------------------------------------
// Autostart migration (Windows, non-fatal).
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum AutostartOutcome {
	Migrated,
	AlreadyMigrated,
	NoOldEntry,
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod autostart_win {
	use super::{AutostartOutcome, NEW_RUN_VALUE, OLD_RUN_VALUE};
	use std::path::Path;
	use windows::{
		Win32::System::Registry::{
			HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
			RegSetKeyValueW,
		},
		core::PCWSTR,
	};

	const RUN_SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
	const MAX_COMMAND: usize = 260;

	fn wide(s: &str) -> Vec<u16> {
		s.encode_utf16().chain(Some(0)).collect()
	}

	pub fn read_value(subkey: &str, value: &str) -> Result<Option<String>, String> {
		let sub_w = wide(subkey);
		let val_w = wide(value);
		let mut data = [0_u16; MAX_COMMAND + 1];
		let mut bytes = std::mem::size_of_val(&data) as u32;
		// SAFETY: names are terminated and live for the call; the buffer size is exact.
		let status = unsafe {
			RegGetValueW(
				HKEY_CURRENT_USER,
				PCWSTR(sub_w.as_ptr()),
				PCWSTR(val_w.as_ptr()),
				RRF_RT_REG_SZ,
				None,
				Some(data.as_mut_ptr().cast()),
				Some(&mut bytes),
			)
		};
		use windows::Win32::Foundation::{
			ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
		};
		if matches!(status, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
			return Ok(None);
		}
		if status != ERROR_SUCCESS {
			return Err("Could not read the Windows startup setting.".to_owned());
		}
		let units = bytes as usize / 2;
		if !bytes.is_multiple_of(2) || units == 0 || units > data.len() || data[units - 1] != 0 {
			return Err("invalid startup entry".to_owned());
		}
		String::from_utf16(&data[..units - 1])
			.map(Some)
			.map_err(|_| "invalid startup entry".to_owned())
	}

	pub fn write_value(subkey: &str, value: &str, command: &str) -> Result<(), String> {
		let sub_w = wide(subkey);
		let val_w = wide(value);
		let data: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
		// SAFETY: names and bounded REG_SZ data are terminated and live for the call.
		let status = unsafe {
			RegSetKeyValueW(
				HKEY_CURRENT_USER,
				PCWSTR(sub_w.as_ptr()),
				PCWSTR(val_w.as_ptr()),
				REG_SZ.0,
				Some(data.as_ptr().cast()),
				(data.len() * 2) as u32,
			)
		};
		use windows::Win32::Foundation::ERROR_SUCCESS;
		if status == ERROR_SUCCESS {
			Ok(())
		} else {
			Err("Could not update the Windows startup setting.".to_owned())
		}
	}

	pub fn delete_value(subkey: &str, value: &str) -> Result<(), String> {
		let sub_w = wide(subkey);
		let val_w = wide(value);
		// SAFETY: names are terminated and live for the call.
		let status = unsafe {
			RegDeleteKeyValueW(
				HKEY_CURRENT_USER,
				PCWSTR(sub_w.as_ptr()),
				PCWSTR(val_w.as_ptr()),
			)
		};
		use windows::Win32::Foundation::{
			ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
		};
		if status == ERROR_SUCCESS || matches!(status, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND)
		{
			Ok(())
		} else {
			Err("Could not update the Windows startup setting.".to_owned())
		}
	}

	fn new_command(current_exe: &Path, minimized: bool) -> Result<String, String> {
		// Same quoting/bounds as platform::startup (new names only there).
		let path = current_exe.to_str().ok_or_else(|| {
			"This executable path cannot be registered for Windows startup.".to_owned()
		})?;
		if !current_exe.is_absolute()
			|| path.ends_with(['\\', '/'])
			|| path.chars().any(|ch| ch == '"' || ch.is_control())
			|| path.encode_utf16().count() > MAX_COMMAND
		{
			return Err(
				"This executable path cannot be registered for Windows startup.".to_owned(),
			);
		}
		let suffix = if minimized { " --start-minimized" } else { "" };
		let command = format!("\"{path}\" --autostart{suffix}");
		if command.encode_utf16().count() > MAX_COMMAND {
			return Err("The executable path is too long for Windows startup.".to_owned());
		}
		Ok(command)
	}

	/// Migrate `OLD_RUN_VALUE` -> `NEW_RUN_VALUE` inside `subkey`.
	/// Idempotent: if the new value already exists, only the old is removed.
	pub fn migrate_at(subkey: &str, current_exe: &Path) -> Result<AutostartOutcome, String> {
		let old = match read_value(subkey, OLD_RUN_VALUE)? {
			None => return Ok(AutostartOutcome::NoOldEntry),
			Some(v) => v,
		};
		if read_value(subkey, NEW_RUN_VALUE)?.is_some() {
			delete_value(subkey, OLD_RUN_VALUE)?;
			return Ok(AutostartOutcome::AlreadyMigrated);
		}
		let minimized = old.contains("--start-minimized");
		let command = new_command(current_exe, minimized)?;
		write_value(subkey, NEW_RUN_VALUE, &command)?;
		// Verify before removing the source.
		match read_value(subkey, NEW_RUN_VALUE)? {
			Some(v) if v == command => {
				delete_value(subkey, OLD_RUN_VALUE)?;
				Ok(AutostartOutcome::Migrated)
			}
			_ => Err("verification read-back differs".to_owned()),
		}
	}

	pub fn migrate_current() -> Result<AutostartOutcome, String> {
		let exe = std::env::current_exe().map_err(|_| "current exe unavailable".to_owned())?;
		migrate_at(RUN_SUBKEY, &exe)
	}

	#[allow(unused_imports)]
	pub use self::{delete_value as delete_at, read_value as read_at, write_value as write_at};
}

#[cfg(target_os = "windows")]
pub use autostart_win::{migrate_at as migrate_autostart_at, migrate_current as migrate_autostart};

// ---------------------------------------------------------------------------
// Shortcut migration (Windows, non-fatal).
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum ShortcutOutcome {
	Migrated,
	AlreadyMigrated,
	NoOldEntry,
}

/// Decide + act. `create_new` actually creates the new link (PowerShell in
/// production, a temp-file write in tests). Deletes `old` only after `new`
/// exists.
pub fn migrate_shortcut(
	old: &Path,
	new: &Path,
	create_new: impl FnOnce() -> Result<(), String>,
) -> Result<ShortcutOutcome, String> {
	if new.exists() {
		if old.exists() {
			std::fs::remove_file(old).map_err(|e| e.to_string())?;
		}
		return Ok(ShortcutOutcome::AlreadyMigrated);
	}
	if !old.exists() {
		return Ok(ShortcutOutcome::NoOldEntry);
	}
	create_new()?;
	if !new.exists() {
		return Err("new shortcut missing after creation".to_owned());
	}
	std::fs::remove_file(old).map_err(|e| e.to_string())?;
	Ok(ShortcutOutcome::Migrated)
}

// ---------------------------------------------------------------------------
// Top-level migration entry point.
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum MigrationError {
	Data(DataMigrationError),
}

impl std::fmt::Display for MigrationError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Data(e) => write!(f, "{e}"),
		}
	}
}

/// Run the one-time migration. Data-dir failure is fatal (Err); keyring,
/// autostart and shortcut failures are logged and ignored (Ok) so the app can
/// still open (re-login may be needed).
pub fn migrate_all() -> Result<(), MigrationError> {
	let data = ensure_data_dir().map_err(MigrationError::Data)?;

	// Keyring: best effort, principal + per-account from the accounts table.
	// Must also run on already-migrated installs (nivra exists, per-account
	// still old): do not depend on the data-dir rename having just happened.
	{
		let store = OsStore;
		// `super::CREDENTIAL_SERVICE` is the new service after ETAPA C.
		let _ = migrate_keyring(&store, super::CREDENTIAL_SERVICE);
		// Per-account: read IDs the app knows (accounts table in the migrated
		// nivra store; if the store cannot be opened/read, skip per-account and
		// still return Ok overall since the data dir itself migrated).
		if let Ok(ids) = read_account_ids(&data) {
			for id in ids {
				let name = super::account_entry(id);
				let _ = migrate_entry_from_sources(&store, super::CREDENTIAL_SERVICE, &name);
			}
		}
	}

	#[cfg(target_os = "windows")]
	{
		let _ = migrate_autostart();
		// Shortcut: best effort via PowerShell (same script as the app's own
		// ensure step, but with the new names; old cleanup only after new exists).
		let programs = std::env::var_os("APPDATA")
			.map(PathBuf::from)
			.map(|roaming| {
				// %APPDATA%\Microsoft\Windows\Start Menu\Programs
				roaming
					.join("Microsoft")
					.join("Windows")
					.join("Start Menu")
					.join("Programs")
			});
		if let Some(dir) = programs {
			let old = dir.join(OLD_SHORTCUT_NAME);
			let new = dir.join(NEW_SHORTCUT_NAME);
			let _ = migrate_shortcut(&old, &new, || create_notification_shortcut(&new));
		}
	}

	Ok(())
}

/// Create the new notification shortcut (new names only). Used by migration and
/// by the app's own ensure step.
#[cfg(target_os = "windows")]
pub fn create_notification_shortcut(shortcut: &Path) -> Result<(), String> {
	let script = format!(
		r#"
$shortcut = '{path}'
$exe = [Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut($shortcut)
$link.TargetPath = $exe
$link.WorkingDirectory = Split-Path -Parent $exe
$link.Save()
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class {class} {{
    [StructLayout(LayoutKind.Sequential)] struct PropertyKey {{ public Guid format; public uint id; }}
    [StructLayout(LayoutKind.Explicit)] struct PropVariant {{
        [FieldOffset(0)] public ushort type;
        [FieldOffset(8)] public IntPtr value;
        [FieldOffset(16)] private IntPtr padding;
    }}
    [ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IPropertyStore {{
        void GetCount(out uint count);
        void GetAt(uint index, out PropertyKey key);
        void GetValue(ref PropertyKey key, out PropVariant value);
        void SetValue(ref PropertyKey key, ref PropVariant value);
        void Commit();
    }}
    [DllImport("shell32.dll", CharSet=CharSet.Unicode, PreserveSig=false)]
    static extern void SHGetPropertyStoreFromParsingName(string path, IntPtr bindContext, uint flags, ref Guid iid, [MarshalAs(UnmanagedType.Interface)] out IPropertyStore store);
    public static void SetAppId(string path) {{
        Guid iid = typeof(IPropertyStore).GUID;
        IPropertyStore store;
        SHGetPropertyStoreFromParsingName(path, IntPtr.Zero, 2, ref iid, out store);
        PropertyKey key = new PropertyKey {{ format = new Guid("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3"), id = 5 }};
        PropVariant value = new PropVariant {{ type = 31, value = Marshal.StringToCoTaskMemUni("{appid}") }};
        try {{ store.SetValue(ref key, ref value); store.Commit(); }}
        finally {{ Marshal.FreeCoTaskMem(value.value); Marshal.FinalReleaseComObject(store); }}
    }}
}}
'@
[{class}]::SetAppId($shortcut)
"#,
		path = shortcut.to_string_lossy().replace('\'', "''"),
		class = NEW_SHORTCUT_CLASS,
		appid = super::SERVICE,
	);
	// Absolute System32 PowerShell + CREATE_NO_WINDOW (Hidden alone still flashes).
	crate::processes::powershell_hidden()
		.args([
			"-NoProfile",
			"-NonInteractive",
			"-WindowStyle",
			"Hidden",
			"-Command",
			&script,
		])
		.output()
		.map_err(|e| e.to_string())?;
	if shortcut.exists() {
		Ok(())
	} else {
		Err("shortcut creation reported success but the link is missing".to_owned())
	}
}

/// Show a fatal migration error (native dialog when possible) for `main` to call
/// before exiting without opening.
pub fn show_fatal_error(message: &str) {
	eprintln!("Nivra migration failed: {message}");
	let _ = rfd::MessageDialog::new()
        .set_title("Nivra")
        .set_description(format!(
            "Could not migrate the old data folder:\n{message}\n\nNivra did not open so your old data stays untouched."
        ))
        .show();
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::collections::hash_map::DefaultHasher;
	use std::hash::{Hash, Hasher};

	fn temp_base(name: &str) -> PathBuf {
		let mut nonce = [0_u8; 8];
		let _ = getrandom::fill(&mut nonce);
		let mut h = DefaultHasher::new();
		std::process::id().hash(&mut h);
		nonce.hash(&mut h);
		name.hash(&mut h);
		// Mix in nanos for parallel-test uniqueness without unstable thread_id_value.
		std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map(|d| d.as_nanos())
			.unwrap_or(0)
			.hash(&mut h);
		std::env::temp_dir().join(format!("nivra-mig-test-{:016x}", h.finish()))
	}

	#[test]
	fn data_dir_moves_once_with_a_file_inside_and_is_idempotent() {
		let base = temp_base("data-once");
		let old = base.join(OLD_DATA_DIR_NAME);
		let new = base.join(NEW_DATA_DIR_NAME);
		let _ = std::fs::remove_dir_all(&base);
		std::fs::create_dir_all(&old).unwrap();
		std::fs::write(old.join("probe.txt"), b"hello").unwrap();

		migrate_data_dir(&old, &new).unwrap();
		assert!(!old.exists(), "old must be gone after rename");
		assert_eq!(std::fs::read(new.join("probe.txt")).unwrap(), b"hello");

		// Idempotent: second run with the destination present does nothing,
		// even if a new old dir appears afterwards it must not overwrite.
		migrate_data_dir(&old, &new).unwrap();
		std::fs::create_dir_all(&old).unwrap();
		std::fs::write(old.join("other.txt"), b"nope").unwrap();
		migrate_data_dir(&old, &new).unwrap();
		assert!(
			!new.join("other.txt").exists(),
			"must not touch dest once it exists"
		);
		assert_eq!(std::fs::read(new.join("probe.txt")).unwrap(), b"hello");
		let _ = std::fs::remove_dir_all(&base);
	}

	#[test]
	fn data_dir_failure_never_creates_an_empty_destination() {
		let base = temp_base("data-fail");
		let _ = std::fs::remove_dir_all(&base);
		std::fs::create_dir_all(&base).unwrap();
		// `old` is a file, `new` is nested under that file: rename must fail.
		let old = base.join(OLD_DATA_DIR_NAME);
		std::fs::write(&old, b"not-a-dir").unwrap();
		let new = old.join(NEW_DATA_DIR_NAME);
		assert!(migrate_data_dir(&old, &new).is_err());
		// The blocking file is still there and no empty dir was opened over it.
		assert!(old.is_file());
		let _ = std::fs::remove_dir_all(&base);
	}

	#[test]
	fn keyring_copies_verifies_and_only_then_deletes() {
		let store = MockStore::with_entry(OLD_CREDENTIAL_SERVICE, "discord-session", "tok-123");
		let out = migrate_one_entry(
			&store,
			OLD_CREDENTIAL_SERVICE,
			"new.service",
			"discord-session",
		)
		.unwrap();
		assert_eq!(out, CredentialOutcome::Migrated);
		assert_eq!(
			store.get("new.service", "discord-session").unwrap(),
			Some("tok-123".to_owned())
		);
		assert_eq!(
			store
				.get(OLD_CREDENTIAL_SERVICE, "discord-session")
				.unwrap(),
			None
		);

		// Idempotent: no old entry left.
		let out = migrate_one_entry(
			&store,
			OLD_CREDENTIAL_SERVICE,
			"new.service",
			"discord-session",
		)
		.unwrap();
		assert_eq!(out, CredentialOutcome::NoOldEntry);
	}

	#[test]
	fn keyring_failure_keeps_the_old_entry() {
		let mut store = MockStore::with_entry(OLD_CREDENTIAL_SERVICE, "discord-session", "tok-abc");
		store.fail_set = true;
		assert!(
			migrate_one_entry(
				&store,
				OLD_CREDENTIAL_SERVICE,
				"new.service",
				"discord-session"
			)
			.is_err()
		);
		// Source untouched, destination still empty.
		assert_eq!(
			store
				.get(OLD_CREDENTIAL_SERVICE, "discord-session")
				.unwrap(),
			Some("tok-abc".to_owned())
		);
		assert_eq!(store.get("new.service", "discord-session").unwrap(), None);
	}

	#[test]
	fn shortcut_creates_new_then_deletes_old_and_is_idempotent() {
		let base = temp_base("lnk");
		let _ = std::fs::remove_dir_all(&base);
		std::fs::create_dir_all(&base).unwrap();
		let old = base.join(OLD_SHORTCUT_NAME);
		let new = base.join(NEW_SHORTCUT_NAME);
		std::fs::write(&old, b"old").unwrap();

		let out = migrate_shortcut(&old, &new, || {
			std::fs::write(&new, b"new").map_err(|e| e.to_string())
		})
		.unwrap();
		assert_eq!(out, ShortcutOutcome::Migrated);
		assert!(!old.exists());
		assert!(new.exists());

		// Idempotent: new exists, old reappears -> only old is cleaned.
		std::fs::write(&old, b"old2").unwrap();
		let out =
			migrate_shortcut(&old, &new, || panic!("must not recreate when new exists")).unwrap();
		assert_eq!(out, ShortcutOutcome::AlreadyMigrated);
		assert!(!old.exists());
		let _ = std::fs::remove_dir_all(&base);
	}

	#[test]
	fn shortcut_failure_keeps_the_old_link() {
		let base = temp_base("lnk-fail");
		let _ = std::fs::remove_dir_all(&base);
		std::fs::create_dir_all(&base).unwrap();
		let old = base.join(OLD_SHORTCUT_NAME);
		let new = base.join(NEW_SHORTCUT_NAME);
		std::fs::write(&old, b"old").unwrap();
		assert!(migrate_shortcut(&old, &new, || Err("no powershell".to_owned())).is_err());
		assert!(old.exists());
		assert!(!new.exists());
		let _ = std::fs::remove_dir_all(&base);
	}

	#[cfg(target_os = "windows")]
	#[allow(unsafe_code)]
	#[test]
	fn autostart_migrates_and_is_idempotent_without_touching_the_real_run_key() {
		use std::sync::atomic::{AtomicU64, Ordering};
		static N: AtomicU64 = AtomicU64::new(0);
		let n = N.fetch_add(1, Ordering::Relaxed) + std::process::id() as u64;
		let subkey = format!("Software\\NivraMigTest-{n:016x}\\Run");
		// Ensure the synthetic key exists by writing then deleting the new value.
		let exe = std::path::Path::new(r"C:\Synthetic Folder\Nivra.exe");
		autostart_win::write_value(
			&subkey,
			NEW_RUN_VALUE,
			r#""C:\Synthetic Folder\Nivra.exe" --autostart"#,
		)
		.unwrap();
		autostart_win::delete_value(&subkey, NEW_RUN_VALUE).unwrap();
		// Old entry with minimized flag.
		autostart_win::write_value(
			&subkey,
			OLD_RUN_VALUE,
			r#""C:\Old\SereinExt.exe" --autostart --start-minimized"#,
		)
		.unwrap();

		let out = autostart_win::migrate_at(&subkey, exe).unwrap();
		assert_eq!(out, AutostartOutcome::Migrated);
		let new = autostart_win::read_value(&subkey, NEW_RUN_VALUE)
			.unwrap()
			.unwrap();
		assert!(new.contains("Nivra.exe"), "{new}");
		assert!(new.contains("--start-minimized"), "{new}");
		assert_eq!(
			autostart_win::read_value(&subkey, OLD_RUN_VALUE).unwrap(),
			None
		);

		// Idempotent: old reappears while new exists -> only old is cleaned.
		autostart_win::write_value(
			&subkey,
			OLD_RUN_VALUE,
			r#""C:\Old\SereinExt.exe" --autostart"#,
		)
		.unwrap();
		let out = autostart_win::migrate_at(&subkey, exe).unwrap();
		assert_eq!(out, AutostartOutcome::AlreadyMigrated);
		assert_eq!(
			autostart_win::read_value(&subkey, OLD_RUN_VALUE).unwrap(),
			None
		);

		// Cleanup synthetic key.
		{
			use windows::{
				Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteKeyW},
				core::PCWSTR,
			};
			let parent = format!("Software\\NivraMigTest-{n:016x}");
			let w: Vec<u16> = parent.encode_utf16().chain(Some(0)).collect();
			// SAFETY: owned terminated name for this test key only.
			unsafe {
				let _ = RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(w.as_ptr()));
			}
		}
	}

	#[test]
	fn per_account_migrates_from_newer_old_source_only() {
		let store = MockStore::default();
		store
			.set(OLD_CREDENTIAL_SERVICE, "discord-session.42", "tok-newer")
			.unwrap();
		let out = migrate_entry_from_sources(&store, "new.service", "discord-session.42").unwrap();
		assert_eq!(out, CredentialOutcome::Migrated);
		assert_eq!(
			store.get("new.service", "discord-session.42").unwrap(),
			Some("tok-newer".to_owned())
		);
		assert_eq!(
			store
				.get(OLD_CREDENTIAL_SERVICE, "discord-session.42")
				.unwrap(),
			None
		);
		assert_eq!(store.get(OLD_APP_ID, "discord-session.42").unwrap(), None);
	}

	#[test]
	fn per_account_migrates_from_oldest_when_newer_empty() {
		let store = MockStore::default();
		store
			.set(OLD_APP_ID, "discord-session.43", "tok-oldest")
			.unwrap();
		let out = migrate_entry_from_sources(&store, "new.service", "discord-session.43").unwrap();
		assert_eq!(out, CredentialOutcome::Migrated);
		assert_eq!(
			store.get("new.service", "discord-session.43").unwrap(),
			Some("tok-oldest".to_owned())
		);
		assert_eq!(store.get(OLD_APP_ID, "discord-session.43").unwrap(), None);
	}

	#[test]
	fn per_account_uses_newer_when_both_olds_differ_and_keeps_different() {
		let store = MockStore::default();
		store
			.set(OLD_CREDENTIAL_SERVICE, "discord-session.44", "tok-newer")
			.unwrap();
		store
			.set(OLD_APP_ID, "discord-session.44", "tok-older-different")
			.unwrap();
		let out = migrate_entry_from_sources(&store, "new.service", "discord-session.44").unwrap();
		assert_eq!(out, CredentialOutcome::Migrated);
		// Newer wins, older different kept.
		assert_eq!(
			store.get("new.service", "discord-session.44").unwrap(),
			Some("tok-newer".to_owned())
		);
		assert_eq!(
			store
				.get(OLD_CREDENTIAL_SERVICE, "discord-session.44")
				.unwrap(),
			None
		);
		assert_eq!(
			store.get(OLD_APP_ID, "discord-session.44").unwrap(),
			Some("tok-older-different".to_owned())
		);
	}

	#[test]
	fn per_account_new_existing_different_is_not_overwritten() {
		let store = MockStore::default();
		store
			.set("new.service", "discord-session.45", "tok-current")
			.unwrap();
		store
			.set(OLD_CREDENTIAL_SERVICE, "discord-session.45", "tok-stale")
			.unwrap();
		assert!(migrate_entry_from_sources(&store, "new.service", "discord-session.45").is_err());
		assert_eq!(
			store.get("new.service", "discord-session.45").unwrap(),
			Some("tok-current".to_owned())
		);
		assert_eq!(
			store
				.get(OLD_CREDENTIAL_SERVICE, "discord-session.45")
				.unwrap(),
			Some("tok-stale".to_owned())
		);
	}

	#[test]
	fn per_account_second_run_has_no_effect() {
		let store = MockStore::default();
		store
			.set(OLD_CREDENTIAL_SERVICE, "discord-session.46", "tok-once")
			.unwrap();
		let first =
			migrate_entry_from_sources(&store, "new.service", "discord-session.46").unwrap();
		assert_eq!(first, CredentialOutcome::Migrated);
		let second =
			migrate_entry_from_sources(&store, "new.service", "discord-session.46").unwrap();
		assert_eq!(second, CredentialOutcome::NoOldEntry);
		assert_eq!(
			store.get("new.service", "discord-session.46").unwrap(),
			Some("tok-once".to_owned())
		);
	}
}
