//! Guild soundboard sounds: browse and play. Creating, editing and deleting
//! sounds stay in the official client.
use crate::{
	State,
	auth::{AuthState, Failure},
};
use model::{Id, Sound};
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
	List {
		guild: Id,
		request: u64,
	},
	Play {
		channel: Id,
		sound: Id,
		source: Option<Id>,
		request: u64,
	},
}
pub enum Event {
	Listed {
		guild: Id,
		request: u64,
		result: Result<Vec<Sound>, Failure>,
	},
	Played {
		channel: Id,
		request: u64,
		result: Result<(), Failure>,
	},
}

/// Sounds per guild plus the single in-flight list or play.
#[derive(Default)]
pub struct Board {
	lists: BTreeMap<Id, Vec<Sound>>,
	loaded_at: BTreeMap<Id, Instant>,
	loading: Option<(Id, u64)>,
	error: Option<&'static str>,
	playing: Option<(Id, u64)>,
	sequence: u64,
}
impl Board {
	pub fn sounds(&self, guild: Id) -> &[Sound] {
		self.lists.get(&guild).map_or(&[], Vec::as_slice)
	}
	pub fn loading(&self, guild: Id) -> bool {
		self.loading.is_some_and(|(id, _)| id == guild)
	}
	pub fn error(&self) -> Option<&'static str> {
		self.error
	}
	pub fn playing(&self) -> bool {
		self.playing.is_some()
	}
	pub fn bytes(&self) -> usize {
		self.lists
			.values()
			.map(|sounds| {
				MAP_ENTRY_BYTES
					+ sounds.capacity() * size_of::<Sound>()
					+ sounds.iter().map(Sound::heap_bytes).sum::<usize>()
			})
			.sum()
	}
}
impl State {
	/// Loads one guild's sounds. Successful lists are reused for one minute;
	/// errors allow an explicit retry without waiting for expiry.
	pub fn request_sounds(&mut self, guild: Id) -> Option<crate::Command> {
		if self.auth != AuthState::Authenticated
			|| !self.gateway_connected
			|| guild.0 == 0
			|| self.soundboard.loading.is_some()
		{
			return None;
		}
		if self
			.soundboard
			.loaded_at
			.get(&guild)
			.is_some_and(|loaded| loaded.elapsed() < CACHE_TTL)
			&& self.soundboard.error.is_none()
		{
			return None;
		}
		if !self.guilds.iter().any(|g| g.id == guild) {
			return None;
		}
		self.soundboard.sequence = self.soundboard.sequence.wrapping_add(1);
		let request = self.soundboard.sequence;
		self.soundboard.loading = Some((guild, request));
		self.soundboard.error = None;
		self.revision += 1;
		Some(crate::Command::Soundboard(Command::List { guild, request }))
	}

	/// Documented soundboard playback requirements. Suppression is represented by
	/// server_muted in the Gateway roster. Self microphone mute alone is allowed.
	/// https://docs.discord.com/developers/resources/soundboard#send-soundboard-sound
	pub fn soundboard_unavailable(&self, channel: Id) -> Option<&'static str> {
		if self.is_stage(channel) {
			return Some("Stage audience can only listen");
		}
		if !self.can_call(channel)
			|| !self.can_speak(channel)
			|| self.permission(channel, model::permissions::USE_SOUNDBOARD) != Some(true)
		{
			return Some("Soundboard unavailable with current connection or permissions");
		}
		let Some(call) = self
			.voice
			.active
			.as_ref()
			.filter(|call| call.channel == channel && call.guild.is_some())
		else {
			return Some("Join a server voice channel to play sounds");
		};
		if !matches!(
			call.phase,
			crate::voice::Phase::Connected | crate::voice::Phase::Waiting
		) {
			return Some("Wait for the voice connection before playing sounds");
		}
		if call.server_muted || call.server_deafened || call.deafened {
			return Some("Soundboard unavailable while server-muted, suppressed or deafened");
		}
		if self.soundboard.playing.is_some() {
			return Some("Wait for the current soundboard request");
		}
		None
	}

	/// Plays one known, available sound to the joined voice channel.
	pub fn play_sound(&mut self, channel: Id, sound: Id) -> Option<crate::Command> {
		if let Some(reason) = self.soundboard_unavailable(channel) {
			self.status = reason;
			return None;
		}
		let call = self.voice.active.as_ref()?;
		let guild = call.guild?;
		let known = self
			.soundboard
			.sounds(guild)
			.iter()
			.find(|known| known.id == sound && known.available)?;
		let _ = known;
		self.soundboard.sequence = self.soundboard.sequence.wrapping_add(1);
		let request = self.soundboard.sequence;
		self.soundboard.playing = Some((channel, request));
		self.revision += 1;
		Some(crate::Command::Soundboard(Command::Play {
			channel,
			sound,
			source: Some(guild),
			request,
		}))
	}

	pub fn apply_soundboard(&mut self, event: Event) -> Result<(), &'static str> {
		match event {
			Event::Listed {
				guild,
				request,
				result,
			} => {
				if self.soundboard.loading != Some((guild, request)) {
					return Ok(());
				}
				self.soundboard.loading = None;
				match result {
					Ok(sounds) => {
						let mut sounds = sounds;
						sounds.truncate(model::MAX_SOUNDS);
						sounds.shrink_to_fit();
						let previous = self.soundboard.lists.get(&guild).map_or(0, |items| {
							MAP_ENTRY_BYTES
								+ items.capacity() * size_of::<Sound>()
								+ items.iter().map(Sound::heap_bytes).sum::<usize>()
						});
						let incoming = MAP_ENTRY_BYTES
							+ sounds.capacity() * size_of::<Sound>()
							+ sounds.iter().map(Sound::heap_bytes).sum::<usize>();
						if incoming > MAX_CACHE_BYTES {
							self.soundboard.error = Some("Cached list exceeds the session budget");
							self.revision += 1;
							return Ok(());
						}
						let mut projected = self
							.soundboard
							.bytes()
							.saturating_sub(previous)
							.saturating_add(incoming);
						while projected > MAX_CACHE_BYTES
							|| (!self.soundboard.lists.contains_key(&guild)
								&& self.soundboard.lists.len() >= MAX_CACHED_GUILDS)
						{
							let Some(oldest) = self
								.soundboard
								.loaded_at
								.iter()
								.filter(|(id, _)| **id != guild)
								.min_by_key(|(id, loaded)| (**loaded, **id))
								.map(|(id, _)| *id)
							else {
								self.soundboard.error =
									Some("Cached list exceeds the session budget");
								self.revision += 1;
								return Ok(());
							};
							self.soundboard.lists.remove(&oldest);
							self.soundboard.loaded_at.remove(&oldest);
							projected = self
								.soundboard
								.bytes()
								.saturating_sub(previous)
								.saturating_add(incoming);
						}
						self.soundboard.lists.insert(guild, sounds);
						self.soundboard.loaded_at.insert(guild, Instant::now());
						self.soundboard.error = None;
					}
					Err(failure) => {
						self.soundboard.error = Some(failure.label());
						if failure.ends_session() {
							self.fail(failure);
						}
					}
				}
				self.revision += 1;
			}
			Event::Played {
				channel,
				request,
				result,
			} => {
				if self.soundboard.playing != Some((channel, request)) {
					return Ok(());
				}
				self.soundboard.playing = None;
				if let Err(failure) = result {
					self.revision += 1;
					self.status = failure.label();
					if failure.ends_session() {
						self.fail(failure);
					}
				}
			}
		}
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
			user: Some(model::User {
				id: Id(1),
				name: "Owner".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
			}),
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

	fn sound(id: u64) -> Sound {
		Sound {
			id: Id(id),
			guild_id: Some(Id(9)),
			name: format!("Sound {id}"),
			volume: 1.0,
			emoji: None,
			available: true,
		}
	}

	#[test]
	fn cache_refreshes_expired_entries_and_rejects_oversized_incoming_list() {
		let mut state = guild_state();
		state.soundboard.lists.insert(Id(9), vec![sound(7)]);
		state
			.soundboard
			.loaded_at
			.insert(Id(9), Instant::now() - CACHE_TTL);
		let command = state.request_sounds(Id(9));
		assert!(command.is_some());
		let mut oversized = sound(7);
		oversized.name = "x".repeat(MAX_CACHE_BYTES);
		state
			.apply_soundboard(Event::Listed {
				guild: Id(9),
				request: state.soundboard.sequence,
				result: Ok(vec![oversized]),
			})
			.unwrap();
		assert_eq!(state.soundboard.lists[&Id(9)][0].name, sound(7).name);
		assert!(state.soundboard.error().is_some());
		assert!(state.soundboard.loading.is_none());
	}

	#[test]
	fn cache_evicts_oldest_when_incoming_list_needs_byte_space() {
		let mut state = guild_state();
		let mut large = sound(7);
		large.name = "x".repeat(MAX_CACHE_BYTES / 2 + 1);
		state.soundboard.lists.insert(Id(100), vec![large.clone()]);
		state
			.soundboard
			.loaded_at
			.insert(Id(100), Instant::now() - CACHE_TTL);
		state.soundboard.loading = Some((Id(9), 1));
		state
			.apply_soundboard(Event::Listed {
				guild: Id(9),
				request: 1,
				result: Ok(vec![large]),
			})
			.unwrap();
		assert!(state.soundboard.lists.contains_key(&Id(9)));
		assert!(!state.soundboard.lists.contains_key(&Id(100)));
		assert!(!state.soundboard.loaded_at.contains_key(&Id(100)));
		assert!(state.soundboard.bytes() <= MAX_CACHE_BYTES);
	}

	#[test]
	fn cache_evicts_oldest_to_load_a_seventeenth_guild() {
		let mut state = guild_state();
		for id in 100..100 + MAX_CACHED_GUILDS as u64 {
			state.soundboard.lists.insert(Id(id), vec![]);
			state
				.soundboard
				.loaded_at
				.insert(Id(id), Instant::now() - Duration::from_secs(200 - id));
		}
		state.soundboard.loading = Some((Id(9), 1));
		state
			.apply_soundboard(Event::Listed {
				guild: Id(9),
				request: 1,
				result: Ok(vec![sound(7)]),
			})
			.unwrap();
		assert_eq!(state.soundboard.lists.len(), MAX_CACHED_GUILDS);
		assert!(state.soundboard.lists.contains_key(&Id(9)));
		assert!(!state.soundboard.lists.contains_key(&Id(100)));
		assert!(!state.soundboard.loaded_at.contains_key(&Id(100)));
		assert_eq!(state.soundboard.loaded_at.len(), MAX_CACHED_GUILDS);
		assert!(state.soundboard.bytes() <= MAX_CACHE_BYTES);
	}

	#[test]
	fn sound_list_loads_once_and_reloads_after_errors() {
		let mut state = guild_state();
		let crate::Command::Soundboard(Command::List { guild, .. }) =
			state.request_sounds(Id(9)).unwrap()
		else {
			panic!()
		};
		assert_eq!(guild, Id(9));
		assert!(state.soundboard.loading(Id(9)));
		// A second request while loading is refused.
		assert!(state.request_sounds(Id(9)).is_none());
		// Unknown guilds never queue.
		assert!(state.request_sounds(Id(10)).is_none());
		state
			.apply_soundboard(Event::Listed {
				guild: Id(9),
				request: state.soundboard.sequence,
				result: Ok(vec![sound(7), sound(8)]),
			})
			.unwrap();
		assert_eq!(state.soundboard.sounds(Id(9)).len(), 2);
		assert!(!state.soundboard.loading(Id(9)));
		// A loaded list stays cached.
		assert!(state.request_sounds(Id(9)).is_none());
		assert!(state.soundboard.bytes() > 0);
	}

	#[test]
	fn soundboard_rechecks_permissions_and_refuses_stage() {
		let mut state = guild_state();
		state.channels.push(model::Channel {
			id: Id(2),
			guild: Some(Id(9)),
			kind: 2,
			name: "Voice".into(),
			parent_id: None,
			position: 0,
			recipients: vec![],
			icon: None,
			member_list_id: None,
			message_count: None,
			tags: None,
			last_message: None,
		});
		crate::tests::grant_permissions(&mut state);
		state.start_call(Id(2), false).unwrap();
		state.voice.active.as_mut().unwrap().phase = crate::voice::Phase::Connected;
		state.soundboard.lists.insert(Id(9), vec![sound(7)]);
		let command = state.play_sound(Id(2), Id(7)).unwrap();
		state.command_rejected(command);
		// Self microphone mute is allowed by the endpoint; server mute/suppress and deaf are not.
		state.voice.active.as_mut().unwrap().muted = true;
		assert!(state.soundboard_unavailable(Id(2)).is_none());
		for blocked in 0..3 {
			let call = state.voice.active.as_mut().unwrap();
			call.server_muted = blocked == 0;
			call.server_deafened = blocked == 1;
			call.deafened = blocked == 2;
			assert!(state.play_sound(Id(2), Id(7)).is_none());
		}
		let call = state.voice.active.as_mut().unwrap();
		call.server_muted = false;
		call.server_deafened = false;
		call.deafened = false;
		state.channels[0].kind = 13;
		assert!(state.play_sound(Id(2), Id(7)).is_none());
		state.channels[0].kind = 2;
		state
			.permissions
			.replace(model::permissions::Snapshot {
				guilds: vec![],
				channels: vec![],
			})
			.unwrap();
		assert!(state.play_sound(Id(2), Id(7)).is_none());
	}

	#[test]
	fn play_needs_a_joined_call_and_a_known_sound() {
		let mut state = guild_state();
		// No call, no play.
		assert!(state.play_sound(Id(2), Id(7)).is_none());
	}
}
