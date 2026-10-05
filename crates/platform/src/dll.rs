//! Windows DLL search hardening and read-only diagnostics.
//!
//! Two independent guards: the executable links with `/DEPENDENTLOADFLAG:0x800`
//! (import-time dependencies come from `System32` only) and startup narrows the
//! process-wide default search path. Both keep a system-named DLL planted next to
//! `Nivra.exe` — the pattern used by Online-Fix style cracks — from being loaded.
use std::path::{Path, PathBuf};

/// System DLL names that Windows searches for in the application directory first and
/// that are routinely planted next to an executable to inject into its process.
pub const SYSTEM_DLL_NAMES: &[&str] = &[
	"winmm.dll",
	"version.dll",
	"dinput8.dll",
	"winhttp.dll",
	"d3d11.dll",
	"dxgi.dll",
	"dsound.dll",
	"xinput1_3.dll",
	"xinput9_1_0.dll",
	"xinput1_4.dll",
	"msimg32.dll",
	"dbghelp.dll",
	"wininet.dll",
	"wsock32.dll",
	"wtsapi32.dll",
	"comctl32.dll",
	"uxtheme.dll",
	"opengl32.dll",
	"glu32.dll",
	"netapi32.dll",
	"secur32.dll",
	"mpr.dll",
	"cryptbase.dll",
];

/// File names in `dir` that match a system DLL the loader would search first. Pure
/// filesystem check, testable on every platform.
pub fn planted_system_dlls(dir: &Path) -> Vec<String> {
	let Ok(entries) = std::fs::read_dir(dir) else {
		return Vec::new();
	};
	let mut found = Vec::new();
	for entry in entries.flatten() {
		let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
		if SYSTEM_DLL_NAMES.contains(&name.as_str()) {
			found.push(name);
		}
	}
	found.sort();
	found
}

/// Loaded modules outside `System32`, `WinSxS` and the application directory. Pure
/// filter so a test can exercise the classification without a real process.
pub fn foreign_module_paths(
	exe_dir: &Path,
	modules: impl IntoIterator<Item = String>,
) -> Vec<String> {
	let windows = std::env::var_os("SystemRoot")
		.map(PathBuf::from)
		.unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
	// Windows paths are case-insensitive and may use either separator; normalize both so
	// the same classification runs in tests on Unix, where `Path::join` uses `/`.
	let normalize = |path: &str| path.replace('\\', "/").to_ascii_lowercase();
	let system32 = normalize(&windows.join("System32").to_string_lossy());
	let winsxs = normalize(&windows.join("WinSxS").to_string_lossy());
	let exe_dir = normalize(&exe_dir.to_string_lossy());
	// Compare on component boundaries: `C:/Apps/Nivra-old` must not be trusted just
	// because it starts with `C:/Apps/Nivra` (Codex #89 P2).
	let trusted = |path: &str, prefix: &str| {
		path == prefix
			|| path
				.strip_prefix(prefix)
				.is_some_and(|rest| rest.starts_with('/'))
	};
	let mut foreign: Vec<String> = modules
		.into_iter()
		.filter(|module| {
			let path = normalize(module);
			!trusted(&path, &system32) && !trusted(&path, &winsxs) && !trusted(&path, &exe_dir)
		})
		.collect();
	foreign.sort();
	foreign.dedup();
	foreign
}

/// Narrows the process-wide default DLL search path to `System32`. Nivra loads no
/// DLL of its own, so the application directory must not satisfy a runtime
/// `LoadLibrary` either. Safe to call more than once.
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
pub fn harden_search_path() {
	use windows::Win32::System::LibraryLoader::{
		LOAD_LIBRARY_SEARCH_SYSTEM32, SetDefaultDllDirectories,
	};
	// SAFETY: process-wide loader policy, set before any runtime LoadLibrary call.
	unsafe {
		let _ = SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32);
	}
}

/// Paths of every module currently loaded in this process. Read-only; bounded.
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
pub fn loaded_modules() -> Vec<String> {
	use windows::Win32::Foundation::HMODULE;
	use windows::Win32::System::ProcessStatus::{K32EnumProcessModules, K32GetModuleFileNameExW};
	use windows::Win32::System::Threading::GetCurrentProcess;
	const MAX_MODULES: usize = 256;
	let mut handles = [HMODULE::default(); MAX_MODULES];
	let mut needed = 0u32;
	// SAFETY: current process pseudo-handle; the array is valid for the stated size.
	let count = unsafe {
		if !K32EnumProcessModules(
			GetCurrentProcess(),
			handles.as_mut_ptr(),
			(MAX_MODULES * size_of::<HMODULE>()) as u32,
			&mut needed,
		)
		.as_bool()
		{
			return Vec::new();
		}
		(needed as usize / size_of::<HMODULE>()).min(MAX_MODULES)
	};
	let mut out = Vec::with_capacity(count);
	for handle in &handles[..count] {
		let mut buffer = [0u16; 260];
		// SAFETY: current process pseudo-handle and a buffer of the stated length.
		let len = unsafe {
			K32GetModuleFileNameExW(Some(GetCurrentProcess()), Some(*handle), &mut buffer)
		};
		if len == 0 {
			continue;
		}
		out.push(String::from_utf16_lossy(&buffer[..len as usize]));
	}
	out
}

/// One startup scan: suspicious neighbours plus foreign loaded modules, already
/// formatted as bounded, content-free log lines.
#[cfg(target_os = "windows")]
pub fn startup_report() -> (Vec<String>, Vec<String>) {
	let exe_dir = std::env::current_exe()
		.ok()
		.and_then(|exe| exe.parent().map(Path::to_path_buf))
		.unwrap_or_default();
	let planted = planted_system_dlls(&exe_dir);
	let foreign = foreign_module_paths(&exe_dir, loaded_modules());
	(planted, foreign)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn planted_system_dlls_finds_only_system_named_files() {
		let dir = std::env::temp_dir().join(format!(
			"nivra-dll-{}-{:?}",
			std::process::id(),
			std::thread::current().id()
		));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(dir.join("winmm.dll"), b"synthetic").unwrap();
		std::fs::write(dir.join("dxgi.dll"), b"synthetic").unwrap();
		std::fs::write(dir.join("Nivra.exe"), b"synthetic").unwrap();
		std::fs::write(dir.join("readme.txt"), b"synthetic").unwrap();
		assert_eq!(
			planted_system_dlls(&dir),
			vec!["dxgi.dll".to_string(), "winmm.dll".to_string()]
		);
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn foreign_module_paths_keeps_only_outside_system_and_app() {
		let exe_dir = PathBuf::from(r"C:\Apps\Nivra");
		let modules = vec![
			r"C:\Windows\System32\ntdll.dll".to_string(),
			r"C:\Windows\WinSxS\amd64_x\version.dll".to_string(),
			r"C:\Apps\Nivra\helper.dll".to_string(),
			r"C:\Users\Public\planted.dll".to_string(),
			r"C:\Users\Public\planted.dll".to_string(),
			// Sibling directories are not the trusted directory itself.
			r"C:\Windows\System32-old\hook.dll".to_string(),
			r"C:\Apps\Nivra-old\hook.dll".to_string(),
		];
		assert_eq!(
			foreign_module_paths(&exe_dir, modules),
			vec![
				r"C:\Apps\Nivra-old\hook.dll".to_string(),
				r"C:\Users\Public\planted.dll".to_string(),
				r"C:\Windows\System32-old\hook.dll".to_string(),
			]
		);
	}
}
