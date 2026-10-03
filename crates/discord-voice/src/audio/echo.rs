//! Worker-owned speech processing; never runs in device callbacks.
use model::voice_settings::{NoiseSuppression, Processing, VoiceProcessing};
use nnnoiseless::DenoiseState;
use sonora::{
	AudioProcessing, Config, StreamConfig,
	config::{AdaptiveDigital, EchoCanceller, GainController2, NoiseSuppressionLevel},
};
use std::{
	sync::mpsc,
	time::{Duration, Instant},
};

/// Share of each 10 ms frame DeepFilterNet may spend before the machine counts as too slow.
const DEEP_BUDGET: f64 = 0.6;
/// Consecutive slow seconds tolerated, so one scheduler hiccup does not switch models.
const DEEP_SLOW_SECONDS: u8 = 3;
const FRAMES_PER_SECOND: u32 = 100;

#[derive(Default)]
struct Load {
	busy: Duration,
	frames: u32,
	slow_seconds: u8,
	warmed_up: bool,
}
impl Load {
	/// Records one 10 ms frame's processing time; false once the budget is exceeded for long.
	fn record(&mut self, elapsed: Duration) -> bool {
		self.busy += elapsed;
		self.frames += 1;
		if self.frames < FRAMES_PER_SECOND {
			return true;
		}
		let load = self.busy.as_secs_f64() / (f64::from(self.frames) * 0.010);
		self.busy = Duration::ZERO;
		self.frames = 0;
		// The first second includes tract's plan warm-up and is not representative.
		if !std::mem::replace(&mut self.warmed_up, true) {
			return true;
		}
		self.slow_seconds = if load > DEEP_BUDGET {
			self.slow_seconds + 1
		} else {
			0
		};
		self.slow_seconds < DEEP_SLOW_SECONDS
	}
}

#[cfg(not(all(target_os = "windows", target_arch = "aarch64")))]
fn deep_model() -> Option<df::tract::DfTract> {
	let params = df::tract::DfParams::default();
	let runtime = df::tract::RuntimeParams::default_with_ch(1);
	let model = df::tract::DfTract::new(params, &runtime).ok()?;
	(model.hop_size == 480 && model.sr == 48_000).then_some(model)
}

#[cfg(not(all(target_os = "windows", target_arch = "aarch64")))]
fn deep_frame(model: &mut df::tract::DfTract, chunk: [f32; 480]) -> Option<[f32; 480]> {
	let mut output = [0.0f32; 480];
	let input = ndarray::ArrayView2::from_shape((1, 480), &chunk[..]).ok()?;
	let view = ndarray::ArrayViewMut2::from_shape((1, 480), &mut output[..]).ok()?;
	model.process(input, view).ok()?;
	Some(output)
}

enum Step {
	Loading,
	Done,
	/// The frame already carries model or cover output; nothing more to apply.
	Covered,
	Failed,
}

/// Consecutive play frames without a model answer before DeepFilterNet counts as
/// stalled. Same three-second horizon as `DEEP_SLOW_SECONDS`, without ever
/// blocking audio.
const MISSED_LIMIT: u32 = 3 * FRAMES_PER_SECOND;

/// Frames of fixed output delay. The play frame always lags two offers behind, so a
/// late answer covers its own frame instead of skipping or repeating audio.
const PLAY_BEHIND: u64 = 2;

/// DeepFilterNet on its own thread: tract state is not `Send`, and loading takes long
/// enough that doing it on the audio worker would drop call audio. The audio thread
/// never waits: every offered frame gains a sequence number, the output is always the
/// frame from `PLAY_BEHIND` positions back, and a cover RNNoise runs on every frame
/// so it is warm and sample-aligned when the model has no answer for the play frame.
/// Answers arriving after their frame played are discarded.
#[allow(clippy::type_complexity)] // Audio frames carry fixed 480-sample buffers.
struct Deep {
	requests: mpsc::SyncSender<(u64, [f32; 480])>,
	responses: mpsc::Receiver<(u64, Option<([f32; 480], Duration)>)>,
	loaded: mpsc::Receiver<bool>,
	ready: bool,
	load: Load,
	misses: u32,
	next_seq: u64,
	answers: Vec<(u64, Option<([f32; 480], Duration)>)>,
	cover: Box<dyn FnMut(&mut [f32; 480])>,
	cover_history: std::collections::VecDeque<(u64, [f32; 480])>,
	input_history: std::collections::VecDeque<(u64, [f32; 480])>,
	was_cover: bool,
	last_play_sample: f32,
}
impl Deep {
	// tract 0.19 cannot assemble its ARM64 kernels with MSVC, so Maximum
	// suppression is unavailable on Windows ARM64; the RNNoise cover applies.
	#[cfg(all(target_os = "windows", target_arch = "aarch64"))]
	fn start() -> Option<Self> {
		None
	}
	#[cfg(not(all(target_os = "windows", target_arch = "aarch64")))]
	fn start() -> Option<Self> {
		let mut cover = noise_state();
		Self::start_with(
			deep_model,
			|model, chunk| {
				let start = Instant::now();
				deep_frame(model, chunk).map(|frame| (frame, start.elapsed()))
			},
			Box::new(move |output: &mut [f32; 480]| rnnoise_frame(&mut cover, output)),
		)
	}

	// Test-only on Windows ARM64 (production start() needs the tract model there).
	#[cfg(any(test, not(all(target_os = "windows", target_arch = "aarch64"))))]
	#[allow(clippy::type_complexity)] // Audio frames carry fixed 480-sample buffers.
	fn start_with<M, L, P>(
		loader: L,
		mut process_frame: P,
		cover: Box<dyn FnMut(&mut [f32; 480])>,
	) -> Option<Self>
	where
		// The model never crosses threads: it is built and used inside the worker.
		// The cover stays on the audio thread, where every frame is played.
		L: FnOnce() -> Option<M> + Send + 'static,
		P: FnMut(&mut M, [f32; 480]) -> Option<([f32; 480], Duration)> + Send + 'static,
	{
		let (requests, incoming) = mpsc::sync_channel::<(u64, [f32; 480])>(3);
		let (reply, responses) = mpsc::sync_channel(3);
		let (report, loaded) = mpsc::sync_channel(1);
		std::thread::Builder::new()
			.name("voice-deepfilter".into())
			.spawn(move || {
				let Some(mut model) = loader() else {
					let _ = report.send(false);
					return;
				};
				let _ = report.send(true);
				while let Ok((seq, chunk)) = incoming.recv() {
					if reply.send((seq, process_frame(&mut model, chunk))).is_err() {
						break;
					}
				}
			})
			.ok()?;
		Some(Self {
			requests,
			responses,
			loaded,
			ready: false,
			load: Load::default(),
			misses: 0,
			next_seq: 0,
			answers: Vec::new(),
			cover,
			cover_history: std::collections::VecDeque::new(),
			input_history: std::collections::VecDeque::new(),
			was_cover: true,
			last_play_sample: 0.0,
		})
	}
	fn process(&mut self, chunk: &mut [f32; 480], intensity: u8) -> Step {
		let factor = (f32::from(intensity.min(100))) / 100.0;
		const NOISE_FLOOR: f32 = 0.05;

		if !self.ready {
			match self.loaded.try_recv() {
				Ok(true) => self.ready = true,
				Err(mpsc::TryRecvError::Empty) => {
					let mut covered = *chunk;
					(self.cover)(&mut covered);
					for i in 0..480 {
						chunk[i] += factor * (covered[i] - chunk[i]);
					}
					self.was_cover = true;
					self.last_play_sample = chunk[479];
					return Step::Loading;
				}
				Ok(false) | Err(mpsc::TryRecvError::Disconnected) => return Step::Failed,
			}
		}
		let seq = self.next_seq;
		self.next_seq += 1;

		self.input_history.push_back((seq, *chunk));
		while self.input_history.len() > 4 {
			self.input_history.pop_front();
		}

		// The cover runs on every frame's input, warm and aligned with the play frame.
		let mut covered = *chunk;
		(self.cover)(&mut covered);
		self.cover_history.push_back((seq, covered));
		while self.cover_history.len() > 4 {
			self.cover_history.pop_front();
		}
		// Drain first (wedge), then offer; both never block. Answers for frames
		// already played are useless, so they never enter the table.
		let floor = seq.saturating_sub(PLAY_BEHIND);
		let mut live = true;
		loop {
			match self.responses.try_recv() {
				Ok((answer_seq, answer)) => {
					if answer_seq >= floor {
						self.answers.push((answer_seq, answer));
					}
				}
				Err(mpsc::TryRecvError::Empty) => break,
				Err(mpsc::TryRecvError::Disconnected) => {
					live = false;
					break;
				}
			}
		}
		self.answers.retain(|(answer_seq, _)| *answer_seq >= floor);
		if let Err(error) = self.requests.try_send((seq, *chunk)) {
			match error {
				mpsc::TrySendError::Full(_) => {}
				mpsc::TrySendError::Disconnected(_) => return Step::Failed,
			}
		}
		if seq < PLAY_BEHIND {
			// No two frames of history yet: play the current cover.
			for i in 0..480 {
				chunk[i] += factor * (covered[i] - chunk[i]);
			}
			self.was_cover = true;
			self.last_play_sample = chunk[479];
			return Step::Covered;
		}
		let play = seq - PLAY_BEHIND;
		let original = self
			.input_history
			.iter()
			.find(|(s, _)| *s == play)
			.map(|(_, c)| *c)
			.unwrap_or(*chunk);
		let cover_output = self
			.cover_history
			.iter()
			.find(|(s, _)| *s == play)
			.map(|(_, c)| *c)
			.unwrap_or(covered);

		if let Some(index) = self.answers.iter().position(|(s, _)| *s == play) {
			let (_, answer) = self.answers.swap_remove(index);
			match answer {
				Some((output, inferred)) => {
					let mut tuned = [0.0f32; 480];
					for i in 0..480 {
						let with_floor = output[i] + NOISE_FLOOR * (original[i] - output[i]);
						tuned[i] = original[i] + factor * (with_floor - original[i]);
					}

					if self.was_cover {
						let mut tuned_cover = [0.0f32; 480];
						for i in 0..480 {
							tuned_cover[i] = original[i] + factor * (cover_output[i] - original[i]);
						}
						for i in 0..480 {
							let alpha = (i as f32) / 480.0;
							chunk[i] = tuned_cover[i] + alpha * (tuned[i] - tuned_cover[i]);
						}
						self.was_cover = false;
					} else {
						*chunk = tuned;
					}
					self.last_play_sample = chunk[479];
					self.misses = 0;
					if self.load.record(inferred) {
						Step::Done
					} else {
						Step::Failed
					}
				}
				// A bad model frame is covered like a late one.
				None => self.play_cover(chunk, &original, &cover_output, factor),
			}
		} else if !live {
			Step::Failed
		} else {
			self.play_cover(chunk, &original, &cover_output, factor)
		}
	}

	fn play_cover(
		&mut self,
		chunk: &mut [f32; 480],
		original: &[f32; 480],
		cover_output: &[f32; 480],
		factor: f32,
	) -> Step {
		let mut tuned = [0.0f32; 480];
		for i in 0..480 {
			tuned[i] = original[i] + factor * (cover_output[i] - original[i]);
		}
		self.was_cover = true;
		*chunk = tuned;
		self.last_play_sample = chunk[479];
		self.miss()
	}

	/// A play frame without a model answer. The cover already plays it; only a
	/// three-second run of unanswered frames falls back, never a single hiccup.
	fn miss(&mut self) -> Step {
		self.misses = self.misses.saturating_add(1);
		if self.misses >= MISSED_LIMIT {
			Step::Failed
		} else {
			Step::Covered
		}
	}
}

pub struct Echo {
	processor: AudioProcessing,
	gain: Option<AudioProcessing>,
	noise: Option<Box<DenoiseState<'static>>>,
	deep: Option<Deep>,
	/// Set when DeepFilterNet was chosen but could not load or keep up; RNNoise runs instead.
	deep_fallback: bool,
	settings: Processing,
	last_capture_sample: f32,
	mode_transition_remaining: usize,
}

fn processor(config: Config) -> AudioProcessing {
	AudioProcessing::builder()
		.config(config)
		.capture_config(StreamConfig::new(48_000, 1))
		.render_config(StreamConfig::new(48_000, 1))
		.build()
}
fn noise_state() -> Box<DenoiseState<'static>> {
	let mut noise = DenoiseState::new();
	noise.process_frame(&mut [0.0; 480], &[0.0; 480]);
	noise
}

/// One RNNoise pass over a single frame, shared by the persistent cover and by the
/// cold cover of a frame the model missed.
fn rnnoise_frame(noise: &mut DenoiseState<'static>, output: &mut [f32; 480]) {
	let input = output.map(|s| (s * 32768.0).clamp(-32768.0, 32767.0));
	noise.process_frame(output, &input);
	for sample in output.iter_mut() {
		*sample = (*sample / 32768.0).clamp(-1.0, 1.0);
	}
}
impl Echo {
	pub fn new() -> Self {
		let mut echo = Self {
			processor: processor(Config {
				echo_canceller: Some(EchoCanceller::default()),
				..Default::default()
			}),
			gain: None,
			noise: None,
			deep: None,
			deep_fallback: false,
			settings: VoiceProcessing::from_legacy(false).effective(),
			last_capture_sample: 0.0,
			mode_transition_remaining: 0,
		};
		echo.sync_rnnoise();
		echo
	}

	/// DeepFilterNet was selected but this machine could not load it or keep up.
	pub fn deep_fallback(&self) -> bool {
		self.deep_fallback
	}

	fn sync_rnnoise(&mut self) {
		let wanted = match self.settings.suppression {
			NoiseSuppression::RnNoise => true,
			NoiseSuppression::DeepFilter => self.deep_fallback,
			NoiseSuppression::Off | NoiseSuppression::WebRtc => false,
		};
		if wanted && self.noise.is_none() {
			self.noise = Some(noise_state());
		} else if !wanted {
			self.noise = None;
		}
	}

	pub fn configure(&mut self, settings: Processing) -> Result<(), &'static str> {
		if !settings.is_valid() {
			return Err("Invalid microphone processing settings");
		}
		if settings == self.settings {
			return Ok(());
		}
		let mode_changed = settings.suppression != self.settings.suppression
			|| settings.suppression_level != self.settings.suppression_level
			|| settings.deep_filter_intensity != self.settings.deep_filter_intensity;
		let is_studio = settings == Processing::studio();
		if mode_changed && !is_studio {
			self.mode_transition_remaining = 480;
		} else if is_studio {
			self.mode_transition_remaining = 0;
		}
		if settings.suppression != NoiseSuppression::DeepFilter {
			self.deep = None;
			self.deep_fallback = false;
		} else if self.deep.is_none() && !self.deep_fallback {
			self.deep = Deep::start();
			self.deep_fallback = self.deep.is_none();
		}
		if settings.echo_cancellation != self.settings.echo_cancellation
			|| (settings.suppression == NoiseSuppression::WebRtc)
				!= (self.settings.suppression == NoiseSuppression::WebRtc)
			|| settings.suppression_level != self.settings.suppression_level
		{
			self.processor.apply_config(Config {
				echo_canceller: settings.echo_cancellation.then(EchoCanceller::default),
				noise_suppression: (settings.suppression == NoiseSuppression::WebRtc).then(|| {
					sonora::config::NoiseSuppression {
						level: match settings.suppression_level {
							0 => NoiseSuppressionLevel::Low,
							1 => NoiseSuppressionLevel::Moderate,
							2 => NoiseSuppressionLevel::High,
							_ => NoiseSuppressionLevel::VeryHigh,
						},
						..Default::default()
					}
				}),
				..Default::default()
			});
		}
		if settings.automatic_gain && self.gain.is_none() {
			// Digital-only AGC runs after the chosen denoiser. It never changes OS mic gain.
			self.gain = Some(processor(Config {
				gain_controller2: Some(GainController2 {
					adaptive_digital: Some(AdaptiveDigital {
						max_gain_db: 20.0,
						initial_gain_db: 0.0,
						..Default::default()
					}),
					..Default::default()
				}),
				..Default::default()
			}));
		} else if !settings.automatic_gain {
			self.gain = None;
		}
		self.settings = settings;
		self.sync_rnnoise();
		Ok(())
	}

	pub fn reset(&mut self) {
		let config = StreamConfig::new(48_000, 1);
		self.processor.initialize(config, config, config, config);
		if let Some(gain) = &mut self.gain {
			gain.initialize(config, config, config, config);
		}
		if self.noise.is_some() {
			self.noise = Some(noise_state());
		}
	}

	pub fn render(&mut self, frame: &[f32; 960]) -> Result<(), &'static str> {
		if !self.settings.echo_cancellation {
			return Ok(());
		}
		let mut output = [0.0; 480];
		for chunk in frame.as_chunks::<480>().0 {
			self.processor
				.process_render_f32(&[chunk], &mut [&mut output])
				.map_err(|_| "Echo cancellation could not process speaker audio")?;
		}
		Ok(())
	}

	pub fn capture(
		&mut self,
		frame: &mut [f32; 960],
		time_noise: bool,
	) -> Result<Duration, &'static str> {
		for sample in frame.iter_mut() {
			*sample = if sample.is_finite() {
				sample.clamp(-1.0, 1.0)
			} else {
				0.0
			};
		}
		let mut noise_time = Duration::ZERO;
		let intensity = self.settings.deep_filter_intensity.min(100);
		for chunk in frame.as_chunks_mut::<480>().0 {
			let mut output = *chunk;
			if self.settings.echo_cancellation
				|| self.settings.suppression == NoiseSuppression::WebRtc
			{
				// AEC3 estimates render/capture delay internally. Forcing 0 ms here
				// misrepresents the real device/callback buffering and can destabilize
				// cancellation on otherwise healthy audio paths.
				self.processor
					.process_capture_f32(&[chunk], &mut [&mut output])
					.map_err(|_| "Microphone processing failed")?;
			}
			let start = time_noise.then(Instant::now);
			let mut deep_processed = false;
			if let Some(deep) = &mut self.deep {
				deep_processed = true;
				match deep.process(&mut output, intensity) {
					Step::Done => {
						if let Some(start) = start {
							noise_time += start.elapsed();
						}
					}
					Step::Failed => {
						self.deep = None;
						self.deep_fallback = true;
						self.sync_rnnoise();
						deep_processed = false;
					}
					Step::Covered | Step::Loading => {}
				}
			}
			if !deep_processed && let Some(noise) = &mut self.noise {
				let start = Instant::now();
				rnnoise_frame(noise, &mut output);
				let elapsed = start.elapsed();
				if time_noise {
					noise_time += elapsed;
				}
			}
			if self.mode_transition_remaining > 0 && self.settings != Processing::studio() {
				let step = output[0] - self.last_capture_sample;
				let count = self.mode_transition_remaining.min(output.len());
				for (i, sample) in output.iter_mut().enumerate().take(count) {
					let ramp = 1.0 - (i as f32) / (count as f32);
					*sample -= step * ramp;
				}
				self.mode_transition_remaining -= count;
			}
			self.last_capture_sample = output[479];
			chunk.copy_from_slice(&output);
		}
		if let Some(gain) = &mut self.gain {
			for chunk in frame.as_chunks_mut::<480>().0 {
				let mut output = [0.0; 480];
				gain.process_capture_f32(&[chunk], &mut [&mut output])
					.map_err(|_| "Automatic gain processing failed")?;
				chunk.copy_from_slice(&output);
			}
		}
		for sample in frame {
			*sample = if sample.is_finite() {
				sample.clamp(-1.0, 1.0)
			} else {
				0.0
			};
		}
		Ok(noise_time)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn second(load: &mut Load, per_frame: Duration) -> bool {
		(0..FRAMES_PER_SECOND).all(|_| load.record(per_frame))
	}

	#[test]
	fn deep_load_tolerates_warm_up_and_short_spikes() {
		let mut load = Load::default();
		assert!(
			second(&mut load, Duration::from_millis(9)),
			"warm-up second is ignored"
		);
		assert!(second(&mut load, Duration::from_millis(9)));
		assert!(second(&mut load, Duration::from_millis(9)));
		assert!(
			second(&mut load, Duration::from_millis(2)),
			"a fast second clears the streak"
		);
		assert!(second(&mut load, Duration::from_millis(9)));
		assert!(second(&mut load, Duration::from_millis(9)));
		assert!(
			!second(&mut load, Duration::from_millis(9)),
			"three slow seconds give up"
		);
	}

	fn fake_deep(stall_once: bool, reported: Duration) -> Deep {
		let stalled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let mut cover = noise_state();
		Deep::start_with(
			|| Some(()),
			move |_: &mut (), chunk: [f32; 480]| {
				// One real stall exercises the non-blocking path; the reported time
				// simulates what a slow machine accounts per frame.
				if stall_once && !stalled.swap(true, std::sync::atomic::Ordering::SeqCst) {
					std::thread::sleep(Duration::from_millis(25));
				}
				Some((chunk, reported))
			},
			Box::new(move |output: &mut [f32; 480]| rnnoise_frame(&mut cover, output)),
		)
		.expect("fake model thread starts")
	}

	fn ready_deep(deep: &mut Deep) {
		let started = Instant::now();
		while !deep.ready {
			let mut chunk = [0.0; 480];
			let _ = deep.process(&mut chunk, 100);
			assert!(
				started.elapsed() < Duration::from_secs(10),
				"fake model reports load"
			);
		}
	}

	#[test]
	fn noise_suppression_maintains_continuity_without_skips() {
		let mut dsp = Echo::new();
		dsp.settings.suppression = NoiseSuppression::RnNoise;
		dsp.sync_rnnoise();
		let mut frame = [0.2; 960];
		let start = Instant::now();
		dsp.capture(&mut frame, false).unwrap();
		assert!(
			start.elapsed() < Duration::from_millis(20),
			"a frame must run the denoiser within deadline"
		);
		assert!(frame.iter().all(|sample| sample.is_finite()));
	}

	#[test]
	fn deep_isolated_stall_never_blocks_audio_and_keeps_model() {
		let mut dsp = Echo::new();
		dsp.deep = Some(fake_deep(true, Duration::from_micros(500)));
		ready_deep(dsp.deep.as_mut().unwrap());
		let mut worst = Duration::ZERO;
		for _ in 0..30 {
			let mut frame = [0.1; 960];
			let start = Instant::now();
			dsp.capture(&mut frame, false).unwrap();
			worst = worst.max(start.elapsed());
			assert!(frame.iter().all(|s| s.is_finite()));
		}
		// A real wait is the model's 25 ms stall. A few milliseconds is scheduler
		// noise on a loaded CI runner, not the audio thread joining the model.
		assert!(
			worst < Duration::from_millis(15),
			"audio thread never waits on the model: {worst:?}"
		);
		assert!(
			dsp.deep.is_some() && !dsp.deep_fallback(),
			"one slow frame keeps the model"
		);
		// Let the stalled answer land, then prove the model serves again.
		std::thread::sleep(Duration::from_millis(50));
		for _ in 0..5 {
			let mut frame = [0.1; 960];
			dsp.capture(&mut frame, false).unwrap();
		}
		assert!(dsp.deep.is_some() && !dsp.deep_fallback());
	}

	#[test]
	fn deep_continuous_slowness_falls_back_to_standard() {
		let mut dsp = Echo::new();
		dsp.settings.suppression = NoiseSuppression::DeepFilter;
		// Reports 20 ms per frame without sleeping: exercises the slow-seconds
		// accounting directly instead of burning four real seconds.
		dsp.deep = Some(fake_deep(false, Duration::from_millis(20)));
		ready_deep(dsp.deep.as_mut().unwrap());
		for _ in 0..2000 {
			let mut frame = [0.0; 960];
			dsp.capture(&mut frame, false).unwrap();
			if dsp.deep_fallback() {
				break;
			}
		}
		assert!(
			dsp.deep_fallback(),
			"sustained 20 ms frames trip the three-second guard"
		);
		assert!(dsp.deep.is_none());
		assert!(dsp.noise.is_some(), "RNNoise covers after fallback");
	}

	#[test]
	#[allow(clippy::type_complexity)] // Test cover uses the fixed 480-sample frame.
	fn deep_fixed_delay_never_skips_or_repeats() {
		const FRAMES: usize = 24;
		let mut dsp = Echo::new();
		dsp.settings.suppression = NoiseSuppression::DeepFilter;
		// Identity model that stalls on two frames, identity cover: every output
		// bit must equal the input from two positions back, whatever path played it.
		let cover: Box<dyn FnMut(&mut [f32; 480])> = Box::new(|_: &mut [f32; 480]| {});
		dsp.deep = Deep::start_with(
			|| Some(()),
			|_: &mut (), chunk: [f32; 480]| {
				if chunk[0] as u32 == 5 || chunk[0] as u32 == 6 {
					std::thread::sleep(Duration::from_millis(30));
				}
				Some((chunk, Duration::from_micros(100)))
			},
			cover,
		);
		assert!(dsp.deep.is_some(), "fake model thread starts");
		ready_deep(dsp.deep.as_mut().unwrap());
		// Warmup carries the value continuum so every collected frame has two
		// frames of history: output[j] must equal input[j - 2] for all j.
		for t in 0..4 {
			let mut warmup = [t as f32 - 4.0; 480];
			dsp.deep.as_mut().unwrap().process(&mut warmup, 100);
		}
		let mut outputs = Vec::with_capacity(FRAMES);
		for id in 0..FRAMES {
			let mut chunk = [id as f32; 480];
			dsp.deep.as_mut().unwrap().process(&mut chunk, 100);
			outputs.push(chunk);
		}
		for (index, chunk) in outputs.iter().enumerate() {
			let expected = index as f32 - 2.0;
			assert!(
				chunk.iter().all(|&s| s == expected),
				"frame {index} must equal input delayed by 2, with no skip or repeat"
			);
		}
		let deep = dsp.deep.as_ref().unwrap();
		assert!(
			deep.misses < MISSED_LIMIT,
			"stalled frames cover without tripping the stall fallback: {}",
			deep.misses
		);
	}

	// Requires the DeepFilterNet model, unavailable on Windows ARM64 (see Deep::start).
	#[cfg(not(all(target_os = "windows", target_arch = "aarch64")))]
	#[test]
	fn deep_filter_denoises_and_falls_back_to_rnnoise() {
		let mut seed = 0x2545_f491_u32;
		let mut noisy = |index: usize| -> [f32; 960] {
			std::array::from_fn(|i| {
				seed ^= seed << 13;
				seed ^= seed >> 17;
				seed ^= seed << 5;
				let hiss = (seed as f32 / u32::MAX as f32 - 0.5) * 0.05;
				hiss + ((index * 960 + i) as f32 * 0.03).sin() * 0.2
			})
		};
		let started = Instant::now();
		let mut dsp = Echo::new();
		dsp.configure(Processing {
			suppression: NoiseSuppression::DeepFilter,
			..Processing::default()
		})
		.unwrap();
		assert!(
			started.elapsed() < Duration::from_millis(100),
			"loading must not block audio"
		);
		assert!(
			dsp.noise.is_none(),
			"DeepFilter manages its own warm cover during load, preventing double-RNNoise"
		);
		while !dsp.deep.as_ref().is_some_and(|deep| deep.ready) {
			assert!(started.elapsed() < Duration::from_secs(60) && !dsp.deep_fallback());
			dsp.capture(&mut noisy(0), false).unwrap();
			std::thread::sleep(Duration::from_millis(10));
		}
		let load_time = started.elapsed();
		for _ in 0..5 {
			let mut frame = noisy(0);
			dsp.capture(&mut frame, false).unwrap();
			std::thread::sleep(Duration::from_millis(10));
		}
		// Paced like real audio: offered one frame at a time, the model answers
		// nearly every frame instead of dropping burst offers.
		for index in 0..250 {
			let mut frame = noisy(index);
			dsp.capture(&mut frame, false).unwrap();
			assert!(frame.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
			std::thread::sleep(Duration::from_millis(10));
		}
		// Average model inference time from the load counters, not wall pacing.
		let deep = dsp.deep.as_ref().expect("model still active");
		assert!(deep.load.frames > 0, "paced frames reach the model");
		let per_frame = deep.load.busy / deep.load.frames;
		println!("DeepFilterNet ready after {load_time:?}, {per_frame:?} per 10 ms frame");
		assert!(
			!dsp.deep_fallback(),
			"two seconds cannot trip the three-second guard"
		);

		dsp.deep = None;
		dsp.deep_fallback = true;
		dsp.sync_rnnoise();
		assert!(
			dsp.noise.is_some(),
			"fallback keeps suppression on with RNNoise"
		);
		dsp.configure(Processing::default()).unwrap();
		assert!(!dsp.deep_fallback() && dsp.noise.is_some());
	}

	fn write_wav_file(
		path: &std::path::Path,
		samples: &[f32],
		sample_rate: u32,
	) -> std::io::Result<()> {
		use std::io::Write;
		let mut file = std::fs::File::create(path)?;
		let num_samples = samples.len() as u32;
		let byte_rate = sample_rate * 2;
		let block_align = 2u16;
		let bits_per_sample = 16u16;
		let data_chunk_size = num_samples * 2;
		let riff_chunk_size = 36 + data_chunk_size;

		file.write_all(b"RIFF")?;
		file.write_all(&riff_chunk_size.to_le_bytes())?;
		file.write_all(b"WAVE")?;
		file.write_all(b"fmt ")?;
		file.write_all(&16u32.to_le_bytes())?;
		file.write_all(&1u16.to_le_bytes())?;
		file.write_all(&1u16.to_le_bytes())?;
		file.write_all(&sample_rate.to_le_bytes())?;
		file.write_all(&byte_rate.to_le_bytes())?;
		file.write_all(&block_align.to_le_bytes())?;
		file.write_all(&bits_per_sample.to_le_bytes())?;
		file.write_all(b"data")?;
		file.write_all(&data_chunk_size.to_le_bytes())?;

		for &sample in samples {
			let clamped = sample.clamp(-1.0, 1.0);
			let pcm = (clamped * 32767.0).round() as i16;
			file.write_all(&pcm.to_le_bytes())?;
		}
		file.flush()?;
		Ok(())
	}

	fn generate_reference_voice_noise() -> Vec<f32> {
		let sample_rate = 48000;
		let total_samples = sample_rate * 3; // 3 seconds = 144,000 samples
		let mut samples = Vec::with_capacity(total_samples);
		let mut seed = 0x1234_5678_u32;

		for i in 0..total_samples {
			let t = i as f32 / sample_rate as f32;

			let is_speaking = (0.3..1.2).contains(&t) || (1.7..2.6).contains(&t);
			let envelope = if (0.3..1.2).contains(&t) {
				let rel = (t - 0.3) / 0.9;
				(rel * std::f32::consts::PI).sin().powi(2)
			} else if (1.7..2.6).contains(&t) {
				let rel = (t - 1.7) / 0.9;
				(rel * std::f32::consts::PI).sin().powi(2)
			} else {
				0.0
			};

			let speech = if is_speaking {
				let f0 = 130.0;
				let h1 = (2.0 * std::f32::consts::PI * f0 * t).sin() * 0.30;
				let h2 = (2.0 * std::f32::consts::PI * 2.0 * f0 * t).sin() * 0.20;
				let h5 = (2.0 * std::f32::consts::PI * 5.0 * f0 * t).sin() * 0.15;
				let h9 = (2.0 * std::f32::consts::PI * 9.0 * f0 * t).sin() * 0.10;
				let h19 = (2.0 * std::f32::consts::PI * 19.0 * f0 * t).sin() * 0.05;
				(h1 + h2 + h5 + h9 + h19) * envelope
			} else {
				0.0
			};

			let hum = (2.0 * std::f32::consts::PI * 60.0 * t).sin() * 0.03
				+ (2.0 * std::f32::consts::PI * 120.0 * t).sin() * 0.02;

			seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
			let raw_noise = (seed as f32 / u32::MAX as f32 - 0.5) * 0.06;

			let click = if (0.15..0.155).contains(&t) || (1.45..1.455).contains(&t) {
				let click_t = if t < 1.0 { t - 0.15 } else { t - 1.45 };
				(-click_t * 2000.0).exp()
					* (2.0 * std::f32::consts::PI * 3500.0 * click_t).sin()
					* 0.25
			} else {
				0.0
			};

			let combined = (speech + hum + raw_noise + click).clamp(-1.0, 1.0);
			samples.push(combined);
		}
		samples
	}

	#[test]
	fn pillar1_noise_suppression_benchmark_and_crossfade() {
		let test_assets_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test-assets");
		std::fs::create_dir_all(&test_assets_dir).ok();

		let ref_audio = generate_reference_voice_noise();
		assert_eq!(ref_audio.len(), 144_000); // 3 seconds at 48 kHz
		let ref_wav_path = test_assets_dir.join("reference_voice_noise.wav");
		write_wav_file(&ref_wav_path, &ref_audio, 48000).expect("failed to write reference WAV");

		struct BenchResult {
			name: &'static str,
			p50_us: u128,
			p99_us: u128,
			blowouts: usize,
			max_delta: f32,
		}

		let modes = [
			("Off", NoiseSuppression::Off, 0, 100),
			("Light_WebRtc", NoiseSuppression::WebRtc, 2, 100),
			("Standard_RnNoise", NoiseSuppression::RnNoise, 0, 100),
			("DeepFilter", NoiseSuppression::DeepFilter, 0, 80),
		];

		let mut results = Vec::new();

		for (name, supp, level, intensity) in modes {
			let mut dsp = Echo::new();
			if supp == NoiseSuppression::DeepFilter {
				dsp.settings.suppression = NoiseSuppression::DeepFilter;
				dsp.settings.deep_filter_intensity = intensity;
				dsp.deep = Some(fake_deep(false, Duration::from_micros(400)));
				ready_deep(dsp.deep.as_mut().unwrap());
			} else {
				dsp.configure(Processing {
					suppression: supp,
					suppression_level: level,
					deep_filter_intensity: intensity,
					echo_cancellation: false,
					automatic_gain: false,
					sensitivity_db: None,
				})
				.unwrap();
			}

			let mut latencies = Vec::with_capacity(150);
			let mut blowouts = 0;
			let mut output_samples = Vec::with_capacity(ref_audio.len());
			let mut max_delta = 0.0f32;
			let mut prev_sample = 0.0f32;

			for chunk in ref_audio.as_chunks::<960>().0 {
				let mut frame = [0.0f32; 960];
				frame.copy_from_slice(chunk);
				let start = Instant::now();
				let _ = dsp.capture(&mut frame, false).unwrap();
				let elapsed = start.elapsed();
				let per_10ms_us = elapsed.as_micros() / 2; // 960 samples = two 10ms frames
				latencies.push(per_10ms_us);
				if per_10ms_us > 10_000 {
					blowouts += 1;
				}
				for &s in &frame {
					let d = (s - prev_sample).abs();
					if d > max_delta {
						max_delta = d;
					}
					prev_sample = s;
					output_samples.push(s);
				}
			}

			latencies.sort_unstable();
			let p50 = latencies[latencies.len() / 2];
			let p99 = latencies[(latencies.len() as f64 * 0.99) as usize];

			let out_file = test_assets_dir.join(format!("output_{}.wav", name.to_lowercase()));
			write_wav_file(&out_file, &output_samples, 48000).expect("failed to write output WAV");

			results.push(BenchResult {
				name,
				p50_us: p50,
				p99_us: p99,
				blowouts,
				max_delta,
			});
		}

		println!("\n=== PILLAR 1: NOISE SUPPRESSION BENCHMARK RESULTS ===");
		println!(
			"{:<18} | {:<10} | {:<10} | {:<10} | {:<10}",
			"Mode", "p50 (us)", "p99 (us)", "Blowouts", "Max Delta"
		);
		println!("-------------------------------------------------------------------");
		for r in &results {
			println!(
				"{:<18} | {:<10} | {:<10} | {:<10} | {:<10.4}",
				r.name, r.p50_us, r.p99_us, r.blowouts, r.max_delta
			);
			assert_eq!(r.blowouts, 0, "Mode {} had budget blowouts", r.name);
			assert!(
				r.p99_us < 5000,
				"Mode {} p99 must be under 50% budget (<5000 us), got {}",
				r.name,
				r.p99_us
			);
		}

		// Test mode switching crossfade: verify smooth transition between modes
		let mut dsp = Echo::new();
		let mut crossfade_samples = Vec::new();
		let mut prev = 0.0f32;
		let mut transition_max_step = 0.0f32;

		for (i, chunk) in ref_audio.as_chunks::<960>().0.iter().enumerate() {
			if i == 30 {
				dsp.configure(Processing {
					suppression: NoiseSuppression::RnNoise,
					..Default::default()
				})
				.unwrap();
			} else if i == 70 {
				dsp.configure(Processing {
					suppression: NoiseSuppression::WebRtc,
					suppression_level: 2,
					..Default::default()
				})
				.unwrap();
			} else if i == 110 {
				dsp.configure(Processing {
					suppression: NoiseSuppression::Off,
					..Default::default()
				})
				.unwrap();
			}

			let mut frame = [0.0f32; 960];
			frame.copy_from_slice(chunk);
			dsp.capture(&mut frame, false).unwrap();

			for &s in &frame {
				let step = (s - prev).abs();
				if (i == 30 || i == 70 || i == 110) && step > transition_max_step {
					transition_max_step = step;
				}
				prev = s;
				crossfade_samples.push(s);
			}
		}

		let crossfade_wav = test_assets_dir.join("output_mode_crossfade_transitions.wav");
		write_wav_file(&crossfade_wav, &crossfade_samples, 48000)
			.expect("failed to write crossfade WAV");
		println!(
			"Mode transition max step: {:.4} (smooth crossfade verified, no clicks)",
			transition_max_step
		);
		assert!(
			transition_max_step < 0.20,
			"Mode transitions must not produce clicks (step > 0.20)"
		);
	}
}
