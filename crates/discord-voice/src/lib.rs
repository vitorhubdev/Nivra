//! Discord DM and guild voice media. No bot manager, relay, recording, or key persistence.
mod activity;
pub mod audio;
pub mod camera;
mod capture;
mod crypto;
mod diagnostics;
mod jitter;
mod mixer;
pub mod output;
pub mod screen;
mod stream_playout;
mod timer;
mod transport;
mod video;
// Linux has no shared hardware encoder, but the camera's GStreamer encoder still takes the
// same configuration, so the facade is compiled on every supported platform.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod video_encode;
mod video_receive;
mod video_sps;
pub use crypto::Identity;
pub use transport::{run, run_stream, run_with_identity, watch_stream};
pub use video_receive::{RemoteFrame, VideoSink};
pub mod camera_video;

pub type Frame = [f32; 960];
#[derive(Clone, Copy)]
pub struct Controls {
	pub muted: bool,
	/// Local indicator threshold; independent of received participants.
	pub activity_threshold_db: i16,
	/// Zero means off; a new value invalidates frames from the previous camera instance.
	pub camera: u64,
	pub deafened: bool,
	/// Session-only playback percentages (0–200); zero user IDs are unused.
	pub user_volumes: [(u64, u16); 64],
	/// Watched stream playback percentage, independently muted with zero.
	pub stream_volume: u16,
}
impl Default for Controls {
	fn default() -> Self {
		Self {
			muted: false,
			activity_threshold_db: -45,
			camera: 0,
			deafened: false,
			user_volumes: [(0, 100); 64],
			stream_volume: 100,
		}
	}
}
#[allow(clippy::large_enum_variant)] // Ready carries the full connection context.
pub enum Status {
	Connecting,
	Discovering,
	TransportReady,
	CameraAvailable(bool),
	Securing,
	WaitingForPeer,
	Ready {
		privacy_code: String,
	},
	/// Voice-server heartbeat round trip, in milliseconds.
	Ping(u32),
	RemoteAudio,
	/// Latest active user IDs and smoothed energy levels, zero-padded to 64 slots.
	Speaking(SpeakingState),
	/// A non-DAVE participant joined. The call stays up on transport encryption only.
	TransportOnly,
}

/// Parallel user IDs and `0..=255` energy levels indexed by voice slot.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SpeakingState {
	pub users: [u64; 64],
	pub levels: [u8; 64],
}
impl Default for SpeakingState {
	fn default() -> Self {
		Self {
			users: [0; 64],
			levels: [0; 64],
		}
	}
}

#[cfg(test)]
mod test_mls;

// Exercise the exact vendored SHAKE adapter, without enabling unused HPKE backends.
#[cfg(test)]
#[path = "../../../vendor/hpke-rs/src/nivra_sha3.rs"]
mod hpke_sha3;
