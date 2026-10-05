//! One bounded, lazy audio worker. Bundled cues never interrupt attachment playback.
use cpal::traits::{DeviceTrait, StreamTrait};
use model::notification_preferences::Sound;
use std::{
	io::Cursor,
	sync::{
		Arc,
		atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
		mpsc::{self, SyncSender},
	},
	time::{Duration, Instant},
};

// Each complete ringtone plus a short gap; shared with the automatic call timers.
pub const RING_INTERVAL: Duration = Duration::from_secs(6);
pub const OUTGOING_RING_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct Sounds {
	send: Option<SyncSender<Cue>>,
	/// Stop epoch: only `stop` advances it. Accepted cues are never preempted
	/// by newer cues, so an ordered queue plays in full, in order.
	generation: Arc<AtomicU64>,
	status: Arc<AtomicU8>,
}
/// One queued cue: what to play, how loud, and on which output. The output is
/// resolved per cue so a changed default (or a changed voice setting) applies
/// to the next sound without migrating live streams.
struct Cue {
	epoch: u64,
	sound: Sound,
	volume: u8,
	output: Option<String>,
}
impl Sounds {
	pub fn status(&self) -> &'static str {
		match self.status.load(Ordering::Acquire) {
			1 => "Playing notification sound...",
			2 => "Audio output unavailable. Check your system sound settings.",
			_ => "",
		}
	}
	pub fn stop(&mut self) {
		self.generation.fetch_add(1, Ordering::AcqRel);
		self.status.store(0, Ordering::Release);
	}
	/// Returns whether the worker accepted the cue. A full queue leaves the caller to retry.
	/// `output` is the voice output selection (`None` = system default).
	pub fn play(
		&mut self,
		sound: Sound,
		volume: u8,
		ctx: &eframe::egui::Context,
		output_id: Option<&str>,
	) -> bool {
		if self.send.is_none() {
			let (send, receive) = mpsc::sync_channel::<Cue>(8);
			let generation = self.generation.clone();
			let status = self.status.clone();
			let context = ctx.clone();
			if std::thread::Builder::new()
				.name("nivra-notification-audio".into())
				.spawn(move || {
					while let Ok(cue) = receive.recv() {
						// Only an explicit stop cuts a cue: newer cues never preempt queued ones.
						if generation.load(Ordering::Acquire) != cue.epoch {
							continue;
						}
						let finished = Arc::new(AtomicBool::new(false));
						match open(
							cue.sound,
							cue.volume,
							cue.output.as_deref(),
							generation.clone(),
							cue.epoch,
							status.clone(),
							finished.clone(),
						) {
							Ok((stream, duration)) => {
								let deadline = Instant::now() + duration + Duration::from_secs(2);
								while !finished.load(Ordering::Acquire) && Instant::now() < deadline
								{
									if generation.load(Ordering::Acquire) != cue.epoch {
										break;
									}
									std::thread::sleep(Duration::from_millis(20));
								}
								drop(stream);
								if generation.load(Ordering::Acquire) == cue.epoch {
									let _ = status.compare_exchange(
										1,
										if finished.load(Ordering::Acquire) {
											0
										} else {
											2
										},
										Ordering::AcqRel,
										Ordering::Acquire,
									);
								}
							}
							Err(()) => {
								if generation.load(Ordering::Acquire) == cue.epoch {
									status.store(2, Ordering::Release);
								}
							}
						}
						context.request_repaint();
					}
				})
				.is_err()
			{
				self.status.store(2, Ordering::Release);
				return false;
			}
			self.send = Some(send);
		}
		let epoch = self.generation.load(Ordering::Acquire);
		self.status.store(1, Ordering::Release);
		if self
			.send
			.as_ref()
			.expect("worker started")
			.try_send(Cue {
				epoch,
				sound,
				volume,
				output: output_id.map(str::to_owned),
			})
			.is_err()
		{
			self.status.store(0, Ordering::Release);
			return false;
		}
		true
	}
}
impl Drop for Sounds {
	fn drop(&mut self) {
		self.stop();
	}
}

/// Every notification cue, in the order the settings screen lists them.
#[allow(dead_code)] // Exercised by the decode-budget test.
const ALL_SOUNDS: [Sound; 14] = [
	Sound::Message,
	Sound::CurrentChannel,
	Sound::IncomingRing,
	Sound::OutgoingRing,
	Sound::Mute,
	Sound::Unmute,
	Sound::MemberMute,
	Sound::MemberUnmute,
	Sound::Deafen,
	Sound::Undeafen,
	Sound::CameraOn,
	Sound::ScreenShareOn,
	Sound::UserJoin,
	Sound::UserLeave,
];

fn samples(sound: Sound, rate: u32, current: &impl Fn() -> bool) -> Result<Vec<[f32; 2]>, ()> {
	let bytes: &[u8] = match sound {
		Sound::Message => include_bytes!("../../../assets/sounds/nivra/message.ogg"),
		Sound::CurrentChannel => {
			include_bytes!("../../../assets/sounds/nivra/current-channel.ogg")
		}
		Sound::IncomingRing => include_bytes!("../../../assets/sounds/nivra/incoming-ring.ogg"),
		Sound::OutgoingRing => include_bytes!("../../../assets/sounds/nivra/outgoing-ring.ogg"),
		Sound::Mute => include_bytes!("../../../assets/sounds/nivra/mute.ogg"),
		Sound::Unmute => include_bytes!("../../../assets/sounds/nivra/unmute.ogg"),
		// Member toggles reuse the local mute/unmute cues, played at a lower volume.
		Sound::MemberMute => include_bytes!("../../../assets/sounds/nivra/mute.ogg"),
		Sound::MemberUnmute => include_bytes!("../../../assets/sounds/nivra/unmute.ogg"),
		Sound::Deafen => include_bytes!("../../../assets/sounds/nivra/deafen.ogg"),
		Sound::Undeafen => include_bytes!("../../../assets/sounds/nivra/undeafen.ogg"),
		Sound::CameraOn => include_bytes!("../../../assets/sounds/nivra/camera-on.ogg"),
		Sound::ScreenShareOn => {
			include_bytes!("../../../assets/sounds/nivra/screen-share-on.ogg")
		}
		Sound::UserJoin => include_bytes!("../../../assets/sounds/nivra/user-join.ogg"),
		Sound::UserLeave => include_bytes!("../../../assets/sounds/nivra/user-leave.ogg"),
	};
	if bytes.len() > 128 * 1024 || !(8000..=192000).contains(&rate) {
		return Err(());
	}
	let mut pcm = Vec::new();
	let mut source_rate = 0;
	crate::audio::decode_stream(
		Box::new(Cursor::new(bytes)),
		current,
		&mut |chunk, channels, rate, _| {
			if channels != 2
				|| !matches!(rate, 44100 | 48000)
				|| (source_rate != 0 && source_rate != rate)
				|| pcm.len() + chunk.len() > rate as usize * 2 * 6
			{
				return Err("Invalid bundled notification sound");
			}
			source_rate = rate;
			pcm.extend(chunk.iter().map(|sample| {
				if sample.is_finite() {
					sample.clamp(-1.0, 1.0)
				} else {
					0.0
				}
			}));
			Ok(())
		},
	)
	.map_err(|_| ())?;
	if !current() || pcm.is_empty() {
		return Err(());
	}
	let frames = pcm.len() / 2;
	// ponytail: linear rate conversion; use a band-limited resampler if quality measurements require it.
	Ok((0..(frames * rate as usize).div_ceil(source_rate as usize))
		.map(|frame| {
			let position = frame as f64 * f64::from(source_rate) / f64::from(rate);
			let index = (position as usize).min(frames - 1);
			let next = (index + 1).min(frames - 1);
			std::array::from_fn(|channel| {
				let a = pcm[index * 2 + channel];
				let b = pcm[next * 2 + channel];
				a + (b - a) * (position - index as f64) as f32
			})
		})
		.collect())
}
fn open(
	sound: Sound,
	volume: u8,
	output_id: Option<&str>,
	generation: Arc<AtomicU64>,
	request: u64,
	status: Arc<AtomicU8>,
	finished: Arc<AtomicBool>,
) -> Result<(cpal::Stream, Duration), ()> {
	// Every interface sound follows the configured call output (or the system
	// default while the choice is "Default"); see discord_voice::output.
	let device = discord_voice::output::device(&cpal::default_host(), output_id).ok_or(())?;
	let supported = device.default_output_config().map_err(|_| ())?;
	let config = supported.config();
	if !(8000..=192000).contains(&config.sample_rate) || !(1..=8).contains(&config.channels) {
		return Err(());
	}
	let samples = samples(sound, config.sample_rate, &|| {
		generation.load(Ordering::Acquire) == request
	})?;
	let duration = Duration::from_secs_f64(samples.len() as f64 / f64::from(config.sample_rate));
	let stream = match supported.sample_format() {
		cpal::SampleFormat::F32 => output::<f32>(
			&device,
			config,
			samples,
			volume,
			(generation, request),
			status,
			finished,
		),
		cpal::SampleFormat::I16 => output::<i16>(
			&device,
			config,
			samples,
			volume,
			(generation, request),
			status,
			finished,
		),
		cpal::SampleFormat::I32 => output::<i32>(
			&device,
			config,
			samples,
			volume,
			(generation, request),
			status,
			finished,
		),
		cpal::SampleFormat::U16 => output::<u16>(
			&device,
			config,
			samples,
			volume,
			(generation, request),
			status,
			finished,
		),
		_ => return Err(()),
	}
	.map_err(|_| ())?;
	stream.play().map_err(|_| ())?;
	Ok((stream, duration))
}
fn output<T: cpal::SizedSample + cpal::FromSample<f32>>(
	device: &cpal::Device,
	config: cpal::StreamConfig,
	samples: Vec<[f32; 2]>,
	volume: u8,
	(generation, request): (Arc<AtomicU64>, u64),
	status: Arc<AtomicU8>,
	finished: Arc<AtomicBool>,
) -> Result<cpal::Stream, cpal::Error> {
	let errors = generation.clone();
	let failed = finished.clone();
	device.build_output_stream(
		config,
		callback::<T>(config, samples, volume, generation, request, finished),
		move |_| {
			failed.store(true, Ordering::Release);
			if errors.load(Ordering::Acquire) == request {
				status.store(2, Ordering::Release);
			}
		},
		None,
	)
}
fn callback<T: cpal::SizedSample + cpal::FromSample<f32>>(
	config: cpal::StreamConfig,
	samples: Vec<[f32; 2]>,
	volume: u8,
	generation: Arc<AtomicU64>,
	request: u64,
	finished: Arc<AtomicBool>,
) -> impl FnMut(&mut [T], &cpal::OutputCallbackInfo) + Send + 'static {
	let mut position = 0;
	let mut end = None;
	let gain = (f32::from(volume) / 100.0).clamp(0.0, 1.0);
	move |data: &mut [T], info| {
		data.fill(T::from_sample(0.0));
		if generation.load(Ordering::Acquire) != request {
			return;
		}
		let timestamp = info.timestamp();
		if end.is_some_and(|end| timestamp.callback >= end) {
			finished.store(true, Ordering::Release);
			return;
		}
		for frame in data.chunks_exact_mut(usize::from(config.channels)) {
			let Some(sample) = samples.get(position) else {
				break;
			};
			if frame.len() == 1 {
				frame[0] = T::from_sample((sample[0] + sample[1]) * 0.5 * gain);
			} else {
				for (target, value) in frame.iter_mut().zip(sample) {
					*target = T::from_sample(*value * gain);
				}
			}
			position += 1;
		}
		if position == samples.len() && end.is_none() {
			// Wait until the final buffer reaches the device, including cold-start latency.
			end = timestamp.playback.checked_add(Duration::from_secs_f64(
				data.len() as f64 / f64::from(config.channels) / f64::from(config.sample_rate),
			));
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn every_bundled_cue_decodes_and_stays_inside_the_playback_budget() {
		for sound in ALL_SOUNDS {
			let pcm = samples(sound, 48_000, &|| true)
				.unwrap_or_else(|()| panic!("{sound:?} was rejected by the decoder"));
			assert!(
				pcm.len() <= 48_000 * 6,
				"{sound:?} decodes to {} frames, past the six second cap",
				pcm.len()
			);
			assert!(
				pcm.iter()
					.any(|frame| frame[0].abs() > 0.001 || frame[1].abs() > 0.001),
				"{sound:?} decoded to silence"
			);
		}
	}
	#[test]
	fn callback_drains_delayed_output_and_cancellation_is_request_local() {
		let config = cpal::StreamConfig {
			channels: 4,
			sample_rate: 8000,
			buffer_size: cpal::BufferSize::Default,
		};
		let generation = Arc::new(AtomicU64::new(1));
		let finished = Arc::new(AtomicBool::new(false));
		let mut render = callback(
			config,
			vec![[0.2, 0.4]; 2],
			100,
			generation.clone(),
			1,
			finished.clone(),
		);
		let info = |callback_ms, playback_ms| {
			cpal::OutputCallbackInfo::new(cpal::OutputStreamTimestamp {
				callback: cpal::StreamInstant::from_millis(callback_ms),
				playback: cpal::StreamInstant::from_millis(playback_ms),
			})
		};
		let mut data = [1.0_f32; 8];
		render(&mut data, &info(0, 1000));
		assert_eq!(data, [0.2, 0.4, 0.0, 0.0, 0.2, 0.4, 0.0, 0.0]);
		render(&mut data, &info(500, 1500));
		assert_eq!(data, [0.0; 8]);
		assert!(!finished.load(Ordering::Acquire));
		render(&mut data, &info(1001, 2001));
		assert!(finished.load(Ordering::Acquire));

		let next_finished = Arc::new(AtomicBool::new(false));
		let mut next = callback(
			cpal::StreamConfig {
				channels: 1,
				..config
			},
			vec![[0.2, 0.4]; 2],
			100,
			generation.clone(),
			2,
			next_finished.clone(),
		);
		generation.store(2, Ordering::Release);
		let mut cancelled = callback(
			config,
			vec![[1.0, 1.0]; 2],
			100,
			generation.clone(),
			1,
			finished,
		);
		cancelled(&mut data, &info(2000, 3000));
		assert_eq!(data, [0.0; 8]);
		assert!(!next_finished.load(Ordering::Acquire));
		next(&mut data[..2], &info(2000, 3000));
		assert_eq!(&data[..2], &[0.3, 0.3]);
	}
	#[test]
	fn callback_applies_volume_gain() {
		let config = cpal::StreamConfig {
			channels: 2,
			sample_rate: 8000,
			buffer_size: cpal::BufferSize::Default,
		};
		let generation = Arc::new(AtomicU64::new(1));
		let finished = Arc::new(AtomicBool::new(false));
		let mut render = callback::<f32>(
			config,
			vec![[0.4, 0.8]],
			50,
			generation.clone(),
			1,
			finished.clone(),
		);
		let mut data = [0.0_f32; 2];
		let info = cpal::OutputCallbackInfo::new(cpal::OutputStreamTimestamp {
			callback: cpal::StreamInstant::from_millis(0),
			playback: cpal::StreamInstant::from_millis(10),
		});
		render(&mut data, &info);
		assert!((data[0] - 0.2).abs() < 1e-4);
		assert!((data[1] - 0.4).abs() < 1e-4);
	}
	#[test]
	fn bundled_cues_decode_in_full_at_supported_rates_and_cancel() {
		for rate in [8000, 44100, 48000, 192000] {
			let cues = [
				Sound::Message,
				Sound::CurrentChannel,
				Sound::IncomingRing,
				Sound::Mute,
				Sound::Unmute,
				Sound::Deafen,
				Sound::Undeafen,
				Sound::CameraOn,
				Sound::ScreenShareOn,
				Sound::OutgoingRing,
				Sound::UserJoin,
				Sound::UserLeave,
			]
			.map(|s| samples(s, rate, &|| true).unwrap());
			assert_ne!(cues[0], cues[1]);
			assert_ne!(cues[3], cues[4]);
			assert_ne!(cues[5], cues[6]);
			// Durations of the bundled Nivra cues, in the order listed above.
			// Durations of the bundled Nivra cues, in the order listed above.
			let expectations = [
				(0.28, 0.33),
				(0.21, 0.26),
				(3.02, 3.10),
				(0.20, 0.25),
				(0.20, 0.25),
				(0.23, 0.28),
				(0.22, 0.27),
				(0.18, 0.23),
				(0.29, 0.34),
				(1.70, 1.78),
				(0.25, 0.30),
				(0.25, 0.30),
			];
			// cues[9] is the outgoing ring, which repeats on OUTGOING_RING_INTERVAL.
			assert!(
				Duration::from_secs_f64(cues[9].len() as f64 / f64::from(rate))
					+ Duration::from_millis(100)
					< OUTGOING_RING_INTERVAL
			);
			for (cue, (min, max)) in cues.iter().zip(expectations) {
				let seconds = cue.len() as f64 / f64::from(rate);
				assert!(
					(min..max).contains(&seconds),
					"unexpected cue duration {seconds}"
				);
				assert!(cue.len() <= rate as usize * 6);
				assert!(
					cue.iter()
						.flatten()
						.all(|s| s.is_finite() && s.abs() <= 1.0)
				);
				assert!(cue.iter().flatten().any(|s| s.abs() > 0.01));
				assert!(
					Duration::from_secs_f64(seconds) + Duration::from_millis(100) < RING_INTERVAL
				);
			}
		}
		assert!(samples(Sound::Message, 48000, &|| false).is_err());
		assert!(samples(Sound::Message, 0, &|| true).is_err());
	}
}
