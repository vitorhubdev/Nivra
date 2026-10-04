//! Poll card: question, answers, the caller's own vote and the live tally.
//! The card only paints; the click becomes a `Vote` action the host turns into
//! a write command.
use model::Poll;

#[derive(Debug, PartialEq)]
pub enum Action {
	Vote(u64),
}

fn now_nanos() -> i128 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|elapsed| elapsed.as_nanos() as i128)
		.unwrap_or(0)
}

/// Height of the card for the timeline layout pass. The row is measured on the
/// first paint; this only avoids a visible jump before that.
pub fn estimated_height(ui: &egui::Ui, poll: Option<&Poll>, width: f32) -> f32 {
	let Some(poll) = poll else {
		return 0.0;
	};
	let font = egui::TextStyle::Body.resolve(ui.style());
	let text_width = ui
		.painter()
		.layout_no_wrap(poll.question.clone(), font.clone(), egui::Color32::WHITE)
		.size()
		.x;
	let width = width.max(160.0);
	let question_rows = (text_width / width).ceil().clamp(1.0, 3.0);
	// Frame padding, title row, question, one row per answer and the footer.
	10.0 + 14.0 + question_rows * font.size + poll.answers.len() as f32 * 28.0 + 20.0
}

/// Paints the card. `enabled` is the caller's permission to vote; a finalized
/// or expired poll is never clickable, and `busy` blocks a second write.
pub fn show(ui: &mut egui::Ui, poll: &Poll, enabled: bool, busy: bool) -> Option<Action> {
	let colors = crate::design::palette(ui);
	let closed = poll.closed(now_nanos());
	let interactive = enabled && !closed && !busy;
	let mut action = None;
	egui::Frame::new()
		.fill(colors.base)
		.stroke(egui::Stroke::new(1.0, colors.border))
		.corner_radius(8)
		.inner_margin(10)
		.show(ui, |ui| {
			ui.set_max_width(ui.available_width());
			ui.horizontal(|ui| {
				ui.label(
					egui::RichText::new(crate::tr_ui!(ui, "Poll"))
						.small()
						.color(colors.muted),
				);
				if closed {
					ui.label(
						egui::RichText::new(crate::tr_ui!(ui, "Poll closed"))
							.small()
							.color(colors.muted),
					);
				}
			});
			ui.label(egui::RichText::new(&poll.question).strong());
			ui.add_space(4.0);
			let total = poll.total_votes();
			for answer in &poll.answers {
				let label = match &answer.emoji {
					Some(emoji) => format!("{} {}", emoji.label(), answer.text),
					None => answer.text.clone(),
				};
				let selected = poll.me_voted(answer.answer_id);
				ui.horizontal(|ui| {
					let response =
						ui.add_enabled(interactive, egui::Button::selectable(selected, label));
					if response.clicked() {
						action = Some(Action::Vote(answer.answer_id));
					}
					let count = poll.count(answer.answer_id);
					let percent = (u64::from(count) * 100 / u64::from(total.max(1))) as u32;
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						ui.label(
							egui::RichText::new(format!("{count} ({percent}%)"))
								.small()
								.color(colors.muted),
						);
					});
				});
			}
			ui.add_space(2.0);
			let key = if total == 1 { "vote" } else { "votes" };
			ui.label(
				egui::RichText::new(format!("{total} {}", crate::tr_ui!(ui, key)))
					.small()
					.color(colors.muted),
			);
		});
	action
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::{PollAnswer, PollCount};

	fn poll(multiselect: bool, finalized: bool) -> Poll {
		Poll {
			question: "Best layout?".into(),
			answers: vec![
				PollAnswer {
					answer_id: 1,
					text: "Cozy".into(),
					emoji: None,
				},
				PollAnswer {
					answer_id: 2,
					text: "Compact".into(),
					emoji: Some(model::PollEmoji {
						id: None,
						name: Some("🧸".into()),
					}),
				},
			],
			counts: vec![
				PollCount {
					answer_id: 1,
					count: 3,
					me_voted: true,
				},
				PollCount {
					answer_id: 2,
					count: 1,
					me_voted: false,
				},
			],
			counts_known: true,
			expiry: None,
			allow_multiselect: multiselect,
			finalized,
			duration: 24,
		}
	}

	#[test]
	fn clicking_an_answer_reports_the_vote() {
		use egui_kittest::kittest::Queryable as _;
		struct Fixture {
			poll: Poll,
			action: Option<Action>,
		}
		let fixture = Fixture {
			poll: poll(false, false),
			action: None,
		};
		let mut harness = egui_kittest::HarnessBuilder::default()
			.allow_missing_glyphs()
			.build_ui_state(
				|ui, fixture: &mut Fixture| {
					if let Some(action) = show(ui, &fixture.poll, true, false) {
						fixture.action = Some(action);
					}
				},
				fixture,
			);
		harness.run();
		harness
			.get_by_role_and_label(egui::Role::Button, "🧸 Compact")
			.click();
		harness.run();
		assert_eq!(harness.state().action, Some(Action::Vote(2)));
	}

	#[test]
	fn a_closed_poll_renders_results_without_accepting_clicks() {
		use egui_kittest::kittest::Queryable as _;
		struct Fixture {
			poll: Poll,
			action: Option<Action>,
		}
		let fixture = Fixture {
			poll: poll(false, true),
			action: None,
		};
		let mut harness = egui_kittest::HarnessBuilder::default()
			.allow_missing_glyphs()
			.build_ui_state(
				|ui, fixture: &mut Fixture| {
					if let Some(action) = show(ui, &fixture.poll, true, false) {
						fixture.action = Some(action);
					}
				},
				fixture,
			);
		harness.run();
		// The question, the tallies and the closed marker all paint.
		assert!(harness.query_by_label("Best layout?").is_some());
		assert!(harness.query_by_label("3 (75%)").is_some());
		assert!(harness.query_by_label("1 (25%)").is_some());
		assert!(harness.query_by_label("Poll closed").is_some());
		harness.get_by_label("Cozy").click();
		harness.run();
		assert_eq!(
			harness.state().action,
			None,
			"a closed poll cannot be voted"
		);
	}

	#[test]
	fn the_single_vote_pluralizes_the_footer() {
		let poll = Poll {
			counts: vec![PollCount {
				answer_id: 1,
				count: 1,
				me_voted: true,
			}],
			..poll(false, false)
		};
		assert_eq!(poll.total_votes(), 1);
		assert_eq!(poll.count(2), 0);
	}
}
