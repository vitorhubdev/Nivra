//! Guild scheduled events: browse upcoming gatherings. RSVP stays out until
//! the write path is live verified; this ships the documented list only.
//! Documented shape: `GET /guilds/{guild}/scheduled-events` lists events.
use crate::Id;
use serde::{Deserialize, Serialize};

/// Wire bound on events kept per guild.
pub const MAX_EVENTS: usize = 100;
/// Discord event names run 1–100 characters, descriptions top out at 1000.
pub const MAX_EVENT_NAME_CHARS: usize = 100;
pub const MAX_EVENT_DESCRIPTION_CHARS: usize = 1000;

/// Service lifecycle: 1 scheduled, 2 active, 3 completed, 4 canceled.
pub const STATUS_SCHEDULED: u8 = 1;
pub const STATUS_ACTIVE: u8 = 2;

/// Venue: 1 stage instance, 2 voice channel, 3 somewhere else.
pub const ENTITY_STAGE: u8 = 1;
pub const ENTITY_VOICE: u8 = 2;
pub const ENTITY_EXTERNAL: u8 = 3;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledEvent {
	pub id: Id,
	pub guild_id: Id,
	/// Voice or stage channel for channel events; absent for external ones.
	#[serde(default)]
	pub channel_id: Option<Id>,
	pub name: String,
	#[serde(default)]
	pub description: Option<String>,
	/// Nanoseconds since the Unix epoch, same convention as `Message::edited_at`.
	pub start: i128,
	#[serde(default)]
	pub end: Option<i128>,
	#[serde(default)]
	pub status: u8,
	#[serde(default)]
	pub entity_type: u8,
	/// Physical location for external events.
	#[serde(default)]
	pub location: Option<String>,
	/// How many members marked themselves interested, when the service said.
	#[serde(default)]
	pub user_count: u32,
}
impl ScheduledEvent {
	/// Listed while it has not ended or been canceled.
	pub fn upcoming(&self) -> bool {
		matches!(self.status, STATUS_SCHEDULED | STATUS_ACTIVE)
	}
	pub fn heap_bytes(&self) -> usize {
		self.name.capacity()
			+ self.description.as_ref().map_or(0, String::capacity)
			+ self.location.as_ref().map_or(0, String::capacity)
	}
}

pub fn valid_event(event: &ScheduledEvent) -> bool {
	event.id.0 != 0
		&& event.guild_id.0 != 0
		&& !event.name.is_empty()
		&& event.name.chars().count() <= MAX_EVENT_NAME_CHARS
		&& !event.name.chars().any(char::is_control)
		&& event.description.as_ref().is_none_or(|description| {
			description.chars().count() <= MAX_EVENT_DESCRIPTION_CHARS
				&& !description.chars().any(char::is_control)
		}) && event.location.as_ref().is_none_or(|location| {
		!location.is_empty()
			&& location.chars().count() <= MAX_EVENT_DESCRIPTION_CHARS
			&& !location.chars().any(char::is_control)
	}) && event.end.is_none_or(|end| end >= event.start)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn event() -> ScheduledEvent {
		ScheduledEvent {
			id: Id(5),
			guild_id: Id(9),
			channel_id: Some(Id(2)),
			name: "Game night".into(),
			description: Some("Bring dice".into()),
			start: 1_800_000_000_000_000_000,
			end: None,
			status: STATUS_SCHEDULED,
			entity_type: ENTITY_VOICE,
			location: None,
			user_count: 7,
		}
	}

	#[test]
	fn bounds_accept_documented_shapes_and_reject_overreach() {
		let shown = event();
		assert!(valid_event(&shown));
		assert!(shown.upcoming());
		let mut long = shown.clone();
		long.name = "x".repeat(MAX_EVENT_NAME_CHARS + 1);
		assert!(!valid_event(&long), "long names are refused");
		let mut ended = shown.clone();
		ended.status = 3;
		assert!(!ended.upcoming());
		let mut backwards = shown.clone();
		backwards.end = Some(backwards.start - 1);
		assert!(!valid_event(&backwards), "end precedes start");
		let mut blank = shown.clone();
		blank.name = String::new();
		assert!(!valid_event(&blank), "blank names are refused");
		assert!(shown.heap_bytes() > 0);
	}
}
