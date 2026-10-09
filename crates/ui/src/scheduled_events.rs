//! Guild scheduled events dialog: upcoming gatherings with their time,
//! place and interested count. Read-only; RSVP stays out until the write path
//! is live verified.
use client_core::{Command, State};
use model::{Id, ScheduledEvent};

/// One upcoming event row: absolute start, relative distance, place and count.
pub(super) struct Row {
	pub name: String,
	pub when: String,
	pub distance: String,
	pub place: Option<String>,
	pub count: u32,
	pub description: Option<String>,
}

fn seconds(nanos: i128) -> Option<i64> {
	nanos.div_euclid(1_000_000_000).try_into().ok()
}

/// Builds display rows for upcoming events; unparseable times are dropped.
pub(super) fn rows(events: &[ScheduledEvent], state: &State) -> Vec<Row> {
	let mut rows = Vec::new();
	for event in events.iter().filter(|event| event.upcoming()) {
		let Some(start) = seconds(event.start) else {
			continue;
		};
		let (Some(when), Some(distance)) = (
			crate::local_time::discord_timestamp(start, b'f'),
			crate::local_time::discord_timestamp(start, b'R'),
		) else {
			continue;
		};
		let place = event
			.channel_id
			.and_then(|channel| {
				state
					.channels
					.iter()
					.find(|candidate| candidate.id == channel)
					.map(|candidate| candidate.name.clone())
			})
			.or_else(|| event.location.clone());
		rows.push(Row {
			name: event.name.clone(),
			when,
			distance,
			place,
			count: event.user_count,
			description: event.description.clone(),
		});
	}
	rows.sort_by(|a, b| a.when.cmp(&b.when));
	rows
}

#[derive(Default)]
pub(super) struct EventsDialog;

impl EventsDialog {
	/// Returns true when the Close button was pressed.
	pub fn show(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		guild: Id,
		commands: &mut Vec<Command>,
	) -> bool {
		let language = crate::i18n::interface_language(ctx);
		let t = |english: &'static str| crate::i18n::text(language, english);
		let mut close = false;
		crate::dialog::Dialog::new("server-events", t("Events"))
			.width(520.0)
			.show(ctx, |d| {
				d.content(|ui| {
					if state.events.loading(guild) {
						ui.horizontal(|ui| {
							ui.spinner();
							ui.label(t("Loading events…"));
						});
					} else if let Some(error) = state.events.error() {
						ui.label(error);
						if ui.button(t("Retry")).clicked()
							&& let Some(command) = state.request_events(guild)
						{
							commands.push(command);
						}
					} else {
						let rows = rows(state.events.events(guild), state);
						if rows.is_empty() {
							ui.label(t("No upcoming events"));
						} else {
							let interested = t("interested");
							egui::ScrollArea::vertical()
								.max_height(420.0)
								.show(ui, |ui| {
									for row in rows {
										ui.group(|ui| {
											ui.set_width(ui.available_width());
											ui.label(egui::RichText::new(&row.name).strong());
											ui.label(
												egui::RichText::new(format!(
													"{} ({})",
													row.when, row.distance
												))
												.small(),
											);
											if let Some(place) = row.place {
												ui.label(
													egui::RichText::new(place)
														.small()
														.color(crate::design::palette(ui).muted),
												);
											}
											if row.count > 0 {
												ui.label(
													egui::RichText::new(format!(
														"{} {interested}",
														row.count
													))
													.small()
													.color(crate::design::palette(ui).muted),
												);
											}
											if let Some(description) = row.description {
												ui.label(egui::RichText::new(description).small());
											}
										});
										ui.add_space(4.0);
									}
								});
						}
					}
				});
				d.footer(|ui| {
					if crate::dialog::action(ui, t("Close"), crate::dialog::Action::Neutral)
						.clicked()
					{
						close = true;
					}
				});
			});
		close
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn event(id: u64, start: i128, status: u8) -> ScheduledEvent {
		ScheduledEvent {
			id: Id(id),
			guild_id: Id(9),
			channel_id: Some(Id(2)),
			name: format!("Event {id}"),
			description: None,
			start,
			end: None,
			status,
			entity_type: model::ENTITY_VOICE,
			location: None,
			user_count: 3,
		}
	}

	#[test]
	fn rows_keep_upcoming_events_with_readable_times() {
		let state = test_support::demo_state();
		let rows = rows(
			&[
				event(1, 1_800_000_000_000_000_000, model::STATUS_SCHEDULED),
				event(2, 1_700_000_000_000_000_000, 3),
			],
			&state,
		);
		assert_eq!(rows.len(), 1);
		assert_eq!(rows[0].name, "Event 1");
		assert!(!rows[0].when.is_empty());
		assert!(!rows[0].distance.is_empty());
	}
}
