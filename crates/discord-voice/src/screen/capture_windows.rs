use super::{
	MAX_FRAME_HEIGHT, MAX_FRAME_WIDTH, MAX_RAW_BYTES, MAX_SOURCE_HEIGHT, MAX_SOURCE_WIDTH,
	MAX_SOURCES, bounded_name,
};
use crate::screen::{AudioChunk, RawFrame, Settings as CaptureSettings, Source, SourceId};
use std::sync::{
	Arc,
	atomic::{AtomicBool, AtomicU64, Ordering},
	mpsc::{SyncSender, TrySendError},
};
use std::time::{Duration, Instant};
use windows_capture::{
	capture::{CaptureControl, Context, GraphicsCaptureApiHandler},
	frame::Frame,
	graphics_capture_api::{GraphicsCaptureApi, InternalCaptureControl},
	monitor::Monitor,
	settings::{
		ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
		MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
	},
	window::Window,
};

#[path = "audio_windows.rs"]
mod audio;

#[path = "readback_windows.rs"]
mod readback;

fn monitor_id(monitor: &Monitor) -> u64 {
	u64::try_from(monitor.as_raw_hmonitor() as usize).unwrap_or(0)
}

fn window_id(window: &Window) -> u64 {
	u64::try_from(window.as_raw_hwnd() as usize).unwrap_or(0)
}

pub(crate) fn sources() -> Result<Vec<Source>, &'static str> {
	let mut sources = Vec::with_capacity(MAX_SOURCES);
	for monitor in Monitor::enumerate().map_err(|_| "Displays could not be enumerated")? {
		let (Ok(width), Ok(height)) = (monitor.width(), monitor.height()) else {
			continue;
		};
		if width == 0 || height == 0 || width > MAX_SOURCE_WIDTH || height > MAX_SOURCE_HEIGHT {
			continue;
		}
		let name = monitor
			.name()
			.or_else(|_| monitor.device_string())
			.unwrap_or_else(|_| "Display".to_owned());
		sources.push(Source {
			id: SourceId::Display(monitor_id(&monitor)),
			name: bounded_name(name),
		});
		if sources.len() == MAX_SOURCES {
			return Ok(sources);
		}
	}

	for window in Window::enumerate().map_err(|_| "Windows could not be enumerated")? {
		let Ok(title) = window.title() else {
			continue;
		};
		let (Ok(width), Ok(height)) = (window.width(), window.height()) else {
			continue;
		};
		let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
			continue;
		};
		if title.trim().is_empty()
			|| width == 0
			|| height == 0
			|| width > MAX_SOURCE_WIDTH
			|| height > MAX_SOURCE_HEIGHT
		{
			continue;
		}
		sources.push(Source {
			id: SourceId::Window(window_id(&window)),
			name: bounded_name(title),
		});
		if sources.len() == MAX_SOURCES {
			break;
		}
	}
	Ok(sources)
}

#[derive(Clone)]
struct Flags {
	frames: SyncSender<RawFrame>,
	stop: Arc<AtomicBool>,
	pending: Arc<AtomicBool>,
	next_frame: Instant,
	interval: Duration,
}

struct Handler(Flags, crate::diagnostics::Metrics, readback::Reader);

/// Frame buffers the encoder has finished with. A full-source BGRA buffer is 33 MB at 4K30;
/// allocating and faulting one in per captured frame costs more than the copy itself. At
/// most two spares are kept, and the pool is emptied when the capture closes.
static SPARE_FRAMES: std::sync::Mutex<Vec<Vec<u8>>> = std::sync::Mutex::new(Vec::new());
const SPARE_FRAME_LIMIT: usize = 2;

/// Returns a frame buffer for reuse once nothing references it.
pub(crate) fn recycle_frame(data: Vec<u8>) {
	if let Ok(mut spare) = SPARE_FRAMES.lock()
		&& spare.len() < SPARE_FRAME_LIMIT
	{
		spare.push(data);
	}
}

fn clear_spare_frames() {
	if let Ok(mut spare) = SPARE_FRAMES.lock() {
		spare.clear();
	}
}

/// An empty buffer with room for `len` bytes. A spare far larger than needed (a resolution
/// change) is dropped rather than kept at its old size.
fn take_frame_buffer(len: usize) -> Vec<u8> {
	let mut data = SPARE_FRAMES
		.lock()
		.ok()
		.and_then(|mut spare| spare.pop())
		.filter(|spare| spare.capacity() <= len.saturating_mul(2))
		.unwrap_or_default();
	data.clear();
	data.reserve(len);
	data
}

/// Copies the visible `row_bytes` of each `stride`-wide source row into `out`.
fn copy_rows(source: &[u8], stride: usize, row_bytes: usize, out: &mut Vec<u8>) {
	if stride == row_bytes {
		out.extend_from_slice(source);
		return;
	}
	for row in source.chunks_exact(stride) {
		out.extend_from_slice(&row[..row_bytes]);
	}
}

impl GraphicsCaptureApiHandler for Handler {
	type Flags = Flags;
	type Error = &'static str;

	fn new(context: Context<Self::Flags>) -> Result<Self, Self::Error> {
		Ok(Self(
			context.flags,
			crate::diagnostics::Metrics::new(crate::diagnostics::Scope::ScreenVideo),
			readback::Reader::default(),
		))
	}

	fn on_frame_arrived(
		&mut self,
		frame: &mut Frame,
		control: InternalCaptureControl,
	) -> Result<(), Self::Error> {
		if self.0.stop.load(Ordering::Acquire) {
			control.stop();
			return Ok(());
		}
		let now = Instant::now();
		if now + Duration::from_millis(2) < self.0.next_frame
			|| self.0.pending.swap(true, Ordering::AcqRel)
		{
			return Ok(());
		}
		self.0.next_frame = (self.0.next_frame + self.0.interval).max(now + self.0.interval / 2);
		let (width, height) = (frame.width(), frame.height());
		let Some(row_bytes) = (width as usize).checked_mul(4) else {
			self.0.stop.store(true, Ordering::Release);
			control.stop();
			return Err("Captured frame dimensions are invalid");
		};
		let Some(data_len) = row_bytes.checked_mul(height as usize) else {
			self.0.stop.store(true, Ordering::Release);
			control.stop();
			return Err("Captured frame dimensions are invalid");
		};
		if width == 0
			|| height == 0
			|| width > MAX_FRAME_WIDTH
			|| height > MAX_FRAME_HEIGHT
			|| data_len > MAX_RAW_BYTES
		{
			self.0.stop.store(true, Ordering::Release);
			control.stop();
			return Err("Captured frame exceeds the 4K limit");
		}
		// Measure the actual GPU readback, excluding time between compositor callbacks.
		// Metrics enqueue fixed-size aggregates only; the reporter owns all log I/O.
		let started = self.1.start();
		let buffer = self.2.read(frame).map_err(|_| {
			self.0.stop.store(true, Ordering::Release);
			"Captured frame could not be read"
		});
		self.1.finish(crate::diagnostics::Stage::Receive, started);
		let buffer = buffer?;
		let started = self.1.start();
		let stride = buffer.row_pitch() as usize;
		let Some(source_len) = stride.checked_mul(height as usize) else {
			self.0.stop.store(true, Ordering::Release);
			return Err("Captured frame stride is invalid");
		};
		if stride < row_bytes || source_len > MAX_RAW_BYTES {
			self.0.stop.store(true, Ordering::Release);
			return Err("Captured frame stride exceeds the 4K limit");
		}
		let source = buffer.bytes();
		if source.len() < source_len {
			self.0.stop.store(true, Ordering::Release);
			return Err("Captured frame buffer is incomplete");
		}
		let mut data = take_frame_buffer(data_len);
		copy_rows(&source[..source_len], stride, row_bytes, &mut data);
		let dropped = match self.0.frames.try_send(RawFrame {
			width,
			height,
			stride: row_bytes,
			data,
		}) {
			Ok(()) => false,
			// A refused frame goes back to the pool instead of being freed.
			Err(TrySendError::Full(frame) | TrySendError::Disconnected(frame)) => {
				recycle_frame(frame.data);
				true
			}
		};
		if dropped {
			self.0.pending.store(false, Ordering::Release);
		}
		self.1.finish(crate::diagnostics::Stage::Mix, started);
		self.1.poll(false, u64::from(dropped), false, 0);
		Ok(())
	}

	fn on_closed(&mut self) -> Result<(), Self::Error> {
		self.0.stop.store(true, Ordering::Release);
		clear_spare_frames();
		Ok(())
	}
}

pub(crate) struct Capture {
	control: Option<CaptureControl<Handler, &'static str>>,
	audio: Option<audio::Audio>,
}

impl Capture {
	pub(crate) fn start(
		settings: CaptureSettings,
		frames: SyncSender<RawFrame>,
		audio: Option<tokio::sync::mpsc::Sender<AudioChunk>>,
		stop: Arc<AtomicBool>,
		ready: Arc<AtomicBool>,
		audio_epoch: Arc<AtomicU64>,
		pending: Arc<AtomicBool>,
	) -> Result<Self, &'static str> {
		if settings.width == 0
			|| settings.height == 0
			|| settings.width > MAX_FRAME_WIDTH
			|| settings.height > MAX_FRAME_HEIGHT
			|| settings.fps == 0
		{
			return Err("Invalid screen capture settings");
		}
		let mut capture = match settings.source {
			SourceId::Display(id) => {
				let monitor = Monitor::enumerate()
					.map_err(|_| "Displays could not be enumerated")?
					.into_iter()
					.find(|monitor| monitor_id(monitor) == id)
					.ok_or("Selected display is no longer available")?;
				start_item(settings, monitor, frames, stop.clone(), pending)
			}
			SourceId::Window(id) => {
				let window = Window::enumerate()
					.map_err(|_| "Windows could not be enumerated")?
					.into_iter()
					.find(|window| window_id(window) == id)
					.ok_or("Selected window is no longer available")?;
				start_item(settings, window, frames, stop.clone(), pending)
			}
			#[allow(unreachable_patterns)] // Portal may be absent from platform-scoped models.
			_ => return Err("The desktop screen picker is available only on Linux"),
		}?;
		if let Some(send) = audio {
			capture.audio = Some(audio::Audio::start(send, stop, ready, audio_epoch)?);
		}
		Ok(capture)
	}

	pub(crate) fn failed(&self) -> bool {
		self.audio.as_ref().is_some_and(audio::Audio::failed)
	}
}

trait NativeSource:
	TryInto<windows_capture::settings::GraphicsCaptureItemType> + Send + 'static
{
	fn dimensions(&self) -> Option<(u32, u32)>;
}

impl NativeSource for Monitor {
	fn dimensions(&self) -> Option<(u32, u32)> {
		self.width().ok().zip(self.height().ok())
	}
}

impl NativeSource for Window {
	fn dimensions(&self) -> Option<(u32, u32)> {
		self.width()
			.ok()
			.and_then(|width| u32::try_from(width).ok())
			.zip(
				self.height()
					.ok()
					.and_then(|height| u32::try_from(height).ok()),
			)
	}
}

fn start_item<T>(
	settings: CaptureSettings,
	item: T,
	frames: SyncSender<RawFrame>,
	stop: Arc<AtomicBool>,
	pending: Arc<AtomicBool>,
) -> Result<Capture, &'static str>
where
	T: NativeSource,
{
	let Some((width, height)) = item.dimensions() else {
		return Err("Selected source dimensions are unavailable");
	};
	if width == 0 || height == 0 || width > MAX_FRAME_WIDTH || height > MAX_FRAME_HEIGHT {
		return Err("Selected source exceeds the 4K capture limit");
	}
	let flags = Flags {
		frames,
		stop,
		pending,
		next_frame: Instant::now(),
		interval: Duration::from_secs_f64(1.0 / f64::from(settings.fps)),
	};
	let native = Settings::new(
		item,
		if settings.cursor {
			CursorCaptureSettings::Default
		} else {
			CursorCaptureSettings::WithoutCursor
		},
		DrawBorderSettings::WithoutBorder,
		SecondaryWindowSettings::Default,
		// Pace the compositor as well as the consumer. Dropping callbacks alone still
		// asks Windows to capture at the display's refresh rate (often 144/240 Hz).
		// Older Windows releases keep the existing callback-side throttle.
		if GraphicsCaptureApi::is_minimum_update_interval_supported().unwrap_or(false) {
			MinimumUpdateIntervalSettings::Custom(flags.interval)
		} else {
			MinimumUpdateIntervalSettings::Default
		},
		DirtyRegionSettings::Default,
		ColorFormat::Bgra8,
		flags,
	);
	let control =
		Handler::start_free_threaded(native).map_err(|_| "Screen capture could not be started")?;
	Ok(Capture {
		control: Some(control),
		audio: None,
	})
}

impl Drop for Capture {
	fn drop(&mut self) {
		self.audio.take();
		if let Some(control) = self.control.take() {
			let _ = control.stop();
		}
	}
}

#[cfg(test)]
mod frame_buffer_tests {
	use super::*;

	fn padded_source(width: usize, height: usize, stride: usize) -> Vec<u8> {
		let mut source = vec![0xEE; stride * height];
		for row in 0..height {
			for column in 0..width * 4 {
				source[row * stride + column] = (row * 31 + column) as u8;
			}
		}
		source
	}

	#[test]
	fn copy_drops_row_padding_and_keeps_visible_pixels() {
		let (width, height, stride) = (3, 2, 16);
		let source = padded_source(width, height, stride);
		let mut out = Vec::new();
		copy_rows(&source, stride, width * 4, &mut out);
		assert_eq!(out.len(), width * 4 * height);
		for row in 0..height {
			assert_eq!(
				&out[row * width * 4..(row + 1) * width * 4],
				&source[row * stride..row * stride + width * 4]
			);
		}
	}

	#[test]
	fn unpadded_copy_is_a_single_exact_copy() {
		let (width, height) = (4, 3);
		let source = padded_source(width, height, width * 4);
		let mut out = Vec::new();
		copy_rows(&source, width * 4, width * 4, &mut out);
		assert_eq!(out, source);
	}

	#[test]
	fn recycled_buffer_is_reused_cleared_and_bounded() {
		clear_spare_frames();
		let first = take_frame_buffer(1024);
		let pointer = first.as_ptr();
		recycle_frame(first);
		let reused = take_frame_buffer(1024);
		assert!(reused.is_empty());
		assert_eq!(reused.as_ptr(), pointer, "the spare allocation is reused");
		// The pool never keeps more than its limit.
		for _ in 0..SPARE_FRAME_LIMIT + 3 {
			recycle_frame(Vec::with_capacity(1024));
		}
		assert_eq!(SPARE_FRAMES.lock().unwrap().len(), SPARE_FRAME_LIMIT);
		clear_spare_frames();
	}

	#[test]
	fn a_spare_far_larger_than_the_next_frame_is_not_kept_at_its_old_size() {
		clear_spare_frames();
		recycle_frame(Vec::with_capacity(33_177_600));
		let small = take_frame_buffer(1920 * 1080 * 4);
		assert!(small.capacity() < 33_177_600);
		clear_spare_frames();
	}

	/// Fresh allocation per frame (the previous path) against the pooled path, on the same
	/// 4K BGRA source. Run: `cargo test -p discord-voice pooled_versus_fresh -- --ignored --nocapture`.
	#[test]
	#[ignore = "measurement only"]
	fn pooled_versus_fresh_frame_copy_4k() {
		let (width, height) = (3840usize, 2160usize);
		let stride = width * 4;
		let source = padded_source(width, height, stride);
		let data_len = stride * height;
		let samples = 30;
		let mut fresh = Vec::new();
		let mut pooled = Vec::new();
		for round in 0..samples + 3 {
			let started = Instant::now();
			let mut data = Vec::with_capacity(data_len);
			for row in source.chunks_exact(stride) {
				data.extend_from_slice(&row[..stride]);
			}
			let fresh_ms = started.elapsed().as_secs_f64() * 1000.0;
			drop(data);
			let started = Instant::now();
			let mut data = take_frame_buffer(data_len);
			copy_rows(&source, stride, stride, &mut data);
			let pooled_ms = started.elapsed().as_secs_f64() * 1000.0;
			recycle_frame(data);
			if round >= 3 {
				fresh.push(fresh_ms);
				pooled.push(pooled_ms);
			}
		}
		clear_spare_frames();
		fresh.sort_by(f64::total_cmp);
		pooled.sort_by(f64::total_cmp);
		println!(
			"4K BGRA copy, debug build, {samples} samples: fresh p50 {:.2} ms p95 {:.2} ms; pooled p50 {:.2} ms p95 {:.2} ms",
			fresh[samples / 2],
			fresh[samples * 95 / 100],
			pooled[samples / 2],
			pooled[samples * 95 / 100],
		);
	}
}
