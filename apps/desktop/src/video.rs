//! One explicit inline player; native decoding and network reads stay on one lazy worker.
/// Stderr diagnostics budget macro (defined before the child modules so they
/// share it textually); see `vlog` below.
macro_rules! vlog {
	($budget:expr, $($arg:tt)*) => {
		$crate::video::vlog($budget, format_args!($($arg)*))
	};
}
#[cfg(target_os = "macos")]
mod fallback;
mod output;
mod source;
use eframe::egui;
use std::sync::{
	Arc, Mutex,
	atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
};
use std::time::Duration;
use ui::{VideoCommand, VideoState, VideoUi};
/// Stderr diagnostics budget: one playback emits at most this many lines, so
/// repeated playback cannot grow an unbounded diagnostic stream (Codex PR #34).
/// One budgeted diagnostic line; silently dropped once the session budget runs out.
fn vlog(budget: &AtomicUsize, args: std::fmt::Arguments<'_>) {
	if budget
		.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |lines| {
			lines.checked_sub(1)
		})
		.is_ok()
	{
		eprintln!("[Nivra video] {args}");
	}
}
/// Native decoder opens cannot be interrupted: a stuck open keeps its thread
/// until the OS call returns. Cap concurrent workers so retries cannot grow
/// threads without bound; the slot frees when the worker exits (Codex PR #34).
const MAX_VIDEO_WORKERS: usize = 4;
/// Stderr lines per playback (open, first frame/byte, stalls, errors).
const LOG_BUDGET: usize = 24;
static LIVE_VIDEO_WORKERS: AtomicUsize = AtomicUsize::new(0);
fn try_acquire_video_worker(live: &AtomicUsize) -> bool {
	try_acquire_bounded(live, worker_limit())
}
fn try_acquire_bounded(live: &AtomicUsize, limit: usize) -> bool {
	if live.fetch_add(1, Ordering::AcqRel) >= limit {
		live.fetch_sub(1, Ordering::AcqRel);
		false
	} else {
		true
	}
}
/// Concurrency bound; tests may lower it to exercise the refusal without
/// taking the process-wide slots from tests running in parallel.
#[cfg(test)]
static TEST_WORKER_LIMIT: AtomicUsize = AtomicUsize::new(MAX_VIDEO_WORKERS);
/// Tests that lower the worker limit or open real workers take turns, so the
/// process-wide slot budget and the limit override stay deterministic.
#[cfg(test)]
static VIDEO_START_LOCK: Mutex<()> = Mutex::new(());
fn worker_limit() -> usize {
	#[cfg(test)]
	{
		TEST_WORKER_LIMIT.load(Ordering::Acquire)
	}
	#[cfg(not(test))]
	{
		MAX_VIDEO_WORKERS
	}
}
/// Startup transients free slots in milliseconds while stuck native opens do
/// not: poll briefly so rapid zapping never fails, then refuse with a clean
/// error instead of growing threads without bound.
struct WorkerSlot<'a>(&'a AtomicUsize);
impl Drop for WorkerSlot<'_> {
	fn drop(&mut self) {
		self.0.fetch_sub(1, Ordering::AcqRel);
	}
}
/// Stall watchdog predicate: an unpaused player that made no progress for 10 s
/// restarts the decoder once, then fails. It also covers a missing first frame
/// (`preview_needed` never clears while the clock never advances).
fn stall_timed_out(paused: bool, idle: Duration) -> bool {
	!paused && idle > Duration::from_secs(10)
}

/// One decoded frame as the UI texture source. Built on the worker so the render
/// thread only swaps buffers. A frame larger than the preview box is scaled down here
/// as the final safety net for decoders that ignore the requested output size; the
/// render pass never sees an oversized frame (owner request, 2026-10-04).
fn frame_image(width: u32, height: u32, rgba: &[u8]) -> Result<egui::ColorImage, &'static str> {
	let (width, height) = (width as usize, height as usize);
	if width == 0 || height == 0 || rgba.len() != width * height * 4 {
		return Err("Video frame has an unsupported size");
	}
	let (fit_w, fit_h) = platform::video::preview_dimensions(width as u32, height as u32);
	if (fit_w as usize, fit_h as usize) != (width, height) {
		let scaled = downscale_rgba(width, height, rgba, fit_w as usize, fit_h as usize);
		return color_image(fit_w as usize, fit_h as usize, &scaled);
	}
	color_image(width, height, rgba)
}

/// Fast nearest-neighbour downscale for a decoder that ignored the requested output
/// size. The source is a validated RGBA buffer, so every index is in range.
fn downscale_rgba(width: usize, height: usize, rgba: &[u8], out_w: usize, out_h: usize) -> Vec<u8> {
	let mut out = vec![0u8; out_w * out_h * 4];
	for y in 0..out_h {
		let source_row = (y * height / out_h) * width * 4;
		let target_row = y * out_w * 4;
		for x in 0..out_w {
			let source = source_row + (x * width / out_w) * 4;
			let target = target_row + x * 4;
			out[target..target + 4].copy_from_slice(&rgba[source..source + 4]);
		}
	}
	out
}

fn color_image(width: usize, height: usize, rgba: &[u8]) -> Result<egui::ColorImage, &'static str> {
	if rgba.len() != width * height * 4 {
		return Err("Video frame has an unsupported size");
	}
	let pixels = rgba
		.as_chunks::<4>()
		.0
		.iter()
		.map(|&[r, g, b, a]| eframe::egui::Color32::from_rgba_unmultiplied(r, g, b, a))
		.collect::<Vec<_>>();
	Ok(eframe::egui::ColorImage {
		size: [width, height],
		source_size: eframe::egui::vec2(width as f32, height as f32),
		pixels,
	})
}

#[derive(Default)]
struct Update {
	state: VideoState,
	position: f64,
	duration: f64,
	frame: Option<egui::ColorImage>,
}
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

struct Session {
	id: u64,
	/// Wanted voice output selection, refreshed by the UI (`None` = default).
	output: std::sync::Mutex<Option<String>>,
	cancelled: Arc<AtomicBool>,
	/// Remaining stderr diagnostic lines for this playback (see `vlog`).
	log_budget: Arc<AtomicUsize>,
	/// Set by the 20 s open watchdog together with `cancelled`: a stall surfaces
	/// as `Failed` (with retry) instead of freezing the player in `Loading`.
	open_timed_out: Arc<AtomicBool>,
	paused: Arc<AtomicBool>,
	volume: Arc<AtomicU32>,
	seek: Arc<AtomicU64>,
	update: Mutex<Update>,
}
impl Session {
	fn new(volume: f32, id: u64) -> Self {
		Self {
			id,
			output: std::sync::Mutex::new(None),
			cancelled: Arc::new(AtomicBool::new(false)),
			log_budget: Arc::new(AtomicUsize::new(LOG_BUDGET)),
			open_timed_out: Arc::new(AtomicBool::new(false)),
			paused: Arc::new(AtomicBool::new(false)),
			volume: Arc::new(AtomicU32::new(volume.to_bits())),
			seek: Arc::new(AtomicU64::new(u64::MAX)),
			update: Mutex::new(Update {
				state: VideoState::Loading,
				..Default::default()
			}),
		}
	}
}
#[derive(Clone)]
struct Request {
	session: Arc<Session>,
	url: Option<url::Url>,
	fallback: Option<url::Url>,
	size: usize,
}
#[derive(Default)]
pub struct Video {
	session: Option<Arc<Session>>,
	worker: Option<std::thread::JoinHandle<()>>,
}
impl Video {
	/// Whether a media session is live (used by the stall watchdog's phase).
	pub fn is_active(&self) -> bool {
		self.session.is_some()
	}
	/// Bounded one-line snapshot for the freeze log; never player metadata or URLs.
	pub fn snapshot(&self) -> String {
		let Some(session) = &self.session else {
			return "none".into();
		};
		let (state, position) = session
			.update
			.try_lock()
			.map(|update| (Some(update.state), update.position))
			.unwrap_or((None, 0.0));
		format!(
			"session={} paused={} position={position:.2} workers={} state={}",
			session.id,
			session.paused.load(Ordering::Relaxed),
			LIVE_VIDEO_WORKERS.load(Ordering::Relaxed),
			match state {
				Some(VideoState::Playing) => "playing",
				Some(VideoState::Paused) => "paused",
				Some(VideoState::Loading) => "loading",
				Some(VideoState::Ended) => "ended",
				Some(VideoState::Failed(_)) => "failed",
				_ => "idle",
			}
		)
	}
	pub fn stop(&mut self) {
		if let Some(session) = self.session.take() {
			session.cancelled.store(true, Ordering::Release);
		}
		self.worker = None;
	}
	pub fn poll(&self, player: &mut VideoUi, ctx: &eframe::egui::Context, output: Option<&str>) {
		let Some(session) = &self.session else {
			return;
		};
		let fresh = output.map(str::to_owned);
		// Never block the render thread on a lock the worker also takes.
		if let Ok(mut wanted) = session.output.try_lock()
			&& *wanted != fresh
		{
			*wanted = fresh;
		}
		if let Ok(mut update) = session.update.try_lock() {
			player.state = update.state;
			player.position = update.position;
			player.duration = update.duration;
			// The drag preview belongs to the UI: it is cleared when the seek is issued,
			// never by position, or a backward drag would snap back to the old position.
			let frame = update.frame.take();
			drop(update);
			if let Some(frame) = frame {
				player.accept_frame(ctx, frame);
			}
		}
	}
	pub fn command(
		&mut self,
		command: VideoCommand,
		player: &mut VideoUi,
		runtime: &tokio::runtime::Handle,
		ctx: &eframe::egui::Context,
		demo: bool,
	) {
		match command {
			VideoCommand::Stop => self.stop(),
			VideoCommand::Play(attachment) => {
				self.stop();
				if let Err(error) = self.start(attachment, player.volume, runtime, ctx, demo, true)
				{
					player.state = VideoState::Failed(error);
				}
			}
			VideoCommand::Preview(attachment) => {
				self.stop();
				if let Err(error) = self.start(attachment, player.volume, runtime, ctx, demo, false)
				{
					player.state = VideoState::Failed(error);
				}
			}
			VideoCommand::Pause(paused) => {
				if let Some(s) = &self.session {
					s.paused.store(paused, Ordering::Release);
				}
			}
			VideoCommand::Volume(volume) => {
				if volume.is_finite()
					&& let Some(s) = &self.session
				{
					s.volume
						.store(volume.clamp(0., 1.).to_bits(), Ordering::Release);
				}
			}
			VideoCommand::Seek(seconds) => {
				if seconds.is_finite()
					&& let Some(s) = &self.session
				{
					s.seek
						.store((seconds.clamp(0., 7200.) * 1000.) as u64, Ordering::Release);
				}
			}
		}
	}
	fn start(
		&mut self,
		attachment: model::Attachment,
		volume: f32,
		runtime: &tokio::runtime::Handle,
		ctx: &eframe::egui::Context,
		demo: bool,
		autoplay: bool,
	) -> Result<(), &'static str> {
		if !attachment.is_video() {
			return Err("This file is not a video");
		}
		// An embed preview has no attachment record, so it carries the direct file URL
		// and no size: the allowlist accepts it and the source learns the length.
		let embed = attachment.size == 0;
		if !embed && attachment.size > 100 * 1024 * 1024 {
			return Err("Video preview limit: 100 MiB");
		}
		let (url, fallback, size) = if demo {
			(None, None, 0)
		} else if embed {
			let raw = model::web_media::direct_embed_video(
				attachment.media.proxy_url.as_deref(),
				attachment.media.url.as_deref(),
			)
			.ok_or("Unsupported embed video provider or URL")?;
			let url =
				url::Url::parse(raw).map_err(|_| "Unsupported embed video provider or URL")?;
			(Some(url), None, 0)
		} else {
			(
				Some(
					crate::downloads::original_url(&attachment)
						.ok_or("Video attachment unavailable")?,
				),
				crate::downloads::proxy_attachment_url(&attachment),
				attachment.size as usize,
			)
		};
		let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
		let session = Arc::new(Session::new(volume, session_id));
		// A poster open stays paused until the play button resumes it; the worker
		// still decodes and publishes the first frame.
		session.paused.store(!autoplay, Ordering::Release);
		if !try_acquire_video_worker(&LIVE_VIDEO_WORKERS) {
			// No waiting: start() runs on the render thread, and four healthy
			// concurrent plays must fail fast instead of freezing input.
			// Startup transients free their slot within milliseconds.
			return Err("Video is busy finishing another open; retry in a moment");
		}
		let request = Request {
			session: session.clone(),
			url,
			fallback,
			size,
		};
		let worker_session = session.clone();
		let runtime = runtime.clone();
		let ctx = ctx.clone();
		let worker = std::thread::Builder::new()
			.name(format!("nivra-attachment-video-{session_id}"))
			.spawn(move || {
				let _slot = WorkerSlot(&LIVE_VIDEO_WORKERS);
				let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
					play(&request, &runtime, &ctx)
				}));
				match result {
					Ok(Err(error)) => {
						if report_worker_error(&worker_session) {
							if let Ok(mut update) = worker_session
								.update
								.try_lock()
								.or_else(|_| worker_session.update.lock())
							{
								update.state = VideoState::Failed(error);
							}
							ctx.request_repaint();
						}
					}
					Err(_) => {
						if !worker_session.cancelled.load(Ordering::Acquire) {
							if let Ok(mut update) = worker_session
								.update
								.try_lock()
								.or_else(|_| worker_session.update.lock())
							{
								update.state =
									VideoState::Failed("The video could not be decoded safely.");
							}
							ctx.request_repaint();
						}
					}
					Ok(Ok(())) => {}
				}
			})
			.map_err(|_| {
				LIVE_VIDEO_WORKERS.fetch_sub(1, Ordering::AcqRel);
				"Could not start video worker"
			})?;
		self.worker = Some(worker);
		self.session = Some(session);
		Ok(())
	}
}
impl Drop for Video {
	fn drop(&mut self) {
		self.stop();
	}
}

/// Frames decoded before a seek target are dropped: the decoder came back from the
/// previous keyframe, and only the frame at or after the target may be shown, so the
/// image and the sound start together.
fn frame_reaches(pts: f64, target: f64) -> bool {
	pts >= target - 0.01
}

/// A 20 s open watchdog and a voluntary stop both set `cancelled`; only the
/// watchdog also sets `open_timed_out`. A stall must surface as `Failed` (with
/// retry) instead of freezing the player in `Loading`.
fn report_worker_error(session: &Session) -> bool {
	!session.cancelled.load(Ordering::Acquire) || session.open_timed_out.load(Ordering::Acquire)
}
fn play(
	request: &Request,
	runtime: &tokio::runtime::Handle,
	ctx: &eframe::egui::Context,
) -> Result<(), &'static str> {
	use platform::video::Decoder;
	use std::time::{Duration, Instant};
	let session = &request.session;
	if session.cancelled.load(Ordering::Acquire) {
		return Ok(());
	}
	#[cfg(test)]
	if source::OFFLINE_PROBE.load(Ordering::Acquire) {
		return Ok(());
	}
	let start_time = Instant::now();
	vlog!(
		&session.log_budget,
		"open_source: starting session {}",
		session.id
	);
	let url = if let Some(primary) = request.url.clone() {
		match source::resolve_media_url(
			primary,
			request.fallback.clone(),
			session.cancelled.clone(),
			runtime.clone(),
		) {
			Ok(url) => Some(url),
			Err(_) if session.cancelled.load(Ordering::Acquire) => return Ok(()),
			Err(error) => return Err(error),
		}
	} else {
		None
	};
	let source = source::source(
		url.clone(),
		request.size,
		session.cancelled.clone(),
		session.log_budget.clone(),
		runtime.clone(),
	)?;
	vlog!(
		&session.log_budget,
		"open_source: ready in {:.3} ms",
		start_time.elapsed().as_secs_f64() * 1000.0
	);
	let open_started = Instant::now();
	vlog!(&session.log_budget, "open_decoder: starting");
	session.open_timed_out.store(false, Ordering::Release);
	let open_timed_out_clone = session.open_timed_out.clone();
	let cancelled_clone = session.cancelled.clone();
	let watchdog = runtime.spawn(async move {
		tokio::time::sleep(Duration::from_secs(20)).await;
		open_timed_out_clone.store(true, Ordering::Release);
		cancelled_clone.store(true, Ordering::Release);
	});
	let decoder = Decoder::open(source);
	#[cfg(target_os = "macos")]
	let decoder = match decoder {
		Err(platform::video::UNSUPPORTED | platform::video::INVALID)
			if !session.open_timed_out.load(Ordering::Acquire) =>
		{
			let source = source::source(
				url.clone(),
				request.size,
				session.cancelled.clone(),
				session.log_budget.clone(),
				runtime.clone(),
			)?;
			fallback::open(source, &session.cancelled)
		}
		result => result,
	};
	watchdog.abort();
	if session.open_timed_out.load(Ordering::Acquire) {
		vlog!(&session.log_budget, "open_decoder: timed out after 20s");
		return Err("Video buffering stalled; retry or download to play externally");
	}
	let _ = url;
	let decoder = decoder?;
	vlog!(
		&session.log_budget,
		"open_decoder: ready in {:.3} ms",
		open_started.elapsed().as_secs_f64() * 1000.0
	);
	let result = play_decoded(decoder, session, ctx, start_time);
	// Cancellation aborts in-flight source reads; that is a clean stop, not a decode failure.
	if session.cancelled.load(Ordering::Acquire) {
		return Ok(());
	}
	result
}

fn play_decoded(
	mut decoder: platform::video::Decoder,
	session: &Session,
	ctx: &eframe::egui::Context,
	start_time: std::time::Instant,
) -> Result<(), &'static str> {
	use platform::video::Sample;
	use std::{
		collections::VecDeque,
		task::Poll::{Pending, Ready},
		time::{Duration, Instant},
	};
	let info = decoder.info();
	let mut target = 0.;
	let mut seeking = false;
	let mut stall_restarts: u32 = 0;
	let mut first_frame_logged = false;
	'seek: loop {
		if session.cancelled.load(Ordering::Acquire) {
			return Ok(());
		}
		if seeking {
			vlog!(&session.log_budget, "seek: starting at {target:.3}s");
			let seek_started = Instant::now();
			decoder.seek(target)?;
			vlog!(
				&session.log_budget,
				"seek: ready in {:.0} ms",
				seek_started.elapsed().as_secs_f64() * 1000.0
			);
		}
		let position = Arc::new(AtomicU64::new(0));
		let eof = Arc::new(AtomicBool::new(false));
		let failed = Arc::new(AtomicBool::new(false));
		let wanted_output = || session.output.lock().ok().and_then(|guard| guard.clone());
		let mut opened: Option<(Option<String>, String)> = None;
		let mut output = if info.sample_rate > 0 {
			Some(
				output::open(
					info.sample_rate,
					output::Controls {
						cancelled: session.cancelled.clone(),
						paused: session.paused.clone(),
						seek: session.seek.clone(),
						volume: session.volume.clone(),
						position: position.clone(),
						eof: eof.clone(),
						failed: failed.clone(),
					},
					wanted_output().as_deref(),
				)
				.map(|(output, id)| {
					opened.replace((wanted_output(), id));
					output
				})?,
			)
		} else {
			None
		};
		let mut frames: VecDeque<(f64, u32, u32, Vec<u8>)> = VecDeque::new();
		let mut seek_preview: Option<(f64, u32, u32, Vec<u8>)> = None;
		let mut pending_audio: Option<(Vec<[f32; 2]>, usize)> = None;
		let mut queued_audio = 0u64;
		let mut video_ended = false;
		let mut audio_ended = output.is_none();
		let mut preview_needed = true;
		// Media clock anchored on the monotonic wall clock instead of accumulated deltas:
		// a slow read used to drop elapsed time ("freeze the clock") and a silent or
		// short-audio clip could never reach its end, leaving the card black forever.
		let mut anchor = Instant::now();
		let mut anchor_at = target;
		// Whether the media clock is currently held (paused, or before the poster).
		let mut holding = true;
		let mut ticks = 0u64;
		let mut last_progress = Instant::now();
		let mut previous_position = target;
		loop {
			if session.cancelled.load(Ordering::Acquire) {
				return Ok(());
			}
			let seek = session.seek.swap(u64::MAX, Ordering::AcqRel);
			if seek != u64::MAX {
				// Release the old device/ring before a potentially blocking native seek.
				drop(output);
				target = (seek as f64 / 1000.).min((info.duration - 0.001).max(0.));
				seeking = true;
				if let Ok(mut update) = session.update.try_lock().or_else(|_| session.update.lock())
				{
					update.frame = None;
					update.state = VideoState::Loading;
					update.position = target;
				}
				ctx.request_repaint();
				continue 'seek;
			}
			if failed.load(Ordering::Acquire) {
				return Err("Video audio output stopped");
			}
			// A changed voice output re-seeks in place: the seek cycle drops the
			// device and reopens it on the new selection without losing position.
			ticks += 1;
			let wanted = wanted_output();
			// Explicit switches migrate every frame; a moved system default is re-resolved about once a second.
			// A silent/audio-less clip has no output to migrate; comparing a missing
			// handle with the system default re-seeked the player every second and
			// left the card black forever (owner P0, 2026-10-03).
			let default_moved = output.is_some()
				&& wanted.is_none()
				&& ticks.is_multiple_of(60)
				&& opened.as_ref().map(|(_, id)| id)
					!= discord_voice::output::default_id(&cpal::default_host()).as_ref();
			if output.is_some()
				&& opened
					.as_ref()
					.and_then(|(selection, _)| selection.as_ref())
					!= wanted.as_ref()
				|| default_moved
			{
				session
					.seek
					.store((previous_position * 1000.0) as u64, Ordering::Release);
			}
			let now = Instant::now();
			let paused = session.paused.load(Ordering::Acquire);
			// Holding (paused, or before the first frame shows) freezes the media clock
			// at the time it reached: on entering the hold the elapsed offset moves into
			// `anchor_at` once, so a pause neither jumps back to the seek target
			// (Codex #78 P1) nor accumulates paused time.
			if paused || preview_needed {
				if !holding {
					anchor_at += now.saturating_duration_since(anchor).as_secs_f64();
					holding = true;
				}
				anchor = now;
			} else {
				holding = false;
			}
			let wall = anchor_at + now.duration_since(anchor).as_secs_f64();
			let audio_position = target
				+ position.load(Ordering::Acquire) as f64 / f64::from(info.sample_rate.max(1));
			let audio_drained = audio_ended
				&& pending_audio.is_none()
				&& position.load(Ordering::Acquire) >= queued_audio;
			let current = if output.is_some() && !audio_drained {
				audio_position
			} else {
				wall
			};
			if current > previous_position {
				last_progress = now;
				previous_position = current;
			}
			let mut frame = None;
			while frames
				.front()
				.is_some_and(|(pts, _, _, _)| *pts <= current + 0.01 || preview_needed)
			{
				let (_, width, height, rgba) = frames.pop_front().expect("front exists");
				// Pixel conversion stays on this worker; the render thread only swaps buffers.
				frame = Some(frame_image(width, height, rgba.as_slice())?);
				preview_needed = false;
			}
			if frame.is_some() {
				if !first_frame_logged {
					first_frame_logged = true;
					vlog!(
						&session.log_budget,
						"first_frame: ready in {:.3} ms",
						start_time.elapsed().as_secs_f64() * 1000.0
					);
				}
				last_progress = now;
			}
			let finished = video_ended && audio_drained && frames.is_empty();
			let mut changed = false;
			if let Ok(mut update) = session.update.try_lock().or_else(|_| session.update.lock()) {
				let state = if finished {
					VideoState::Ended
				} else if paused {
					VideoState::Paused
				} else if preview_needed
					|| now.duration_since(last_progress) > Duration::from_millis(250)
				{
					VideoState::Loading
				} else {
					VideoState::Playing
				};
				changed = update.state != state
					|| (update.position * 10.) as u64 != (current.min(info.duration) * 10.) as u64
					|| frame.is_some();
				update.state = state;
				update.position = current.min(info.duration);
				update.duration = info.duration;
				if frame.is_some() {
					update.frame = frame;
				}
			}
			if changed {
				ctx.request_repaint();
			}
			if finished {
				return Ok(());
			}
			if paused && !preview_needed {
				last_progress = now;
				std::thread::sleep(Duration::from_millis(20));
				continue;
			}
			// The stall bound also covers a missing first frame: while the decoder
			// opens but never yields a sample, `preview_needed` stays true and the
			// clock never advances, so without this the player sits in Loading
			// forever (Codex PR #34 P2).
			if stall_timed_out(paused, now.duration_since(last_progress)) {
				if stall_restarts == 0 {
					stall_restarts += 1;
					vlog!(
						&session.log_budget,
						"watchdog: stalled for 10s at position {:.3}s, restarting decoder",
						current
					);
					drop(output);
					target = current;
					seeking = true;
					last_progress = Instant::now();
					if let Ok(mut update) =
						session.update.try_lock().or_else(|_| session.update.lock())
					{
						update.frame = None;
						update.state = VideoState::Loading;
						update.position = target;
					}
					ctx.request_repaint();
					continue 'seek;
				} else {
					vlog!(
						&session.log_budget,
						"watchdog: stalled again at position {:.3}s, failing",
						current
					);
					return Err("Video buffering stalled; retry or download to play externally");
				}
			}
			if let Some((samples, offset)) = &mut pending_audio {
				let output = output.as_mut().ok_or("Unexpected video audio track")?;
				while *offset < samples.len() && output.producer.push(samples[*offset]).is_ok() {
					*offset += 1;
					queued_audio += 1;
				}
				if *offset == samples.len() {
					pending_audio = None;
				}
			}
			let mut decoded = false;
			// Request each track independently so short/missing audio cannot hold video EOF hostage.
			if !audio_ended && !paused && pending_audio.is_none() {
				let sample = decoder.poll_audio()?;
				decoded |= sample.is_ready();
				match sample {
					Ready(Some(Sample::Audio {
						pts,
						frames: samples,
					})) => {
						let packet_start =
							((pts - target) * f64::from(info.sample_rate)).round() as i64;
						let skip = (queued_audio as i64 - packet_start).max(0) as usize;
						if packet_start > queued_audio as i64 + info.sample_rate as i64 * 2 {
							return Err("Unsupported video audio timing");
						}
						let gap = (packet_start - queued_audio as i64).max(0) as usize;
						if gap > 0 {
							let mut padded = vec![[0.; 2]; gap];
							padded.extend_from_slice(&samples);
							pending_audio = Some((padded, 0));
						} else if skip < samples.len() {
							pending_audio = Some((samples, skip));
						}
					}
					Ready(None) => {
						audio_ended = true;
						eof.store(true, Ordering::Release);
					}
					Pending => {}
					_ => return Err("Unexpected video audio track"),
				}
			}
			// Two frames (<=16 MiB) ahead, plus one bounded audio packet and one second of PCM.
			if !video_ended && frames.len() < 2 {
				let sample = decoder.poll_video()?;
				decoded |= sample.is_ready();
				match sample {
					Ready(Some(Sample::Video {
						pts,
						width,
						height,
						rgba,
					})) => {
						// Frames before the target are dropped (the decoder went back to
						// the previous keyframe); the first one at or after it shows.
						if frame_reaches(pts, target) {
							seek_preview = None;
							frames.push_back((pts, width, height, rgba));
						} else {
							seek_preview = Some((target, width, height, rgba));
						}
					}
					Ready(None) => {
						video_ended = true;
						if preview_needed && let Some(frame) = seek_preview.take() {
							frames.push_back(frame);
						}
					}
					Pending => {}
					_ => return Err("Unexpected video track"),
				}
			}
			if !decoded {
				std::thread::sleep(Duration::from_millis(5));
			}
		}
	}
}

#[cfg(test)]
mod frame_image_tests {
	use super::*;
	use std::time::{Duration, Instant};

	#[test]
	fn oversized_frames_are_scaled_into_the_preview_box() {
		let (w, h) = (4500u32, 3000u32);
		let rgba = vec![120u8; w as usize * h as usize * 4];
		let started = Instant::now();
		let image = frame_image(w, h, &rgba).expect("a large frame is scaled, not refused");
		let elapsed = started.elapsed();
		assert_eq!(image.size, [1620, 1080]);
		assert_eq!(image.pixels.len(), 1620 * 1080);
		// Debug build bound; the release build is several times faster and the Windows
		// normal path never reaches this safety net.
		assert!(
			elapsed < Duration::from_secs(1),
			"downscale took {elapsed:?}"
		);
		println!("4500x3000 worker downscale: {elapsed:?}");
	}

	#[test]
	fn small_frames_keep_their_size_and_portrait_scales_to_its_own_box() {
		let (w, h) = (640u32, 360u32);
		let image =
			frame_image(w, h, &vec![200u8; w as usize * h as usize * 4]).expect("small frame");
		assert_eq!(image.size, [640, 360]);
		let (w, h) = (2160u32, 3840u32);
		let image =
			frame_image(w, h, &vec![80u8; w as usize * h as usize * 4]).expect("portrait frame");
		assert_eq!(image.size, [1080, 1920]);
	}
}

#[cfg(all(test, feature = "demo"))]
mod tests {
	use super::*;
	use std::time::{Duration, Instant};

	/// Synthetic local clip only; zero-volume output, no account or microphone access.
	#[test]
	#[ignore = "NIVRA_VIDEO_SAMPLE supplies an offline clip; opens muted local output"]
	fn local_video_keeps_up_with_realtime() {
		let path = std::env::var("NIVRA_VIDEO_SAMPLE").expect("NIVRA_VIDEO_SAMPLE path");
		assert!(std::fs::metadata(&path).unwrap().len() <= 100 * 1024 * 1024);
		let bytes = std::fs::read(path).unwrap();
		let session = Arc::new(Session::new(0., 1));
		let worker_session = session.clone();
		let started = Instant::now();
		let thread = std::thread::spawn(move || {
			let decoder = platform::video::Decoder::open(Box::new(std::io::Cursor::new(bytes)))?;
			play_decoded(
				decoder,
				&worker_session,
				&eframe::egui::Context::default(),
				started,
			)
		});
		while !thread.is_finished() {
			let update = session.update.lock().unwrap();
			let deadline = update.duration + 5.;
			drop(update);
			if started.elapsed().as_secs_f64() > deadline {
				session.cancelled.store(true, Ordering::Release);
				let _ = thread.join();
				panic!("playback could not keep up with realtime");
			}
			std::thread::sleep(Duration::from_millis(20));
		}
		assert_eq!(thread.join().unwrap(), Ok(()));
		let update = session.update.lock().unwrap();
		assert_eq!(update.state, VideoState::Ended);
		assert!(update.position >= update.duration - 0.1);
		eprintln!(
			"{:.3}s clip played in {:.3}s",
			update.duration,
			started.elapsed().as_secs_f64()
		);
	}

	#[test]
	#[ignore = "opens the local audio output device at zero volume; explicit offline playback check"]
	fn inline_video_plays_pauses_seeks_and_cancels() {
		let runtime = tokio::runtime::Runtime::new().unwrap();
		let session = Arc::new(Session::new(0., 2));
		let request = Request {
			session: session.clone(),
			url: None,
			fallback: None,
			size: 120000,
		};
		let handle = runtime.handle().clone();
		let thread =
			std::thread::spawn(move || play(&request, &handle, &eframe::egui::Context::default()));
		let wait = |predicate: &dyn Fn(&Update) -> bool| {
			let start = Instant::now();
			loop {
				let update = session.update.lock().unwrap();
				assert!(
					!matches!(update.state, VideoState::Failed(_)),
					"{:?}",
					update.state
				);
				if predicate(&update) {
					break;
				}
				drop(update);
				assert!(
					start.elapsed() < Duration::from_secs(8),
					"playback timed out"
				);
				std::thread::sleep(Duration::from_millis(20));
			}
		};
		wait(&|s| s.position > 0.2 && s.frame.is_some());
		session.paused.store(true, Ordering::Release);
		wait(&|s| s.state == VideoState::Paused);
		let before = session.update.lock().unwrap().position;
		std::thread::sleep(Duration::from_millis(100));
		assert!((session.update.lock().unwrap().position - before).abs() < 0.03);
		session.seek.store(2990, Ordering::Release);
		wait(&|s| s.position >= 2.98 && s.frame.is_some());
		session.seek.store(1500, Ordering::Release);
		wait(&|s| (1.49..1.6).contains(&s.position) && s.frame.is_some());
		session.paused.store(false, Ordering::Release);
		wait(&|s| s.position > 1.7);
		session.cancelled.store(true, Ordering::Release);
		let result = thread.join().unwrap();
		assert!(result.is_ok(), "{result:?}");
		for bytes in [
			include_bytes!("../tests/fixtures/video-silent.mov").as_slice(),
			include_bytes!("../tests/fixtures/video-short-audio.mov").as_slice(),
		] {
			let session = Arc::new(Session::new(0., 3));
			let worker_session = session.clone();
			let thread = std::thread::spawn(move || {
				let decoder =
					platform::video::Decoder::open(Box::new(std::io::Cursor::new(bytes))).unwrap();
				play_decoded(
					decoder,
					&worker_session,
					&eframe::egui::Context::default(),
					Instant::now(),
				)
			});
			let start = Instant::now();
			while !thread.is_finished() {
				assert!(
					start.elapsed() < Duration::from_secs(8),
					"video tail stalled"
				);
				std::thread::sleep(Duration::from_millis(20));
			}
			assert!(thread.join().unwrap().is_ok());
			assert_eq!(session.update.lock().unwrap().state, VideoState::Ended);
			assert!(session.update.lock().unwrap().position > 2.8);
		}
	}
}

#[cfg(test)]
mod attachment_url_tests {
	use super::*;
	#[test]
	fn video_worker_slots_are_bounded_and_released() {
		let live = AtomicUsize::new(0);
		for _ in 0..MAX_VIDEO_WORKERS {
			assert!(try_acquire_bounded(&live, MAX_VIDEO_WORKERS));
		}
		assert!(
			!try_acquire_bounded(&live, MAX_VIDEO_WORKERS),
			"a fifth concurrent open is refused"
		);
		assert_eq!(live.load(Ordering::Acquire), MAX_VIDEO_WORKERS);
		live.fetch_sub(1, Ordering::AcqRel);
		assert!(
			try_acquire_bounded(&live, MAX_VIDEO_WORKERS),
			"a freed slot is reusable"
		);
		live.fetch_sub(1, Ordering::AcqRel);
	}
	#[test]
	fn busy_video_open_fails_fast_without_blocking_the_render_thread() {
		let _turn = VIDEO_START_LOCK
			.lock()
			.unwrap_or_else(|poison| poison.into_inner());
		source::OFFLINE_PROBE.store(true, Ordering::Release);
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(1)
			.enable_all()
			.build()
			.unwrap();
		// Every worker slot counts as busy (0 = at the limit) without stealing the
		// process-wide slots from tests running in parallel.
		TEST_WORKER_LIMIT.store(0, Ordering::Release);
		let attachment = model::Attachment {
			id: model::Id(9001),
			filename: "busy.mp4".into(),
			description: None,
			content_type: Some("video/mp4".into()),
			size: 1024,
			spoiler: false,
			duration_ms: None,
			waveform: Vec::new(),
			media: model::EmbedMedia {
				url: Some("https://cdn.discordapp.com/attachments/1/9001/busy.mp4?ex=68dc&is=68db&hm=abc&backend=b2".into()),
				..Default::default()
			},
		};
		let mut video = Video::default();
		let started = std::time::Instant::now();
		let error = video
			.start(
				attachment,
				0.0,
				runtime.handle(),
				&eframe::egui::Context::default(),
				false,
				true,
			)
			.expect_err("full slots refuse");
		assert!(
			started.elapsed() < Duration::from_millis(100),
			"refusal must not wait"
		);
		assert!(error.contains("busy"));
		TEST_WORKER_LIMIT.store(MAX_VIDEO_WORKERS, Ordering::Release);
		source::OFFLINE_PROBE.store(false, Ordering::Release);
	}
	#[test]
	fn diagnostic_budget_caps_lines_per_playback() {
		let budget = AtomicUsize::new(2);
		vlog(&budget, format_args!("one"));
		vlog(&budget, format_args!("two"));
		assert_eq!(budget.load(Ordering::Acquire), 0);
		vlog(&budget, format_args!("three is dropped"));
		assert_eq!(
			budget.load(Ordering::Acquire),
			0,
			"over-budget lines are suppressed"
		);
	}
	#[test]
	fn stall_watchdog_covers_a_missing_first_frame() {
		assert!(stall_timed_out(false, Duration::from_secs(11)));
		assert!(!stall_timed_out(false, Duration::from_secs(5)));
		assert!(
			!stall_timed_out(true, Duration::from_secs(60)),
			"paused players never trip the watchdog"
		);
	}
	#[test]
	fn decoder_timeout_surfaces_failed_instead_of_sticking_in_loading() {
		let session = Session::new(0., 1);
		// Ordinary decode error with a live session: reported.
		assert!(report_worker_error(&session));
		// Voluntary stop: a cancelled session stays quiet, no Failed state.
		session.cancelled.store(true, Ordering::Release);
		assert!(!report_worker_error(&session));
		// Watchdog stall: cancelled by the 20 s timer, but the timeout flag
		// distinguishes it from a voluntary stop, so the player shows Failed.
		session.open_timed_out.store(true, Ordering::Release);
		assert!(report_worker_error(&session));
	}

	#[test]
	fn frames_before_the_seek_target_are_dropped() {
		// The decoder comes back from the previous keyframe; only the frame at or
		// after the target may show, so image and sound start together.
		assert!(!frame_reaches(4.98, 5.0));
		assert!(frame_reaches(5.0, 5.0));
		assert!(frame_reaches(5.04, 5.0));
		for target in [3.0, 30.0, 300.0, 3000.0] {
			assert!(!frame_reaches(target - 0.05, target));
			assert!(frame_reaches(target + 0.01, target));
		}
	}

	#[test]
	fn play_accepts_attachment_url_with_backend() {
		let raw = "https://cdn.discordapp.com/attachments/1395223214048673894/2/oobe-intro.mp4?ex=68dc&is=68db&hm=abc&backend=b2";
		let attachment = model::Attachment {
			id: model::Id(2),
			filename: "oobe-intro.mp4".into(),
			description: None,
			content_type: Some("video/mp4".into()),
			size: 1024,
			spoiler: false,
			duration_ms: None,
			waveform: Vec::new(),
			media: model::EmbedMedia {
				url: Some(raw.into()),
				..Default::default()
			},
		};
		let resolved = crate::downloads::original_url(&attachment).expect("signed url");
		assert!(resolved.as_str().contains("backend=b2"));
		source::OFFLINE_PROBE.store(true, Ordering::Release);
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(1)
			.enable_all()
			.build()
			.unwrap();
		let mut player = VideoUi::default();
		let mut video = Video::default();
		video.command(
			VideoCommand::Play(attachment),
			&mut player,
			runtime.handle(),
			&eframe::egui::Context::default(),
			false,
		);
		assert!(
			!matches!(
				player.state,
				VideoState::Failed("Video attachment unavailable")
			),
			"{:?}",
			player.state
		);
		video.stop();
		drop(video);
		std::thread::sleep(std::time::Duration::from_millis(50));
		source::OFFLINE_PROBE.store(false, Ordering::Release);
		drop(runtime);
	}

	#[test]
	fn embed_video_plays_in_app_without_a_webview() {
		let file = "https://media.discordapp.net/external/video.twimg.com/ext/oobe-intro.mp4";
		let attachment = model::Attachment {
			id: model::Id(1),
			filename: "oobe-intro.mp4".into(),
			description: None,
			content_type: Some("video/mp4".into()),
			// An embed preview has no attachment record, so it carries no size.
			size: 0,
			spoiler: false,
			duration_ms: None,
			waveform: Vec::new(),
			media: model::EmbedMedia {
				url: Some(file.into()),
				..Default::default()
			},
		};
		source::OFFLINE_PROBE.store(true, Ordering::Release);
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(1)
			.enable_all()
			.build()
			.unwrap();
		let mut player = VideoUi::default();
		let mut video = Video::default();
		video.command(
			VideoCommand::Play(attachment),
			&mut player,
			runtime.handle(),
			&eframe::egui::Context::default(),
			false,
		);
		assert!(
			!matches!(
				player.state,
				VideoState::Failed("Video attachment unavailable")
					| VideoState::Failed("Video preview limit: 100 MiB")
			),
			"{:?}",
			player.state
		);
		video.stop();
		drop(video);
		std::thread::sleep(std::time::Duration::from_millis(50));
		source::OFFLINE_PROBE.store(false, Ordering::Release);
		drop(runtime);
	}

	#[test]
	fn embed_from_a_host_outside_the_allowlist_is_refused_by_name() {
		let attachment = model::Attachment {
			id: model::Id(1),
			filename: "clip.mp4".into(),
			description: None,
			content_type: Some("video/mp4".into()),
			size: 0,
			spoiler: false,
			duration_ms: None,
			waveform: Vec::new(),
			media: model::EmbedMedia {
				url: Some("https://evil.test/external/clip.mp4".into()),
				..Default::default()
			},
		};
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(1)
			.enable_all()
			.build()
			.unwrap();
		let mut player = VideoUi::default();
		let mut video = Video::default();
		video.command(
			VideoCommand::Play(attachment),
			&mut player,
			runtime.handle(),
			&eframe::egui::Context::default(),
			false,
		);
		assert_eq!(
			player.state,
			VideoState::Failed("Unsupported embed video provider or URL")
		);
		drop(video);
		drop(runtime);
	}

	#[test]
	fn opening_subsequent_video_cancels_previous_without_blocking() {
		let _turn = VIDEO_START_LOCK
			.lock()
			.unwrap_or_else(|poison| poison.into_inner());
		source::OFFLINE_PROBE.store(true, Ordering::Release);
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(2)
			.enable_all()
			.build()
			.unwrap();
		let mut player = VideoUi::default();
		let mut video = Video::default();
		let make_attachment = |id: u64| model::Attachment {
			id: model::Id(id),
			filename: format!("video_{id}.mp4"),
			description: None,
			content_type: Some("video/mp4".into()),
			size: 1024,
			spoiler: false,
			duration_ms: None,
			waveform: Vec::new(),
			media: model::EmbedMedia {
				url: Some(format!(
					"https://cdn.discordapp.com/attachments/1/{id}/video_{id}.mp4?ex=68dc&is=68db&hm=abc&backend=b2"
				)),
				..Default::default()
			},
		};
		// Start first video
		video.command(
			VideoCommand::Play(make_attachment(1)),
			&mut player,
			runtime.handle(),
			&eframe::egui::Context::default(),
			false,
		);
		let session1 = video.session.as_ref().unwrap().clone();
		assert!(!session1.cancelled.load(Ordering::Acquire));

		// Start second video immediately
		video.command(
			VideoCommand::Play(make_attachment(2)),
			&mut player,
			runtime.handle(),
			&eframe::egui::Context::default(),
			false,
		);
		let session2 = video.session.as_ref().unwrap().clone();
		assert!(session1.cancelled.load(Ordering::Acquire));
		assert!(!session2.cancelled.load(Ordering::Acquire));
		assert_ne!(session1.id, session2.id);

		video.stop();
		assert!(session2.cancelled.load(Ordering::Acquire));
		source::OFFLINE_PROBE.store(false, Ordering::Release);
	}

	#[test]
	fn opening_and_closing_50_videos_releases_resources() {
		let _turn = VIDEO_START_LOCK
			.lock()
			.unwrap_or_else(|poison| poison.into_inner());
		source::OFFLINE_PROBE.store(true, Ordering::Release);
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(2)
			.enable_all()
			.build()
			.unwrap();
		let mut player = VideoUi::default();
		let mut video = Video::default();
		let mut sessions = Vec::new();
		for i in 1..=50 {
			let attachment = model::Attachment {
				id: model::Id(i),
				filename: format!("video_{i}.mp4"),
				description: None,
				content_type: Some("video/mp4".into()),
				size: 1024,
				spoiler: false,
				duration_ms: None,
				waveform: Vec::new(),
				media: model::EmbedMedia {
					url: Some(format!(
						"https://cdn.discordapp.com/attachments/1/{i}/video_{i}.mp4?ex=68dc&is=68db&hm=abc&backend=b2"
					)),
					..Default::default()
				},
			};
			video.command(
				VideoCommand::Play(attachment.clone()),
				&mut player,
				runtime.handle(),
				&eframe::egui::Context::default(),
				false,
			);
			// A tight loop outruns exiting workers; production clicks never do.
			// Bounded retries (in the test only, never on the render thread)
			// prove slots recycle instead of leaking.
			for _ in 0..200 {
				if video.session.is_some() {
					break;
				}
				std::thread::sleep(Duration::from_millis(1));
				video.command(
					VideoCommand::Play(attachment.clone()),
					&mut player,
					runtime.handle(),
					&eframe::egui::Context::default(),
					false,
				);
			}
			let s = video.session.clone().expect("session created");
			sessions.push(s);
		}
		assert_eq!(sessions.len(), 50);
		video.stop();
		for s in &sessions {
			assert!(s.cancelled.load(Ordering::Acquire));
		}
		source::OFFLINE_PROBE.store(false, Ordering::Release);
	}

	#[test]
	fn worker_panic_is_caught_safely_and_next_video_plays() {
		let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
		let session = Arc::new(Session::new(1.0, session_id));
		let worker_session = session.clone();
		let ctx = eframe::egui::Context::default();
		let worker = std::thread::spawn(move || {
			let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
				panic!("simulated decoder panic");
			}));
			if result.is_err() && !worker_session.cancelled.load(Ordering::Acquire) {
				if let Ok(mut update) = worker_session.update.lock() {
					update.state = VideoState::Failed("The video could not be decoded safely.");
				}
				ctx.request_repaint();
			}
		});
		worker.join().unwrap();
		let update = session.update.lock().unwrap();
		assert_eq!(
			update.state,
			VideoState::Failed("The video could not be decoded safely.")
		);
	}
}

/// End-to-end media-path tests. They run in CI without a live account or audio
/// device: a local HTTP server speaks Discord CDN shapes (signed query, 206 +
/// `Content-Range`, `video/mp4`, an expired primary that must fall back to the
/// proxy) and the `null-sink` output id replaces the audio device with a
/// real-time drain that advances the same position clock.
#[cfg(test)]
mod player_tests {
	use super::*;
	use std::io::{Read, Write};
	use std::net::{TcpListener, TcpStream};
	use std::sync::{Arc, Mutex};
	use std::time::Instant;

	/// One local CDN: ranged media, an expired primary and a never-answering path.
	type Seen = Arc<Mutex<Vec<(String, Option<String>)>>>;
	struct Cdn {
		origin: String,
		requests: Seen,
	}

	fn start_cdn(bytes: &'static [u8]) -> Cdn {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let origin = format!("http://{}", listener.local_addr().unwrap());
		let requests = Arc::new(Mutex::new(Vec::new()));
		let seen = requests.clone();
		std::thread::spawn(move || {
			for stream in listener.incoming() {
				let Ok(stream) = stream else { continue };
				let seen = seen.clone();
				std::thread::spawn(move || serve(stream, bytes, seen));
			}
		});
		Cdn { origin, requests }
	}

	fn serve(mut stream: TcpStream, bytes: &'static [u8], seen: Seen) {
		let mut header = Vec::new();
		let mut byte = [0u8; 1];
		while stream.read_exact(&mut byte).is_ok() {
			header.push(byte[0]);
			if header.ends_with(b"\r\n\r\n") {
				break;
			}
			if header.len() > 8192 {
				return;
			}
		}
		let text = String::from_utf8_lossy(&header).to_string();
		let path = text
			.lines()
			.next()
			.and_then(|line| line.split_whitespace().nth(1))
			.unwrap_or("/")
			.to_string();
		let range = text.lines().find_map(|line| {
			let (name, value) = line.split_once(':')?;
			name.eq_ignore_ascii_case("range")
				.then(|| value.trim().to_string())
		});
		seen.lock().unwrap().push((path.clone(), range.clone()));
		if path.starts_with("/expired") {
			let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
			return;
		}
		if path.starts_with("/stuck") {
			// A network that accepts and never answers: the media worker must stay
			// blocked here while the render thread keeps drawing fast frames.
			std::thread::sleep(Duration::from_secs(60));
			return;
		}
		let total = bytes.len();
		let (start, end) = match range.as_deref().and_then(parse_range) {
			Some((start, end)) => (start, end.min(total.saturating_sub(1))),
			None => (0, total.saturating_sub(1)),
		};
		let body = &bytes[start.min(total)..=end];
		let response = format!(
			"HTTP/1.1 206 Partial Content\r\nContent-Type: video/mp4\r\nAccept-Ranges: bytes\r\nContent-Range: bytes {start}-{end}/{total}\r\nContent-Length: {}\r\n\r\n",
			body.len()
		);
		let _ = stream.write_all(response.as_bytes());
		let _ = stream.write_all(body);
	}

	fn parse_range(value: &str) -> Option<(usize, usize)> {
		let value = value.strip_prefix("bytes=")?;
		let (start, end) = value.split_once('-')?;
		Some((start.parse().ok()?, end.parse().ok()?))
	}

	/// The null sink and its frame counter are process-wide, so the media-path
	/// tests take turns.
	static SINK_LOCK: Mutex<()> = Mutex::new(());

	#[test]
	fn cdn_style_mp4_shows_a_poster_plays_with_audio_and_seeks() {
		let _turn = SINK_LOCK
			.lock()
			.unwrap_or_else(|poison| poison.into_inner());
		discord_api::ensure_tls_provider();
		let bytes: &'static [u8] = include_bytes!("../tests/fixtures/video.mov");
		let cdn = start_cdn(bytes);
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(2)
			.enable_all()
			.build()
			.unwrap();
		let session = Arc::new(Session::new(0., 41));
		// Poster open: paused until the user presses play. Pinning the "voice output"
		// to the null sink also keeps the default-device migration check quiet.
		session.paused.store(true, Ordering::Release);
		*session.output.lock().unwrap() = Some("null-sink".into());
		let primary = url::Url::parse(&format!(
			"{}/expired/attachments/1395223214048673894/2/oobe-intro1.mp4?ex=68dc&is=68db&hm=abc",
			cdn.origin
		))
		.unwrap();
		let proxy = url::Url::parse(&format!(
			"{}/media/attachments/1395223214048673894/2/oobe-intro1.mp4?ex=68dc&is=68db&hm=abc",
			cdn.origin
		))
		.unwrap();
		let request = Request {
			session: session.clone(),
			url: Some(primary),
			fallback: Some(proxy),
			size: bytes.len(),
		};
		let handle = runtime.handle().clone();
		let outcome: Arc<Mutex<Option<Result<(), &'static str>>>> = Arc::new(Mutex::new(None));
		let outcome_slot = outcome.clone();
		let worker = {
			let session = session.clone();
			let request = request.clone();
			std::thread::spawn(move || {
				let ctx = eframe::egui::Context::default();
				let result = play(&request, &handle, &ctx);
				// Cancellation during shutdown is a clean stop.
				let result = if session.cancelled.load(Ordering::Acquire) {
					Ok(())
				} else {
					result
				};
				*outcome_slot.lock().unwrap() = Some(result);
				result
			})
		};
		// A worker that stops early reports its exact error instead of a timeout.
		let wait = |limit: Duration, label: &str, ready: &dyn Fn(&Update) -> bool| {
			wait_for(&session, &outcome, limit, label, ready)
		};
		// Poster: the first frame must be decoded and staged while still paused.
		let poster_ms = wait(Duration::from_secs(2), "poster frame", &|update| {
			update.frame.is_some() && update.state == VideoState::Paused
		});
		// Seek while still paused: the worker consumes the flag at the top of its loop
		// before playback can reach the end, so the jump is observed deterministically
		// even when a starved test thread misses the early position (macOS main flake).
		session.seek.store(1500, Ordering::Release);
		wait(Duration::from_secs(3), "seek to 1.5 s", &|update| {
			(1.45..1.75).contains(&update.position) && update.frame.is_some()
		});
		// Play: audio is decoded into the null sink and the position advances past the jump.
		session.paused.store(false, Ordering::Release);
		wait(Duration::from_secs(4), "position with sound", &|update| {
			update.position > 1.6
		});
		let audio_frames = output::NULL_SINK_FRAMES.load(Ordering::Acquire);
		assert!(
			audio_frames > 480,
			"decoded audio reached the sink: {audio_frames}"
		);
		// Ends on its own and releases the worker.
		wait(Duration::from_secs(6), "clip end", &|update| {
			update.state == VideoState::Ended
		});
		assert_eq!(worker.join().unwrap(), Ok(()));
		let requests = cdn.requests.lock().unwrap().clone();
		let status_shapes = requests
			.iter()
			.filter(|(path, _)| path.starts_with("/media"))
			.count();
		assert!(status_shapes > 1, "ranged reads: {requests:?}");
		assert!(
			requests
				.iter()
				.any(|(path, range)| path.starts_with("/expired") && range.is_some()),
			"the expired primary is probed before the proxy fallback"
		);
		assert!(
			requests
				.iter()
				.any(|(path, range)| path.starts_with("/media") && range.is_some()),
			"ranged proxy reads: {requests:?}"
		);
		eprintln!(
			"video-evidence host={} status=206 bytes={} codec=h264+aac first_frame_ms={} seek=1500ms ended=ok",
			cdn.origin.trim_start_matches("http://"),
			bytes.len(),
			poster_ms
		);
	}

	#[test]
	fn a_stuck_media_path_never_blocks_the_render_thread() {
		let _turn = SINK_LOCK
			.lock()
			.unwrap_or_else(|poison| poison.into_inner());
		discord_api::ensure_tls_provider();
		let bytes: &'static [u8] = include_bytes!("../tests/fixtures/video.mov");
		let cdn = start_cdn(bytes);
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(2)
			.enable_all()
			.build()
			.unwrap();
		let session = Arc::new(Session::new(0., 42));
		*session.output.lock().unwrap() = Some("null-sink".into());
		let request = Request {
			session: session.clone(),
			url: Some(url::Url::parse(&format!("{}/stuck/media.mp4", cdn.origin)).unwrap()),
			fallback: None,
			size: bytes.len(),
		};
		let handle = runtime.handle().clone();
		let worker = {
			let request = request.clone();
			std::thread::spawn(move || {
				let ctx = eframe::egui::Context::default();
				play(&request, &handle, &ctx)
			})
		};
		// The worker is parked in the media read; the render thread must keep
		// running its per-frame media entry point well under a frame budget.
		let mut video = Video {
			session: Some(session.clone()),
			worker: None,
		};
		let mut player = VideoUi::default();
		let ctx = eframe::egui::Context::default();
		let mut samples = Vec::with_capacity(40);
		// A fixed count keeps the check independent of runner load; each iteration is
		// one render frame entry point while the media worker stays parked.
		for _ in 0..40 {
			let started = Instant::now();
			video.poll(&mut player, &ctx, None);
			samples.push(started.elapsed());
			std::thread::sleep(Duration::from_millis(5));
		}
		// One descheduled sample is runner noise; the 90th percentile is the frame cost.
		samples.sort_unstable();
		let p90 = samples[samples.len() * 9 / 10];
		assert!(
			p90 < Duration::from_millis(50),
			"media work reached the render thread: p90 frame {p90:?}"
		);
		assert!(
			samples.iter().sum::<Duration>() < Duration::from_millis(500),
			"frames stayed cheap in total: {:?}",
			samples.iter().sum::<Duration>()
		);
		assert!(
			!worker.is_finished(),
			"the media worker is still blocked (the test must exercise a stall)"
		);
		// Cancel: the stuck read observes cancellation and the worker exits.
		video.stop();
		let _ = worker.join();
	}

	#[test]
	fn pausing_a_silent_clip_keeps_its_media_time() {
		let _turn = SINK_LOCK
			.lock()
			.unwrap_or_else(|poison| poison.into_inner());
		discord_api::ensure_tls_provider();
		// A clip with no audio runs on the wall-clock path; pausing must hold the
		// position instead of jumping back to the seek anchor (Codex #78 P1).
		let bytes: &'static [u8] = include_bytes!("../tests/fixtures/video-silent.mov");
		let cdn = start_cdn(bytes);
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(2)
			.enable_all()
			.build()
			.unwrap();
		let session = Arc::new(Session::new(0., 43));
		*session.output.lock().unwrap() = Some("null-sink".into());
		session.paused.store(true, Ordering::Release);
		let request = Request {
			session: session.clone(),
			url: Some(url::Url::parse(&format!("{}/media/silent.mov", cdn.origin)).unwrap()),
			fallback: None,
			size: bytes.len(),
		};
		let handle = runtime.handle().clone();
		let outcome: Arc<Mutex<Option<Result<(), &'static str>>>> = Arc::new(Mutex::new(None));
		let outcome_slot = outcome.clone();
		let worker = {
			let request = request.clone();
			std::thread::spawn(move || {
				let ctx = eframe::egui::Context::default();
				let result = play(&request, &handle, &ctx);
				*outcome_slot.lock().unwrap() = Some(result);
				result
			})
		};
		let wait = |limit: Duration, label: &str, ready: &dyn Fn(&Update) -> bool| {
			wait_for(&session, &outcome, limit, label, ready)
		};
		wait(Duration::from_secs(2), "poster", &|update| {
			update.frame.is_some() && update.state == VideoState::Paused
		});
		session.paused.store(false, Ordering::Release);
		wait(Duration::from_secs(4), "progress", &|update| {
			update.position > 0.5
		});
		session.paused.store(true, Ordering::Release);
		// Wait for the worker to publish the paused state, so `held` is the frozen
		// media time rather than the last playing sample.
		wait(Duration::from_secs(2), "paused", &|update| {
			update.state == VideoState::Paused
		});
		let held = session.update.lock().unwrap().position;
		assert!(held > 0.4, "paused after progress: {held}");
		std::thread::sleep(Duration::from_millis(600));
		let after = session.update.lock().unwrap();
		assert!(
			(after.position - held).abs() < 0.05 && after.position > 0.4,
			"a paused clock holds its media time: {held} -> {}",
			after.position
		);
		assert_eq!(after.state, VideoState::Paused);
		drop(after);
		// Resuming continues from the held position, not from zero.
		session.paused.store(false, Ordering::Release);
		wait(Duration::from_secs(4), "resume", &|update| {
			update.position > held + 0.2
		});
		session.cancelled.store(true, Ordering::Release);
		let _ = worker.join();
	}

	#[test]
	fn accesskit_labels_do_not_relock_the_context() {
		// The real app enables accesskit. Any `widget_info` closure that looks up a
		// translated string takes the Context read lock while egui holds the write
		// lock, which froze the whole window on 1.0.10 (owner P0, 2026-10-03).
		discord_api::ensure_tls_provider();
		let ctx = eframe::egui::Context::default();
		ctx.enable_accesskit();
		let message = test_support::message(1, model::Id(2));
		let attachment = model::Attachment {
			id: model::Id(3),
			filename: "oobe-intro1.mp4".into(),
			description: None,
			content_type: Some("video/mp4".into()),
			size: 4096,
			media: model::EmbedMedia {
				width: 1080,
				height: 1920,
				..Default::default()
			},
			spoiler: false,
			duration_ms: Some(4000),
			waveform: Vec::new(),
		};
		let mut view = ui::VideoUi::default();
		view.begin(&message, &attachment, true);
		// Playing with a control bar: the state that drew the freezing closure.
		view.state = ui::VideoState::Playing;
		view.position = 1.0;
		view.duration = 4.0;
		let mut download = ui::DownloadUi::default();
		let mut opening = None;
		let screen = egui::vec2(800.0, 600.0);
		let mut stage = egui::Rect::NOTHING;
		let center = std::cell::Cell::new(egui::Pos2::ZERO);
		let mut frame = |pointer: Option<egui::Pos2>| {
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
					focused: true,
					events: pointer.into_iter().map(egui::Event::PointerMoved).collect(),
					..Default::default()
				},
				|ui| {
					stage = view
						.show(ui, &message, &attachment, &mut download, &mut opening, true)
						.rect;
					center.set(stage.center());
				},
			)
		};
		frame(None).drop_without_applying_deltas();
		// Hovering the stage keeps the control bar (and its slider) visible, which is
		// the state the real app froze in.
		let output = frame(Some(center.get()));
		assert!(
			output.platform_output.accesskit_update.is_some(),
			"accesskit must build a tree while the controls render"
		);
		output.drop_without_applying_deltas();
	}

	/// Waits for a state the player publishes through its shared update slot, with the
	/// last observation in the panic so a CI failure is diagnosable without a rerun.
	fn wait_for(
		session: &Session,
		outcome: &Arc<Mutex<Option<Result<(), &'static str>>>>,
		limit: Duration,
		label: &str,
		ready: &dyn Fn(&Update) -> bool,
	) -> u128 {
		let started = Instant::now();
		while started.elapsed() < limit {
			if let Some(Err(error)) = *outcome.lock().unwrap() {
				panic!("media worker failed while waiting for {label}: {error}");
			}
			if let Ok(update) = session.update.try_lock()
				&& ready(&update)
			{
				return started.elapsed().as_millis();
			}
			std::thread::sleep(Duration::from_millis(10));
		}
		let last = session
			.update
			.try_lock()
			.map(|update| {
				format!(
					"state={:?} position={:.2} frame={} audio={}",
					update.state,
					update.position,
					update.frame.is_some(),
					output::NULL_SINK_FRAMES.load(Ordering::Acquire)
				)
			})
			.unwrap_or_else(|_| "update slot busy".into());
		panic!("timed out waiting for {label}; last update: {last}");
	}
}
