//! Ultra diagnostics pulse: every few seconds one line with what the window and the call are
//! doing, and a `WARN` for anything that looks like the problems that slipped through before
//! (a hidden window that keeps drawing, an idle UI at full speed, a call stuck while joining).
//! It only runs in ultra mode, writes to the bounded local ultra log and sends nothing anywhere.

use std::time::{Duration, Instant};

const PULSE: Duration = Duration::from_secs(5);
/// A call that stays in one setup phase this long is reported as stuck.
const STUCK_AFTER: Duration = Duration::from_secs(20);
/// A frame that takes longer than this between the start of `logic` and the end of `ui` is a
/// UI stall. Idle gaps between frames are normal and never counted.
const STALL_GAP: Duration = Duration::from_millis(250);
const HIDDEN_FPS: f32 = 20.0;
const IDLE_FPS: f32 = 40.0;
/// Process CPU above this share of one core, over a pulse, is reported.
const HIGH_CPU: f32 = 60.0;
/// Memory or handle growth inside one pulse that looks like a leak or a burst worth a look.
const GROWTH_KIB: i64 = 150 * 1024;
const GROWTH_HANDLES: i64 = 100;

/// The part of a call the pulse reports.
pub struct CallView {
	pub phase: String,
	/// Whether the phase is a settled one (connected or waiting for others).
	pub settled: bool,
	pub people: usize,
	pub live: usize,
	pub watching: usize,
	pub camera: bool,
}

pub struct Snapshot {
	pub hidden: bool,
	pub protect: bool,
	pub call: Option<CallView>,
	/// Memory, handles and CPU time of this process, where the OS gives them cheaply.
	pub process: Option<platform::process_stats::ProcessStats>,
}

/// A call phase as a fixed word, so watching it every frame never allocates.
pub fn phase_name(phase: client_core::voice::Phase) -> &'static str {
	use client_core::voice::Phase;
	match phase {
		Phase::Connecting => "Connecting",
		Phase::ConnectingTransport => "ConnectingTransport",
		Phase::Discovering => "Discovering",
		Phase::OpeningAudio => "OpeningAudio",
		Phase::Ringing => "Ringing",
		Phase::Securing => "Securing",
		Phase::Connected => "Connected",
		Phase::Waiting => "Waiting",
		Phase::Failed => "Failed",
	}
}

/// What changes the story of a session: the pulse samples every few seconds, but a phase that
/// lasts two seconds or a stream that starts and stops between two pulses must still be on record.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Observed {
	pub phase: Option<&'static str>,
	pub people: usize,
	pub live: usize,
	pub watching: usize,
	pub camera: bool,
	pub hidden: bool,
	pub protect: bool,
}

#[derive(Default)]
pub struct Timeline {
	last: Option<Observed>,
}

impl Timeline {
	/// One line per thing that changed since the previous call. The first call records a baseline
	/// and says nothing, so a run does not start with a burst of "changes".
	pub fn observe(&mut self, now: Observed) -> Vec<String> {
		let Some(before) = self.last.replace(now) else {
			return Vec::new();
		};
		let mut lines = Vec::new();
		match (before.phase, now.phase) {
			(None, Some(phase)) => lines.push(format!("call joined, phase {phase}")),
			(Some(_), None) => lines.push("call left".to_owned()),
			(Some(from), Some(to)) if from != to => {
				lines.push(format!("call phase {from} -> {to}"));
			}
			_ => {}
		}
		let mut count = |name: &str, from: usize, to: usize| {
			if from != to {
				lines.push(format!("{name} {from} -> {to}"));
			}
		};
		count("people", before.people, now.people);
		count("live streams", before.live, now.live);
		count("watching", before.watching, now.watching);
		let mut flag = |name: &str, from: bool, to: bool| {
			if from != to {
				lines.push(format!("{name} {}", if to { "on" } else { "off" }));
			}
		};
		flag("camera", before.camera, now.camera);
		flag("window hidden", before.hidden, now.hidden);
		flag("PROTECT", before.protect, now.protect);
		lines
	}
}

/// What the pulse reports about the app right now. Reads existing state only.
pub fn snapshot(state: &client_core::State, hidden: bool) -> Snapshot {
	use client_core::voice::Phase;
	let call = state.voice.active.as_ref().map(|call| CallView {
		phase: phase_name(call.phase).to_owned(),
		settled: matches!(call.phase, Phase::Connected | Phase::Waiting),
		people: call.participants.len(),
		live: call.participants.iter().filter(|p| p.streaming).count(),
		watching: call.watching.len(),
		camera: call.camera,
	});
	Snapshot {
		hidden,
		protect: protect::enabled(),
		call,
		process: platform::process_stats::read(),
	}
}

pub struct Pulse {
	window: Instant,
	frames: u32,
	started: Option<Instant>,
	stalls: u32,
	longest_gap: Duration,
	phase: Option<(String, Instant)>,
	process: Option<platform::process_stats::ProcessStats>,
}

pub struct Report {
	pub line: String,
	pub warnings: Vec<String>,
}

impl Pulse {
	pub fn new(now: Instant) -> Self {
		Self {
			window: now,
			frames: 0,
			started: None,
			stalls: 0,
			longest_gap: Duration::ZERO,
			phase: None,
			process: None,
		}
	}

	/// Marks the start of a frame, in `logic`.
	pub fn frame_started(&mut self, now: Instant) {
		self.started = Some(now);
	}

	/// Marks the end of a frame, in `ui`. A hidden window never reaches it, so its frames are
	/// never counted as slow.
	pub fn frame_finished(&mut self, now: Instant) {
		if let Some(started) = self.started.take() {
			let cost = now.saturating_duration_since(started);
			if cost > STALL_GAP {
				self.stalls = self.stalls.saturating_add(1);
				self.longest_gap = self.longest_gap.max(cost);
			}
		}
	}

	/// Called once per frame. `snapshot` runs only when a report is due, so a frame that is not
	/// the reporting one costs a few integer operations.
	pub fn tick(&mut self, now: Instant, snapshot: impl FnOnce() -> Snapshot) -> Option<Report> {
		self.frames = self.frames.saturating_add(1);
		let elapsed = now.saturating_duration_since(self.window);
		if elapsed < PULSE {
			return None;
		}
		let snapshot = snapshot();
		let seconds = elapsed.as_secs_f32().max(0.001);
		let fps = self.frames as f32 / seconds;
		let mut warnings = Vec::new();
		if snapshot.hidden && fps > HIDDEN_FPS {
			warnings.push(format!("hidden window is drawing {fps:.0} frames/s"));
		}
		if !snapshot.hidden && snapshot.call.is_none() && fps > IDLE_FPS {
			warnings.push(format!("idle UI is drawing {fps:.0} frames/s with no call"));
		}
		if self.stalls > 0 {
			warnings.push(format!(
				"{} slow frames, longest {} ms",
				self.stalls,
				self.longest_gap.as_millis()
			));
		}
		let call = match &snapshot.call {
			Some(call) => {
				let since = match &self.phase {
					Some((phase, since)) if *phase == call.phase => *since,
					_ => now,
				};
				self.phase = Some((call.phase.clone(), since));
				let stuck = now.saturating_duration_since(since);
				if !call.settled && stuck > STUCK_AFTER {
					warnings.push(format!(
						"call has been in phase {} for {} s",
						call.phase,
						stuck.as_secs()
					));
				}
				format!(
					"call={} people={} live={} watching={} camera={}",
					call.phase, call.people, call.live, call.watching, call.camera
				)
			}
			None => {
				self.phase = None;
				"call=none".to_owned()
			}
		};
		let process = match snapshot.process {
			Some(stats) => {
				let mut text = format!(
					" ram={}MB private={}MB handles={}",
					stats.working_set_kib / 1024,
					stats.private_kib / 1024,
					stats.handles
				);
				if let Some(previous) = self.process {
					let delta = platform::process_stats::delta(
						&previous,
						&stats,
						elapsed.as_millis() as u64,
					);
					text.push_str(&format!(" cpu={:.0}%", delta.cpu_percent));
					if delta.cpu_percent > HIGH_CPU {
						warnings.push(format!(
							"process CPU is at {:.0}% of one core",
							delta.cpu_percent
						));
					}
					if delta.working_set_growth_kib > GROWTH_KIB {
						warnings.push(format!(
							"memory grew {} MB in {} s",
							delta.working_set_growth_kib / 1024,
							elapsed.as_secs()
						));
					}
					if delta.handle_growth > GROWTH_HANDLES {
						warnings.push(format!(
							"handles grew by {} in {} s",
							delta.handle_growth,
							elapsed.as_secs()
						));
					}
				}
				self.process = Some(stats);
				text
			}
			None => String::new(),
		};
		let line = format!(
			"fps={fps:.1} slow={} hidden={} protect={} {call}{process}",
			self.stalls, snapshot.hidden, snapshot.protect
		);
		self.window = now;
		self.frames = 0;
		self.stalls = 0;
		self.longest_gap = Duration::ZERO;
		Some(Report { line, warnings })
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_timeline_records_each_change_once_and_stays_silent_when_nothing_changes() {
		let mut timeline = Timeline::default();
		let idle = Observed::default();
		assert!(
			timeline.observe(idle).is_empty(),
			"the first call is a baseline"
		);
		assert!(timeline.observe(idle).is_empty());
		let joining = Observed {
			phase: Some("Connecting"),
			people: 1,
			..idle
		};
		assert_eq!(
			timeline.observe(joining),
			["call joined, phase Connecting", "people 0 -> 1"]
		);
		let connected = Observed {
			phase: Some("Connected"),
			people: 4,
			live: 1,
			camera: true,
			protect: true,
			..joining
		};
		assert_eq!(
			timeline.observe(connected),
			[
				"call phase Connecting -> Connected",
				"people 1 -> 4",
				"live streams 0 -> 1",
				"camera on",
				"PROTECT on",
			]
		);
		let minimized = Observed {
			hidden: true,
			watching: 2,
			..connected
		};
		assert_eq!(
			timeline.observe(minimized),
			["watching 0 -> 2", "window hidden on"]
		);
		assert_eq!(
			timeline.observe(Observed::default()),
			[
				"call left",
				"people 4 -> 0",
				"live streams 1 -> 0",
				"watching 2 -> 0",
				"camera off",
				"window hidden off",
				"PROTECT off",
			]
		);
	}

	fn snapshot(hidden: bool, call: Option<CallView>) -> Snapshot {
		Snapshot {
			hidden,
			protect: false,
			call,
			process: None,
		}
	}

	fn call(phase: &str, settled: bool) -> CallView {
		CallView {
			phase: phase.to_owned(),
			settled,
			people: 3,
			live: 1,
			watching: 1,
			camera: false,
		}
	}

	/// Frames at `step` intervals until the pulse reports.
	fn report_after(
		pulse: &mut Pulse,
		start: Instant,
		step: Duration,
		view: impl Fn() -> Snapshot,
	) -> Report {
		let mut now = start;
		for _ in 0..10_000 {
			now += step;
			if let Some(report) = pulse.tick(now, &view) {
				return report;
			}
		}
		panic!("no report");
	}

	#[test]
	fn a_calm_visible_window_reports_without_warnings() {
		let start = Instant::now();
		let mut pulse = Pulse::new(start);
		let report = report_after(&mut pulse, start, Duration::from_millis(100), || {
			snapshot(false, Some(call("Connected", true)))
		});
		assert!(report.warnings.is_empty(), "{:?}", report.warnings);
		assert!(report.line.contains("fps=10."), "{}", report.line);
		assert!(
			report
				.line
				.contains("call=Connected people=3 live=1 watching=1")
		);
	}

	#[test]
	fn a_hidden_window_that_keeps_drawing_is_flagged() {
		let start = Instant::now();
		let mut pulse = Pulse::new(start);
		let report = report_after(&mut pulse, start, Duration::from_millis(16), || {
			snapshot(true, None)
		});
		assert!(
			report.warnings.iter().any(|w| w.contains("hidden window")),
			"{:?}",
			report.warnings
		);
	}

	#[test]
	fn an_idle_window_at_full_speed_is_flagged_but_a_call_is_not() {
		let start = Instant::now();
		let mut pulse = Pulse::new(start);
		let idle = report_after(&mut pulse, start, Duration::from_millis(8), || {
			snapshot(false, None)
		});
		assert!(idle.warnings.iter().any(|w| w.contains("idle UI")));
		let mut pulse = Pulse::new(start);
		let busy = report_after(&mut pulse, start, Duration::from_millis(8), || {
			snapshot(false, Some(call("Connected", true)))
		});
		assert!(busy.warnings.is_empty(), "{:?}", busy.warnings);
	}

	#[test]
	fn a_slow_frame_is_counted_but_idle_gaps_are_not() {
		let start = Instant::now();
		let mut pulse = Pulse::new(start);
		// Five idle seconds between two quick frames: nothing slow happened.
		pulse.frame_started(start);
		pulse.frame_finished(start + Duration::from_millis(3));
		pulse.frame_started(start + Duration::from_secs(4));
		pulse.frame_finished(start + Duration::from_secs(4) + Duration::from_millis(5));
		let calm = pulse
			.tick(start + Duration::from_secs(5), || snapshot(false, None))
			.expect("due");
		assert!(
			!calm.warnings.iter().any(|w| w.contains("slow frames")),
			"{:?}",
			calm.warnings
		);
		// One frame that blocks for 800 ms is a stall.
		pulse.frame_started(start + Duration::from_secs(6));
		pulse.frame_finished(start + Duration::from_secs(6) + Duration::from_millis(800));
		let slow = pulse
			.tick(start + Duration::from_secs(10), || snapshot(false, None))
			.expect("due");
		assert!(
			slow.warnings
				.iter()
				.any(|w| w.contains("1 slow frames, longest 800 ms")),
			"{:?}",
			slow.warnings
		);
		// A frame that never finished (hidden window) is not counted.
		pulse.frame_started(start + Duration::from_secs(11));
		let hidden = pulse
			.tick(start + Duration::from_secs(15), || snapshot(true, None))
			.expect("due");
		assert!(!hidden.warnings.iter().any(|w| w.contains("slow frames")));
	}

	#[test]
	fn a_call_stuck_in_one_setup_phase_is_flagged_after_twenty_seconds() {
		let start = Instant::now();
		let mut pulse = Pulse::new(start);
		let mut now = start;
		let mut warned = false;
		for _ in 0..8 {
			now += Duration::from_secs(5);
			let report = pulse
				.tick(now, || snapshot(false, Some(call("Securing", false))))
				.expect("a report every five seconds");
			warned |= report.warnings.iter().any(|w| w.contains("phase Securing"));
		}
		assert!(warned, "35 s in Securing must be reported");
		// A phase that changes restarts the clock.
		let mut pulse = Pulse::new(start);
		let mut now = start;
		for phase in ["Connecting", "Discovering", "Securing", "Connected"] {
			now += Duration::from_secs(5);
			let report = pulse
				.tick(now, || {
					snapshot(false, Some(call(phase, phase == "Connected")))
				})
				.unwrap();
			// One frame per pulse is itself a stall; only a phase warning is wrong here.
			assert!(
				!report.warnings.iter().any(|w| w.contains("phase")),
				"{phase}: {:?}",
				report.warnings
			);
		}
	}

	#[test]
	fn the_pulse_reports_process_numbers_and_warns_on_cpu_and_growth() {
		use platform::process_stats::ProcessStats;
		let stats = |ws_mb: u64, handles: u32, cpu_ms: u64| ProcessStats {
			working_set_kib: ws_mb * 1024,
			private_kib: ws_mb * 1024,
			handles,
			cpu_ms,
		};
		let start = Instant::now();
		let mut pulse = Pulse::new(start);
		let first = report_after(&mut pulse, start, Duration::from_millis(100), || Snapshot {
			process: Some(stats(200, 500, 1_000)),
			..snapshot(false, Some(call("Connected", true)))
		});
		assert!(
			first.line.contains(" ram=200MB private=200MB handles=500"),
			"{}",
			first.line
		);
		assert!(
			!first.line.contains(" cpu="),
			"the first window has nothing to compare with"
		);
		// The next window: 4 s of CPU in about 5 s, 300 MB and 150 handles more.
		let second = report_after(
			&mut pulse,
			start + Duration::from_secs(5),
			Duration::from_millis(100),
			|| Snapshot {
				process: Some(stats(500, 650, 5_000)),
				..snapshot(false, Some(call("Connected", true)))
			},
		);
		assert!(second.line.contains(" cpu="), "{}", second.line);
		for expected in [
			"process CPU is at",
			"memory grew 300 MB",
			"handles grew by 150",
		] {
			assert!(
				second
					.warnings
					.iter()
					.any(|warning| warning.contains(expected)),
				"{expected}: {:?}",
				second.warnings
			);
		}
	}
}
