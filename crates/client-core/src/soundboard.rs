//! Guild soundboard sounds: browse and play. Creating, editing and deleting
//! sounds stay in the official client.
use crate::{
	State,
	auth::{AuthState, Failure},
};
use model::{Id, Sound};
use std::collections::BTreeMap;

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
				size_of::<Id>()
					+ sounds.capacity() * size_of::<Sound>()
					+ sounds.iter().map(Sound::heap_bytes).sum::<usize>()
			})
			.sum()
	}
}
impl State {
	/// Loads one guild's soundboard sounds; a loaded list stays until the
	/// guild changes, an error allows an explicit retry.
	pub fn request_sounds(&mut self, guild: Id) -> Option<crate::Command> {
		if self.auth != AuthState::Authenticated
			|| !self.gateway_connected
			|| guild.0 == 0
			|| self.soundboard.loading.is_some()
		{
			return None;
		}
		if self.soundboard.lists.contains_key(&guild) && self.soundboard.error.is_none() {
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

	/// Plays one known, available sound to the joined voice channel.
	pub fn play_sound(&mut self, channel: Id, sound: Id) -> Option<crate::Command> {
		if self.auth != AuthState::Authenticated
			|| !self.gateway_connected
			|| self.soundboard.playing.is_some()
		{
			return None;
		}
		let call = self.voice.active.as_ref()?;
		if call.channel != channel || call.guild.is_none() {
			return None;
		}
		if !matches!(
			call.phase,
			crate::voice::Phase::Connected | crate::voice::Phase::Waiting
		) {
			return None;
		}
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
						if self.soundboard.bytes() > model::account::MAX_BYTES {
							self.soundboard.error =
								Some("Soundboard sounds exceed the session budget");
							return Ok(());
						}
						self.soundboard.lists.insert(guild, sounds);
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
	fn play_needs_a_joined_call_and_a_known_sound() {
		let mut state = guild_state();
		// No call, no play.
		assert!(state.play_sound(Id(2), Id(7)).is_none());
	}
}
