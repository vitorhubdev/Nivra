//! Turns a `nivra-ultra.log` into a short written analysis, so an exported report starts with
//! what stood out instead of tens of thousands of lines. It only reads text that is already in
//! the bounded, redacted log and writes text; nothing here touches the network or the disk.

use std::collections::BTreeMap;

/// Events kept in the written timeline; the full list stays in the log itself.
const MAX_EVENTS: usize = 60;
const BUSY_FPS: f64 = 60.0;
const SLOW_PING_MS: f64 = 300.0;

struct Line<'a> {
	time: &'a str,
	scope: &'a str,
	rest: &'a str,
}

fn parse(line: &str) -> Option<Line<'_>> {
	// "2026-10-16 17:40:21 scope message": the date, the time, then the scope.
	let mut parts = line.splitn(4, ' ');
	let date = parts.next()?;
	let time = parts.next()?;
	let scope = parts.next()?;
	let rest = parts.next().unwrap_or("");
	if date.len() != 10 || time.len() != 8 {
		return None;
	}
	Some(Line { time, scope, rest })
}

/// The number after ` key=` in a line, if there is one.
fn num(text: &str, key: &str) -> Option<f64> {
	let marker = format!(" {key}=");
	let start = text.find(&marker)? + marker.len();
	let digits: String = text[start..]
		.chars()
		.take_while(|c| c.is_ascii_digit() || *c == '.')
		.collect();
	digits.parse().ok()
}

/// The first number of ` key=[calls, total, max]`.
fn first_of(text: &str, key: &str) -> Option<f64> {
	let marker = format!(" {key}=[");
	let start = text.find(&marker)? + marker.len();
	let digits: String = text[start..]
		.chars()
		.take_while(char::is_ascii_digit)
		.collect();
	digits.parse().ok()
}

/// A warning with its numbers replaced by `N`, so "22 frames/s" and "31 frames/s" count together.
fn generalize(text: &str) -> String {
	let mut out = String::new();
	let mut in_number = false;
	for c in text.chars() {
		if c.is_ascii_digit() {
			if !in_number {
				out.push('N');
			}
			in_number = true;
		} else {
			in_number = false;
			out.push(c);
		}
	}
	out
}

fn percentile(sorted: &[f64], percent: usize) -> f64 {
	if sorted.is_empty() {
		return 0.0;
	}
	sorted[(sorted.len() - 1) * percent / 100]
}

fn sorted(mut values: Vec<f64>) -> Vec<f64> {
	values.sort_by(f64::total_cmp);
	values
}

#[derive(Default)]
struct Audio {
	windows: usize,
	with_packets: usize,
	/// Windows where packets arrived and not one reached the mixer.
	heard_nothing: usize,
	reasons: BTreeMap<&'static str, u64>,
	dave_not_ready: usize,
	dave_no_members: usize,
	first_silent: Option<String>,
	last_silent: Option<String>,
}

/// What the pulses say about this process: CPU, memory and handles.
#[derive(Default)]
struct Process {
	ram_mb: Vec<f64>,
	handles: Vec<f64>,
	cpu: Vec<f64>,
}

impl Process {
	fn observe(&mut self, text: &str) {
		if let Some(ram) = num(text, "ram") {
			self.ram_mb.push(ram);
		}
		if let Some(handles) = num(text, "handles") {
			self.handles.push(handles);
		}
		if let Some(cpu) = num(text, "cpu") {
			self.cpu.push(cpu);
		}
	}

	fn findings(&self) -> Vec<String> {
		let mut out = Vec::new();
		let (Some(first), Some(last)) = (self.ram_mb.first(), self.ram_mb.last()) else {
			return out;
		};
		if last - first > 300.0 {
			out.push(format!(
				"MEMORY GROWTH: the app went from {first:.0} MB to {last:.0} MB resident over this log."
			));
		}
		if let (Some(first), Some(last)) = (self.handles.first(), self.handles.last())
			&& last - first > 300.0
		{
			out.push(format!(
				"HANDLE GROWTH: open handles went from {first:.0} to {last:.0}, which usually means something is not being closed."
			));
		}
		let hot = self.cpu.iter().filter(|cpu| **cpu > 60.0).count();
		if hot > 0 {
			out.push(format!(
				"HIGH PROCESS CPU: {hot} of {} pulses used more than 60% of one core (max {:.0}%).",
				self.cpu.len(),
				self.cpu.iter().copied().fold(0.0, f64::max)
			));
		}
		out
	}

	fn summary(&self) -> String {
		let (Some(first), Some(last)) = (self.ram_mb.first(), self.ram_mb.last()) else {
			return String::new();
		};
		let ram = sorted(self.ram_mb.clone());
		let cpu = sorted(self.cpu.clone());
		format!(
			"Process over {} pulses: resident {first:.0} -> {last:.0} MB (max {:.0}), handles {:.0} -> {:.0}, CPU median {:.0}% / 95th {:.0}% / max {:.0}% of one core\n",
			ram.len(),
			ram.last().copied().unwrap_or(0.0),
			self.handles.first().copied().unwrap_or(0.0),
			self.handles.last().copied().unwrap_or(0.0),
			percentile(&cpu, 50),
			percentile(&cpu, 95),
			cpu.last().copied().unwrap_or(0.0)
		)
	}
}

/// Receive errors on the media sockets, as the transport reports them.
#[derive(Default)]
struct Media {
	tolerated: u64,
	kinds: BTreeMap<String, u64>,
	fatal: Vec<String>,
}

impl Media {
	fn observe(&mut self, time: &str, rest: &str) {
		if let Some((_, tail)) = rest.split_once(": ")
			&& rest.contains(" tolerated receive error(s)")
		{
			let count = tail
				.split_whitespace()
				.next()
				.and_then(|value| value.parse::<u64>().ok())
				.unwrap_or(1);
			self.tolerated += count;
			if let Some(kind) = tail
				.split("kind=")
				.nth(1)
				.and_then(|k| k.split_whitespace().next())
			{
				*self.kinds.entry(kind.to_owned()).or_default() += count;
			}
		} else if rest.contains(" receive failed: ") && self.fatal.len() < 5 {
			self.fatal.push(format!("{time} {rest}"));
		}
	}

	fn findings(&self) -> Vec<String> {
		let mut out = Vec::new();
		for failure in &self.fatal {
			out.push(format!("MEDIA SOCKET FAILED: {failure}."));
		}
		if self.tolerated > 0 {
			let kinds = self
				.kinds
				.iter()
				.map(|(kind, count)| format!("{kind} x{count}"))
				.collect::<Vec<_>>()
				.join(", ");
			out.push(format!(
				"MEDIA SOCKET ERRORS TOLERATED: {} receive errors were skipped ({kinds}). A burst of resets points at a route that refuses the media port.",
				self.tolerated
			));
		}
		out
	}
}

/// What the `protect` lines say about the route and the in-process tunnel.
#[derive(Default)]
struct Tunnel {
	summaries: usize,
	/// Summaries where datagrams left and nothing came back.
	one_way: usize,
	first_one_way: Option<String>,
	decrypt_errors: u64,
	errors: BTreeMap<String, u64>,
	inbound_full: u64,
	downlink_dropped: u64,
	uplink_blocked: u64,
	uplink_dropped: u64,
	tcp_aborted: u64,
	send_failed: u64,
	worst_loss: f64,
	route_changes: usize,
	last_route_change: Option<String>,
	datagrams_ended: usize,
	first_datagram_end: Option<String>,
	stopped: Option<String>,
	association_ends: BTreeMap<String, usize>,
	canary_ok: usize,
	canary_failed: usize,
	canary_worst_ms: f64,
	first_canary_failure: Option<String>,
	refused_lines: usize,
	connect_failures: BTreeMap<String, usize>,
	relays_failed: usize,
	first_relay_failure: Option<String>,
}

impl Tunnel {
	fn observe(&mut self, time: &str, rest: &str) {
		let text = format!(" {rest}");
		if rest.starts_with("tunnel: ") {
			self.summaries += 1;
			let received = num(&text, "received").unwrap_or(0.0);
			if num(&text, "sent").unwrap_or(0.0) > 0.0 && received == 0.0 {
				self.one_way += 1;
				self.first_one_way.get_or_insert(time.to_owned());
			}
			let count = |key| num(&text, key).unwrap_or(0.0) as u64;
			self.decrypt_errors += count("decrypt_errors");
			self.inbound_full += count("inbound_queue_full");
			self.downlink_dropped += count("downlink_dropped");
			self.uplink_blocked += count("uplink_blocked");
			self.tcp_aborted += count("tcp_aborted");
			self.send_failed += count("send_failed");
			self.worst_loss = self.worst_loss.max(num(&text, "loss").unwrap_or(0.0));
			if let Some(error) = text
				.split(" last_error=")
				.nth(1)
				.and_then(|tail| tail.split_whitespace().next())
				.filter(|error| *error != "none")
			{
				*self.errors.entry(error.to_owned()).or_default() += 1;
			}
		} else if rest.starts_with("route applied") {
			self.route_changes += 1;
			self.last_route_change = Some(time.to_owned());
		} else if rest.starts_with("datagram ended") {
			self.datagrams_ended += 1;
			self.first_datagram_end.get_or_insert(time.to_owned());
		} else if rest.starts_with("tunnel stopped") {
			self.stopped.get_or_insert(time.to_owned());
		} else if let Some(tail) = rest.strip_prefix("canary: dns ok in ") {
			self.canary_ok += 1;
			let ms = tail
				.split_whitespace()
				.next()
				.and_then(|value| value.parse::<f64>().ok())
				.unwrap_or(0.0);
			self.canary_worst_ms = self.canary_worst_ms.max(ms);
		} else if rest.starts_with("canary: dns failed") {
			self.canary_failed += 1;
			self.first_canary_failure.get_or_insert(time.to_owned());
		} else if rest.starts_with("inlet refused a client") {
			self.refused_lines += 1;
		} else if rest.starts_with("connect to port ") || rest.starts_with("name lookup failed") {
			// "connect to port 443 failed: <reason>" and "name lookup failed: <reason>".
			let reason = rest.rsplit(": ").next().unwrap_or(rest).to_owned();
			*self.connect_failures.entry(reason).or_default() += 1;
		} else if rest.starts_with("relay to port ") && rest.contains(" failed after ") {
			self.relays_failed += 1;
			self.first_relay_failure.get_or_insert(time.to_owned());
		} else if let Some(tail) = rest.strip_prefix("udp association ended: ") {
			let reason = tail.split(';').next().unwrap_or("").to_owned();
			*self.association_ends.entry(reason).or_default() += 1;
			self.uplink_dropped += num(&text, "uplink_dropped").unwrap_or(0.0) as u64;
		}
	}

	fn findings(&self) -> Vec<String> {
		let mut out = Vec::new();
		if self.datagrams_ended > 0 {
			out.push(format!(
				"ROUTE CHANGED UNDER A LIVE SOCKET: {} media socket(s) opened on an older PROTECT route were closed, first at {} ({} route changes in this log, the last at {}). A call on that socket fails and rejoins.",
				self.datagrams_ended,
				self.first_datagram_end.as_deref().unwrap_or("-"),
				self.route_changes,
				self.last_route_change.as_deref().unwrap_or("-")
			));
		}
		if let Some(time) = &self.stopped {
			out.push(format!(
				"TUNNEL STOPPED at {time}: the in-process WireGuard engine ended, so everything routed through it failed at once."
			));
		}
		if self.one_way > 0 {
			out.push(format!(
				"TUNNEL ONE-WAY: in {} of {} five-second summaries datagrams were sent and none came back, first at {}. That is a dead or blocked endpoint, not a busy one.",
				self.one_way,
				self.summaries,
				self.first_one_way.as_deref().unwrap_or("-")
			));
		}
		let lost = self.decrypt_errors
			+ self.inbound_full
			+ self.downlink_dropped
			+ self.uplink_blocked
			+ self.uplink_dropped
			+ self.tcp_aborted
			+ self.send_failed;
		if lost > 0 || self.worst_loss > 0.2 {
			let errors = self
				.errors
				.iter()
				.map(|(error, windows)| format!("{error} x{windows}"))
				.collect::<Vec<_>>()
				.join(", ");
			out.push(format!(
				"TUNNEL LOSS: decrypt_errors={} inbound_queue_full={} downlink_dropped={} uplink_dropped={} uplink_blocked={} send_failed={} tcp_aborted={} worst_estimated_loss={:.2}{}.",
				self.decrypt_errors,
				self.inbound_full,
				self.downlink_dropped,
				self.uplink_dropped,
				self.uplink_blocked,
				self.send_failed,
				self.tcp_aborted,
				self.worst_loss,
				if errors.is_empty() {
					String::new()
				} else {
					format!("; decrypt errors seen: {errors}")
				}
			));
		}
		if self.canary_failed > 0 {
			out.push(format!(
				"TUNNEL CANARY FAILED: {} of {} name lookups sent through the tunnel got no answer, first at {} (slowest answer {:.0} ms). When a call dies near that time the tunnel was already dead.",
				self.canary_failed,
				self.canary_failed + self.canary_ok,
				self.first_canary_failure.as_deref().unwrap_or("-"),
				self.canary_worst_ms
			));
		} else if self.canary_ok > 0 && self.canary_worst_ms > 1500.0 {
			out.push(format!(
				"TUNNEL SLOW: the slowest of {} name lookups through the tunnel took {:.0} ms.",
				self.canary_ok, self.canary_worst_ms
			));
		}
		if self.refused_lines > 0 {
			out.push(
				"INLET FULL: the local SOCKS5 server refused clients because every slot was busy."
					.to_owned(),
			);
		}
		if !self.connect_failures.is_empty() {
			let reasons = self
				.connect_failures
				.iter()
				.map(|(reason, count)| format!("{reason} x{count}"))
				.collect::<Vec<_>>()
				.join(", ");
			out.push(format!("TUNNEL CONNECTIONS REFUSED: {reasons}."));
		}
		if self.relays_failed > 0 {
			out.push(format!(
				"TUNNEL RELAYS FAILED: {} proxied connection(s) broke mid-stream, first at {}.",
				self.relays_failed,
				self.first_relay_failure.as_deref().unwrap_or("-")
			));
		}
		if !self.association_ends.is_empty() {
			let ends = self
				.association_ends
				.iter()
				.map(|(reason, count)| format!("{reason} x{count}"))
				.collect::<Vec<_>>()
				.join(", ");
			out.push(format!("UDP ASSOCIATIONS ENDED: {ends}."));
		}
		out
	}
}

/// The written analysis of one ultra log.
pub fn analyze(log: &str) -> String {
	let mut first = None;
	let mut last = None;
	let mut pulse_fps = Vec::new();
	let (mut slow_frames, mut busy_windows, mut hidden_drawing) = (0.0, 0usize, 0usize);
	let mut anomalies: BTreeMap<String, usize> = BTreeMap::new();
	let mut events: Vec<String> = Vec::new();
	let mut dropped_events = 0usize;
	let mut audio = Audio::default();
	let (mut idle_share_windows, mut capture_frames) = (0usize, 0.0);
	let mut tunnel = Tunnel::default();
	let mut process = Process::default();
	let mut media = Media::default();
	let mut pings = Vec::new();
	let mut lines = 0usize;

	for raw in log.lines() {
		let Some(line) = parse(raw) else { continue };
		lines += 1;
		first.get_or_insert(line.time.to_owned());
		last = Some(line.time.to_owned());
		match line.scope {
			"pulse" => {
				if let Some(fps) = num(&format!(" {}", line.rest), "fps") {
					pulse_fps.push(fps);
					busy_windows += usize::from(fps > BUSY_FPS);
				}
				slow_frames += num(&format!(" {}", line.rest), "slow").unwrap_or(0.0);
				process.observe(&format!(" {}", line.rest));
			}
			"protect" => tunnel.observe(line.time, line.rest),
			"anomaly" => {
				let class = generalize(line.rest);
				hidden_drawing += usize::from(line.rest.starts_with("hidden window"));
				*anomalies.entry(class).or_default() += 1;
			}
			"event" => {
				if events.len() < MAX_EVENTS {
					events.push(format!("{}  {}", line.time, line.rest));
				} else {
					dropped_events += 1;
				}
			}
			"voice" => {
				media.observe(line.time, line.rest);
				let text = format!(" {}", line.rest);
				if let Some(ms) = text
					.strip_prefix(" heartbeat ping ")
					.and_then(|rest| rest.split_whitespace().next())
					.and_then(|value| value.parse::<f64>().ok())
				{
					pings.push(ms);
				} else if line.rest.contains("[Nivra voice Transport]") {
					audio.windows += 1;
					let packets = num(&text, "packets").unwrap_or(0.0);
					// `packets=` also names the video counter; only the audio group carries `pushed=`.
					if let Some(pushed) = num(&text, "pushed") {
						let audio_packets = text
							.split(" audio: ")
							.nth(1)
							.and_then(|rest| num(&format!(" {rest}"), "packets"))
							.unwrap_or(packets);
						audio.with_packets += 1;
						if pushed == 0.0 && audio_packets > 0.0 {
							audio.heard_nothing += 1;
							audio.first_silent.get_or_insert(line.time.to_owned());
							audio.last_silent = Some(line.time.to_owned());
						}
						for (name, key) in [
							("sender not in the encrypted group", "not_member"),
							("encryption not ready", "not_ready"),
							("unknown sender (SSRC)", "unknown_ssrc"),
							("deafened", "deafened"),
							("decryption failed", "decrypt_failed"),
						] {
							let part = text.split(" audio: ").nth(1).unwrap_or("");
							*audio.reasons.entry(name).or_default() +=
								num(&format!(" {part}"), key).unwrap_or(0.0) as u64;
						}
					}
					if text.contains(" dave: ready=0") {
						audio.dave_not_ready += 1;
					}
					if text.contains(" members=0") {
						audio.dave_no_members += 1;
					}
				} else if line.rest.contains("[Nivra voice StreamSend]") {
					let waiting = num(&text, "waiting").unwrap_or(0.0);
					let announced = num(&text, "announced").unwrap_or(0.0);
					let sent = first_of(&text, "video_send").unwrap_or(0.0);
					if waiting > 0.0 && announced == 0.0 && sent == 0.0 {
						idle_share_windows += 1;
					}
				} else if line.rest.contains("[Nivra voice ScreenVideo]") {
					capture_frames += first_of(&text, "mix").unwrap_or(0.0);
				}
			}
			_ => {}
		}
	}

	let mut out = String::new();
	out.push_str("Nivra ultra diagnostics analysis\n");
	out.push_str(&format!(
		"Log: {lines} lines, {} to {}\n\n",
		first.as_deref().unwrap_or("-"),
		last.as_deref().unwrap_or("-")
	));
	let mut findings: Vec<String> = Vec::new();

	if audio.heard_nothing > 0 {
		let reason = audio
			.reasons
			.iter()
			.filter(|(_, count)| **count > 0)
			.max_by_key(|(_, count)| **count)
			.map_or("no single reason", |(name, _)| *name);
		findings.push(format!(
			"AUDIO NOT HEARD: in {} of {} voice windows audio packets arrived and none reached the mixer (mostly: {reason}), from {} to {}.",
			audio.heard_nothing,
			audio.windows,
			audio.first_silent.as_deref().unwrap_or("-"),
			audio.last_silent.as_deref().unwrap_or("-"),
		));
	}
	if audio.dave_not_ready > 0 {
		findings.push(format!(
			"ENCRYPTED GROUP NOT READY: {} voice windows reported the end-to-end group as not ready ({} with no members).",
			audio.dave_not_ready, audio.dave_no_members
		));
	}
	if idle_share_windows > 0 {
		findings.push(format!(
			"SHARE WITH NO VIEWER: {idle_share_windows} stream windows were waiting for a viewer with nothing encoded; the capture read {capture_frames:.0} frames in those and nearby windows."
		));
	}
	if busy_windows > 0 {
		findings.push(format!(
			"BUSY UI: {busy_windows} of {} pulses drew more than {BUSY_FPS:.0} frames per second. That is normal while the pointer moves over the window; it matters only together with slow frames.",
			pulse_fps.len()
		));
	}
	if hidden_drawing > 0 {
		findings.push(format!(
			"HIDDEN WINDOW DRAWING: {hidden_drawing} pulses with the window hidden drew more than 20 frames per second."
		));
	}
	if slow_frames > 0.0 {
		findings.push(format!(
			"SLOW FRAMES: {slow_frames:.0} frames took longer than 250 ms between the start of logic and the end of ui."
		));
	}
	findings.extend(media.findings());
	findings.extend(tunnel.findings());
	findings.extend(process.findings());
	let ping = sorted(pings);
	if ping.last().is_some_and(|max| *max > SLOW_PING_MS) {
		findings.push(format!(
			"SLOW VOICE HEARTBEAT: {} of {} acknowledgements took over {SLOW_PING_MS:.0} ms (max {:.0} ms).",
			ping.iter().filter(|ms| **ms > SLOW_PING_MS).count(),
			ping.len(),
			ping.last().copied().unwrap_or(0.0)
		));
	}
	if findings.is_empty() {
		out.push_str("FINDINGS\nNothing stood out in this log.\n");
	} else {
		out.push_str("FINDINGS\n");
		for finding in &findings {
			out.push_str(&format!("- {finding}\n"));
		}
	}

	let fps = sorted(pulse_fps);
	out.push_str(&format!(
		"\nUI frames per second over {} pulses: median {:.1}, 95th {:.1}, max {:.1}\n",
		fps.len(),
		percentile(&fps, 50),
		percentile(&fps, 95),
		fps.last().copied().unwrap_or(0.0)
	));
	out.push_str(&process.summary());
	if !ping.is_empty() {
		out.push_str(&format!(
			"Voice heartbeat over {} acknowledgements: median {:.0} ms, 95th {:.0} ms, max {:.0} ms\n",
			ping.len(),
			percentile(&ping, 50),
			percentile(&ping, 95),
			ping.last().copied().unwrap_or(0.0)
		));
	}
	out.push_str(&format!(
		"Voice windows: {} total, {} with audio counters\n",
		audio.windows, audio.with_packets
	));
	if !anomalies.is_empty() {
		out.push_str("\nWarnings raised while running\n");
		for (class, count) in &anomalies {
			out.push_str(&format!("- {class}: {count}\n"));
		}
	}
	if !events.is_empty() {
		out.push_str("\nTimeline\n");
		for event in &events {
			out.push_str(&format!("{event}\n"));
		}
		if dropped_events > 0 {
			out.push_str(&format!("... and {dropped_events} more in the log\n"));
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	const SAMPLE: &str = "\
2026-10-16 17:40:00 event call joined, phase Connecting
2026-10-16 17:40:05 event call phase Connecting -> Connected
2026-10-16 17:40:10 pulse fps=1.4 slow=0 hidden=false protect=true call=Connected people=3 live=0 watching=0 camera=false
2026-10-16 17:40:15 pulse fps=97.0 slow=2 hidden=false protect=true call=Connected people=3 live=1 watching=0 camera=false
2026-10-16 17:40:15 anomaly 2 slow frames, longest 800 ms
2026-10-16 17:40:20 voice [Nivra voice Transport] window_ms=5000 stages(calls,total_us,max_us): receive=[0, 0, 0] video: packets=0 audio: packets=250 unknown_ssrc=0 not_ready=0 not_member=250 deafened=0 decrypt_failed=0 pushed=0 dave: ready=1 transport_only=0 members=0
2026-10-16 17:40:25 voice [Nivra voice Transport] window_ms=5000 stages(calls,total_us,max_us): receive=[200, 1, 1] audio: packets=250 unknown_ssrc=0 not_ready=0 not_member=0 deafened=0 decrypt_failed=0 pushed=250 dave: ready=1 transport_only=0 members=3
2026-10-16 17:40:30 voice [Nivra voice StreamSend] window_ms=5000 video_send=[0, 0, 0] stream_ticks: transport_key=251 dave_ready=0 group_ready=0 pending=0 waiting=251 announced=0 capture_ready=0 audio_enabled=0
2026-10-16 17:40:30 voice [Nivra voice ScreenVideo] window_ms=5000 stages(calls,total_us,max_us): echo_render=[0, 0, 0] echo_capture=[0, 0, 0] noise=[0, 0, 0] encode=[0, 0, 0] mix=[200, 1, 1] receive=[200, 1, 1]
2026-10-16 17:40:31 voice heartbeat ping 40 ms
2026-10-16 17:40:36 voice heartbeat ping 900 ms
";

	#[test]
	fn a_silent_call_and_an_idle_share_are_named_in_the_findings() {
		let report = analyze(SAMPLE);
		assert!(
			report.contains("AUDIO NOT HEARD: in 1 of 2 voice windows"),
			"{report}"
		);
		assert!(
			report.contains("sender not in the encrypted group"),
			"{report}"
		);
		assert!(
			report.contains("SHARE WITH NO VIEWER: 1 stream windows"),
			"{report}"
		);
		assert!(report.contains("capture read 200 frames"), "{report}");
		assert!(report.contains("BUSY UI: 1 of 2 pulses"), "{report}");
		assert!(report.contains("SLOW FRAMES: 2 frames"), "{report}");
		assert!(report.contains("SLOW VOICE HEARTBEAT: 1 of 2"), "{report}");
		assert!(
			report.contains("call phase Connecting -> Connected"),
			"{report}"
		);
		assert!(
			report.contains("- N slow frames, longest N ms: 1"),
			"{report}"
		);
	}

	#[test]
	fn tunnel_trouble_is_named_with_its_numbers() {
		let log = "\
2026-10-16 22:46:17 protect route applied: generation 2 primary=Complete reserve=false
2026-10-16 22:46:22 protect tunnel: handshake_age=4s tx_bytes=900 rx_bytes=800 loss=0.00 rtt_ms=40 datagrams sent=10 send_failed=0 received=10 decrypt_errors=0 last_error=none inbound_queue_full=0 downlink_dropped=0 uplink_blocked=0 tcp_aborted=0 tcp=1 udp=1
2026-10-16 22:46:27 protect tunnel: handshake_age=9s tx_bytes=9000 rx_bytes=800 loss=0.40 rtt_ms=none datagrams sent=250 send_failed=2 received=0 decrypt_errors=3 last_error=InvalidMac inbound_queue_full=4 downlink_dropped=5 uplink_blocked=6 tcp_aborted=1 tcp=1 udp=1
2026-10-16 22:47:27 protect datagram ended: opened on generation Some(1), route is now generation 2
2026-10-16 22:47:28 protect udp association ended: the app closed it; uplink_dropped=7
2026-10-16 22:47:29 protect tunnel stopped: the endpoint reader ended
";
		let report = analyze(log);
		assert!(
			report.contains("ROUTE CHANGED UNDER A LIVE SOCKET: 1 media socket"),
			"{report}"
		);
		assert!(report.contains("TUNNEL STOPPED at 22:47:29"), "{report}");
		assert!(
			report.contains("TUNNEL ONE-WAY: in 1 of 2 five-second summaries"),
			"{report}"
		);
		assert!(
			report.contains(
				"decrypt_errors=3 inbound_queue_full=4 downlink_dropped=5 uplink_dropped=7"
			),
			"{report}"
		);
		assert!(report.contains("InvalidMac x1"), "{report}");
		assert!(report.contains("the app closed it x1"), "{report}");
	}

	#[test]
	fn process_media_and_tunnel_probes_are_named_in_the_findings() {
		let log = "\
2026-10-16 22:46:20 pulse fps=60.0 slow=0 hidden=false protect=true call=Connected people=2 live=0 watching=0 camera=false ram=400MB private=450MB handles=800
2026-10-16 22:46:25 pulse fps=60.0 slow=0 hidden=false protect=true call=Connected people=2 live=0 watching=0 camera=false ram=900MB private=950MB handles=1300 cpu=85%
2026-10-16 22:46:26 voice voice udp: 4 tolerated receive error(s), latest kind=ConnectionReset os=10054
2026-10-16 22:46:27 voice voice udp receive failed: kind=ConnectionAborted os=none
2026-10-16 22:46:30 protect canary: dns ok in 150 ms
2026-10-16 22:46:40 protect canary: dns failed (DNS lookup timed out) after 10001 ms
2026-10-16 22:46:41 protect inlet refused a client: all 64 slots are busy (1 refused so far)
2026-10-16 22:46:42 protect connect to port 443 failed: TCP connection timed out
2026-10-16 22:46:43 protect relay to port 443 failed after 31 s: kind=ConnectionReset
";
		let report = analyze(log);
		for expected in [
			"MEMORY GROWTH: the app went from 400 MB to 900 MB",
			"HANDLE GROWTH: open handles went from 800 to 1300",
			"HIGH PROCESS CPU: 1 of 1 pulses",
			"MEDIA SOCKET ERRORS TOLERATED: 4 receive errors were skipped (ConnectionReset x4)",
			"MEDIA SOCKET FAILED: 22:46:27 voice udp receive failed: kind=ConnectionAborted",
			"TUNNEL CANARY FAILED: 1 of 2 name lookups",
			"INLET FULL",
			"TUNNEL CONNECTIONS REFUSED: TCP connection timed out x1",
			"TUNNEL RELAYS FAILED: 1 proxied connection(s)",
			"Process over 2 pulses: resident 400 -> 900 MB",
		] {
			assert!(
				report.contains(expected),
				"{expected:?} missing from:\n{report}"
			);
		}
	}

	#[test]
	fn a_healthy_tunnel_adds_no_finding() {
		let log = "\
2026-10-16 22:46:22 protect tunnel: handshake_age=4s tx_bytes=900 rx_bytes=800 loss=0.00 rtt_ms=40 datagrams sent=10 send_failed=0 received=10 decrypt_errors=0 last_error=none inbound_queue_full=0 downlink_dropped=0 uplink_blocked=0 tcp_aborted=0 tcp=1 udp=1
";
		let report = analyze(log);
		assert!(report.contains("Nothing stood out"), "{report}");
	}

	#[test]
	fn a_quiet_log_says_nothing_stood_out_and_junk_lines_are_ignored() {
		let log = "not a log line\n2026-10-16 17:40:10 pulse fps=1.4 slow=0 hidden=false protect=false call=none\n";
		let report = analyze(log);
		assert!(report.contains("Nothing stood out"), "{report}");
		assert!(report.contains("1 lines"), "{report}");
		assert!(analyze("").contains("Nothing stood out"));
	}

	/// Prints the analysis of a real log: `NIVRA_ULTRA_LOG=path cargo test -p platform
	/// analysis_of_a_real_log -- --ignored --nocapture`.
	#[test]
	#[ignore = "reads a log named by NIVRA_ULTRA_LOG"]
	fn analysis_of_a_real_log() {
		let path = std::env::var_os("NIVRA_ULTRA_LOG").expect("set NIVRA_ULTRA_LOG");
		let text = std::fs::read_to_string(path).unwrap();
		println!("{}", analyze(&text));
	}

	#[test]
	fn the_event_list_is_capped() {
		let log: String = (0..MAX_EVENTS + 5)
			.map(|i| format!("2026-10-16 17:40:00 event people {i} -> {}\n", i + 1))
			.collect();
		let report = analyze(&log);
		assert!(report.contains("... and 5 more in the log"), "{report}");
	}
}
