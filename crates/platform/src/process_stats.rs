#![allow(unsafe_code)]
//! Memory, handle count and CPU time of this process, for ultra diagnostics. Read-only: a few
//! cheap OS calls on the current process, made once per pulse. Nothing is stored or sent.

/// What the OS reports about the current process right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessStats {
	/// Memory resident in RAM, in KiB.
	pub working_set_kib: u64,
	/// Memory committed to this process (its private bytes), in KiB.
	pub private_kib: u64,
	/// Open kernel handles; a count that keeps growing is a leak.
	pub handles: u32,
	/// CPU time spent so far, kernel plus user, in milliseconds.
	pub cpu_ms: u64,
}

/// `None` where the OS offers no cheap way to read it, or when a call fails.
#[cfg(target_os = "windows")]
pub fn read() -> Option<ProcessStats> {
	use windows::Win32::Foundation::FILETIME;
	use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
	use windows::Win32::System::Threading::{
		GetCurrentProcess, GetProcessHandleCount, GetProcessTimes,
	};
	let ticks = |time: FILETIME| {
		((u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)) / 10_000
	};
	let mut memory = PROCESS_MEMORY_COUNTERS {
		cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
		..Default::default()
	};
	let (mut handles, mut created, mut exited, mut kernel, mut user) = (
		0u32,
		FILETIME::default(),
		FILETIME::default(),
		FILETIME::default(),
		FILETIME::default(),
	);
	// SAFETY: the current-process pseudo-handle needs no closing, and every out pointer refers
	// to a live local of the type the call expects, with the stated size for the counters.
	unsafe {
		let process = GetCurrentProcess();
		if !K32GetProcessMemoryInfo(process, &mut memory, memory.cb).as_bool() {
			return None;
		}
		GetProcessHandleCount(process, &mut handles).ok()?;
		GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user).ok()?;
	}
	Some(ProcessStats {
		working_set_kib: memory.WorkingSetSize as u64 / 1024,
		private_kib: memory.PagefileUsage as u64 / 1024,
		handles,
		cpu_ms: ticks(kernel) + ticks(user),
	})
}

#[cfg(not(target_os = "windows"))]
pub fn read() -> Option<ProcessStats> {
	None
}

/// How a pulse window moved the numbers: CPU as a share of one core, and growth since the last
/// window. The first window has nothing to compare with and reports no growth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Delta {
	pub cpu_percent: f32,
	pub working_set_growth_kib: i64,
	pub handle_growth: i64,
}

pub fn delta(previous: &ProcessStats, now: &ProcessStats, elapsed_ms: u64) -> Delta {
	let spent = now.cpu_ms.saturating_sub(previous.cpu_ms);
	Delta {
		cpu_percent: spent as f32 * 100.0 / elapsed_ms.max(1) as f32,
		working_set_growth_kib: now.working_set_kib as i64 - previous.working_set_kib as i64,
		handle_growth: i64::from(now.handles) - i64::from(previous.handles),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn stats(working_set_kib: u64, handles: u32, cpu_ms: u64) -> ProcessStats {
		ProcessStats {
			working_set_kib,
			private_kib: working_set_kib,
			handles,
			cpu_ms,
		}
	}

	#[test]
	fn a_window_reports_cpu_as_a_share_of_one_core_and_the_growth() {
		let delta = delta(
			&stats(100_000, 500, 1_000),
			&stats(160_000, 540, 3_500),
			5_000,
		);
		assert!((delta.cpu_percent - 50.0).abs() < 0.01, "{delta:?}");
		assert_eq!(delta.working_set_growth_kib, 60_000);
		assert_eq!(delta.handle_growth, 40);
	}

	#[test]
	fn shrinking_numbers_and_a_zero_window_do_not_panic() {
		let delta = delta(&stats(200_000, 900, 9_000), &stats(100_000, 800, 8_000), 0);
		assert_eq!(delta.working_set_growth_kib, -100_000);
		assert_eq!(delta.handle_growth, -100);
		assert!(delta.cpu_percent.is_finite());
	}

	#[cfg(target_os = "windows")]
	#[test]
	fn this_process_reports_plausible_numbers() {
		let now = read().expect("process counters");
		assert!(now.working_set_kib > 1_000, "{now:?}");
		assert!(now.handles > 5, "{now:?}");
		let later = read().expect("process counters");
		assert!(later.cpu_ms >= now.cpu_ms);
	}
}
