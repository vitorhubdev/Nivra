//! System-default following and debouncing for transient endpoint churn (Windows WASAPI).
use super::Devices;
use std::time::{Duration, Instant};

/// How long a new system default must remain before reopening streams.
pub const DEFAULT_FOLLOW_GRACE: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct HostEndpoints {
	pub default_input: Option<String>,
	pub default_output: Option<String>,
	/// Resolved ID when `Devices.input` is set and that device is present.
	pub selected_input: Option<String>,
	pub selected_output: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct OpenedEndpoints {
	pub input: Option<String>,
	pub output: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointsMigration {
	None,
	/// Explicit selection became available or moved; reopen immediately.
	Immediate,
	/// Followed system default moved; caller must debounce before reopening.
	DefaultFollow,
}

pub fn endpoints_migration(
	settings: &Devices,
	opened: &OpenedEndpoints,
	host: &HostEndpoints,
) -> EndpointsMigration {
	let input = side_migration(
		settings.input.as_ref(),
		&opened.input,
		host.selected_input.as_ref(),
		host.default_input.as_ref(),
	);
	let output = side_migration(
		settings.output.as_ref(),
		&opened.output,
		host.selected_output.as_ref(),
		host.default_output.as_ref(),
	);
	match (input, output) {
		(Some(false), _) | (_, Some(false)) => EndpointsMigration::Immediate,
		(Some(true), _) | (_, Some(true)) => EndpointsMigration::DefaultFollow,
		_ => EndpointsMigration::None,
	}
}

/// Returns `Some(false)` for immediate migration, `Some(true)` for debounced default follow.
fn side_migration(
	explicit: Option<&String>,
	opened: &Option<String>,
	selected: Option<&String>,
	default_id: Option<&String>,
) -> Option<bool> {
	match explicit {
		Some(_) => selected
			.filter(|id| Some(*id) != opened.as_ref())
			.map(|_| false),
		None => (opened.is_some() && default_id != opened.as_ref()).then_some(true),
	}
}

#[derive(Default)]
pub struct DefaultFollowGrace {
	since: Option<Instant>,
}

impl DefaultFollowGrace {
	pub fn reset(&mut self) {
		self.since = None;
	}

	/// Returns true once `DEFAULT_FOLLOW_GRACE` has elapsed with continuous default-follow drift.
	pub fn poll(&mut self, drift: bool, now: Instant) -> bool {
		if !drift {
			self.since = None;
			return false;
		}
		match self.since {
			None => {
				self.since = Some(now);
				false
			}
			Some(since) if now.saturating_duration_since(since) >= DEFAULT_FOLLOW_GRACE => {
				self.since = None;
				true
			}
			Some(_) => false,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn host(default_in: &str, default_out: &str) -> HostEndpoints {
		HostEndpoints {
			default_input: Some(default_in.into()),
			default_output: Some(default_out.into()),
			selected_input: None,
			selected_output: None,
		}
	}

	fn opened(input: &str, output: &str) -> OpenedEndpoints {
		OpenedEndpoints {
			input: Some(input.into()),
			output: Some(output.into()),
		}
	}

	#[test]
	fn stable_default_change_requests_debounced_migration() {
		let settings = Devices::default();
		let opened = opened("wasapi-in-a", "wasapi-out-a");
		let mut snapshot = host("wasapi-in-a", "wasapi-out-a");
		assert_eq!(
			endpoints_migration(&settings, &opened, &snapshot),
			EndpointsMigration::None
		);
		snapshot.default_input = Some("wasapi-in-b".into());
		snapshot.default_output = Some("wasapi-out-b".into());
		assert_eq!(
			endpoints_migration(&settings, &opened, &snapshot),
			EndpointsMigration::DefaultFollow
		);
	}

	#[test]
	fn transient_default_flip_does_not_reopen_mid_call() {
		let settings = Devices::default();
		let opened = opened("wasapi-in-a", "wasapi-out-a");
		let mut snapshot = host("wasapi-in-a", "wasapi-out-a");
		let mut grace = DefaultFollowGrace::default();
		let t0 = Instant::now();

		snapshot.default_input = Some("wasapi-in-b".into());
		snapshot.default_output = Some("wasapi-out-b".into());
		assert_eq!(
			endpoints_migration(&settings, &opened, &snapshot),
			EndpointsMigration::DefaultFollow
		);
		assert!(!grace.poll(true, t0));
		assert!(!grace.poll(true, t0 + Duration::from_secs(1)));

		// IMM/WASAPI often reverts within a second; cancel the pending reopen.
		snapshot.default_input = Some("wasapi-in-a".into());
		snapshot.default_output = Some("wasapi-out-a".into());
		assert_eq!(
			endpoints_migration(&settings, &opened, &snapshot),
			EndpointsMigration::None
		);
		assert!(!grace.poll(false, t0 + Duration::from_millis(1500)));
		// Drift resumes after a cancel: grace must observe the full window again.
		assert!(!grace.poll(true, t0 + Duration::from_millis(1600)));
		assert!(!grace.poll(true, t0 + Duration::from_secs(3)));
		assert!(grace.poll(true, t0 + Duration::from_secs(3) + DEFAULT_FOLLOW_GRACE));
	}

	#[test]
	fn default_follow_grace_elapses_before_reopen() {
		let settings = Devices::default();
		let opened = opened("wasapi-in-a", "wasapi-out-a");
		let mut snapshot = host("wasapi-in-b", "wasapi-out-b");
		assert_eq!(
			endpoints_migration(&settings, &opened, &snapshot),
			EndpointsMigration::DefaultFollow
		);
		let mut grace = DefaultFollowGrace::default();
		let t0 = Instant::now();
		assert!(!grace.poll(true, t0));
		assert!(!grace.poll(true, t0 + Duration::from_secs(1)));
		assert!(grace.poll(true, t0 + DEFAULT_FOLLOW_GRACE));
		// Still drifting after grace: start a new observation window.
		assert!(!grace.poll(true, t0 + DEFAULT_FOLLOW_GRACE + Duration::from_millis(1)));
		let _ = &mut snapshot;
	}

	#[test]
	fn selected_device_return_is_immediate_not_debounced() {
		let settings = Devices {
			input: Some("usb-mic-id".into()),
			output: None,
		};
		let opened = OpenedEndpoints {
			input: Some("fallback-default-mic".into()),
			output: Some("wasapi-out-a".into()),
		};
		let host = HostEndpoints {
			default_input: Some("fallback-default-mic".into()),
			default_output: Some("wasapi-out-a".into()),
			selected_input: Some("usb-mic-id".into()),
			selected_output: None,
		};
		assert_eq!(
			endpoints_migration(&settings, &opened, &host),
			EndpointsMigration::Immediate
		);
	}

	#[test]
	fn set_devices_same_defaults_do_not_imply_migration() {
		let settings = Devices::default();
		let opened = opened("wasapi-in-a", "wasapi-out-a");
		let snapshot = host("wasapi-in-a", "wasapi-out-a");
		assert_eq!(
			endpoints_migration(&settings, &opened, &snapshot),
			EndpointsMigration::None
		);
	}
}
