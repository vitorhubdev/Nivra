//! Read-only enumeration of this user's running executable paths for local game detection.
//! Never inspects another user's processes, memory, arguments, environment or open files.

/// Bounds both the syscall/parse work and the memory a hostile process table can force.
pub const MAX_PROCESSES: usize = 4096;
const MAX_PATH: usize = 512;

/// Executable paths of the processes visible to this user, in no particular order.
/// A partial list is normal: processes exit while the table is read.
pub fn running() -> std::io::Result<Vec<String>> {
	native::running()
}

/// `CREATE_NO_WINDOW` (0x08000000). The process is created without a console, so a
/// console app does not flash a window. `STARTF_USESHOWWINDOW` / Hidden is not enough:
/// the console exists before the child can hide it.
/// https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Every Windows spawn in `apps/` and `crates/` goes through here.
#[cfg(target_os = "windows")]
pub fn hidden_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
	use std::os::windows::process::CommandExt;
	let mut command = std::process::Command::new(program);
	command.creation_flags(CREATE_NO_WINDOW);
	command
}

/// Absolute PowerShell 5.1 with no console window (no flash).
/// Fixed `System32` path (not PATH) plus [`hidden_command`].
#[cfg(target_os = "windows")]
pub fn powershell_hidden() -> std::process::Command {
	let root = std::env::var_os("SystemRoot")
		.map(std::path::PathBuf::from)
		.unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
	hidden_command(root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
}

fn accept(path: &str, into: &mut Vec<String>) {
	let path = path.trim();
	if path.is_empty()
		|| path.len() > MAX_PATH
		|| path.chars().any(char::is_control)
		|| into.len() >= MAX_PROCESSES
	{
		return;
	}
	into.push(path.to_owned());
}

#[cfg(any(target_os = "linux", test))]
fn accept_cmdline(reader: impl std::io::Read, into: &mut Vec<String>) -> std::io::Result<()> {
	use std::io::Read;

	// One extra byte distinguishes an overlong argv[0] from an exactly bounded path.
	let mut bytes = Vec::with_capacity(MAX_PATH + 1);
	reader.take((MAX_PATH + 1) as u64).read_to_end(&mut bytes)?;
	if let Some(first) = bytes.split(|byte| *byte == 0).next()
		&& first.len() <= MAX_PATH
		&& let Ok(first) = std::str::from_utf8(first)
	{
		accept(first, into);
	}
	Ok(())
}

#[cfg(target_os = "linux")]
mod native {
	use super::{MAX_PROCESSES, accept, accept_cmdline};
	use std::fs;

	/// `/proc/<pid>/exe` is the real image; `cmdline`'s first word covers interpreted
	/// launches (and Wine/Proton, where the Windows executable only appears there).
	pub fn running() -> std::io::Result<Vec<String>> {
		let mut paths = Vec::new();
		for entry in fs::read_dir("/proc")? {
			if paths.len() >= MAX_PROCESSES {
				break;
			}
			let Ok(entry) = entry else { continue };
			let name = entry.file_name();
			let Some(name) = name.to_str() else { continue };
			if name.parse::<u32>().is_err() {
				continue;
			}
			let directory = entry.path();
			if let Ok(target) = fs::read_link(directory.join("exe"))
				&& let Some(target) = target.to_str()
			{
				accept(target, &mut paths);
			}
			// Bounded read: a command line may be megabytes, but only argv[0] is used.
			if let Ok(cmdline) = fs::File::open(directory.join("cmdline")) {
				let _ = accept_cmdline(cmdline, &mut paths);
			}
		}
		Ok(paths)
	}
}

#[cfg(target_os = "macos")]
mod native {
	use super::{MAX_PROCESSES, accept};
	use std::process::Command;

	/// `ps` reports the executable path of every process this user may see, without
	/// arguments and without the private-API entitlements a direct sysctl walk needs.
	pub fn running() -> std::io::Result<Vec<String>> {
		let output = Command::new("/bin/ps").args(["-Axo", "comm="]).output()?;
		if !output.status.success() {
			return Err(std::io::Error::other("process list is unavailable"));
		}
		let mut paths = Vec::new();
		for line in String::from_utf8_lossy(&output.stdout).lines() {
			if paths.len() >= MAX_PROCESSES {
				break;
			}
			accept(line, &mut paths);
		}
		Ok(paths)
	}
}

#[cfg(target_os = "windows")]
mod native {
	use super::{MAX_PROCESSES, accept, hidden_command};

	/// `tasklist` lists image names without opening another process' handle. Paths are
	/// unavailable this way, which is fine: detectable entries are image names on Windows.
	pub fn running() -> std::io::Result<Vec<String>> {
		let output = hidden_command("tasklist.exe")
			.args(["/nh", "/fo", "csv"])
			.output()?;
		if !output.status.success() {
			return Err(std::io::Error::other("process list is unavailable"));
		}
		let mut paths = Vec::new();
		for line in String::from_utf8_lossy(&output.stdout).lines() {
			if paths.len() >= MAX_PROCESSES {
				break;
			}
			// `"image.exe","1234","Console","1","12,345 K"`; only the quoted image name is used.
			let Some(name) = line.strip_prefix('"').and_then(|l| l.split('"').next()) else {
				continue;
			};
			accept(name, &mut paths);
		}
		Ok(paths)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn cmdline_reads_are_bounded_and_only_accept_complete_first_paths() {
		let mut command = b"/usr/bin/game\0".to_vec();
		command.extend(vec![b'x'; 1024 * 1024]);
		let mut reader = std::io::Cursor::new(command);
		let mut paths = Vec::new();
		accept_cmdline(&mut reader, &mut paths).unwrap();
		assert_eq!(reader.position(), (MAX_PATH + 1) as u64);
		assert_eq!(paths, ["/usr/bin/game"]);

		let exact = vec![b'x'; MAX_PATH];
		accept_cmdline(exact.as_slice(), &mut paths).unwrap();
		let mut terminated = exact.clone();
		terminated.push(0);
		accept_cmdline(terminated.as_slice(), &mut paths).unwrap();
		assert_eq!(paths.len(), 3);
		assert_eq!(paths[1].len(), MAX_PATH);
		assert_eq!(paths[1], paths[2]);

		let overlong = vec![b'x'; MAX_PATH + 1];
		for invalid in [overlong.as_slice(), b"\xff\0", b"\0", b""] {
			accept_cmdline(invalid, &mut paths).unwrap();
		}
		assert_eq!(paths.len(), 3);
	}

	#[test]
	fn own_process_is_listed_within_bounds() {
		let paths = running().expect("the current user's process list must be readable");
		assert!(paths.len() <= MAX_PROCESSES);
		assert!(paths.iter().all(|path| path.len() <= MAX_PATH));
		let current = std::env::current_exe().unwrap();
		let name = current
			.file_name()
			.unwrap()
			.to_string_lossy()
			.to_lowercase();
		assert!(
			paths
				.iter()
				.any(|path| path.to_lowercase().contains(name.trim_end_matches(".exe"))),
			"the test binary must appear in {paths:?}"
		);
	}

	#[test]
	fn unbounded_and_control_character_paths_are_dropped() {
		let mut paths = Vec::new();
		accept("", &mut paths);
		accept("  ", &mut paths);
		accept("/usr/bin/game\u{7}", &mut paths);
		accept(&"x".repeat(MAX_PATH + 1), &mut paths);
		assert!(paths.is_empty());
		accept("  /usr/bin/game  ", &mut paths);
		assert_eq!(paths, ["/usr/bin/game"]);
		let mut full = vec![String::new(); MAX_PROCESSES];
		accept("/usr/bin/game", &mut full);
		assert_eq!(full.len(), MAX_PROCESSES);
	}

	#[test]
	fn windows_spawns_use_the_hidden_command_helper() {
		assert_eq!(CREATE_NO_WINDOW, 0x0800_0000);
		let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
		let mut offenders = Vec::new();
		for relative in ["apps/desktop/src", "crates"] {
			let directory = root.join(relative);
			for path in walkdir(&directory) {
				if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
					continue;
				}
				let text = std::fs::read_to_string(&path).unwrap();
				if let Some(hit) = windows_command_new(&text) {
					offenders.push(format!(
						"{}: {hit}",
						path.strip_prefix(&root).unwrap_or(&path).display()
					));
				}
			}
		}
		assert!(
			offenders.is_empty(),
			"a Windows process spawn must go through hidden_command:\n{}",
			offenders.join("\n")
		);
	}
}

#[cfg(test)]
fn walkdir(directory: &std::path::Path) -> Vec<std::path::PathBuf> {
	let mut files = Vec::new();
	let Ok(entries) = std::fs::read_dir(directory) else {
		return files;
	};
	for entry in entries.flatten() {
		let path = entry.path();
		if path.is_dir() {
			let name = path
				.file_name()
				.and_then(|name| name.to_str())
				.unwrap_or("");
			if name == "target" || name == "examples" {
				continue;
			}
			files.extend(walkdir(&path));
		} else {
			files.push(path);
		}
	}
	files
}

/// `Command::new` compiled into the Windows app, outside `hidden_command`.
#[cfg(test)]
fn windows_command_new(source: &str) -> Option<String> {
	for line in source.lines() {
		let trimmed = line.trim();
		if trimmed.starts_with("#!") {
			let as_outer = trimmed.replacen("#!", "#", 1);
			if !cfg_runs_on_windows(&as_outer) {
				return None;
			}
		}
		if !trimmed.is_empty() && !trimmed.starts_with("#!") && !trimmed.starts_with("//") {
			break;
		}
	}
	let mut brace = 0_i32;
	let mut skip_until: Option<i32> = None;
	let mut pending_cfg: Vec<String> = Vec::new();
	let mut function = String::new();
	let mut function_brace: Option<i32> = None;
	for (index, line) in source.lines().enumerate() {
		let trimmed = line.trim();
		if trimmed.starts_with("//") || trimmed.is_empty() {
			continue;
		}
		if trimmed.starts_with("#[") {
			pending_cfg.push(trimmed.to_owned());
			continue;
		}
		if skip_until.is_none() && !pending_cfg.is_empty() {
			let capable = pending_cfg.iter().all(|attr| cfg_runs_on_windows(attr));
			pending_cfg.clear();
			if !capable {
				skip_until = Some(brace);
			}
		} else if !trimmed.starts_with('#') {
			pending_cfg.clear();
		}
		if let Some(name) = trimmed
			.strip_prefix("pub fn ")
			.or_else(|| trimmed.strip_prefix("fn "))
			.or_else(|| trimmed.strip_prefix("pub(super) fn "))
			.and_then(|rest| rest.split(['(', '<', ' ']).next())
		{
			function = name.to_owned();
			function_brace = Some(brace);
		}
		if skip_until.is_none()
			&& trimmed.contains("Command::new(")
			&& function != "hidden_command"
			&& function != "windows_command_new"
		{
			return Some(format!("line {}: {trimmed}", index + 1));
		}
		brace += trimmed.matches('{').count() as i32;
		brace -= trimmed.matches('}').count() as i32;
		if let Some(start) = skip_until
			&& brace <= start
		{
			skip_until = None;
		}
		if let Some(start) = function_brace
			&& brace <= start
		{
			function.clear();
			function_brace = None;
		}
	}
	None
}

#[cfg(test)]
fn cfg_runs_on_windows(attr: &str) -> bool {
	let Some(inner) = attr
		.trim()
		.strip_prefix("#[cfg(")
		.and_then(|rest| rest.strip_suffix(")]"))
	else {
		return true;
	};
	let inner = inner.replace(' ', "");
	if inner.starts_with("not(windows)") || inner.starts_with("not(target_os=\"windows\")") {
		return false;
	}
	if inner.contains("not(any(") && inner.contains("windows") {
		return false;
	}
	let names_windows = inner.contains("windows") || inner.contains("target_os=\"windows\"");
	let names_other = inner.contains("unix")
		|| inner.contains("target_os=\"linux\"")
		|| inner.contains("target_os=\"macos\"");
	if names_other && !names_windows {
		return false;
	}
	true
}
