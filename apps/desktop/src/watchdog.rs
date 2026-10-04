//! Render-thread stall watchdog. One bounded report per stall, written next to the
//! local cache, so a frozen window leaves evidence instead of only a task-manager kill.
//!
//! The render thread beats once per frame with the phase it is drawing. A watchdog
//! thread notices a gap longer than [`STALL`], writes the phase, the gap and the media
//! session snapshot, and rate-limits repeats. A truly blocked thread cannot report its
//! own stack, so the report includes the watchdog's stack plus the last known phase; the
//! file never grows past [`MAX_LOG_BYTES`].
use std::{
	path::{Path, PathBuf},
	sync::{
		Mutex, OnceLock,
		atomic::{AtomicU8, AtomicU64, Ordering},
	},
	time::{Duration, Instant},
};

/// A frame gap longer than this is a stall, not a slow frame.
pub const STALL: Duration = Duration::from_secs(2);
/// Reports are capped so a permanently stuck window cannot fill the disk.
const MAX_LOG_BYTES: u64 = 64 * 1024;
/// At most one report per window; repeats inside it are counted in the next entry.
const REPORT_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Phase {
	Startup = 0,
	Idle = 1,
	Timeline = 2,
	Settings = 3,
	Voice = 4,
	Video = 5,
	Overlay = 6,
}

impl Phase {
	pub const fn label(self) -> &'static str {
		match self {
			Phase::Startup => "startup",
			Phase::Idle => "idle",
			Phase::Timeline => "timeline",
			Phase::Settings => "settings",
			Phase::Voice => "voice",
			Phase::Video => "video",
			Phase::Overlay => "overlay",
		}
	}

	fn from_u8(value: u8) -> Self {
		match value {
			1 => Phase::Idle,
			2 => Phase::Timeline,
			3 => Phase::Settings,
			4 => Phase::Voice,
			5 => Phase::Video,
			6 => Phase::Overlay,
			_ => Phase::Startup,
		}
	}
}

static BASE: OnceLock<Instant> = OnceLock::new();
static LAST_BEAT_MS: AtomicU64 = AtomicU64::new(0);
static PHASE: AtomicU8 = AtomicU8::new(Phase::Startup as u8);
static VIDEO: Mutex<String> = Mutex::new(String::new());
static INSTALLED: OnceLock<()> = OnceLock::new();
static LAST_REPORT_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
	let base = *BASE.get_or_init(Instant::now);
	base.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

/// Marks one completed render frame; call once per frame with what it drew.
pub fn beat(phase: Phase) {
	PHASE.store(phase as u8, Ordering::Relaxed);
	LAST_BEAT_MS.store(now_ms(), Ordering::Relaxed);
}

/// Bounded media-session snapshot (state, position, worker count) for the report.
pub fn note_video(snapshot: String) {
	if let Ok(mut slot) = VIDEO.lock()
		&& *slot != snapshot
	{
		slot.clear();
		slot.push_str(&snapshot.chars().take(240).collect::<String>());
	}
}

/// Starts the watchdog thread once; later calls are no-ops.
pub fn install(dir: Option<PathBuf>) {
	if INSTALLED.set(()).is_err() {
		return;
	}
	let Some(dir) = dir else {
		return;
	};
	let _ = std::thread::Builder::new()
		.name("nivra-ui-watchdog".into())
		.spawn(move || {
			loop {
				std::thread::sleep(Duration::from_millis(500));
				let now = now_ms();
				let last_report = LAST_REPORT_MS.load(Ordering::Relaxed);
				if report_if_stalled(
					now,
					LAST_BEAT_MS.load(Ordering::Relaxed),
					Phase::from_u8(PHASE.load(Ordering::Relaxed)),
					&dir.join("nivra-freeze.log"),
					last_report,
				) {
					LAST_REPORT_MS.store(now, Ordering::Relaxed);
				}
			}
		});
}

/// Writes one bounded report when the render thread missed at least one beat window.
/// Returns whether a report was written, so the caller can rate-limit.
pub fn report_if_stalled(
	now_ms: u64,
	last_beat_ms: u64,
	phase: Phase,
	log: &Path,
	last_report_ms: u64,
) -> bool {
	let gap = Duration::from_millis(now_ms.saturating_sub(last_beat_ms));
	let since_report = now_ms.saturating_sub(last_report_ms);
	// `last_report_ms == 0` means no report yet (`now_ms` counts from install).
	if gap < STALL || (last_report_ms != 0 && since_report < REPORT_INTERVAL.as_millis() as u64) {
		return false;
	}
	let video = VIDEO.lock().map(|slot| slot.clone()).unwrap_or_default();
	let stack = std::backtrace::Backtrace::force_capture();
	let entry = format!(
		"[Nivra] render thread stalled {:.1}s in phase {} (video: {})\n{stack}\n\n",
		gap.as_secs_f64(),
		phase.label(),
		if video.is_empty() { "none" } else { &video },
	);
	if let Ok(metadata) = std::fs::metadata(log)
		&& metadata.len() + entry.len() as u64 > MAX_LOG_BYTES
	{
		let _ = std::fs::remove_file(log);
	}
	if let Ok(mut file) = std::fs::OpenOptions::new()
		.create(true)
		.append(true)
		.open(log)
	{
		use std::io::Write;
		let _ = file.write_all(entry.as_bytes());
		let _ = file.flush();
	}
	true
}

#[cfg(test)]
mod tests {
	use super::*;

	/// `VIDEO` is process-wide, so the watchdog test owns it while it runs.
	static SNAPSHOT_LOCK: Mutex<()> = Mutex::new(());

	#[test]
	fn stalls_write_one_bounded_report_with_phase_and_snapshot() {
		let _turn = SNAPSHOT_LOCK
			.lock()
			.unwrap_or_else(|poison| poison.into_inner());
		let dir = std::env::temp_dir().join(format!(
			"nivra-watchdog-{}-{:?}",
			std::process::id(),
			std::thread::current().id()
		));
		let _ = std::fs::create_dir_all(&dir);
		let log = dir.join("nivra-freeze.log");
		let _ = std::fs::remove_file(&log);
		// A 1.9 s gap is a slow frame, not a stall.
		assert!(!report_if_stalled(5_000, 3_100, Phase::Video, &log, 0));
		assert!(!log.exists());
		// A 2.5 s gap writes the phase, the snapshot and a stack.
		*VIDEO.lock().unwrap() = String::new();
		assert!(report_if_stalled(5_600, 3_100, Phase::Video, &log, 0));
		let report = std::fs::read_to_string(&log).unwrap();
		assert!(report.contains("stalled 2.5s"), "{report}");
		assert!(report.contains("phase video"), "{report}");
		assert!(report.contains("video: none"), "{report}");
		// Rate-limited: a second stall inside the window is not appended.
		assert!(!report_if_stalled(6_000, 3_100, Phase::Video, &log, 5_600));
		assert_eq!(std::fs::read_to_string(&log).unwrap(), report);
		// The media snapshot is trimmed and carried into the next report.
		note_video(format!(
			"state=Playing position=1.2 workers=1 {}",
			"x".repeat(400)
		));
		assert!(report_if_stalled(40_000, 6_000, Phase::Video, &log, 5_600));
		let report = std::fs::read_to_string(&log).unwrap();
		assert!(report.contains("state=Playing"), "{report}");
		// The snapshot is trimmed to 240 chars; the stack length varies by platform,
		// so only the file cap bounds the report.
		assert!(report.contains("xxxx"), "snapshot present");
		assert!(!report.contains(&"x".repeat(400)), "snapshot trimmed");
		assert!(
			report.len() < MAX_LOG_BYTES as usize,
			"report must stay under the file cap: {}",
			report.len()
		);
		let _ = std::fs::remove_dir_all(&dir);
	}
}
