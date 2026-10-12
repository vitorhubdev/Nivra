#![allow(unsafe_code)]
//! Lowers the calling thread's CPU and I/O priority, for work that must never compete with
//! the user, such as the update check that runs when the computer starts.

/// Puts the current thread in the lowest scheduling class each OS offers to applications.
/// Failure is ignored: the work still runs, only at normal priority.
pub fn enter_background_priority() {
	#[cfg(windows)]
	{
		use windows::Win32::System::Threading::{
			GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
		};
		// SAFETY: the pseudo-handle from GetCurrentThread needs no closing and only names this thread.
		let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) };
	}
	#[cfg(target_os = "linux")]
	{
		unsafe extern "C" {
			fn setpriority(which: i32, who: u32, priority: i32) -> i32;
		}
		const PRIO_PROCESS: i32 = 0;
		// On Linux, `who = 0` with PRIO_PROCESS names the calling thread, not the whole process.
		// SAFETY: plain scheduling call on the calling thread.
		let _ = unsafe { setpriority(PRIO_PROCESS, 0, 19) };
	}
	#[cfg(target_os = "macos")]
	{
		unsafe extern "C" {
			fn pthread_set_qos_class_self_np(qos: u32, relative_priority: i32) -> i32;
		}
		const QOS_CLASS_BACKGROUND: u32 = 0x09;
		// SAFETY: sets the QoS class of the calling thread only.
		let _ = unsafe { pthread_set_qos_class_self_np(QOS_CLASS_BACKGROUND, 0) };
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn lowering_priority_never_panics() {
		super::enter_background_priority();
		super::enter_background_priority();
	}
}
