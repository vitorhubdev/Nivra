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
/// What Discord's voice close codes mean for the session (official "Voice Close Event
/// Codes"). `Resume` re-opens the socket and replays op 7; every other outcome ends this
/// transport with a specific reason and leaves the rejoin decision to the call layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseDisposition {
	/// The voice session survives: normal closure, going away, abnormal closure (nothing
	/// received, 1006) and a crashed voice server (4015) are resumed.
	Resume,
	/// 4006/4009: the old session is gone; a fresh voice session must be opened.
	SessionExpired,
	/// 4014 and protocol-level codes: this browser session was disconnected; no resume.
	Disconnected,
	/// 4008/4021: rate limited; stop and let the user retry later.
	RateLimited,
	/// 4022: the call itself was terminated. A new VOICE_SERVER_UPDATE opens the new server.
	Terminated,
}

/// Classifies a voice close frame code; `None` is a close without a status code (1005),
/// which network drops surface as and which preserves the session like 1006.
pub fn close_disposition(code: Option<u16>) -> CloseDisposition {
	match code {
		None | Some(1000 | 1001 | 1006 | 4015) => CloseDisposition::Resume,
		Some(4006 | 4009) => CloseDisposition::SessionExpired,
		Some(4008 | 4021) => CloseDisposition::RateLimited,
		Some(4022) => CloseDisposition::Terminated,
		// 4014 (kicked or the main gateway session ended) and any other code end this socket.
		_ => CloseDisposition::Disconnected,
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
	/// The voice socket is being resumed after a drop that preserves the session.
	Resuming {
		attempt: u8,
	},
	/// The voice socket closed with `code`; the disposition says what follows.
	Closed {
		code: Option<u16>,
		disposition: CloseDisposition,
	},
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
