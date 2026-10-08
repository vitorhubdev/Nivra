//! Documented guild soundboard sounds: list only. Creating, editing and
//! deleting sounds stay in the official client.
use crate::{DecodeError, permissions::List};
use model::{Id, MAX_SOUNDS, Sound, SoundEmoji, valid_sound};
use serde::Deserialize;

pub const MAX_WIRE: usize = 256 * 1024;

#[derive(Deserialize)]
struct WireSound {
	#[serde(default)]
	name: Option<String>,
	#[serde(default)]
	sound_id: Option<Id>,
	#[serde(default)]
	volume: Option<f32>,
	#[serde(default)]
	emoji_id: Option<Id>,
	#[serde(default)]
	emoji_name: Option<String>,
	#[serde(default)]
	guild_id: Option<Id>,
	#[serde(default)]
	available: Option<bool>,
}
impl WireSound {
	fn checked(self, guild: Id) -> Result<Sound, DecodeError> {
		let sound = Sound {
			id: self.sound_id.unwrap_or(Id(0)),
			guild_id: self.guild_id,
			name: self.name.unwrap_or_default(),
			volume: self.volume.unwrap_or(1.0),
			emoji: match (&self.emoji_id, &self.emoji_name) {
				(None, None) => None,
				_ => Some(SoundEmoji {
					id: self.emoji_id,
					name: self.emoji_name,
				}),
			},
			available: self.available.unwrap_or(true),
		};
		if sound.guild_id.is_some_and(|id| id != guild) || !valid_sound(&sound) {
			return Err(DecodeError);
		}
		Ok(sound)
	}
}

/// Bounded guild soundboard list; foreign or invalid rows fail the page.
pub fn sounds(bytes: &[u8], guild: Id) -> Result<Vec<Sound>, DecodeError> {
	if bytes.len() > MAX_WIRE || guild.0 == 0 {
		return Err(DecodeError);
	}
	let rows: List<WireSound, MAX_SOUNDS> = crate::decode(bytes)?;
	rows.0.into_iter().map(|row| row.checked(guild)).collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn sound_list_keeps_valid_rows_and_rejects_foreign_or_wild_ones() {
		let page = sounds(
			r#"[{"name":"Airhorn","sound_id":"7","volume":1.0,"emoji_name":"📯","guild_id":"9","available":true},{"name":"Quiet","sound_id":"8","volume":0.5,"guild_id":"9"}]"#.as_bytes(),
			Id(9),
		)
		.unwrap();
		assert_eq!(page.len(), 2);
		assert_eq!(page[0].name, "Airhorn");
		assert_eq!(page[0].emoji.as_ref().unwrap().character(), Some("📯"));
		assert_eq!(page[1].volume, 0.5);
		assert!(sounds(&vec![b' '; MAX_WIRE + 1], Id(9)).is_err());
		assert!(
			sounds(
				br#"[{"name":"Foreign","sound_id":"7","guild_id":"10"}]"#,
				Id(9)
			)
			.is_err()
		);
		assert!(
			sounds(
				br#"[{"name":"Loud","sound_id":"7","volume":9.0,"guild_id":"9"}]"#,
				Id(9)
			)
			.is_err()
		);
	}
}
