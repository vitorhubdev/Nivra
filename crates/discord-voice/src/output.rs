//! One output-device resolver for every sound the app makes: call audio,
//! notification cues, voice-message and attachment playback, and video audio.
//! The configured call output wins; "Default" (or an unplugged device, or a
//! corrupt stored id) follows the system default instead of failing. Resolving
//! per open keeps short cues and fresh playbacks on a changed default without
//! migrating live streams.
use cpal::traits::{DeviceTrait, HostTrait};

/// Id of the system default output, to notice when "Default" moves underneath playback.
pub fn default_id(host: &cpal::Host) -> Option<String> {
	host.default_output_device()
		.and_then(|device| device.id().ok())
		.map(|id| id.to_string())
}

/// Resolve `selected` (a cpal device id from voice settings, or `None` for the
/// "Default" choice) to a playable output device. Returns `None` only when the
/// system itself reports no default output.
pub fn device(host: &cpal::Host, selected: Option<&str>) -> Option<cpal::Device> {
	if let Some(id) = selected
		&& let Ok(id) = id.parse()
		&& let Some(device) = host.device_by_id(&id)
	{
		return Some(device);
	}
	host.default_output_device()
}

#[cfg(test)]
mod tests {
	use super::*;
	use cpal::traits::HostTrait;

	fn selected_id() -> Option<String> {
		let host = cpal::default_host();
		// Prefer a non-default device so the test proves the selection wins;
		// with a single device both paths agree and the fallback still holds.
		let ids: Vec<String> = host
			.output_devices()
			.map(|devices| {
				devices
					.filter_map(|device| device.id().ok().map(|id| id.to_string()))
					.take(8)
					.collect()
			})
			.unwrap_or_default();
		let def = host
			.default_output_device()
			.and_then(|device| device.id().ok())
			.map(|id| id.to_string());
		ids.into_iter().find(|id| Some(id) != def.as_ref()).or(def)
	}

	#[test]
	fn output_follows_selection_then_default() {
		let host = cpal::default_host();
		let def = host.default_output_device();
		// No selection always means the system default (or nothing on headless CI).
		assert_eq!(
			device(&host, None).and_then(|d| d.id().ok().map(|id| id.to_string())),
			def.and_then(|d| d.id().ok().map(|id| id.to_string()))
		);
		// A corrupt stored id never bricks audio: it falls back to the default.
		assert_eq!(
			device(&host, Some("not-a-device-id"))
				.and_then(|d| d.id().ok().map(|id| id.to_string())),
			host.default_output_device()
				.and_then(|d| d.id().ok().map(|id| id.to_string()))
		);
		if let Some(selected) = selected_id() {
			assert_eq!(
				device(&host, Some(&selected)).and_then(|d| d.id().ok().map(|id| id.to_string())),
				Some(selected)
			);
		}
	}
}
