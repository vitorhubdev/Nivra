//! Poll voting: one bounded optimistic write at a time, reconciled by the
//! service's own tally updates (MESSAGE_UPDATE) and gateway vote deltas.
use crate::{
	State,
	auth::{AuthState, Failure},
};
use model::{Freshness, Id, Poll};
use std::collections::VecDeque;

pub enum Command {
	Vote {
		channel: Id,
		message: Id,
		/// The full selection after the click; empty clears the caller's votes.
		answer_ids: Vec<u64>,
		request: u64,
	},
	/// Publish a new poll as a message; answers carry text only.
	Create {
		channel: Id,
		question: String,
		answers: Vec<String>,
		duration_hours: u32,
		multiselect: bool,
		nonce: String,
		request: u64,
	},
}
pub enum Event {
	/// `MESSAGE_POLL_VOTE_ADD`/`REMOVE`; `user` is the voter.
	Delta {
		channel: Id,
		message: Id,
		answer_id: u64,
		user: Id,
		add: bool,
	},
	Written {
		channel: Id,
		message: Id,
		request: u64,
		result: Result<(), Failure>,
	},
	Created {
		channel: Id,
		request: u64,
		result: Result<Box<model::Message>, Failure>,
	},
}

/// Durations Discord offers for a poll, in hours.
pub const CREATE_DURATIONS_HOURS: [u32; 6] = [1, 4, 8, 24, 72, 168];

/// How many own vote echoes from successful writes may await the Gateway.
const MAX_RECENT_ECHOES: usize = 16;

#[derive(Default)]
pub struct Polls {
	/// The single in-flight vote, so a second click cannot race the first.
	writing: Option<(Id, u64)>,
	/// The single in-flight poll creation, so a double submit cannot duplicate it.
	creating: Option<(Id, u64)>,
	/// One bounded poll before/after the in-flight vote.
	preview: Option<(Id, Poll, Poll)>,
	/// Own vote echoes the in-flight write is expected to produce.
	pending_echoes: Vec<(u64, bool)>,
	/// Bounded own echoes already committed, still waiting on the Gateway.
	recent_echoes: VecDeque<(Id, u64, bool)>,
	sequence: u64,
}
impl Polls {
	pub fn reset(&mut self) {
		self.writing = None;
		self.creating = None;
		self.preview = None;
		self.pending_echoes.clear();
		self.recent_echoes.clear();
		self.sequence = self.sequence.wrapping_add(1);
	}
	pub fn busy(&self) -> bool {
		self.writing.is_some() || self.creating.is_some()
	}
	/// The optimistic tally while a vote is in flight, otherwise the stored poll.
	pub fn display<'a>(&'a self, message: &'a model::Message) -> Option<&'a Poll> {
		self.preview
			.as_ref()
			.filter(|(id, _, _)| *id == message.id)
			.map(|(_, _, after)| after)
			.or(message.poll.as_ref())
	}
	pub fn invalidated(&self, id: Id) -> bool {
		self.writing.is_some_and(|(message, _)| message == id)
	}
}
impl State {
	/// Publishes a new poll. Two to ten non-empty answers, a question, and one
	/// of the offered durations; text answers only. Like app submissions, the
	/// service echo paints the card and failures surface as status.
	pub fn prepare_poll_create(
		&mut self,
		question: &str,
		answers: &[String],
		duration_hours: u32,
		multiselect: bool,
	) -> Option<crate::Command> {
		if self.auth != AuthState::Authenticated
			|| !self.gateway_connected
			|| self.freshness != Freshness::Fresh
			|| self.polls.creating.is_some()
		{
			return None;
		}
		let channel = self.selected?;
		if !self.can_send(channel) {
			self.status = "Sending is unavailable with the current connection or permissions";
			return None;
		}
		let question = question.trim();
		let answers: Vec<String> = answers
			.iter()
			.map(|answer| answer.trim().to_owned())
			.filter(|answer| !answer.is_empty())
			.collect();
		if question.is_empty()
			|| question.chars().count() > model::MAX_POLL_QUESTION_CHARS
			|| answers.len() < 2
			|| answers.len() > model::MAX_POLL_ANSWERS
			|| answers
				.iter()
				.any(|answer| answer.chars().count() > model::MAX_POLL_ANSWER_CHARS)
			|| !CREATE_DURATIONS_HOURS.contains(&duration_hours)
		{
			self.status = "A poll needs a question and 2 to 10 short answers";
			return None;
		}
		self.polls.sequence = self.polls.sequence.wrapping_add(1);
		let request = self.polls.sequence;
		let epoch = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap_or_default()
			.as_millis();
		let nonce = crate::fingerprint::nonce(epoch, request);
		self.polls.creating = Some((channel, request));
		self.revision += 1;
		Some(crate::Command::Polls(Command::Create {
			channel,
			question: question.to_owned(),
			answers,
			duration_hours,
			multiselect,
			nonce,
			request,
		}))
	}
	/// Toggles the caller's vote on one answer and returns the write command.
	pub fn prepare_poll_vote(&mut self, message: Id, answer_id: u64) -> Option<crate::Command> {
		if self.auth != AuthState::Authenticated
			|| !self.gateway_connected
			|| self.freshness != Freshness::Fresh
			|| self.polls.writing.is_some()
		{
			return None;
		}
		let channel = self.selected?;
		if self.timeline.is_deleted(message) {
			return None;
		}
		let source = self.timeline.get(message)?;
		if source.channel != channel || !self.can_read_history(channel) {
			return None;
		}
		let before = self.polls.display(source)?.clone();
		if before.closed(now_nanos()) || before.answer(answer_id).is_none() {
			return None;
		}
		let add = !before.me_voted(answer_id);
		let mut after = before.clone();
		toggle(&mut after, answer_id, add, true).ok()?;
		let changes: Vec<(u64, bool)> = before
			.answers
			.iter()
			.filter_map(|answer| {
				let was = before.me_voted(answer.answer_id);
				let now = after.me_voted(answer.answer_id);
				(was != now).then_some((answer.answer_id, now))
			})
			.collect();
		let answer_ids: Vec<u64> = after
			.counts
			.iter()
			.filter(|count| count.me_voted)
			.map(|count| count.answer_id)
			.collect();
		self.polls.preview = Some((message, before, after));
		self.polls.pending_echoes = changes;
		self.revision += 1;
		self.polls.sequence = self.polls.sequence.wrapping_add(1);
		let request = self.polls.sequence;
		self.polls.writing = Some((message, request));
		Some(crate::Command::Polls(Command::Vote {
			channel,
			message,
			answer_ids,
			request,
		}))
	}
	pub fn apply_polls(&mut self, event: Event) -> Result<(), &'static str> {
		match event {
			Event::Created {
				channel,
				request,
				result,
			} => {
				if self.selected != Some(channel) || self.polls.creating != Some((channel, request))
				{
					return Ok(());
				}
				self.polls.creating = None;
				match result {
					Ok(_) => {
						// The service echo (MESSAGE_CREATE) paints the new poll;
						// nothing optimistic is staged.
						self.revision += 1;
					}
					Err(failure) => {
						self.revision += 1;
						self.status = failure.label();
						if failure.ends_session() {
							self.fail(failure);
						}
					}
				}
			}
			Event::Delta {
				channel,
				message,
				answer_id,
				user,
				add,
			} => {
				if answer_id == 0 || user.0 == 0 {
					return Err("Invalid poll vote delta");
				}
				let own = self.user.as_ref().is_some_and(|me| me.id == user);
				// The REST response and the Gateway dispatch travel independently: an
				// own echo that belongs to a known write is consumed here so it can
				// neither double the optimistic tally nor race the write result.
				if own {
					let correlation = (answer_id, add);
					if let Some(index) = self
						.polls
						.pending_echoes
						.iter()
						.position(|pending| *pending == correlation)
					{
						self.polls.pending_echoes.remove(index);
						return Ok(());
					}
					if let Some(index) =
						self.polls
							.recent_echoes
							.iter()
							.position(|(id, answer, adding)| {
								*id == message && *answer == answer_id && *adding == add
							}) {
						self.polls.recent_echoes.remove(index);
						return Ok(());
					}
				}
				self.update_poll(channel, message, |poll| toggle(poll, answer_id, add, own))?;
			}
			Event::Written {
				channel,
				message,
				request,
				result,
			} => {
				if self.selected != Some(channel) || self.polls.writing != Some((message, request))
				{
					return Ok(());
				}
				self.polls.writing = None;
				match result {
					Ok(()) => {
						for (answer, add) in self.polls.pending_echoes.drain(..) {
							if self.polls.recent_echoes.len() == MAX_RECENT_ECHOES {
								self.polls.recent_echoes.pop_front();
							}
							self.polls.recent_echoes.push_back((message, answer, add));
						}
						// No readback on the vote route: commit the optimistic tally; a later
						// MESSAGE_UPDATE with authoritative results replaces it.
						if let Some((_, _, after)) = self.polls.preview.take() {
							self.timeline.set_poll(message, Some(after))?;
						}
						self.revision += 1;
					}
					Err(failure) => {
						self.polls.pending_echoes.clear();
						if let Some((_, before, _)) = self.polls.preview.take() {
							self.timeline.set_poll(message, Some(before))?;
						}
						self.revision += 1;
						self.status = failure.label();
						if failure.ends_session() {
							self.fail(failure);
						}
					}
				}
			}
		}
		Ok(())
	}
	fn update_poll(
		&mut self,
		channel: Id,
		message: Id,
		update: impl Fn(&mut Poll) -> Result<(), &'static str>,
	) -> Result<(), &'static str> {
		if self.selected != Some(channel)
			|| !self.can_view(channel)
			|| self.freshness == Freshness::Unavailable
			|| !self.gateway_connected
		{
			return Ok(());
		}
		let preview = self
			.polls
			.preview
			.as_ref()
			.filter(|(id, _, _)| *id == message);
		let known = self
			.timeline
			.get(message)
			.and_then(|m| m.poll.as_ref())
			.or_else(|| preview.map(|(_, before, _)| before));
		let Some(mut value) = known.cloned() else {
			return Ok(());
		};
		update(&mut value)?;
		if !model::valid_poll(&value) {
			return Ok(());
		}
		let mut preview = preview.cloned();
		if let Some((_, before, after)) = &mut preview {
			update(before)?;
			update(after)?;
		}
		self.polls.preview = preview;
		self.timeline.set_poll(message, Some(value))?;
		self.revision += 1;
		Ok(())
	}
}

/// Adds or removes one tally. An own vote on a single-select poll replaces the
/// previous choice the way the service does.
pub fn toggle(poll: &mut Poll, answer_id: u64, add: bool, own: bool) -> Result<(), &'static str> {
	if poll.answer(answer_id).is_none() {
		return Err("Unknown poll answer");
	}
	if own && add && !poll.allow_multiselect {
		for count in poll.counts.iter_mut().filter(|c| c.me_voted) {
			count.count = count.count.saturating_sub(1);
			count.me_voted = false;
		}
	}
	if let Some(count) = poll.counts.iter_mut().find(|c| c.answer_id == answer_id) {
		count.count = if add {
			count.count.saturating_add(1)
		} else {
			count.count.saturating_sub(1)
		};
		if own {
			count.me_voted = add;
		}
	} else if add {
		poll.counts.push(model::PollCount {
			answer_id,
			count: 1,
			me_voted: own,
		});
	}
	Ok(())
}

/// Applies a service-shaped selection to a poll for the offline fixture: every
/// listed answer ends up selected and every other answer loses the caller's vote.
pub fn apply_selection(poll: &mut Poll, answer_ids: &[u64]) -> Result<(), &'static str> {
	if answer_ids.len() > model::MAX_POLL_ANSWERS || answer_ids.contains(&0) {
		return Err("Invalid poll selection");
	}
	if !poll.allow_multiselect && answer_ids.len() > 1 {
		return Err("Single-select polls accept one answer");
	}
	for answer_id in answer_ids {
		if poll.answer(*answer_id).is_none() {
			return Err("Unknown poll answer");
		}
	}
	let current: Vec<u64> = poll.answers.iter().map(|answer| answer.answer_id).collect();
	for answer_id in current {
		if !answer_ids.contains(&answer_id) && poll.me_voted(answer_id) {
			toggle(poll, answer_id, false, true)?;
		}
	}
	for answer_id in answer_ids {
		if !poll.me_voted(*answer_id) {
			toggle(poll, *answer_id, true, true)?;
		}
	}
	Ok(())
}

fn now_nanos() -> i128 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|elapsed| elapsed.as_nanos() as i128)
		.unwrap_or(0)
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::{PollAnswer, PollCount};

	fn poll(multiselect: bool, expiry: Option<i128>) -> Poll {
		Poll {
			question: "Pick".into(),
			answers: vec![
				PollAnswer {
					answer_id: 1,
					text: "One".into(),
					emoji: None,
				},
				PollAnswer {
					answer_id: 2,
					text: "Two".into(),
					emoji: None,
				},
			],
			counts: vec![
				PollCount {
					answer_id: 1,
					count: 3,
					me_voted: false,
				},
				PollCount {
					answer_id: 2,
					count: 2,
					me_voted: false,
				},
			],
			counts_known: true,
			expiry,
			allow_multiselect: multiselect,
			finalized: false,
			duration: 24,
		}
	}

	#[test]
	fn service_selection_replaces_single_choice_and_validates_bounds() {
		let mut value = poll(false, None);
		apply_selection(&mut value, &[2]).unwrap();
		assert!(!value.me_voted(1) && value.me_voted(2));
		assert_eq!(value.count(1), 3);
		assert_eq!(value.count(2), 3);
		assert!(apply_selection(&mut value, &[1, 2]).is_err());
		assert!(apply_selection(&mut value, &[0]).is_err());
		assert!(apply_selection(&mut value, &[9]).is_err());
		assert!(apply_selection(&mut value, &[1; model::MAX_POLL_ANSWERS + 1]).is_err());
		apply_selection(&mut value, &[]).unwrap();
		assert!(!value.voted());
		let mut multi = poll(true, None);
		apply_selection(&mut multi, &[1, 2]).unwrap();
		assert!(multi.me_voted(1) && multi.me_voted(2));
	}

	#[test]
	fn own_single_select_vote_moves_the_choice_and_the_count() {
		let mut value = poll(false, None);
		toggle(&mut value, 1, true, true).unwrap();
		assert!(value.me_voted(1));
		assert_eq!(value.count(1), 4);
		// Switching answers releases the old one and claims the new one.
		toggle(&mut value, 2, true, true).unwrap();
		assert!(!value.me_voted(1) && value.me_voted(2));
		assert_eq!(value.count(1), 3);
		assert_eq!(value.count(2), 3);
		assert_eq!(value.total_votes(), 6);
		// Removing the own vote only lowers the tally.
		toggle(&mut value, 2, false, true).unwrap();
		assert!(!value.voted());
		assert_eq!(value.count(2), 2);
	}

	#[test]
	fn multiselect_keeps_every_own_vote_and_remote_deltas_never_mark_me() {
		let mut value = poll(true, None);
		toggle(&mut value, 1, true, true).unwrap();
		toggle(&mut value, 2, true, true).unwrap();
		assert!(value.me_voted(1) && value.me_voted(2));
		assert_eq!(value.total_votes(), 7);
		toggle(&mut value, 1, false, false).unwrap();
		assert!(value.me_voted(1), "a remote removal never clears my vote");
		assert_eq!(value.count(1), 3);
	}

	#[test]
	fn unknown_answers_and_saturated_tallies_never_overflow() {
		let mut value = poll(false, None);
		assert!(toggle(&mut value, 9, true, true).is_err());
		value.counts[0].count = u32::MAX;
		toggle(&mut value, 1, true, false).unwrap();
		assert_eq!(value.count(1), u32::MAX);
		value.counts[0].count = 0;
		toggle(&mut value, 1, false, false).unwrap();
		assert_eq!(value.count(1), 0);
	}

	#[test]
	fn closed_polls_report_closed_by_expiry_or_finalization() {
		let value = poll(false, Some(now_nanos() - 1));
		assert!(value.closed(now_nanos()));
		let mut ended = poll(false, None);
		ended.finalized = true;
		assert!(ended.closed(now_nanos()));
		assert!(!poll(false, Some(now_nanos() + 60_000_000_000)).closed(now_nanos()));
	}

	#[test]
	fn prepare_and_apply_reconciles_one_vote_at_a_time() {
		use crate::auth::AuthState;
		use model::{Channel, Freshness, Id, User, permissions as p};

		let user = User {
			primary_guild: None,
			id: Id(2),
			name: "Synthetic".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
		};
		let mut state = State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			freshness: Freshness::Fresh,
			selected: Some(Id(10)),
			user: Some(user.clone()),
			guilds: vec![model::Guild {
				stickers: None,
				id: Id(1),
				name: "Synthetic".into(),
				icon: None,
				emojis: None,
				premium_tier: 0,
			}],
			channels: vec![Channel {
				id: Id(10),
				guild: Some(Id(1)),
				kind: 0,
				name: "Synthetic".into(),
				parent_id: None,
				last_message: None,
				position: 0,
				recipients: vec![],
				icon: None,
				member_list_id: None,
				message_count: None,
				tags: None,
			}],
			..State::default()
		};
		state
			.permissions
			.replace(p::Snapshot {
				guilds: vec![p::Guild {
					id: Id(1),
					owner: Some(Id(999)),
					roles: Some(vec![p::Role {
						name: String::new(),
						color: 0,
						position: 0,
						hoist: false,
						id: Id(1),
						bits: p::VIEW_CHANNEL | p::READ_MESSAGE_HISTORY,
					}]),
					member: Some(p::Member {
						roles: vec![],
						timeout_until: None,
					}),
				}],
				channels: vec![p::Channel {
					id: Id(10),
					guild: Id(1),
					overwrites: Some(vec![]),
				}],
			})
			.unwrap();
		let mut message = crate::tests::message(50);
		message.channel = Id(10);
		message.author = user;
		state.timeline.insert(message, true, false).unwrap();
		state
			.timeline
			.set_poll(Id(50), Some(poll(false, None)))
			.unwrap();

		let Some(crate::Command::Polls(Command::Vote {
			answer_ids,
			request,
			..
		})) = state.prepare_poll_vote(Id(50), 1)
		else {
			panic!("an open poll accepts a vote")
		};
		assert_eq!(answer_ids, vec![1]);
		let displayed = state
			.polls
			.display(state.timeline.get(Id(50)).unwrap())
			.unwrap();
		assert!(displayed.me_voted(1));
		assert_eq!(displayed.count(1), 4);
		// Our own gateway echo cannot double the optimistic tally.
		state
			.apply_polls(Event::Delta {
				channel: Id(10),
				message: Id(50),
				answer_id: 1,
				user: Id(2),
				add: true,
			})
			.unwrap();
		assert_eq!(
			state
				.polls
				.display(state.timeline.get(Id(50)).unwrap())
				.unwrap()
				.count(1),
			4
		);
		// A successful write commits the optimistic tally.
		state
			.apply_polls(Event::Written {
				channel: Id(10),
				message: Id(50),
				request,
				result: Ok(()),
			})
			.unwrap();
		assert_eq!(
			state
				.timeline
				.get(Id(50))
				.unwrap()
				.poll
				.as_ref()
				.unwrap()
				.count(1),
			4
		);
		// The own echo can arrive after the HTTP completion; it must still be
		// consumed instead of double-counting the committed vote.
		state
			.apply_polls(Event::Delta {
				channel: Id(10),
				message: Id(50),
				answer_id: 1,
				user: Id(2),
				add: true,
			})
			.unwrap();
		assert_eq!(
			state
				.timeline
				.get(Id(50))
				.unwrap()
				.poll
				.as_ref()
				.unwrap()
				.count(1),
			4,
			"a late own echo never double-counts"
		);
		// A rejected write restores the pre-vote poll.
		let Some(crate::Command::Polls(Command::Vote {
			answer_ids,
			request,
			..
		})) = state.prepare_poll_vote(Id(50), 1)
		else {
			panic!("a voted answer can be un-voted")
		};
		assert!(answer_ids.is_empty(), "un-voting clears the selection");
		assert!(
			!state
				.polls
				.display(state.timeline.get(Id(50)).unwrap())
				.unwrap()
				.me_voted(1)
		);
		state
			.apply_polls(Event::Written {
				channel: Id(10),
				message: Id(50),
				request,
				result: Err(Failure::RateLimited),
			})
			.unwrap();
		assert!(
			state
				.timeline
				.get(Id(50))
				.unwrap()
				.poll
				.as_ref()
				.unwrap()
				.me_voted(1),
			"rollback keeps the committed vote"
		);
		// A remote delta raises the tally without touching my vote.
		state
			.apply_polls(Event::Delta {
				channel: Id(10),
				message: Id(50),
				answer_id: 2,
				user: Id(3),
				add: true,
			})
			.unwrap();
		let stored = state.timeline.get(Id(50)).unwrap().poll.clone().unwrap();
		assert_eq!(stored.count(2), 3);
		assert!(!stored.me_voted(2));
		// A disconnect clears the in-flight vote so a lost response cannot block
		// every later vote for the rest of the session.
		assert!(state.prepare_poll_vote(Id(50), 1).is_some());
		assert!(state.polls.busy());
		state.apply(crate::Envelope {
			generation: state.generation,
			event: crate::Event::Disconnected,
		});
		assert!(!state.polls.busy());
		// A closed poll refuses new votes.
		let mut closed = stored;
		closed.finalized = true;
		state.timeline.set_poll(Id(50), Some(closed)).unwrap();
		assert!(state.prepare_poll_vote(Id(50), 1).is_none());
	}
}
