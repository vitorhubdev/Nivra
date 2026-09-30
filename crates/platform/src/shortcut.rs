//! Per-user Start Menu shortcut carrying the app AUMID, registered by the
//! executable itself on startup. No script, no administrator: unpackaged
//! Windows toasts resolve through this link.
use std::path::{Path, PathBuf};

/// Where the notification shortcut lives for the current user.
pub fn programs_link() -> Option<PathBuf> {
	std::env::var_os("APPDATA")
		.map(PathBuf::from)
		.map(|roaming| {
			roaming
				.join("Microsoft")
				.join("Windows")
				.join("Start Menu")
				.join("Programs")
				.join("Nivra.lnk")
		})
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
	/// The link did not exist and was created.
	Created,
	/// The link existed but pointed elsewhere (or lacked the AUMID) and was rewritten.
	Updated,
	/// The link already points at this executable with the AUMID set.
	Current,
}

/// Strip an extended `\\?\` prefix (and expand `\\?\UNC\`) so a stored link
/// target compares equal to `std::env::current_exe` in either form.
pub fn normalize_target(path: &Path) -> PathBuf {
	let text = path.as_os_str().to_string_lossy().replace('/', "\\");
	if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
		return PathBuf::from(format!(r"\\{rest}"));
	}
	if let Some(rest) = text.strip_prefix(r"\\?\") {
		return PathBuf::from(rest);
	}
	PathBuf::from(text)
}

/// Case-insensitive comparison: Windows paths compare without regard to case.
/// Canonical forms win when both sides exist (resolving 8.3 short names like
/// `RUNNER~1`, junctions and prefix variants); otherwise fall back to the
/// normalized text so stale targets still compare.
pub fn same_target(left: &Path, right: &Path) -> bool {
	if let (Ok(canonical_left), Ok(canonical_right)) =
		(std::fs::canonicalize(left), std::fs::canonicalize(right))
	{
		return folded(&normalize_target(&canonical_left))
			== folded(&normalize_target(&canonical_right));
	}
	folded(&normalize_target(left)) == folded(&normalize_target(right))
}
fn folded(path: &Path) -> String {
	path.as_os_str().to_string_lossy().to_lowercase()
}

/// Create the link if missing, rewrite it when it points elsewhere or lacks
/// the AUMID, and leave it untouched otherwise. `dir` is the folder holding
/// `Nivra.lnk` (the real Programs folder in production, a temp dir in tests).
#[cfg(target_os = "windows")]
pub fn ensure_link_in(dir: &Path, exe: &Path) -> Result<Outcome, String> {
	use native::{Com, read_link_app_id, read_link_target, write_link};
	let _com = Com::init()?;
	let link = dir.join("Nivra.lnk");
	let fresh = !link.is_file()
		|| read_link_target(&link).is_none_or(|target| !same_target(&target, exe))
		|| read_link_app_id(&link).is_none_or(|id| id != crate::SERVICE);
	if !fresh {
		return Ok(Outcome::Current);
	}
	let existed = link.is_file();
	write_link(&link, exe)?;
	Ok(if existed {
		Outcome::Updated
	} else {
		Outcome::Created
	})
}

/// Production entry point: the current user's Programs folder and executable.
#[cfg(target_os = "windows")]
pub fn ensure_notification_shortcut() -> Result<Outcome, String> {
	let link = programs_link().ok_or("Cannot locate the Start Menu Programs folder.")?;
	let dir = link
		.parent()
		.ok_or("Cannot locate the Start Menu Programs folder.")?;
	let exe = std::env::current_exe().map_err(|_| "Cannot locate the running executable.")?;
	ensure_link_in(dir, &exe)
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod native {
	use std::path::{Path, PathBuf};
	use windows::{
		Win32::{
			Foundation::PROPERTYKEY,
			System::Com::{
				CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
				CoTaskMemFree, CoUninitialize, IPersistFile, STGM_READ,
				StructuredStorage::{PROPVARIANT, PropVariantClear},
			},
			UI::Shell::{IShellLinkW, SHGetPathFromIDListW, ShellLink},
		},
		core::{GUID, HRESULT, HSTRING, Interface, PWSTR},
	};

	pub(super) struct Com {
		initialized: bool,
	}

	impl Com {
		pub(super) fn init() -> Result<Self, String> {
			// SAFETY: balances every success path with CoUninitialize below; a
			// foreign apartment mode fails here instead of corrupting COM state.
			let code = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
			if code.is_ok() {
				Ok(Self { initialized: true })
			} else if code == HRESULT(1) {
				Ok(Self { initialized: false })
			} else {
				Err("COM is unavailable on this thread.".to_owned())
			}
		}
	}

	impl Drop for Com {
		fn drop(&mut self) {
			if self.initialized {
				// SAFETY: paired with our own successful CoInitializeEx.
				unsafe { CoUninitialize() };
			}
		}
	}

	/// PKEY_AppUserModel_ID: {9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3}, pid 5.
	fn app_id_key() -> PROPERTYKEY {
		PROPERTYKEY {
			fmtid: GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3),
			pid: 5,
		}
	}

	fn shell_link() -> Result<IShellLinkW, String> {
		// SAFETY: in-proc shortcut object; COM is initialized by our caller.
		unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
			.map_err(|_| "Cannot create a shortcut object.".to_owned())
	}

	fn load_link(link: &Path) -> Result<IShellLinkW, String> {
		let target = shell_link()?;
		let persist: IPersistFile = target
			.cast()
			.map_err(|_| "Cannot open a shortcut.".to_owned())?;
		// SAFETY: loads only the given link file for reading.
		unsafe {
			persist
				.Load(&HSTRING::from(link.as_os_str()), STGM_READ)
				.map_err(|_| "Cannot read a shortcut.".to_owned())?;
		}
		Ok(target)
	}

	pub(super) fn read_link_target(link: &Path) -> Option<PathBuf> {
		let target = load_link(link).ok()?;
		// SAFETY: shell-allocated item list; freed below after copying the path out.
		unsafe {
			let pidl = target.GetIDList().map_err(|_| ()).ok()?;
			let mut wide = [0u16; 260];
			let ok = SHGetPathFromIDListW(pidl, &mut wide).as_bool();
			CoTaskMemFree(Some(pidl as _));
			if !ok {
				return None;
			}
			let end = wide
				.iter()
				.position(|unit| *unit == 0)
				.unwrap_or(wide.len());
			String::from_utf16(&wide[..end]).ok().map(PathBuf::from)
		}
	}

	pub(super) fn read_link_app_id(link: &Path) -> Option<String> {
		use windows::Win32::{
			System::Variant::VT_LPWSTR, UI::Shell::PropertiesSystem::IPropertyStore,
		};
		let target = load_link(link).ok()?;
		let store: IPropertyStore = target.cast().ok()?;
		// SAFETY: owned variant; only string payloads are read, then cleared.
		unsafe {
			let mut value = store.GetValue(&app_id_key()).ok()?;
			let id = if value.Anonymous.Anonymous.vt == VT_LPWSTR {
				value.Anonymous.Anonymous.Anonymous.pwszVal.to_string().ok()
			} else {
				None
			};
			let _ = PropVariantClear(&mut value);
			id
		}
	}

	fn string_variant(text: &str) -> Result<PROPVARIANT, String> {
		use windows::Win32::System::{
			Com::{
				CoTaskMemAlloc,
				StructuredStorage::{PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0},
			},
			Variant::VT_LPWSTR,
		};
		let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
		// SAFETY: exact byte count for the NUL-terminated copy; freed by PropVariantClear.
		let copy = unsafe {
			let bytes = CoTaskMemAlloc(wide.len() * 2);
			if bytes.is_null() {
				return Err("Cannot stamp a shortcut.".to_owned());
			}
			std::ptr::copy_nonoverlapping(wide.as_ptr(), bytes as *mut u16, wide.len());
			bytes as *mut u16
		};
		Ok(PROPVARIANT {
			Anonymous: PROPVARIANT_0 {
				Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
					vt: VT_LPWSTR,
					Anonymous: PROPVARIANT_0_0_0 {
						pwszVal: PWSTR(copy),
					},
					..Default::default()
				}),
			},
		})
	}

	pub(super) fn write_link(link: &Path, exe: &Path) -> Result<(), String> {
		use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
		let target = shell_link()?;
		let exe_text = super::normalize_target(exe).to_string_lossy().into_owned();
		let parent = super::normalize_target(exe)
			.parent()
			.map(Path::to_path_buf)
			.unwrap_or_else(|| PathBuf::from(""));
		// SAFETY: string parameters only; the link object owns nothing yet.
		unsafe {
			target
				.SetPath(&HSTRING::from(exe_text.as_str()))
				.map_err(|_| "Cannot point a shortcut at the executable.".to_owned())?;
			target
				.SetWorkingDirectory(&HSTRING::from(parent.to_string_lossy().as_ref()))
				.map_err(|_| "Cannot set a shortcut working directory.".to_owned())?;
			target
				.SetDescription(&HSTRING::from("Nivra notifications"))
				.map_err(|_| "Cannot describe a shortcut.".to_owned())?;
		}
		let store: IPropertyStore = target
			.cast()
			.map_err(|_| "Cannot stamp a shortcut.".to_owned())?;
		let mut value = string_variant(crate::SERVICE)?;
		// SAFETY: the variant carries a task-memory string; cleared right after.
		unsafe {
			let stamped = store.SetValue(&app_id_key(), &value).is_ok();
			let _ = PropVariantClear(&mut value);
			if !stamped {
				return Err("Cannot stamp a shortcut.".to_owned());
			}
		}
		let persist: IPersistFile = target
			.cast()
			.map_err(|_| "Cannot save a shortcut.".to_owned())?;
		// SAFETY: saves only the link file named by our caller.
		unsafe {
			persist
				.Save(&HSTRING::from(link.as_os_str()), true)
				.map_err(|_| "Cannot save a shortcut.".to_owned())?;
		}
		if link.is_file() {
			Ok(())
		} else {
			Err("Shortcut creation reported success but the link is missing.".to_owned())
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{normalize_target, same_target};
	use std::path::Path;

	#[test]
	fn normalizes_extended_prefixes_and_separators() {
		assert_eq!(
			normalize_target(Path::new(r"\\?\C:\Apps\Nivra\Nivra.exe")),
			Path::new(r"C:\Apps\Nivra\Nivra.exe")
		);
		assert_eq!(
			normalize_target(Path::new(r"\\?\UNC\host\share\Nivra.exe")),
			Path::new(r"\\host\share\Nivra.exe")
		);
		assert_eq!(
			normalize_target(Path::new("C:/Apps/Nivra/Nivra.exe")),
			Path::new(r"C:\Apps\Nivra\Nivra.exe")
		);
	}

	#[test]
	fn compares_targets_without_regard_to_case_or_prefix() {
		assert!(same_target(
			Path::new(r"c:\apps\nivra\nivra.exe"),
			Path::new(r"C:\Apps\Nivra\Nivra.exe"),
		));
		assert!(same_target(
			Path::new(r"\\?\C:\Apps\Nivra\Nivra.exe"),
			Path::new(r"C:\Apps\Nivra\Nivra.exe"),
		));
		assert!(!same_target(
			Path::new(r"C:\Apps\Nivra\Nivra.exe"),
			Path::new(r"D:\Apps\Nivra\Nivra.exe"),
		));
	}

	/// The registration is idempotent and follows the executable when it moves.
	#[cfg(target_os = "windows")]
	#[test]
	fn ensure_is_idempotent_and_updates_a_stale_target() {
		use super::native::{Com, read_link_app_id, read_link_target};
		use super::{Outcome, ensure_link_in};
		let _com = Com::init().unwrap();
		let dir = std::env::temp_dir().join(format!("nivra-shortcut-test-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		let first = dir.join("one").join("Nivra.exe");
		let second = dir.join("two").join("Nivra.exe");
		// Like the real executable, link targets exist on disk.
		for exe in [&first, &second] {
			std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
			std::fs::write(exe, b"synthetic").unwrap();
		}
		assert_eq!(ensure_link_in(&dir, &first).unwrap(), Outcome::Created);
		let link = dir.join("Nivra.lnk");
		assert!(link.is_file());
		assert!(super::same_target(
			&read_link_target(&link).unwrap(),
			&first
		));
		assert_eq!(read_link_app_id(&link).as_deref(), Some(crate::SERVICE));
		assert_eq!(ensure_link_in(&dir, &first).unwrap(), Outcome::Current);
		assert_eq!(ensure_link_in(&dir, &second).unwrap(), Outcome::Updated);
		assert!(super::same_target(
			&read_link_target(&link).unwrap(),
			&second
		));
		assert_eq!(ensure_link_in(&dir, &second).unwrap(), Outcome::Current);
		let _ = std::fs::remove_dir_all(&dir);
	}
}
