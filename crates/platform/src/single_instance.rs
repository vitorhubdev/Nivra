#![allow(unsafe_code)]
//! Single instance guard using an exclusive lockfile with PID and active-process verification.

use std::path::{Path, PathBuf};

pub struct InstanceLock {
	path: PathBuf,
}

impl InstanceLock {
	/// Try to acquire the single-instance lock.
	/// Returns `Ok(Some(InstanceLock))` if this is the only active instance.
	/// Returns `Ok(None)` if another active instance already holds the lock.
	/// Returns `Err(...)` if the lock path cannot be accessed.
	pub fn acquire() -> std::io::Result<Option<Self>> {
		let Some(data_dir) = dirs::data_local_dir() else {
			return Ok(None);
		};
		let root = data_dir.join("nivra");
		std::fs::create_dir_all(&root)?;
		let path = root.join("instance.lock");

		Self::acquire_at(&path)
	}

	/// Internal helper allowing tests to point to a temporary path.
	pub fn acquire_at(path: &Path) -> std::io::Result<Option<Self>> {
		let current_pid = std::process::id();

		if path.exists()
			&& let Ok(content) = std::fs::read_to_string(path)
			&& let Ok(pid) = content.trim().parse::<u32>()
			&& pid != current_pid
			&& is_process_running(pid)
		{
			// Another instance is actively running
			return Ok(None);
		}

		// Write our PID into the lockfile
		std::fs::write(path, current_pid.to_string())?;
		Ok(Some(Self {
			path: path.to_path_buf(),
		}))
	}

	pub fn path(&self) -> &Path {
		&self.path
	}
}

impl Drop for InstanceLock {
	fn drop(&mut self) {
		if let Ok(content) = std::fs::read_to_string(&self.path)
			&& content.trim() == std::process::id().to_string()
		{
			let _ = std::fs::remove_file(&self.path);
		}
	}
}

#[cfg(target_os = "windows")]
fn is_process_running(pid: u32) -> bool {
	use windows::Win32::Foundation::CloseHandle;
	use windows::Win32::System::Threading::{
		GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
	};
	unsafe {
		let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
			return false;
		};
		let mut exit_code = 0u32;
		let running =
			GetExitCodeProcess(handle, &mut exit_code).is_ok() && exit_code == 259 /* STILL_ACTIVE */;
		let _ = CloseHandle(handle);
		running
	}
}

#[cfg(unix)]
fn is_process_running(pid: u32) -> bool {
	#[allow(clippy::cast_possible_wrap)]
	let res = unsafe { libc::kill(pid as libc::pid_t, 0) };
	res == 0
}

#[cfg(not(any(target_os = "windows", unix)))]
fn is_process_running(_pid: u32) -> bool {
	false
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn own_pid_is_running() {
		assert!(is_process_running(std::process::id()));
	}

	#[test]
	fn invalid_pid_is_not_running() {
		// u32::MAX is virtually guaranteed to not be an active PID
		assert!(!is_process_running(u32::MAX));
	}

	#[test]
	fn acquire_and_release_lock() {
		let dir = std::env::temp_dir().join(format!("nivra-test-lock-{}", std::process::id()));
		let _ = std::fs::create_dir_all(&dir);
		let lock_path = dir.join("test.lock");

		{
			let lock = InstanceLock::acquire_at(&lock_path).unwrap();
			assert!(lock.is_some());
			assert!(lock_path.exists());
			let content = std::fs::read_to_string(&lock_path).unwrap();
			assert_eq!(content.trim(), std::process::id().to_string());
		}

		// After drop, file should be cleaned up
		assert!(!lock_path.exists());
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn stale_lock_is_reclaimed() {
		let dir = std::env::temp_dir().join(format!("nivra-test-stale-{}", std::process::id()));
		let _ = std::fs::create_dir_all(&dir);
		let lock_path = dir.join("test.lock");

		// Write an inactive PID
		std::fs::write(&lock_path, u32::MAX.to_string()).unwrap();

		let lock = InstanceLock::acquire_at(&lock_path).unwrap();
		assert!(lock.is_some());
		let content = std::fs::read_to_string(&lock_path).unwrap();
		assert_eq!(content.trim(), std::process::id().to_string());
		drop(lock);
		let _ = std::fs::remove_dir_all(&dir);
	}
}
