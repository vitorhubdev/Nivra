#![allow(unsafe_code)]
//! Single instance guard using an OS-held lock (Named Mutex on Windows, flock on Unix)
//! combined with an informative PID lockfile.

use std::{
	io::{BufRead, BufReader, Read, Write},
	net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
	path::{Path, PathBuf},
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	},
	time::{Duration, Instant},
};

/// How long a newer launch waits for the previous instance to release the lock.
const TAKEOVER_WAIT: Duration = Duration::from_secs(15);
static TAKEOVER: AtomicBool = AtomicBool::new(false);
static TAKEOVER_WAKE: std::sync::OnceLock<Arc<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

pub struct InstanceLock {
	path: PathBuf,
	_os_lock: OsLock,
}

/// Set when a newer Nivra asked this instance to close. Reading it clears the request.
pub fn takeover_requested() -> bool {
	TAKEOVER.swap(false, Ordering::AcqRel)
}

/// Wakes the UI when a newer launch asks this instance to close, even if its window is hidden.
pub fn set_takeover_wake(wake: Arc<dyn Fn() + Send + Sync>) {
	let _ = TAKEOVER_WAKE.set(wake);
}

fn lock_path() -> std::io::Result<PathBuf> {
	let Some(data_dir) = dirs::data_local_dir() else {
		return Err(std::io::Error::new(
			std::io::ErrorKind::NotFound,
			"no local data directory for the instance lock",
		));
	};
	let root = data_dir.join("nivra");
	std::fs::create_dir_all(&root)?;
	Ok(root.join("instance.lock"))
}

impl InstanceLock {
	/// Try to acquire the single-instance lock.
	/// Returns `Ok(Some(InstanceLock))` if this is the only active instance.
	/// Returns `Ok(None)` if another active instance already holds the lock.
	/// Returns `Err(...)` if the lock path cannot be accessed, including when
	/// no base directory exists at all (never misreported as already-running).
	pub fn acquire() -> std::io::Result<Option<Self>> {
		Self::acquire_at(&lock_path()?)
	}

	/// Acquires the lock, asking a running instance to close when `take_over` is set.
	/// Returns `Ok(None)` when another instance keeps the lock and no takeover was allowed,
	/// or when the previous instance did not leave within `TAKEOVER_WAIT`.
	pub fn acquire_or_take_over(take_over: bool) -> std::io::Result<Option<Self>> {
		let path = lock_path()?;
		if let Some(lock) = Self::acquire_at(&path)? {
			return Ok(Some(lock));
		}
		if !take_over {
			return Ok(None);
		}
		let control = read_control(&path);
		let asked = control
			.as_ref()
			.is_some_and(|control| request_close(control).unwrap_or(false));
		if !asked {
			// Builds before 1.0.15 have no control channel. Stop only a process whose
			// executable is a Nivra build, never an arbitrary process that reused the PID.
			if let Some(pid) = control.map(|c| c.pid).or_else(|| read_pid(&path)) {
				let _ = terminate_nivra_pid(pid);
			}
		}
		let deadline = Instant::now() + TAKEOVER_WAIT;
		while Instant::now() < deadline {
			if let Some(lock) = Self::acquire_at(&path)? {
				return Ok(Some(lock));
			}
			std::thread::sleep(Duration::from_millis(100));
		}
		Ok(None)
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
			&& content.lines().next().map(str::trim)
				== Some(std::process::id().to_string().as_str())
		{
			let _ = std::fs::remove_file(&self.path);
		}
	}
}

/// Starts the local listener that lets a newer launch ask this instance to close.
/// The port and a random token go into the lock file, which only this user can read.
pub fn serve_takeover(lock: &InstanceLock) -> std::io::Result<()> {
	let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
	let port = listener.local_addr()?.port();
	let mut random = [0u8; 16];
	getrandom::fill(&mut random).map_err(|error| std::io::Error::other(error.to_string()))?;
	let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
	std::fs::write(
		&lock.path,
		format!("{}\n{port}\n{token}\n", std::process::id()),
	)?;
	std::thread::Builder::new()
		.name("nivra-takeover".into())
		.spawn(move || {
			for stream in listener.incoming().flatten() {
				handle_takeover(stream, &token);
			}
		})
		.map(|_| ())
}

fn handle_takeover(mut stream: TcpStream, token: &str) {
	let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
	let mut line = String::new();
	if BufReader::new(&stream)
		.take(256)
		.read_line(&mut line)
		.is_err()
	{
		return;
	}
	if line.trim_end() == format!("takeover {token}") {
		TAKEOVER.store(true, Ordering::Release);
		if let Some(wake) = TAKEOVER_WAKE.get() {
			wake();
		}
		let _ = stream.write_all(b"ok\n");
	}
}

struct Control {
	pid: u32,
	port: u16,
	token: String,
}

fn read_pid(path: &Path) -> Option<u32> {
	std::fs::read_to_string(path)
		.ok()?
		.lines()
		.next()?
		.trim()
		.parse()
		.ok()
}

fn read_control(path: &Path) -> Option<Control> {
	let content = std::fs::read_to_string(path).ok()?;
	let mut lines = content.lines();
	let pid = lines.next()?.trim().parse().ok()?;
	let port = lines.next()?.trim().parse().ok()?;
	let token = lines.next()?.trim().to_owned();
	(token.len() == 32).then_some(Control { pid, port, token })
}

/// Sends the takeover request; `Ok(true)` when the running instance acknowledged it.
fn request_close(control: &Control) -> std::io::Result<bool> {
	let address = SocketAddr::from((Ipv4Addr::LOCALHOST, control.port));
	let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(1))?;
	stream.set_read_timeout(Some(Duration::from_secs(2)))?;
	stream.write_all(format!("takeover {}\n", control.token).as_bytes())?;
	let mut reply = String::new();
	BufReader::new(stream).read_line(&mut reply)?;
	Ok(reply.trim() == "ok")
}

#[cfg(windows)]
fn terminate_nivra_pid(pid: u32) -> std::io::Result<()> {
	use windows::Win32::Foundation::CloseHandle;
	use windows::Win32::System::Threading::{
		OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
		QueryFullProcessImageNameW, TerminateProcess,
	};
	use windows::core::PWSTR;
	if pid == std::process::id() {
		return Ok(());
	}
	// SAFETY: the handle is closed on every path; the buffer length is passed in and bounds the write.
	unsafe {
		let handle = OpenProcess(
			PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE,
			false,
			pid,
		)
		.map_err(|error| std::io::Error::other(error.to_string()))?;
		let mut buffer = [0u16; 1024];
		let mut length = buffer.len() as u32;
		let named = QueryFullProcessImageNameW(
			handle,
			PROCESS_NAME_WIN32,
			PWSTR(buffer.as_mut_ptr()),
			&mut length,
		)
		.is_ok();
		let image = String::from_utf16_lossy(&buffer[..length as usize]);
		let file = image
			.rsplit(['\\', '/'])
			.next()
			.unwrap_or("")
			.to_ascii_lowercase();
		let result = if named && file.starts_with("nivra") && file.ends_with(".exe") {
			TerminateProcess(handle, 0).map_err(|error| std::io::Error::other(error.to_string()))
		} else {
			Ok(())
		};
		let _ = CloseHandle(handle);
		result
	}
}

#[cfg(not(windows))]
fn terminate_nivra_pid(pid: u32) -> std::io::Result<()> {
	let exe = std::fs::read_link(format!("/proc/{pid}/exe")).unwrap_or_default();
	let nivra = exe
		.file_name()
		.and_then(|name| name.to_str())
		.is_some_and(|name| name.to_ascii_lowercase().starts_with("nivra"));
	if nivra && pid != std::process::id() {
		unsafe extern "C" {
			fn kill(pid: i32, signal: i32) -> i32;
		}
		const SIGTERM: i32 = 15;
		// SAFETY: plain signal delivery to a PID verified above as a Nivra executable.
		let _ = unsafe { kill(pid as i32, SIGTERM) };
	}
	Ok(())
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

	#[test]
	fn concurrent_acquisitions_leave_exactly_one_winner() {
		use std::sync::{Arc, Barrier};
		let dir = std::env::temp_dir().join(format!("nivra-test-race-{}", std::process::id()));
		let _ = std::fs::create_dir_all(&dir);
		let lock_path = dir.join("race.lock");
		const RACERS: usize = 8;
		let barrier = Arc::new(Barrier::new(RACERS));
		let racers: Vec<_> = (0..RACERS)
			.map(|_| {
				let barrier = barrier.clone();
				let path = lock_path.clone();
				// Return the lock itself so the winner stays held until every
				// racer finished: dropping early would let a latecomer win too.
				std::thread::spawn(move || {
					barrier.wait();
					InstanceLock::acquire_at(&path).unwrap()
				})
			})
			.collect();
		let mut held = Vec::new();
		for racer in racers {
			if let Some(lock) = racer.join().unwrap() {
				held.push(lock);
			}
		}
		assert_eq!(held.len(), 1, "exactly one racer must hold the OS lock");
		drop(held);
		// Dropping all handles releases the OS lock; a later launch must succeed.
		assert!(InstanceLock::acquire_at(&lock_path).unwrap().is_some());
		let _ = std::fs::remove_dir_all(&dir);
	}
}

#[cfg(test)]
mod takeover_tests {
	use super::*;

	#[test]
	fn a_newer_launch_is_acknowledged_and_flags_the_running_instance() {
		let dir = std::env::temp_dir().join(format!("nivra-test-takeover-{}", std::process::id()));
		let _ = std::fs::create_dir_all(&dir);
		let path = dir.join("takeover.lock");
		let lock = InstanceLock::acquire_at(&path).unwrap().unwrap();
		serve_takeover(&lock).unwrap();
		let control = read_control(&path).expect("control channel is published");
		assert!(
			request_close(&control).unwrap(),
			"the running instance acknowledges"
		);
		assert!(takeover_requested());
		assert!(!takeover_requested(), "a request is consumed once");
		let forged = Control {
			pid: control.pid,
			port: control.port,
			token: "0".repeat(32),
		};
		assert!(!request_close(&forged).unwrap(), "a wrong token is refused");
		assert!(!takeover_requested());
		drop(lock);
		let _ = std::fs::remove_dir_all(&dir);
	}
}
