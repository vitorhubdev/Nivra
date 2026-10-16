//! Local frame-rate diagnostics: how many frames Nivra draws, how long each takes and which
//! lines of code asked for the repaints. It writes one bounded log line a minute, and only
//! while the window is drawing a lot, so a busy GPU leaves a trail without noise when idle.

use std::collections::HashMap;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(60);
/// Below this many frames per second the window is idle enough that nothing is logged.
const BUSY_FPS: f32 = 8.0;
/// Distinct repaint sources kept per window, so a pathological caller cannot grow the map.
const MAX_SOURCES: usize = 64;
const REPORTED_SOURCES: usize = 4;

pub struct FrameStats {
	since: Instant,
	frames: u32,
	started: Option<Instant>,
	cpu: Duration,
	slowest: Duration,
	causes: HashMap<String, u32>,
}

impl Default for FrameStats {
	fn default() -> Self {
		Self {
			since: Instant::now(),
			frames: 0,
			started: None,
			cpu: Duration::ZERO,
			slowest: Duration::ZERO,
			causes: HashMap::new(),
		}
	}
}

/// `crates\ui\src\voice.rs` to `ui/voice.rs:3436`: short enough to read in a log line.
fn source(file: &str, line: u32) -> String {
	let normalized = file.replace('\\', "/");
	let mut parts = normalized.rsplit('/');
	let name = parts.next().unwrap_or(file);
	match parts.next() {
		Some(directory) => format!("{directory}/{name}:{line}"),
		None => format!("{name}:{line}"),
	}
}

impl FrameStats {
	/// Called as a frame starts; `causes` is what asked for it.
	pub fn begin(&mut self, causes: impl IntoIterator<Item = (&'static str, u32)>) {
		self.frames = self.frames.saturating_add(1);
		self.started = Some(Instant::now());
		for (file, line) in causes {
			let key = source(file, line);
			if self.causes.len() < MAX_SOURCES || self.causes.contains_key(&key) {
				let count = self.causes.entry(key).or_default();
				*count = count.saturating_add(1);
			}
		}
	}

	/// Called as the frame ends. Returns the log line when a busy window is due its report.
	pub fn end(&mut self) -> Option<String> {
		if let Some(started) = self.started.take() {
			let took = started.elapsed();
			self.cpu += took;
			self.slowest = self.slowest.max(took);
		}
		let elapsed = self.since.elapsed();
		if elapsed < WINDOW {
			return None;
		}
		let report = self.report(elapsed);
		*self = Self::default();
		report
	}

	fn report(&self, elapsed: Duration) -> Option<String> {
		let seconds = elapsed.as_secs_f32().max(0.001);
		let fps = self.frames as f32 / seconds;
		if fps < BUSY_FPS {
			return None;
		}
		let average = self.cpu.as_secs_f32() * 1000.0 / self.frames.max(1) as f32;
		let mut causes: Vec<_> = self.causes.iter().collect();
		causes.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
		let top = causes
			.iter()
			.take(REPORTED_SOURCES)
			.map(|(source, count)| format!("{source} x{count}"))
			.collect::<Vec<_>>()
			.join(", ");
		Some(format!(
			"frames: {fps:.1}/s over {:.0}s, cpu {average:.1} ms avg {:.0} ms max; repaint from {top}",
			seconds,
			self.slowest.as_secs_f32() * 1000.0,
		))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn sources_are_short_and_use_forward_slashes() {
		assert_eq!(
			source("crates\\ui\\src\\voice.rs", 3436),
			"src/voice.rs:3436"
		);
		assert_eq!(source("voice.rs", 7), "voice.rs:7");
	}

	#[test]
	fn an_idle_window_logs_nothing_and_a_busy_one_names_the_top_sources() {
		let mut stats = FrameStats::default();
		stats.begin([("a/x.rs", 1)]);
		assert!(
			stats.report(Duration::from_secs(60)).is_none(),
			"1 frame in 60 s is idle"
		);
		for _ in 0..600 {
			stats.begin([
				("crates/ui/src/voice.rs", 3436),
				("crates/ui/src/lib.rs", 9),
			]);
		}
		let line = stats
			.report(Duration::from_secs(60))
			.expect("600 frames a minute is busy");
		assert!(line.contains("10.0/s"), "{line}");
		assert!(line.contains("src/voice.rs:3436 x600"), "{line}");
	}

	#[test]
	fn the_source_map_is_bounded() {
		let mut stats = FrameStats::default();
		for line in 0..(MAX_SOURCES as u32 * 3) {
			stats.begin([("a/x.rs", line)]);
		}
		assert!(stats.causes.len() <= MAX_SOURCES);
	}
}
