#![allow(unsafe_code)]
//! Single instance guard using an OS-held lock (Named Mutex on Windows, flock on Unix)
//! combined with an informative PID lockfile.

use std::path::{Path, PathBuf};

pub struct InstanceLock {
	path: PathBuf,
	_os_lock: OsLock,
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
		#[cfg(target_os = "windows")]
		let lock_target = path.to_string_lossy().replace('\\', "/");
		#[cfg(target_os = "windows")]
		let os_lock = try_acquire_os_lock(&format!(
			"Local\\Nivra-Lock-{}",
			sanitize_mutex_name(&lock_target)
		))?;

		#[cfg(unix)]
		let os_lock = try_acquire_os_lock(path)?;

		#[cfg(not(any(target_os = "windows", unix)))]
		let os_lock = try_acquire_os_lock(path)?;

		let Some(os_lock) = os_lock else {
			return Ok(None);
		};

		// Write our PID into the lockfile for visibility/diagnostics
		let current_pid = std::process::id();
		std::fs::write(path, current_pid.to_string())?;

		Ok(Some(Self {
			path: path.to_path_buf(),
			_os_lock: os_lock,
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
fn sanitize_mutex_name(input: &str) -> String {
	input
		.chars()
		.map(|c| if c.is_alphanumeric() { c } else { '_' })
		.collect()
}

#[cfg(target_os = "windows")]
pub struct OsLock(windows::Win32::Foundation::HANDLE);

#[cfg(target_os = "windows")]
unsafe impl Send for OsLock {}
#[cfg(target_os = "windows")]
unsafe impl Sync for OsLock {}

#[cfg(target_os = "windows")]
impl Drop for OsLock {
	fn drop(&mut self) {
		unsafe {
			let _ = windows::Win32::Foundation::CloseHandle(self.0);
		}
	}
}

#[cfg(target_os = "windows")]
fn try_acquire_os_lock(name: &str) -> std::io::Result<Option<OsLock>> {
	use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
	use windows::Win32::System::Threading::CreateMutexW;
	use windows::core::HSTRING;

	let wide_name = HSTRING::from(name);
	let handle = unsafe { CreateMutexW(None, true, windows::core::PCWSTR(wide_name.as_ptr())) }
		.map_err(|e| std::io::Error::other(e.to_string()))?;
	if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
		unsafe {
			let _ = windows::Win32::Foundation::CloseHandle(handle);
		}
		return Ok(None);
	}
	Ok(Some(OsLock(handle)))
}

#[cfg(unix)]
pub struct OsLock(#[allow(dead_code)] std::fs::File);

#[cfg(unix)]
impl Drop for OsLock {
	fn drop(&mut self) {
		use std::os::unix::io::AsRawFd;
		unsafe extern "C" {
			fn flock(fd: i32, operation: i32) -> i32;
		}
		const LOCK_UN: i32 = 8;
		let fd = self.0.as_raw_fd();
		let _ = unsafe { flock(fd, LOCK_UN) };
	}
}

#[cfg(unix)]
fn try_acquire_os_lock(path: &Path) -> std::io::Result<Option<OsLock>> {
	use std::os::unix::io::AsRawFd;

	unsafe extern "C" {
		fn flock(fd: i32, operation: i32) -> i32;
	}
	const LOCK_EX: i32 = 2;
	const LOCK_NB: i32 = 4;

	let file = std::fs::OpenOptions::new()
		.read(true)
		.write(true)
		.create(true)
		.truncate(false)
		.open(path)?;

	let fd = file.as_raw_fd();
	let res = unsafe { flock(fd, LOCK_EX | LOCK_NB) };
	if res != 0 {
		return Ok(None);
	}
	Ok(Some(OsLock(file)))
}

#[cfg(not(any(target_os = "windows", unix)))]
pub struct OsLock;

#[cfg(not(any(target_os = "windows", unix)))]
fn try_acquire_os_lock(_: &Path) -> std::io::Result<Option<OsLock>> {
	Ok(Some(OsLock))
}

#[cfg(test)]
mod tests {
	use super::*;

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

			// Second attempt while lock is held must fail
			let second = InstanceLock::acquire_at(&lock_path).unwrap();
			assert!(
				second.is_none(),
				"second lock must fail while first is held"
			);
		}

		// After drop, file should be cleaned up and re-acquirable
		assert!(!lock_path.exists());
		let lock2 = InstanceLock::acquire_at(&lock_path).unwrap();
		assert!(lock2.is_some(), "must re-acquire after drop");
		drop(lock2);
		let _ = std::fs::remove_dir_all(&dir);
	}
}
