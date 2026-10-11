use crate::Id;
use serde::{Deserialize, Serialize};

/// Discord accepts at most ten answers and a 300-character question; both are
/// enforced here so a hostile payload is bounded before it reaches the UI.
pub const MAX_POLL_ANSWERS: usize = 10;
pub const MAX_POLL_QUESTION_CHARS: usize = 300;
pub const MAX_POLL_ANSWER_CHARS: usize = 55;
pub const MAX_POLL_EMOJI_CHARS: usize = 128;
pub const MAX_POLL_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PollEmoji {
	#[serde(default)]
	pub id: Option<Id>,
	#[serde(default)]
	pub name: Option<String>,
}
impl PollEmoji {
	pub fn valid(&self) -> bool {
		self.name.as_ref().is_none_or(|name| {
			!name.is_empty()
				&& name.chars().count() <= MAX_POLL_EMOJI_CHARS
				&& !name.chars().any(char::is_control)
		}) && (self.id.is_some_and(|id| id.0 != 0) || self.name.is_some())
	}
	pub fn label(&self) -> String {
		match (self.id, self.name.as_deref()) {
			(Some(_), Some(name)) => format!(":{name}:"),
			(Some(_), None) => "Deleted emoji".into(),
			(_, name) => name.unwrap_or("Emoji").into(),
		}
	}
	pub fn heap_bytes(&self) -> usize {
		self.name.as_ref().map_or(0, String::capacity)
	}
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PollAnswer {
	pub answer_id: u64,
	pub text: String,
	#[serde(default)]
	pub emoji: Option<PollEmoji>,
}
impl PollAnswer {
	pub fn heap_bytes(&self) -> usize {
		self.text.capacity() + self.emoji.as_ref().map_or(0, PollEmoji::heap_bytes)
	}
}

/// One answer's live tally; `me_voted` is the session user's own vote.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PollCount {
	pub answer_id: u64,
	pub count: u32,
	#[serde(default)]
	pub me_voted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Poll {
	pub question: String,
	pub answers: Vec<PollAnswer>,
	/// Answers the service reported so far; missing answers read as zero.
	#[serde(default)]
	pub counts: Vec<PollCount>,
	/// False when the service omitted `results` entirely: the stored tally is then
	/// unknown, not an authoritative zero, and updates must not erase known counts.
	#[serde(default)]
	pub counts_known: bool,
	/// Nanoseconds since the Unix epoch, same convention as `Message::edited_at`.
	#[serde(default)]
	pub expiry: Option<i128>,
	#[serde(default)]
	pub allow_multiselect: bool,
	/// The service froze the results (poll ended).
	#[serde(default)]
	pub finalized: bool,
	/// Original duration in hours; 0 means unknown or unlimited.
	#[serde(default)]
	pub duration: u32,
}
impl Poll {
	pub fn answer(&self, answer_id: u64) -> Option<&PollAnswer> {
		self.answers.iter().find(|a| a.answer_id == answer_id)
	}
	pub fn count(&self, answer_id: u64) -> u32 {
		self.counts
			.iter()
			.find(|c| c.answer_id == answer_id)
			.map_or(0, |c| c.count)
	}
	pub fn me_voted(&self, answer_id: u64) -> bool {
		self.counts
			.iter()
			.any(|c| c.answer_id == answer_id && c.me_voted)
	}
	pub fn voted(&self) -> bool {
		self.counts.iter().any(|c| c.me_voted)
	}
	pub fn total_votes(&self) -> u32 {
		self.counts
			.iter()
			.fold(0, |total, c| total.saturating_add(c.count))
	}
	/// Whether the service already closed the poll, by finalization or expiry.
	pub fn closed(&self, now_nanos: i128) -> bool {
		self.finalized || self.expiry.is_some_and(|expiry| expiry <= now_nanos)
	}
	/// Single-select polls only accept another answer while the user has no vote.
	pub fn can_pick_more(&self) -> bool {
		self.allow_multiselect || !self.voted()
	}
	pub fn bytes(&self) -> usize {
		std::mem::size_of::<Self>()
			+ self.question.capacity()
			+ self.answers.capacity() * std::mem::size_of::<PollAnswer>()
			+ self
				.answers
				.iter()
				.map(PollAnswer::heap_bytes)
				.sum::<usize>()
			+ self.counts.capacity() * std::mem::size_of::<PollCount>()
	}
}
pub fn poll_bytes(poll: &Option<Poll>) -> usize {
	poll.as_ref().map_or(0, Poll::bytes)
}
pub fn valid_poll(poll: &Poll) -> bool {
	poll.answers.len() <= MAX_POLL_ANSWERS
		&& !poll.answers.is_empty()
		&& !poll.question.is_empty()
		&& poll.question.chars().count() <= MAX_POLL_QUESTION_CHARS
		&& poll.bytes() <= MAX_POLL_BYTES
		&& poll.answers.iter().enumerate().all(|(index, answer)| {
			answer.answer_id != 0
				&& !answer.text.is_empty()
				&& answer.text.chars().count() <= MAX_POLL_ANSWER_CHARS
				&& answer.emoji.as_ref().is_none_or(PollEmoji::valid)
				&& !poll.answers[..index]
					.iter()
					.any(|other| other.answer_id == answer.answer_id)
		}) && poll
		.counts
		.iter()
		.all(|count| poll.answer(count.answer_id).is_some())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn answer(id: u64) -> PollAnswer {
		PollAnswer {
			answer_id: id,
			text: format!("Option {id}"),
			emoji: None,
		}
	}

	#[test]
	fn bounds_accept_discord_limits_and_reject_overreach() {
		let mut poll = Poll {
			question: "Best layout?".into(),
			answers: (1..=MAX_POLL_ANSWERS as u64).map(answer).collect(),
			counts: vec![PollCount {
				answer_id: 1,
				count: 3,
				me_voted: true,
			}],
			counts_known: true,
			expiry: None,
			allow_multiselect: false,
			finalized: false,
			duration: 24,
		};
		assert!(valid_poll(&poll));
		poll.answers.push(answer(11));
		assert!(!valid_poll(&poll), "eleven answers are refused");
		poll.answers.truncate(1);
		poll.question = "x".repeat(MAX_POLL_QUESTION_CHARS + 1);
		assert!(!valid_poll(&poll), "long questions are refused");
		poll.question = "Best layout?".into();
		poll.answers[0].text = "x".repeat(MAX_POLL_ANSWER_CHARS + 1);
		assert!(!valid_poll(&poll), "long answers are refused");
		poll.answers[0].text = "One".into();
		poll.counts[0].answer_id = 99;
		assert!(!valid_poll(&poll), "counts must reference an answer");
		poll.counts[0].answer_id = 1;
		poll.counts[0].me_voted = false;
		assert!(valid_poll(&poll));
	}

	#[test]
	fn tallies_read_missing_answers_as_zero_and_close_on_expiry() {
		let poll = Poll {
			question: "Pick".into(),
			answers: vec![answer(1), answer(2)],
			counts: vec![
				PollCount {
					answer_id: 1,
					count: 4,
					me_voted: true,
				},
				PollCount {
					answer_id: 2,
					count: 6,
					me_voted: false,
				},
			],
			counts_known: true,
			expiry: Some(100),
			allow_multiselect: false,
			finalized: false,
			duration: 1,
		};
		assert_eq!(poll.count(1), 4);
		assert_eq!(poll.count(2), 6);
		assert_eq!(poll.count(3), 0, "missing answer reads as zero");
		assert_eq!(poll.total_votes(), 10);
		assert!(poll.me_voted(1) && !poll.me_voted(2) && poll.voted());
		assert!(
			!poll.can_pick_more(),
			"single select with a vote cannot pick again"
		);
		assert!(!poll.closed(99));
		assert!(poll.closed(100), "expiry closes the poll");
		let mut ended = poll.clone();
		ended.finalized = true;
		assert!(ended.closed(0), "finalized closes regardless of expiry");
		let multi = Poll {
			allow_multiselect: true,
			..poll
		};
		assert!(multi.can_pick_more());
	}
}
