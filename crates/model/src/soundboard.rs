//! Guild soundboard sounds. List and play only; creating, editing and
//! deleting sounds stay in the official client for now.
//! Documented shapes: `GET /guilds/{guild}/soundboard-sounds` lists sounds,
//! `POST /channels/{channel}/send-soundboard-sound` plays one.
use crate::Id;
use serde::{Deserialize, Serialize};

/// Wire bound on sounds kept per guild; the service allows dozens.
pub const MAX_SOUNDS: usize = 128;
/// Discord sound names top out at 32 characters.
pub const MAX_SOUND_NAME_CHARS: usize = 32;
/// Sound volume is a 0–1 fraction.
pub const MAX_SOUND_VOLUME: f32 = 1.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoundEmoji {
	#[serde(default)]
	pub id: Option<Id>,
	#[serde(default)]
	pub name: Option<String>,
}
impl SoundEmoji {
	pub fn valid(&self) -> bool {
		(self.id.is_some() || self.name.as_ref().is_some_and(|name| !name.is_empty()))
			&& self.name.as_ref().is_none_or(|name| {
				!name.is_empty()
					&& name.chars().count() <= 128
					&& !name.chars().any(char::is_control)
			})
	}
	/// Unicode character for the button, when the sound uses one.
	pub fn character(&self) -> Option<&str> {
		match (self.id, self.name.as_deref()) {
			(None, Some(name)) => Some(name),
			_ => None,
		}
	}
	pub fn heap_bytes(&self) -> usize {
		self.name.as_ref().map_or(0, String::capacity)
	}
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sound {
	pub id: Id,
	/// Absent for built-in default sounds.
	#[serde(default)]
	pub guild_id: Option<Id>,
	pub name: String,
	/// 0–1 playback volume.
	#[serde(default = "full_volume")]
	pub volume: f32,
	#[serde(default)]
	pub emoji: Option<SoundEmoji>,
	/// Unavailable sounds cannot play.
	#[serde(default = "available_by_default")]
	pub available: bool,
}
fn full_volume() -> f32 {
	1.0
}
fn available_by_default() -> bool {
	true
}
impl Sound {
	pub fn heap_bytes(&self) -> usize {
		self.name.capacity() + self.emoji.as_ref().map_or(0, SoundEmoji::heap_bytes)
	}
}

pub fn valid_sound(sound: &Sound) -> bool {
	sound.id.0 != 0
		&& !sound.name.is_empty()
		&& sound.name.chars().count() <= MAX_SOUND_NAME_CHARS
		&& !sound.name.chars().any(char::is_control)
		&& (0.0..=MAX_SOUND_VOLUME).contains(&sound.volume)
		&& sound.emoji.as_ref().is_none_or(SoundEmoji::valid)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn sound() -> Sound {
		Sound {
			id: Id(7),
			guild_id: Some(Id(9)),
			name: "Airhorn".into(),
			volume: 1.0,
			emoji: Some(SoundEmoji {
				id: None,
				name: Some("📯".into()),
			}),
			available: true,
		}
	}

	#[test]
	fn bounds_accept_documented_shapes_and_reject_overreach() {
		assert!(valid_sound(&sound()));
		let mut long = sound();
		long.name = "x".repeat(MAX_SOUND_NAME_CHARS + 1);
		assert!(!valid_sound(&long), "long names are refused");
		let mut loud = sound();
		loud.volume = 1.5;
		assert!(!valid_sound(&loud), "volume stays in 0-1");
		let mut blank = sound();
		blank.name = String::new();
		assert!(!valid_sound(&blank), "blank names are refused");
		let mut custom = sound();
		custom.emoji = Some(SoundEmoji {
			id: Some(Id(3)),
			name: None,
		});
		assert!(valid_sound(&custom));
		assert_eq!(custom.emoji.as_ref().unwrap().character(), None);
		assert_eq!(sound().emoji.as_ref().unwrap().character(), Some("📯"));
	}
}
