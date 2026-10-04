//! Bounded poll DTOs. Discord polls are a small, fixed shape: one question,
//! up to ten answers and a live tally. Everything is capped while parsing so a
//! hostile payload cannot retain oversized text or answer lists.
use crate::{Id, Timestamp};
use model::Poll;
use serde::{
	Deserialize, Deserializer,
	de::{IgnoredAny, MapAccess, SeqAccess, Visitor},
};
use std::fmt;

/// Field cap matching the old opaque-object limit: a poll never carries more.
const MAX_POLL_FIELDS: usize = 64;

#[derive(Default)]
pub struct PollDto {
	question: PollQuestionDto,
	answers: BoundedAnswers,
	expiry: Option<Timestamp>,
	allow_multiselect: bool,
	layout_type: u8,
	results: Option<PollResultsDto>,
	duration: u8,
}

enum PollField {
	Question,
	Answers,
	Expiry,
	AllowMultiselect,
	Layout,
	Results,
	Duration,
	Unknown,
}
impl<'de> Deserialize<'de> for PollField {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct Field;
		impl Visitor<'_> for Field {
			type Value = PollField;
			fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
				f.write_str("a poll field name")
			}
			fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<PollField, E> {
				Ok(match value {
					"question" => PollField::Question,
					"answers" => PollField::Answers,
					"expiry" => PollField::Expiry,
					"allow_multiselect" => PollField::AllowMultiselect,
					"layout_type" => PollField::Layout,
					"results" => PollField::Results,
					"duration" => PollField::Duration,
					_ => PollField::Unknown,
				})
			}
		}
		deserializer.deserialize_str(Field)
	}
}
impl<'de> Deserialize<'de> for PollDto {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct PollVisitor;
		impl<'de> Visitor<'de> for PollVisitor {
			type Value = PollDto;
			fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
				f.write_str("a bounded poll object")
			}
			fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<PollDto, A::Error> {
				let mut poll = PollDto::default();
				let mut fields = 0usize;
				while let Some(field) = map.next_key::<PollField>()? {
					if fields == MAX_POLL_FIELDS {
						return Err(serde::de::Error::custom("Poll object exceeds capacity"));
					}
					fields += 1;
					match field {
						PollField::Question => poll.question = map.next_value()?,
						PollField::Answers => poll.answers = map.next_value()?,
						PollField::Expiry => poll.expiry = map.next_value()?,
						PollField::AllowMultiselect => poll.allow_multiselect = map.next_value()?,
						PollField::Layout => poll.layout_type = map.next_value()?,
						PollField::Results => poll.results = map.next_value()?,
						PollField::Duration => poll.duration = map.next_value()?,
						PollField::Unknown => {
							map.next_value::<IgnoredAny>()?;
						}
					}
				}
				Ok(poll)
			}
		}
		deserializer.deserialize_map(PollVisitor)
	}
}

#[derive(Deserialize, Default)]
struct PollQuestionDto {
	#[serde(default, deserialize_with = "question_text")]
	text: String,
}

#[derive(Default)]
struct BoundedAnswers {
	answers: Vec<PollAnswerDto>,
	/// More than [`model::MAX_POLL_ANSWERS`] entries arrived: keep presence, drop the card.
	overflow: bool,
}
impl<'de> Deserialize<'de> for BoundedAnswers {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct Answers;
		impl<'de> Visitor<'de> for Answers {
			type Value = BoundedAnswers;
			fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
				f.write_str("a bounded answer list")
			}
			fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<BoundedAnswers, A::Error> {
				let mut answers = Vec::with_capacity(model::MAX_POLL_ANSWERS);
				let mut overflow = false;
				while let Some(answer) = seq.next_element::<PollAnswerDto>()? {
					if answers.len() < model::MAX_POLL_ANSWERS {
						answers.push(answer);
					} else {
						overflow = true;
					}
				}
				Ok(BoundedAnswers { answers, overflow })
			}
		}
		deserializer.deserialize_seq(Answers)
	}
}

#[derive(Deserialize)]
struct PollAnswerDto {
	#[serde(default)]
	answer_id: u64,
	#[serde(default)]
	poll_media: PollMediaDto,
}

#[derive(Deserialize, Default)]
struct PollMediaDto {
	#[serde(default, deserialize_with = "answer_text")]
	text: Option<String>,
	#[serde(default)]
	emoji: Option<PollEmojiDto>,
}

#[derive(Deserialize)]
struct PollEmojiDto {
	#[serde(default)]
	id: Option<Id>,
	#[serde(default, deserialize_with = "emoji_name")]
	name: Option<String>,
}

#[derive(Deserialize)]
struct PollResultsDto {
	#[serde(default)]
	is_finalized: bool,
	#[serde(default, deserialize_with = "bounded_counts")]
	answer_counts: Vec<PollAnswerCountDto>,
}

#[derive(Deserialize)]
pub struct PollAnswerCountDto {
	#[serde(default)]
	id: u64,
	#[serde(default)]
	count: u32,
	#[serde(default)]
	me_voted: bool,
}

/// `MESSAGE_POLL_VOTE_ADD`/`REMOVE` payload: one answer and one voter.
#[derive(Deserialize)]
pub struct PollVoteDelta {
	pub channel_id: Id,
	pub message_id: Id,
	pub user_id: Id,
	pub answer_id: u64,
}

impl PollDto {
	/// A readable poll, or `None` when the shape is unusable (presence stays).
	pub fn into_model(self) -> Option<Poll> {
		if self.answers.overflow {
			return None;
		}
		let question: String = self
			.question
			.text
			.chars()
			.take(model::MAX_POLL_QUESTION_CHARS)
			.collect();
		let mut answers = Vec::with_capacity(self.answers.answers.len());
		for answer in self.answers.answers {
			let text: String = answer
				.poll_media
				.text
				.unwrap_or_default()
				.chars()
				.take(model::MAX_POLL_ANSWER_CHARS)
				.collect();
			let emoji = answer.poll_media.emoji.map(|emoji| model::PollEmoji {
				id: emoji.id,
				name: emoji.name,
			});
			answers.push(model::PollAnswer {
				answer_id: answer.answer_id,
				text,
				emoji,
			});
		}
		let results = self.results;
		let counts = results
			.as_ref()
			.map(|results| {
				results
					.answer_counts
					.iter()
					.filter(|count| count.id != 0)
					.map(|count| model::PollCount {
						answer_id: count.id,
						count: count.count,
						me_voted: count.me_voted,
					})
					.collect::<Vec<_>>()
			})
			.unwrap_or_default();
		let poll = Poll {
			question,
			answers,
			counts,
			expiry: self.expiry.map(|expiry| expiry.0),
			allow_multiselect: self.allow_multiselect,
			finalized: results.is_some_and(|results| results.is_finalized),
			duration: u32::from(self.duration),
		};
		// `layout_type` is presentation-only today; 1 is the default list.
		let _ = self.layout_type;
		model::valid_poll(&poll).then_some(poll)
	}
}

/// Presence bit for a poll patch or full message without retaining the payload.
pub fn poll_patch<T>(value: &model::Patch<T>) -> model::Patch<bool> {
	match value {
		model::Patch::Absent => model::Patch::Absent,
		model::Patch::Null => model::Patch::Null,
		model::Patch::Value(_) => model::Patch::Value(true),
	}
}

/// Reads at most `limit` characters, dropping the rest before storing.
fn bounded_text<'de, D: Deserializer<'de>>(
	deserializer: D,
	limit: usize,
) -> Result<String, D::Error> {
	struct Bounded(usize);
	impl<'de> Visitor<'de> for Bounded {
		type Value = String;
		fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
			write!(f, "at most {} characters", self.0)
		}
		fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<String, E> {
			Ok(value.chars().take(self.0).collect())
		}
	}
	deserializer.deserialize_str(Bounded(limit))
}

/// `None` for a missing or null value; otherwise bounded like [`bounded_text`].
fn bounded_optional_text<'de, D: Deserializer<'de>>(
	deserializer: D,
	limit: usize,
) -> Result<Option<String>, D::Error> {
	struct Bounded(usize);
	impl<'de> Visitor<'de> for Bounded {
		type Value = Option<String>;
		fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
			write!(f, "at most {} characters or null", self.0)
		}
		fn visit_none<E: serde::de::Error>(self) -> Result<Option<String>, E> {
			Ok(None)
		}
		fn visit_unit<E: serde::de::Error>(self) -> Result<Option<String>, E> {
			Ok(None)
		}
		fn visit_some<D: Deserializer<'de>>(
			self,
			deserializer: D,
		) -> Result<Option<String>, D::Error> {
			bounded_text(deserializer, self.0).map(Some)
		}
	}
	deserializer.deserialize_option(Bounded(limit))
}

// Monomorphic wrappers: serde's `deserialize_with` path cannot carry extra arguments.
fn question_text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
	bounded_text(deserializer, model::MAX_POLL_QUESTION_CHARS)
}
fn answer_text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
	bounded_optional_text(deserializer, model::MAX_POLL_ANSWER_CHARS)
}
fn emoji_name<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
	bounded_optional_text(deserializer, model::MAX_POLL_EMOJI_CHARS)
}

/// Up to [`model::MAX_POLL_ANSWERS`] tallies; extras are skipped, not retained.
fn bounded_counts<'de, D: Deserializer<'de>>(
	deserializer: D,
) -> Result<Vec<PollAnswerCountDto>, D::Error> {
	struct Bounded;
	impl<'de> Visitor<'de> for Bounded {
		type Value = Vec<PollAnswerCountDto>;
		fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
			f.write_str("a bounded tally list")
		}
		fn visit_seq<A: SeqAccess<'de>>(
			self,
			mut seq: A,
		) -> Result<Vec<PollAnswerCountDto>, A::Error> {
			let mut counts = Vec::with_capacity(model::MAX_POLL_ANSWERS);
			while let Some(count) = seq.next_element::<PollAnswerCountDto>()? {
				if counts.len() < model::MAX_POLL_ANSWERS {
					counts.push(count);
				}
			}
			Ok(counts)
		}
	}
	deserializer.deserialize_seq(Bounded)
}

#[cfg(test)]
mod tests {
	use crate::{MessageDto, PatchDto, decode};
	use model::Patch;

	fn wire(fields: &str) -> String {
		format!(
			r#"{{"id":"1","channel_id":"2","author":{{"id":"3","username":"Synthetic"}}{fields}}}"#
		)
	}

	fn poll(payload: &str) -> String {
		wire(&format!(r#", "poll":{payload}"#))
	}

	#[test]
	fn readable_polls_map_every_field_and_tally() {
		let message = decode::<MessageDto>(
			poll(r#"{"question":{"text":"Best layout?"},"answers":[{"answer_id":1,"poll_media":{"text":"Cozy","emoji":{"name":"🧸"}}},{"answer_id":2,"poll_media":{"text":"Compact"}}],"expiry":"2026-10-04T00:00:00+00:00","allow_multiselect":false,"layout_type":1,"results":{"is_finalized":false,"answer_counts":[{"id":1,"count":2,"me_voted":true},{"id":2,"count":5,"me_voted":false}]},"duration":24}"#)
				.as_bytes(),
		)
		.unwrap()
		.into_model();
		assert!(message.extra_content.poll);
		let poll = message.poll.expect("poll parses");
		assert_eq!(poll.question, "Best layout?");
		assert_eq!(poll.answers.len(), 2);
		assert_eq!(poll.answers[0].text, "Cozy");
		assert_eq!(
			poll.answers[0]
				.emoji
				.as_ref()
				.and_then(|emoji| emoji.name.as_deref()),
			Some("🧸")
		);
		assert_eq!(poll.count(1), 2);
		assert!(poll.me_voted(1));
		assert_eq!(poll.total_votes(), 7);
		assert!(poll.expiry.is_some());
		assert_eq!(poll.duration, 24);
		assert!(!poll.finalized);
		assert!(model::valid_poll(&poll));
	}

	#[test]
	fn malformed_polls_keep_presence_without_a_card() {
		for payload in [
			r#"{}"#,
			r#"{"question":{"text":"Only a question"}}"#,
			r#"{"question":{"text":"No text"},"answers":[{"answer_id":1,"poll_media":{}}]}"#,
			r#"{"question":{"text":"Zero id"},"answers":[{"answer_id":0,"poll_media":{"text":"x"}}]}"#,
			r#"{"question":{"text":"Dup"},"answers":[{"answer_id":1,"poll_media":{"text":"a"}},{"answer_id":1,"poll_media":{"text":"b"}}]}"#,
		] {
			let message = decode::<MessageDto>(poll(payload).as_bytes())
				.unwrap()
				.into_model();
			assert!(message.extra_content.poll, "presence for {payload}");
			assert!(message.poll.is_none(), "no card for {payload}");
		}
	}

	#[test]
	fn oversized_polls_are_bounded_before_retention() {
		let answers = (1..=model::MAX_POLL_ANSWERS)
			.map(|n| {
				format!(
					r#"{{"answer_id":{n},"poll_media":{{"text":"{}"}}}}"#,
					"a".repeat(64 * 1024)
				)
			})
			.collect::<Vec<_>>()
			.join(",");
		let counts = (1..=100)
			.map(|n| format!(r#"{{"id":{},"count":1}}"#, (n - 1) % 10 + 1))
			.collect::<Vec<_>>()
			.join(",");
		let large = format!(
			r#"{{"question":{{"text":"{}"}},"answers":[{answers}],"results":{{"answer_counts":[{counts}]}}}}"#,
			"q".repeat(64 * 1024)
		);
		let message = decode::<MessageDto>(poll(&large).as_bytes())
			.unwrap()
			.into_model();
		let poll = message.poll.expect("bounded poll still parses");
		assert_eq!(
			poll.question.chars().count(),
			model::MAX_POLL_QUESTION_CHARS
		);
		assert_eq!(
			poll.answers[0].text.chars().count(),
			model::MAX_POLL_ANSWER_CHARS
		);
		assert_eq!(poll.counts.len(), model::MAX_POLL_ANSWERS);
		assert!(poll.bytes() <= model::MAX_POLL_BYTES);
	}

	#[test]
	fn eleven_answers_reject_the_poll_but_keep_presence() {
		let answers = (1..=11)
			.map(|n| format!(r#"{{"answer_id":{n},"poll_media":{{"text":"a"}}}}"#))
			.collect::<Vec<_>>()
			.join(",");
		let message = decode::<MessageDto>(
			poll(&format!(
				r#"{{"question":{{"text":"Pick"}},"answers":[{answers}]}}"#
			))
			.as_bytes(),
		)
		.unwrap()
		.into_model();
		assert!(message.extra_content.poll);
		assert!(message.poll.is_none());
	}

	#[test]
	fn patch_preserves_absence_and_clears_only_explicit_sources() {
		let message = decode::<MessageDto>(
			poll(
				r#"{"question":{"text":"Pick"},"answers":[{"answer_id":1,"poll_media":{"text":"a"}}]}"#,
			)
			.as_bytes(),
		)
		.unwrap()
		.into_model();
		assert!(message.poll.is_some());
		let cleared = decode::<PatchDto>(poll("null").as_bytes())
			.unwrap()
			.into_model();
		assert!(matches!(cleared.poll, Patch::Null));
		let absent = decode::<PatchDto>(wire(r#", "content":"changed""#).as_bytes())
			.unwrap()
			.into_model();
		assert!(matches!(absent.poll, Patch::Absent));
		let updated = decode::<PatchDto>(
			poll(r#"{"question":{"text":"Pick"},"answers":[{"answer_id":1,"poll_media":{"text":"a"}}],"results":{"is_finalized":true,"answer_counts":[{"id":1,"count":9,"me_voted":true}]}}"#)
				.as_bytes(),
		)
		.unwrap()
		.into_model();
		let Patch::Value(Some(updated)) = updated.poll else {
			panic!("typed update expected");
		};
		assert!(updated.finalized);
		assert_eq!(updated.count(1), 9);
	}
}
