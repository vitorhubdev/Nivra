//! Inline attachment decoding on every desktop platform. The caller owns validated, bounded
//! network I/O and hands over an anonymous read-only stream: decoders never see a URL or an
//! account token. Every backend yields the same bounded RGBA frames and stereo PCM samples.
//!
//! * Windows: Media Foundation (`media_foundation`).
//! * macOS: a bounded MPEG-4/MOV demuxer (`mp4`) feeding VideoToolbox and Symphonia's AAC decoder.
//! * Linux: GStreamer's `decodebin` pulling bytes from the same stream through `appsrc`.
use std::io::{Read, Seek};

#[cfg(target_os = "windows")]
mod media_foundation;
#[cfg(target_os = "windows")]
pub use media_foundation::Decoder;

#[cfg(target_os = "macos")]
mod apple;
#[cfg(target_os = "macos")]
mod mp4;
#[cfg(target_os = "macos")]
pub use apple::Decoder;

/// Hardware-backed decoding of one live H.264 elementary stream (camera or Go Live).
/// Windows uses the Media Foundation decoder with a DXGI device manager when available,
/// macOS uses VideoToolbox, Linux lets GStreamer's `decodebin` pick the installed decoder.
#[cfg(target_os = "macos")]
#[path = "live_apple.rs"]
pub mod live;
#[cfg(target_os = "windows")]
#[path = "live_windows.rs"]
pub mod live;
#[cfg(target_os = "linux")]
#[path = "live_gst.rs"]
pub mod live;

/// Largest live access unit any backend accepts.
pub const MAX_ACCESS_UNIT: usize = 2 * 1024 * 1024 + 64 * 1024;

/// One decoded live picture, tightly packed RGBA.
pub struct LiveFrame {
	pub width: u32,
	pub height: u32,
	pub rgba: Vec<u8>,
}

/// Receives live pictures from the decoder's own thread, in decode order.
pub type LiveSink = Box<dyn Fn(LiveFrame) + Send + Sync>;

#[cfg(target_os = "linux")]
mod gst;
#[cfg(target_os = "linux")]
pub use gst::Decoder;

/// Longest attachment any backend plays inline.
pub const MAX_SECONDS: f64 = 2.0 * 60.0 * 60.0;
/// Largest single decoded frame or compressed sample any backend accepts.
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
pub const UNSUPPORTED: &str = "This video format or codec is not supported on this system.";
pub const INVALID: &str = "The video could not be decoded safely.";
pub const TOO_LARGE: &str = "This video is too large to decode safely.";
pub const TOO_LONG: &str = "Videos longer than two hours are not supported.";

pub trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

#[derive(Clone, Copy, Debug)]
pub struct Info {
	pub width: u32,
	pub height: u32,
	pub duration: f64,
	/// Zero when the attachment has no audio track.
	pub sample_rate: u32,
	pub channels: u16,
}

pub enum Sample {
	Video {
		pts: f64,
		width: u32,
		height: u32,
		rgba: Vec<u8>,
	},
	Audio {
		pts: f64,
		frames: Vec<[f32; 2]>,
	},
}

// These backends read tracks independently. Linux must yield when a sibling pipeline
// queue needs draining, so the player uses the same polling interface on every OS.
#[cfg(not(target_os = "linux"))]
impl Decoder {
	pub fn poll_video(&mut self) -> Result<std::task::Poll<Option<Sample>>, &'static str> {
		self.read_video().map(std::task::Poll::Ready)
	}

	pub fn poll_audio(&mut self) -> Result<std::task::Poll<Option<Sample>>, &'static str> {
		self.read_audio().map(std::task::Poll::Ready)
	}
}

/// Largest edge and pixel count any decoder accepts. This is a safety bound against
/// absurd dimensions, not a playback limit: frames larger than the preview box are
/// scaled down by the decoder or by the media worker.
pub const MAX_DECODE_EDGE: u32 = 16_384;
pub const MAX_DECODE_PIXELS: u64 = 16_384 * 16_384;

/// The inline-player preview box: 1920x1080 landscape, 1080x1920 portrait.
pub const PREVIEW_LONG_EDGE: u32 = 1920;
pub const PREVIEW_SHORT_EDGE: u32 = 1080;

/// Fit `width`x`height` inside the preview box without ever upscaling a smaller video.
pub fn preview_dimensions(width: u32, height: u32) -> (u32, u32) {
	if width == 0 || height == 0 {
		return (width, height);
	}
	let (max_w, max_h) = if width >= height {
		(PREVIEW_LONG_EDGE, PREVIEW_SHORT_EDGE)
	} else {
		(PREVIEW_SHORT_EDGE, PREVIEW_LONG_EDGE)
	};
	let scale = (f64::from(max_w) / f64::from(width))
		.min(f64::from(max_h) / f64::from(height))
		.min(1.0);
	if scale >= 1.0 {
		return (width, height);
	}
	(
		((f64::from(width) * scale).round() as u32).max(1),
		((f64::from(height) * scale).round() as u32).max(1),
	)
}

/// Decoder admission check: refuse only dimensions that are not decodable safely.
pub fn check_dimensions(width: u32, height: u32) -> Result<(), &'static str> {
	if width == 0 || height == 0 {
		return Err(INVALID);
	}
	if width > MAX_DECODE_EDGE
		|| height > MAX_DECODE_EDGE
		|| u64::from(width) * u64::from(height) > MAX_DECODE_PIXELS
	{
		return Err(TOO_LARGE);
	}
	Ok(())
}

/// Rotate a packed RGBA frame clockwise by a quarter-turn multiple; returns the new dimensions.
#[cfg(any(target_os = "macos", all(test, target_os = "linux")))]
pub(crate) fn rotate_rgba(
	rgba: &[u8],
	width: u32,
	height: u32,
	rotation: u32,
) -> (u32, u32, Vec<u8>) {
	let (w, h) = (width as usize, height as usize);
	if rotation == 0 || rgba.len() != w * h * 4 {
		return (width, height, rgba.to_vec());
	}
	let (out_w, out_h) = if rotation == 90 || rotation == 270 {
		(h, w)
	} else {
		(w, h)
	};
	let mut out = vec![0; rgba.len()];
	for y in 0..h {
		for x in 0..w {
			let (dx, dy) = match rotation {
				90 => (h - 1 - y, x),
				180 => (w - 1 - x, h - 1 - y),
				270 => (y, w - 1 - x),
				_ => (x, y),
			};
			let source = (y * w + x) * 4;
			let target = (dy * out_w + dx) * 4;
			out[target..target + 4].copy_from_slice(&rgba[source..source + 4]);
		}
	}
	(out_w as u32, out_h as u32, out)
}

#[cfg(all(test, not(target_os = "windows")))]
mod tests {
	use super::*;
	#[test]
	fn rotation_and_bounds() {
		assert!(check_dimensions(1920, 1080).is_ok());
		// Large frames are scaled now, so only absurd dimensions are refused.
		assert!(check_dimensions(1921, 1).is_ok());
		assert!(check_dimensions(1920, 1920).is_ok());
		assert!(check_dimensions(0, 1).is_err());
		assert!(check_dimensions(MAX_DECODE_EDGE + 1, 1).is_err());
		// A 2x1 frame with distinct pixels rotates into a 1x2 column.
		let frame = [1, 1, 1, 255, 2, 2, 2, 255];
		assert_eq!(
			rotate_rgba(&frame, 2, 1, 90),
			(1, 2, vec![1, 1, 1, 255, 2, 2, 2, 255])
		);
		assert_eq!(
			rotate_rgba(&frame, 2, 1, 180),
			(2, 1, vec![2, 2, 2, 255, 1, 1, 1, 255])
		);
		assert_eq!(
			rotate_rgba(&frame, 2, 1, 270),
			(1, 2, vec![2, 2, 2, 255, 1, 1, 1, 255])
		);
	}
}

#[cfg(test)]
mod capability {
	use super::*;

	#[test]
	fn native_playback_keeps_file_pixels_and_names_the_decoder() {
		assert!(check_dimensions(1920, 1080).is_ok());
		// Large files are admitted and scaled; only absurd dimensions are refused.
		assert!(check_dimensions(3840, 2160).is_ok());
		assert!(check_dimensions(MAX_DECODE_EDGE + 1, 1).is_err());
		let backend = if cfg!(windows) {
			"Media Foundation"
		} else if cfg!(target_os = "macos") {
			"VideoToolbox and AAC"
		} else if cfg!(target_os = "linux") {
			"GStreamer decodebin"
		} else {
			"none"
		};
		println!("video backend: {backend}");
		println!(
			"pixels: decodable frames are scaled into the 1920x1080 landscape / 1080x1920 portrait preview box"
		);
		println!(
			"macOS demuxer: MP4/MOV avc1, hvc1, AAC. WebM, Matroska, VP8, VP9 and AV1 have no in-tree demuxer"
		);
		println!("linux: decodebin uses codecs installed on the machine");
		println!("windows: Media Foundation uses codecs installed on the machine");
		println!("bundled ffmpeg: not added. VLC: absent");
	}

	#[test]
	fn preview_box_scales_large_frames_and_never_upscales_small_ones() {
		assert_eq!(preview_dimensions(4500, 3000), (1620, 1080));
		assert_eq!(preview_dimensions(3840, 2160), (1920, 1080));
		assert_eq!(preview_dimensions(2160, 3840), (1080, 1920));
		assert_eq!(preview_dimensions(640, 360), (640, 360));
		assert_eq!(preview_dimensions(1080, 1920), (1080, 1920));
		assert_eq!(preview_dimensions(1920, 1080), (1920, 1080));
	}
}
