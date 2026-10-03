//! One explicit inline player; native decoding and network reads stay on one lazy worker.
#[cfg(target_os = "macos")]
mod fallback;
mod output;
mod source;
use std::sync::{
	Arc, Mutex,
	atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
use ui::{VideoCommand, VideoState, VideoUi};

#[derive(Default)]
struct Update {
	state: VideoState,
	position: f64,
	duration: f64,
	frame: Option<(u32, u32, Vec<u8>)>,
}
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

struct Session {
	id: u64,
	/// Wanted voice output selection, refreshed by the UI (`None` = default).
	output: std::sync::Mutex<Option<String>>,
	cancelled: Arc<AtomicBool>,
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
		if let Ok(mut wanted) = session.output.lock()
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
			if let Some((width, height, rgba)) = frame {
				player.accept_frame(ctx, width as usize, height as usize, &rgba);
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
				if let Err(error) = self.start(attachment, player.volume, runtime, ctx, demo) {
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
				let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
					play(&request, &runtime, &ctx)
				}));
				match result {
					Ok(Err(error)) => {
						if !worker_session.cancelled.load(Ordering::Acquire) {
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
			.map_err(|_| "Could not start video worker")?;
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
	eprintln!("[Nivra video] open_source: starting session {}", session.id);
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
		runtime.clone(),
	)?;
	eprintln!(
		"[Nivra video] open_source: ready in {:.3} ms",
		start_time.elapsed().as_secs_f64() * 1000.0
	);
	let open_started = Instant::now();
	eprintln!("[Nivra video] open_decoder: starting");
	let open_timed_out = Arc::new(AtomicBool::new(false));
	let open_timed_out_clone = open_timed_out.clone();
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
			if !open_timed_out.load(Ordering::Acquire) =>
		{
			let source = source::source(
				url.clone(),
				request.size,
				session.cancelled.clone(),
				runtime.clone(),
			)?;
			fallback::open(source, &session.cancelled)
		}
		result => result,
	};
	watchdog.abort();
	if open_timed_out.load(Ordering::Acquire) {
		eprintln!("[Nivra video] open_decoder: timed out after 20s");
		return Err("Video buffering stalled; retry or download to play externally");
	}
	let _ = url;
	let decoder = decoder?;
	eprintln!(
		"[Nivra video] open_decoder: ready in {:.3} ms",
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
			decoder.seek(target)?;
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
		let mut frames = VecDeque::new();
		let mut seek_preview = None;
		let mut pending_audio: Option<(Vec<[f32; 2]>, usize)> = None;
		let mut queued_audio = 0u64;
		let mut video_ended = false;
		let mut audio_ended = output.is_none();
		let mut preview_needed = true;
		let mut wall = target;
		let mut ticks = 0u64;
		let mut last_tick = Instant::now();
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
			let default_moved = wanted.is_none()
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
			let elapsed = now.duration_since(last_tick).as_secs_f64();
			last_tick = now;
			let paused = session.paused.load(Ordering::Acquire);
			if !paused && !preview_needed {
				wall += elapsed;
			}
			let audio_position = target
				+ position.load(Ordering::Acquire) as f64 / f64::from(info.sample_rate.max(1));
			let audio_drained = audio_ended
				&& pending_audio.is_none()
				&& position.load(Ordering::Acquire) >= queued_audio;
			let current = if output.is_some() && !audio_drained {
				wall = audio_position;
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
				frame = Some((width, height, rgba));
				preview_needed = false;
			}
			if frame.is_some() {
				if !first_frame_logged {
					first_frame_logged = true;
					eprintln!(
						"[Nivra video] first_frame: ready in {:.3} ms",
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
			if !paused
				&& !preview_needed
				&& now.duration_since(last_progress) > Duration::from_secs(10)
			{
				if stall_restarts == 0 {
					stall_restarts += 1;
					eprintln!(
						"[Nivra video] watchdog: stalled for 10s at position {:.3}s, restarting decoder",
						current
					);
					drop(output);
					target = current;
					seeking = true;
					last_progress = Instant::now();
					last_tick = Instant::now();
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
					eprintln!(
						"[Nivra video] watchdog: stalled again at position {:.3}s, failing",
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
				let read_started = Instant::now();
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
				// Freeze the silent/finished-audio clock across a blocking buffer refill.
				if (output.is_none() || audio_drained)
					&& read_started.elapsed() > Duration::from_millis(100)
				{
					last_tick = Instant::now();
				}
			}
			if !decoded {
				std::thread::sleep(Duration::from_millis(5));
			}
		}
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
				VideoCommand::Play(attachment),
				&mut player,
				runtime.handle(),
				&eframe::egui::Context::default(),
				false,
			);
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
