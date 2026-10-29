//! Bounded screen-share settings and ephemeral Gateway negotiation events.
use crate::voice;
use model::Id;

pub const MAX_SOURCES: usize = 64;
pub const MAX_SOURCE_NAME_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceId {
	/// The Linux desktop chooses the source after an explicit Share action.
	Portal,
	/// Explicit whole-desktop capture on a native X11 session, without a portal.
	X11Desktop,
	Display(u64),
	Window(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
	pub id: SourceId,
	pub name: String,
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
	pub source: SourceId,
	pub width: u32,
	pub height: u32,
	pub fps: u32,
	pub cursor: bool,
	/// Share system audio with the screen; call microphone settings are independent.
	pub audio: bool,
}
impl Settings {
	pub fn valid(self) -> bool {
		!matches!(self.source, SourceId::Display(0) | SourceId::Window(0))
			&& matches!(
				(self.width, self.height),
				(854, 480) | (1280, 720) | (1920, 1080)
			) && matches!(self.fps, 15 | 30 | 60)
	}

	pub fn bit_rate(self) -> u32 {
		let base = match (self.width, self.height) {
			(854, 480) => 2_000_000,
			(1920, 1080) => 8_000_000,
			_ => 4_000_000,
		};
		// Keep per-frame quality approximately constant at 15/30/60 fps.
		// The old 15 fps preset spent the same bits/sec as 30 fps.
		base.saturating_mul(self.fps.clamp(15, 60)) / 30
	}
}

#[derive(Debug)]
pub enum Event {
	Created {
		rtc_server: Id,
		rtc_channel: Id,
	},
	Server {
		token: Option<voice::Secret>,
		endpoint: Option<String>,
	},
	/// The stream is gone. `reason` names Discord's cause when it sent one we recognise.
	Deleted {
		reason: Option<&'static str>,
	},
	Failed(&'static str),
}
impl Event {
	pub(crate) fn bytes(&self) -> usize {
		match self {
			Self::Server { token, endpoint } => {
				token.as_ref().map_or(0, voice::Secret::bytes)
					+ endpoint.as_ref().map_or(0, String::capacity)
			}
			_ => 0,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn screen_bitrate_scales_with_selected_frame_rate() {
		let mut settings = Settings {
			source: SourceId::Display(1),
			width: 1920,
			height: 1080,
			fps: 30,
			cursor: true,
			audio: false,
		};
		assert!(settings.valid());
		assert_eq!(settings.bit_rate(), 8_000_000);
		settings.fps = 15;
		assert_eq!(settings.bit_rate(), 4_000_000);
		settings.fps = 60;
		assert_eq!(settings.bit_rate(), 16_000_000);
		settings.fps = 30;
		settings.width = 854;
		settings.height = 480;
		assert_eq!(settings.bit_rate(), 2_000_000);
	}
}
