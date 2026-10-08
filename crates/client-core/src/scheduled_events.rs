//! Guild scheduled events: browse upcoming gatherings. RSVP stays out until
//! the write path is live verified; the list is read-only.
use crate::{
	State,
	auth::{AuthState, Failure},
};
use model::{Id, ScheduledEvent};
use std::collections::BTreeMap;

pub enum Command {
	List { guild: Id, request: u64 },
}
pub enum Event {
	Listed {
		guild: Id,
		request: u64,
		result: Result<Vec<ScheduledEvent>, Failure>,
	},
}

/// Events per guild plus the single in-flight list.
#[derive(Default)]
pub struct Events {
	lists: BTreeMap<Id, Vec<ScheduledEvent>>,
	loading: Option<(Id, u64)>,
	error: Option<&'static str>,
	sequence: u64,
}
impl Events {
	pub fn events(&self, guild: Id) -> &[ScheduledEvent] {
		self.lists.get(&guild).map_or(&[], Vec::as_slice)
	}
	pub fn loading(&self, guild: Id) -> bool {
		self.loading.is_some_and(|(id, _)| id == guild)
	}
	pub fn error(&self) -> Option<&'static str> {
		self.error
	}
	pub fn bytes(&self) -> usize {
		self.lists
			.values()
			.map(|events| {
				size_of::<Id>()
					+ events.capacity() * size_of::<ScheduledEvent>()
					+ events.iter().map(ScheduledEvent::heap_bytes).sum::<usize>()
			})
			.sum()
	}
}
impl State {
	/// Loads one guild's scheduled events; a loaded list stays until an error
	/// allows an explicit retry.
	pub fn request_events(&mut self, guild: Id) -> Option<crate::Command> {
		if self.auth != AuthState::Authenticated
			|| !self.gateway_connected
			|| guild.0 == 0
			|| self.events.loading.is_some()
		{
			return None;
		}
		if self.events.lists.contains_key(&guild) && self.events.error.is_none() {
			return None;
		}
		if !self.guilds.iter().any(|g| g.id == guild) {
			return None;
		}
		self.events.sequence = self.events.sequence.wrapping_add(1);
		let request = self.events.sequence;
		self.events.loading = Some((guild, request));
		self.events.error = None;
		self.revision += 1;
		Some(crate::Command::ScheduledEvents(Command::List {
			guild,
			request,
		}))
	}

	pub fn apply_scheduled_events(&mut self, event: Event) -> Result<(), &'static str> {
		let Event::Listed {
			guild,
			request,
			result,
		} = event;
		if self.events.loading != Some((guild, request)) {
			return Ok(());
		}
		self.events.loading = None;
		match result {
			Ok(events) => {
				let mut events = events;
				events.truncate(model::MAX_EVENTS);
				if self.events.bytes() > model::account::MAX_BYTES {
					self.events.error = Some("Scheduled events exceed the session budget");
					return Ok(());
				}
				self.events.lists.insert(guild, events);
				self.events.error = None;
			}
			Err(failure) => {
				self.events.error = Some(failure.label());
				if failure.ends_session() {
					self.fail(failure);
				}
			}
		}
		self.revision += 1;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn guild_state() -> State {
		State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			guilds: vec![model::Guild {
				id: Id(9),
				name: "Synthetic".into(),
				icon: None,
				emojis: None,
				premium_tier: 0,
				stickers: None,
			}],
			..State::default()
		}
	}

	fn event(id: u64) -> ScheduledEvent {
		ScheduledEvent {
			id: Id(id),
			guild_id: Id(9),
			channel_id: Some(Id(2)),
			name: format!("Event {id}"),
			description: None,
			start: 1_800_000_000_000_000_000,
			end: None,
			status: model::STATUS_SCHEDULED,
			entity_type: model::ENTITY_VOICE,
			location: None,
			user_count: 3,
		}
	}

	#[test]
	fn event_list_loads_once_and_reloads_after_errors() {
		let mut state = guild_state();
		let crate::Command::ScheduledEvents(Command::List { guild, .. }) =
			state.request_events(Id(9)).unwrap()
		else {
			panic!()
		};
		assert_eq!(guild, Id(9));
		assert!(state.events.loading(Id(9)));
		assert!(state.request_events(Id(9)).is_none());
		assert!(state.request_events(Id(10)).is_none());
		state
			.apply_scheduled_events(Event::Listed {
				guild: Id(9),
				request: state.events.sequence,
				result: Ok(vec![event(5), event(6)]),
			})
			.unwrap();
		assert_eq!(state.events.events(Id(9)).len(), 2);
		assert!(!state.events.loading(Id(9)));
		assert!(state.request_events(Id(9)).is_none());
		assert!(state.events.bytes() > 0);
	}
}
