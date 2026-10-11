//! Documented guild scheduled-event list. RSVP stays out until the write
//! path is live verified.
use crate::{DecodeError, Timestamp, permissions::List};
use model::{Id, MAX_EVENTS, ScheduledEvent, valid_event};
use serde::Deserialize;

pub const MAX_WIRE: usize = 256 * 1024;

#[derive(Deserialize)]
struct WireMetadata {
	#[serde(default)]
	location: Option<String>,
}

#[derive(Deserialize)]
struct WireEvent {
	#[serde(default)]
	id: Option<Id>,
	#[serde(default)]
	guild_id: Option<Id>,
	#[serde(default)]
	channel_id: Option<Id>,
	#[serde(default)]
	name: Option<String>,
	#[serde(default)]
	description: Option<String>,
	#[serde(default)]
	scheduled_start_time: Option<Timestamp>,
	#[serde(default)]
	scheduled_end_time: Option<Timestamp>,
	#[serde(default)]
	status: Option<u8>,
	#[serde(default)]
	entity_type: Option<u8>,
	#[serde(default)]
	entity_metadata: Option<WireMetadata>,
	#[serde(default)]
	user_count: Option<u32>,
}
impl WireEvent {
	fn checked(self, guild: Id) -> Result<ScheduledEvent, DecodeError> {
		let event = ScheduledEvent {
			id: self.id.unwrap_or(Id(0)),
			guild_id: self.guild_id.unwrap_or(Id(0)),
			channel_id: self.channel_id.filter(|id| id.0 != 0),
			name: self.name.unwrap_or_default(),
			description: self.description.filter(|text| !text.is_empty()),
			start: self.scheduled_start_time.map(|time| time.0).unwrap_or(0),
			end: self.scheduled_end_time.map(|time| time.0),
			status: self.status.unwrap_or(0),
			entity_type: self.entity_type.unwrap_or(0),
			location: self
				.entity_metadata
				.and_then(|metadata| metadata.location)
				.filter(|location| !location.is_empty()),
			user_count: self.user_count.unwrap_or(0),
		};
		if event.guild_id != guild || event.start <= 0 || !valid_event(&event) {
			return Err(DecodeError);
		}
		Ok(event)
	}
}

/// Bounded guild scheduled-event list; foreign or invalid rows fail the page.
pub fn events(bytes: &[u8], guild: Id) -> Result<Vec<ScheduledEvent>, DecodeError> {
	if bytes.len() > MAX_WIRE || guild.0 == 0 {
		return Err(DecodeError);
	}
	let rows: List<WireEvent, MAX_EVENTS> = crate::decode(bytes)?;
	rows.0.into_iter().map(|row| row.checked(guild)).collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn event_list_keeps_valid_rows_and_rejects_foreign_or_wild_ones() {
		let page = events(
			r#"[{"id":"5","guild_id":"9","channel_id":"2","name":"Game night","description":"Bring dice","scheduled_start_time":"2026-10-20T19:00:00+00:00","status":1,"entity_type":2,"user_count":7},{"id":"6","guild_id":"9","name":"Meetup","scheduled_start_time":"2026-10-21T19:00:00+00:00","status":2,"entity_type":3,"entity_metadata":{"location":"Cafe"},"user_count":3}]"#.as_bytes(),
			Id(9),
		)
		.unwrap();
		assert_eq!(page.len(), 2);
		assert_eq!(page[0].name, "Game night");
		assert_eq!(page[0].channel_id, Some(Id(2)));
		assert_eq!(page[0].user_count, 7);
		assert!(page[0].upcoming());
		assert_eq!(page[1].location.as_deref(), Some("Cafe"));
		assert!(events(&vec![b' '; MAX_WIRE + 1], Id(9)).is_err());
		assert!(
			events(
				br#"[{"id":"5","guild_id":"10","name":"Foreign","scheduled_start_time":"2026-10-20T19:00:00+00:00"}]"#,
				Id(9)
			)
			.is_err()
		);
		assert!(
			events(
				br#"[{"id":"5","guild_id":"9","name":"","scheduled_start_time":"2026-10-20T19:00:00+00:00"}]"#,
				Id(9)
			)
			.is_err()
		);
	}
}
