//! Guild scheduled events: browse upcoming gatherings. RSVP stays out until
//! the write path is live verified; the list is read-only.
use crate::{
	State,
	auth::{AuthState, Failure},
};
use model::{Id, ScheduledEvent};
use std::{
	collections::BTreeMap,
	time::{Duration, Instant},
};

const MAX_CACHED_GUILDS: usize = 16;
const MAX_CACHE_BYTES: usize = 1024 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(60);
// Conservative allowance for B-tree nodes and the timestamp map per guild.
const MAP_ENTRY_BYTES: usize = 256;

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
	loaded_at: BTreeMap<Id, Instant>,
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
				MAP_ENTRY_BYTES
					+ events.capacity() * size_of::<ScheduledEvent>()
					+ events.iter().map(ScheduledEvent::heap_bytes).sum::<usize>()
			})
			.sum()
	}
}
impl State {
	/// Loads one guild's events. Successful lists are reused for one minute;
	/// errors allow an explicit retry without waiting for expiry.
	pub fn request_events(&mut self, guild: Id) -> Option<crate::Command> {
		if self.auth != AuthState::Authenticated
			|| !self.gateway_connected
			|| guild.0 == 0
			|| self.events.loading.is_some()
		{
			return None;
		}
		if self
			.events
			.loaded_at
			.get(&guild)
			.is_some_and(|loaded| loaded.elapsed() < CACHE_TTL)
			&& self.events.error.is_none()
		{
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
				events.shrink_to_fit();
				let previous = self.events.lists.get(&guild).map_or(0, |items| {
					MAP_ENTRY_BYTES
						+ items.capacity() * size_of::<ScheduledEvent>()
						+ items.iter().map(ScheduledEvent::heap_bytes).sum::<usize>()
				});
				let incoming = MAP_ENTRY_BYTES
					+ events.capacity() * size_of::<ScheduledEvent>()
					+ events.iter().map(ScheduledEvent::heap_bytes).sum::<usize>();
				if incoming > MAX_CACHE_BYTES {
					self.events.error = Some("Cached list exceeds the session budget");
					self.revision += 1;
					return Ok(());
				}
				let mut projected = self
					.events
					.bytes()
					.saturating_sub(previous)
					.saturating_add(incoming);
				while projected > MAX_CACHE_BYTES
					|| (!self.events.lists.contains_key(&guild)
						&& self.events.lists.len() >= MAX_CACHED_GUILDS)
				{
					let Some(oldest) = self
						.events
						.loaded_at
						.iter()
						.filter(|(id, _)| **id != guild)
						.min_by_key(|(id, loaded)| (**loaded, **id))
						.map(|(id, _)| *id)
					else {
						self.events.error = Some("Cached list exceeds the session budget");
						self.revision += 1;
						return Ok(());
					};
					self.events.lists.remove(&oldest);
					self.events.loaded_at.remove(&oldest);
					projected = self
						.events
						.bytes()
						.saturating_sub(previous)
						.saturating_add(incoming);
				}
				self.events.lists.insert(guild, events);
				self.events.loaded_at.insert(guild, Instant::now());
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
	fn cache_refreshes_expired_entries_and_rejects_oversized_incoming_list() {
		let mut state = guild_state();
		state.events.lists.insert(Id(9), vec![event(7)]);
		state
			.events
			.loaded_at
			.insert(Id(9), Instant::now() - CACHE_TTL);
		let command = state.request_events(Id(9));
		assert!(command.is_some());
		let mut oversized = event(7);
		oversized.name = "x".repeat(MAX_CACHE_BYTES);
		state
			.apply_scheduled_events(Event::Listed {
				guild: Id(9),
				request: state.events.sequence,
				result: Ok(vec![oversized]),
			})
			.unwrap();
		assert_eq!(state.events.lists[&Id(9)][0].name, event(7).name);
		assert!(state.events.error().is_some());
		assert!(state.events.loading.is_none());
	}

	#[test]
	fn cache_evicts_oldest_when_incoming_list_needs_byte_space() {
		let mut state = guild_state();
		let mut large = event(7);
		large.name = "x".repeat(MAX_CACHE_BYTES / 2 + 1);
		state.events.lists.insert(Id(100), vec![large.clone()]);
		state
			.events
			.loaded_at
			.insert(Id(100), Instant::now() - CACHE_TTL);
		state.events.loading = Some((Id(9), 1));
		state
			.apply_scheduled_events(Event::Listed {
				guild: Id(9),
				request: 1,
				result: Ok(vec![large]),
			})
			.unwrap();
		assert!(state.events.lists.contains_key(&Id(9)));
		assert!(!state.events.lists.contains_key(&Id(100)));
		assert!(!state.events.loaded_at.contains_key(&Id(100)));
		assert!(state.events.bytes() <= MAX_CACHE_BYTES);
	}

	#[test]
	fn cache_evicts_oldest_to_load_a_seventeenth_guild() {
		let mut state = guild_state();
		for id in 100..100 + MAX_CACHED_GUILDS as u64 {
			state.events.lists.insert(Id(id), vec![]);
			state
				.events
				.loaded_at
				.insert(Id(id), Instant::now() - Duration::from_secs(200 - id));
		}
		state.events.loading = Some((Id(9), 1));
		state
			.apply_scheduled_events(Event::Listed {
				guild: Id(9),
				request: 1,
				result: Ok(vec![event(7)]),
			})
			.unwrap();
		assert_eq!(state.events.lists.len(), MAX_CACHED_GUILDS);
		assert!(state.events.lists.contains_key(&Id(9)));
		assert!(!state.events.lists.contains_key(&Id(100)));
		assert!(!state.events.loaded_at.contains_key(&Id(100)));
		assert_eq!(state.events.loaded_at.len(), MAX_CACHED_GUILDS);
		assert!(state.events.bytes() <= MAX_CACHE_BYTES);
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
