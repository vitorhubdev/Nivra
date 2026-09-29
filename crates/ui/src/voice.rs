use crate::channel_marks::{self, Emphasis};
use crate::design::LazyHover;
use crate::{MessagingUi, design};
use client_core::{
	Command, State,
	auth::AuthState,
	voice::{Participant, Phase, RosterEntry},
};
use egui::RichText;
use model::{Id, voice_settings::NoiseSuppression};

/// Local mutes share the 64 per-user volume slots sent to the mixer.
const MAX_USER_MUTES: usize = 64;

pub(super) struct CallSwitch {
	from: (Id, u64),
	channel: Id,
	ring: bool,
	generation: u64,
	confirmed_at: Option<std::time::Instant>,
	audio: Option<(bool, bool)>,
}

impl MessagingUi {
	/// Effective screen-share audio level, independent of participant voice levels.
	pub fn voice_stream_volume(&self) -> u16 {
		if self.voice_stream_muted {
			0
		} else {
			self.voice_stream_volume.unwrap_or(100).min(200)
		}
	}

	/// Fixed session overrides; zero IDs are unused slots. A locally muted speaker is mixed at
	/// zero gain, so unmuting restores the volume chosen for them.
	pub fn voice_user_volumes(&self) -> [(u64, u16); 64] {
		let mut values = self
			.voice_user_volumes
			.as_deref()
			.copied()
			.unwrap_or([(0, 100); 64]);
		for user in self.voice_user_muted.iter().copied() {
			if let Some(slot) = values.iter_mut().find(|(id, _)| *id == user) {
				slot.1 = 0;
			} else if let Some(index) = values.iter().position(|(id, _)| *id == 0).or_else(|| {
				values
					.iter()
					.rposition(|(id, _)| !self.voice_user_muted.contains(id))
			}) {
				values[index] = (user, 0);
			}
		}
		values
	}

	/// Locally muted speakers, for persistence to device settings.
	pub fn voice_user_mutes(&self) -> &[u64] {
		&self.voice_user_muted
	}

	/// Restore persisted local mutes, e.g. at startup.
	pub fn set_voice_user_mutes(&mut self, values: &[u64]) {
		self.voice_user_muted = values
			.iter()
			.copied()
			.filter(|user| *user != 0)
			.take(MAX_USER_MUTES)
			.collect();
	}

	pub(super) fn voice_user_locally_muted(&self, user: Id) -> bool {
		self.voice_user_muted.contains(&user.0)
	}

	pub(super) fn set_voice_user_locally_muted(&mut self, user: Id, muted: bool) {
		self.voice_user_muted.retain(|id| *id != user.0);
		if muted && self.voice_user_muted.len() < MAX_USER_MUTES {
			self.voice_user_muted.push(user.0);
		}
	}

	/// Chosen volume overrides, for persistence to device settings. A bot can be pinned at
	/// 100% on purpose, so only empty slots are skipped.
	pub fn voice_user_volume_overrides(&self) -> Vec<(u64, u16)> {
		self.voice_user_volumes
			.as_deref()
			.into_iter()
			.flatten()
			.filter(|(id, _)| *id != 0)
			.copied()
			.collect()
	}

	/// What the mixer plays: chosen overrides and local mutes, plus hearing protection for
	/// bots in the current call that nobody has set a volume for.
	pub fn voice_mix_volumes(&self, state: &State) -> [(u64, u16); 64] {
		let mut values = self.voice_user_volumes();
		let Some(call) = state
			.voice
			.active
			.as_ref()
			.filter(|_| self.voice_bot_safe_volume)
		else {
			return values;
		};
		for entry in state
			.voice
			.roster
			.iter()
			.filter(|e| e.channel == call.channel)
		{
			let id = entry.participant.user.0;
			if values.iter().any(|(user, _)| *user == id) || !is_bot(resolve_member(state, entry).0)
			{
				continue;
			}
			if let Some(slot) = values.iter_mut().find(|(user, _)| *user == 0) {
				*slot = (id, BOT_SAFE_VOLUME);
			}
		}
		values
	}

	/// The level a participant plays at before anyone picks one for them.
	fn voice_default_volume(&self, user: Option<&model::User>) -> u16 {
		if self.voice_bot_safe_volume && is_bot(user) {
			BOT_SAFE_VOLUME
		} else {
			100
		}
	}

	/// Restore persisted per-user volume overrides, e.g. at startup.
	pub fn set_voice_user_volume_overrides(&mut self, values: &[(u64, u16)]) {
		if values.is_empty() {
			self.voice_user_volumes = None;
			return;
		}
		let mut array = [(0u64, 100u16); 64];
		for (slot, value) in array.iter_mut().zip(values.iter().take(64)) {
			*slot = *value;
		}
		self.voice_user_volumes = Some(Box::new(array));
	}

	pub(super) fn set_voice_user_volume(&mut self, user: Id, volume: u16) {
		self.set_voice_user_volume_from(user, volume, 100);
	}

	/// Stores `volume` as a choice, or forgets it when it equals the participant's default.
	fn set_voice_user_volume_from(&mut self, user: Id, volume: u16, default: u16) {
		if volume == default {
			if let Some(slot) = self
				.voice_user_volumes
				.as_deref_mut()
				.and_then(|values| values.iter_mut().find(|(id, _)| *id == user.0))
			{
				*slot = (0, 100);
			}
			return;
		}
		let values = self
			.voice_user_volumes
			.get_or_insert_with(|| Box::new([(0, 100); 64]));
		let index = values
			.iter()
			.position(|(id, _)| *id == user.0)
			.or_else(|| values.iter().position(|(id, _)| *id == 0))
			.unwrap_or_else(|| {
				values.rotate_left(1);
				63
			});
		values[index] = (user.0, volume);
	}

	fn voice_participant_menu(
		&mut self,
		response: &egui::Response,
		state: &State,
		entry: &RosterEntry,
	) {
		crate::user_menu::popup(response, egui::Popup::default_response_id(response))
			.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
			.show(|ui| {
				ui.set_width(260.0);
				let id = entry.participant.user.0;
				let language = self.language;
				let t = |english: &'static str| crate::i18n::text(language, english);
				if state.user.as_ref().is_some_and(|own| own.id.0 != id) {
					let muted = self.voice_user_locally_muted(entry.participant.user);
					if ui
						.button(if muted { "Unmute" } else { "Mute" })
						.on_hover_text(
							"Silence this person on this device only. Nobody else is affected.",
						)
						.clicked()
					{
						self.set_voice_user_locally_muted(entry.participant.user, !muted);
					}
					let user = resolve_member(state, entry).0;
					let default = self.voice_default_volume(user);
					let mut volume = self
						.voice_user_volumes
						.as_deref()
						.and_then(|values| values.iter().find(|(user, _)| *user == id))
						.map_or(default, |(_, volume)| *volume);
					let changed = volume_control(ui, &mut volume, t("User volume"), language);
					if default != 100 {
						design::hint(
							ui,
							t(
								"Bots start at 50% to protect your hearing. You can still raise it here.",
							),
						);
					}
					let reset = ui
						.add_enabled(volume != default, egui::Button::new(t("Reset volume")))
						.clicked();
					if changed || reset {
						self.set_voice_user_volume_from(
							entry.participant.user,
							if reset { default } else { volume },
							default,
						);
					}
					ui.separator();
				}
				if let Some(user) = resolve_member(state, entry).0 {
					crate::user_menu::contents(
						ui,
						state,
						user,
						&mut self.profile,
						&mut self.user_action,
						None,
					);
				}
			});
	}

	fn is_speaking(&self, state: &State, channel: Id, participant: &Participant) -> bool {
		!self.voice_user_locally_muted(participant.user)
			&& !participant.muted
			&& !participant.deafened
			&& !participant.server_muted
			&& !participant.server_deafened
			&& state.voice.active.as_ref().is_some_and(|call| {
				call.channel == channel
					&& matches!(call.phase, Phase::Connected | Phase::Waiting)
					&& !call.deafened
					&& !call.server_deafened
			}) && self.voice_speaking.contains(&participant.user)
	}

	pub(super) fn voice_channel_button(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		channel: &model::Channel,
		selected: bool,
		draggable: bool,
	) -> egui::Response {
		let colors = design::palette(ui);
		let call = state
			.voice
			.active
			.as_ref()
			.filter(|c| c.channel == channel.id);
		let connected = call.is_some_and(|c| matches!(c.phase, Phase::Waiting | Phase::Connected));
		let elapsed = call.and_then(elapsed_label);
		let access = state.channel_access(channel.id);
		let viewable = !access.hidden();
		let (rect, response) = ui
			.push_id(channel.id, |ui| {
				ui.allocate_exact_size(
					egui::vec2(ui.available_width(), 34.0),
					if viewable {
						if draggable {
							egui::Sense::click_and_drag()
						} else {
							egui::Sense::click()
						}
					} else {
						egui::Sense::hover()
					},
				)
			})
			.inner;
		let row = rect.shrink2(egui::vec2(0.0, 1.0));
		let hovered = viewable && (response.hovered() || response.has_focus());
		if selected {
			ui.painter().rect_filled(row, 8, colors.selected);
		} else if hovered {
			ui.painter()
				.rect_filled(row, 8, crate::design::row_highlight(ui, colors.hover, 1.0));
		}
		let text_color = channel_marks::tint(
			&colors,
			access,
			if !viewable {
				Emphasis::Unavailable
			} else if connected {
				Emphasis::Connected
			} else if selected || hovered {
				Emphasis::Focused
			} else {
				Emphasis::Idle
			},
		);
		let glyph = egui::Rect::from_center_size(
			row.left_center() + egui::vec2(18.0, 0.0),
			egui::Vec2::splat(20.0),
		);
		crate::icons::paint(ui.painter(), crate::icons::Icon::Speaker, glyph, text_color);
		channel_marks::paint(
			ui.painter(),
			access,
			row,
			glyph,
			text_color,
			if selected {
				colors.selected
			} else if hovered {
				crate::design::row_highlight(ui, colors.hover, 1.0)
			} else {
				colors.sidebar
			},
		);
		let marks = channel_marks::trailing(access);
		let participant_count = state
			.voice
			.roster
			.iter()
			.filter(|entry| entry.channel == channel.id)
			.take(99)
			.count();
		let activity_width = if participant_count > 0 { 34.0 } else { 0.0 };
		let elapsed_width = if elapsed.is_some() { 64.0 } else { 0.0 };
		let name = ui.painter().layout(
			channel.name.clone(),
			egui::FontId::new(15.0, design::medium_family(ui.ctx())),
			text_color,
			(row.width() - 40.0 - elapsed_width - activity_width - marks).max(10.0),
		);
		let name_rect = egui::Rect::from_min_size(
			egui::pos2(row.left() + 34.0, row.center().y - name.size().y * 0.5),
			egui::vec2(
				row.width() - 40.0 - elapsed_width - activity_width - marks,
				name.size().y,
			),
		);
		ui.painter()
			.with_clip_rect(name_rect)
			.galley(name_rect.min, name, text_color);
		if participant_count > 0 {
			let right = row.right() - 8.0 - marks - elapsed_width;
			let center = egui::pos2(right - 12.0, row.center().y);
			ui.painter()
				.circle_filled(center - egui::vec2(9.0, 0.0), 3.5, colors.positive);
			ui.painter().text(
				center,
				egui::Align2::LEFT_CENTER,
				participant_count.to_string(),
				egui::FontId::new(11.0, design::medium_family(ui.ctx())),
				colors.positive,
			);
		}
		if let Some(elapsed) = &elapsed {
			ui.painter().text(
				row.right_center() - egui::vec2(8.0 + marks, 0.0),
				egui::Align2::RIGHT_CENTER,
				elapsed,
				egui::FontId::monospace(11.0),
				text_color,
			);
		}
		if elapsed.is_some() && ui.is_rect_visible(response.rect) {
			ui.ctx()
				.request_repaint_after(std::time::Duration::from_secs(1));
		}
		response.widget_info(|| {
			egui::WidgetInfo::selected(
				egui::Role::Button,
				viewable,
				selected,
				format!(
					"{} voice channel{}{}",
					channel.name,
					channel_marks::label(access),
					if connected {
						", connected"
					} else if participant_count > 0 {
						", active"
					} else {
						""
					}
				),
			)
		});
		response.on_hover_text_with(|| {
			voice_channel_hover_text(&channel.name, channel_marks::label(access), connected)
		})
	}

	pub(super) fn voice_participant(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		entry: &RosterEntry,
		draggable: bool,
	) -> Option<egui::Response> {
		if !state.can_view(entry.channel) {
			return None;
		}
		let colors = design::palette(ui);
		let (user, name) = resolve_member(state, entry);
		let response = ui.push_id(
			("voice-participant", entry.channel, entry.participant.user),
			|ui| {
				let (rect, row) = ui.allocate_exact_size(
					egui::vec2(ui.available_width(), 34.0),
					if draggable {
						egui::Sense::click_and_drag()
					} else {
						egui::Sense::click()
					},
				);
				row.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
				let hovered = row.contains_pointer() || row.has_focus();
				if hovered {
					ui.painter().rect_filled(
						rect.shrink2(egui::vec2(0.0, 1.0)),
						8,
						crate::design::row_highlight(ui, colors.hover, 1.0),
					);
				}
				let name_color = if hovered {
					colors.text_strong
				} else {
					colors.muted
				};
				let mut inner = ui.new_child(
					egui::UiBuilder::new()
						.max_rect(rect)
						.layout(egui::Layout::left_to_right(egui::Align::Center)),
				);
				inner.spacing_mut().item_spacing.x = 6.0;
				let avatar = match user {
					Some(user) => self
						.avatars
						.show_plain_quiet(&mut inner, user, 28.0, state.demo),
					None => {
						let (r, response) = inner
							.allocate_exact_size(egui::Vec2::splat(28.0), egui::Sense::hover());
						design::paint_avatar(&inner, name, 28.0, r);
						response
					}
				};
				let speaking = self.is_speaking(state, entry.channel, &entry.participant);
				let level = self
					.voice_speaking_levels
					.get(&entry.participant.user)
					.copied()
					.unwrap_or(0);
				speaking_avatar(
					&inner,
					&avatar,
					name,
					entry.participant.user,
					speaking,
					level,
				);
				let locally_muted = self.voice_user_locally_muted(entry.participant.user);
				inner.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					if entry.participant.deafened {
						status_icon(
							ui,
							crate::icons::Icon::HeadphonesSlash,
							colors.muted,
							if entry.participant.server_deafened {
								"Deafened by server"
							} else {
								"Deafened"
							},
						);
					}
					if entry.participant.muted {
						status_icon(
							ui,
							crate::icons::Icon::MicrophoneSlash,
							colors.muted,
							if entry.participant.server_muted {
								"Muted by server"
							} else {
								"Microphone muted"
							},
						);
					}
					if locally_muted {
						status_icon(
							ui,
							crate::icons::Icon::Speaker,
							colors.danger,
							"Muted for you on this device",
						);
					}
					if entry.participant.streaming {
						live_badge(ui);
					}
					ui.allocate_ui_with_layout(
						egui::vec2(ui.available_width(), 28.0),
						egui::Layout::left_to_right(egui::Align::Center),
						|ui| {
							if is_bot(user) {
								bot_badge(ui, self.language);
							}
							ui.add(
								egui::Label::new(RichText::new(name).color(name_color))
									.truncate()
									.selectable(false),
							)
							.on_hover_text(participant_tip(user, name));
						},
					);
				});
				self.voice_participant_menu(&row, state, entry);
				if let Some(user) = user {
					self.profile.person_click(ui, &row, None, user);
				}
				row
			},
		);
		Some(response.inner)
	}

	/// Guild voice channel: Discord-style black stage with participant tiles and, when
	/// connected, the call control bar; otherwise a Join Voice button.
	pub(super) fn voice_channel(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		channel: Id,
		commands: &mut Vec<Command>,
	) {
		let connected = state
			.voice
			.active
			.as_ref()
			.is_some_and(|call| call.channel == channel);
		let stage = ui.available_rect_before_wrap();
		ui.painter().rect_filled(stage, 0, STAGE_FILL);
		let (rect, _) = ui.allocate_exact_size(stage.size(), egui::Sense::hover());
		let notices = self.stage_notices(state, channel, connected);
		let bottom = CONTROL_HEIGHT + 2.0 * STAGE_MARGIN;
		let body = egui::Rect::from_min_max(
			rect.left_top() + egui::vec2(STAGE_MARGIN, STAGE_MARGIN),
			egui::pos2(rect.right() - STAGE_MARGIN, rect.bottom() - bottom),
		);
		let mut body_ui = ui.new_child(
			egui::UiBuilder::new()
				.max_rect(body)
				.layout(egui::Layout::top_down(egui::Align::Min)),
		);
		stage_notices(&mut body_ui, &notices);
		call_failure(
			&mut body_ui,
			state
				.voice
				.active
				.as_ref()
				.filter(|c| c.channel == channel)
				.and_then(|c| c.error),
			STAGE_TEXT,
		);
		if !state.can_view(channel) {
			body_ui.label(
				RichText::new("Participant list unavailable with the current access.")
					.color(STAGE_MUTED),
			);
		} else {
			let entries = stage_participants(state, channel);
			if entries.is_empty() {
				body_ui.add_space((body_ui.available_height() * 0.4).max(0.0));
				body_ui.vertical_centered(|ui| {
					ui.label(
						design::semibold(
							ui,
							if !state.demo && !state.gateway_connected {
								"Participant list unavailable while disconnected"
							} else {
								"No one's here yet"
							},
							18.0,
						)
						.color(STAGE_TEXT),
					);
				});
			} else {
				if !state.demo && !state.gateway_connected {
					body_ui.label(
						RichText::new("Last known participants · reconnect to refresh")
							.small()
							.color(STAGE_MUTED),
					);
				}
				self.participant_tiles(&mut body_ui, state, channel, &entries, false);
			}
		}
		self.apply_watch_request(state);
		let bar = egui::Rect::from_min_max(
			egui::pos2(rect.left(), rect.bottom() - bottom),
			rect.right_bottom(),
		);
		let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(
			egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
		));
		if connected {
			if self.voice_camera_on_join == Some(channel) {
				let ready = state.voice.active.as_ref().is_some_and(|call| {
					call.channel == channel
						&& matches!(call.phase, Phase::Connected | Phase::Waiting)
						&& !call.camera
				});
				if ready && self.voice_camera_available && state.can_camera(channel) {
					if let Some(command) = state.set_call_camera(true) {
						commands.push(command);
					}
					self.voice_camera_on_join = None;
				} else if state
					.voice
					.active
					.as_ref()
					.is_some_and(|call| call.channel == channel && call.phase == Phase::Failed)
					|| !self.voice_camera_available
					|| !state.can_camera(channel)
				{
					self.voice_camera_on_join = None;
				}
			}
			self.call_controls(&mut bar_ui, state, channel, commands);
		} else {
			bar_ui.horizontal_centered(|ui| {
				self.call_button(ui, state, channel, commands, false);
				if self.voice_camera_available && state.can_camera(channel) {
					ui.add_space(8.0);
					self.call_button(ui, state, channel, commands, true);
				}
			});
		}
	}

	/// Which stage tiles a channel would show, in stage order.
	fn stage_tiles<'a>(
		&self,
		state: &State,
		channel: Id,
		entries: &'a [RosterEntry],
	) -> Vec<Tile<'a>> {
		let call = state
			.voice
			.active
			.as_ref()
			.filter(|call| call.channel == channel && call.phase != Phase::Failed);
		let mut tiles = Vec::with_capacity(entries.len() + 2);
		if call.is_some_and(|call| {
			self.screen.context == Some((state.generation, channel, call.request))
				&& self.screen.busy
				&& self.screen.preview.is_some()
		}) {
			tiles.push(Tile::LocalScreen);
		}
		if let Some(streamer) = call.and_then(|call| call.watching) {
			tiles.push(Tile::Stream(streamer));
		}
		tiles.extend(entries.iter().map(Tile::Participant));
		tiles
	}

	/// True once any stage tile carries video, so the direct-message stage can grow.
	pub(super) fn stage_shows_video(&self, state: &State, channel: Id) -> bool {
		let entries = stage_participants(state, channel);
		self.stage_tiles(state, channel, &entries)
			.iter()
			.any(|tile| self.tile_has_video(state, channel, tile))
	}

	/// Stage tiles: a best-fit grid, or one enlarged video with the rest in a strip below.
	///
	/// `dm` drops the tile plates while no one shares video, matching Discord's
	/// direct-message calls where idle participants are avatars on the stage.
	fn participant_tiles(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		channel: Id,
		entries: &[RosterEntry],
		dm: bool,
	) {
		let tiles = self.stage_tiles(state, channel, entries);
		if tiles.is_empty() {
			return;
		}
		// Focus survives only while that tile still shows video; Escape restores the grid.
		let focus = self.voice_focus.filter(|focus| {
			tiles
				.iter()
				.any(|tile| tile.focus() == *focus && self.tile_has_video(state, channel, tile))
		});
		let focus = focus.filter(|_| !ui.input(|input| input.key_pressed(egui::Key::Escape)));
		self.voice_focus = focus;
		let video = tiles
			.iter()
			.any(|tile| self.tile_has_video(state, channel, tile));
		let frameless = dm && !video && !state.is_group_dm(channel);
		let area = ui.available_rect_before_wrap();
		if area.width() < 40.0 || area.height() < 40.0 {
			return;
		}
		let mut toggle = None;
		if let Some(focus) = focus {
			let index = tiles
				.iter()
				.position(|tile| tile.focus() == focus)
				.expect("validated focus");
			// The enlarged tile keeps the stage; everyone else becomes a small strip below it,
			// only when the pill-bar toggle asks for them.
			let strip = if tiles.len() > 1 && self.voice_focus_participants {
				(area.height() * 0.18).clamp(64.0, 124.0)
			} else {
				0.0
			};
			let main = egui::Rect::from_min_size(
				area.min,
				egui::vec2(
					area.width(),
					(area.height() - strip - if strip > 0.0 { TILE_GAP } else { 0.0 }).max(80.0),
				),
			);
			toggle = self.tile(ui, state, channel, &tiles[index], main, false, true);
			if strip > 0.0 {
				let size = egui::vec2(strip * 16.0 / 9.0, strip);
				let others = (tiles.len() - 1) as f32;
				let row = size.x * others + TILE_GAP * (others - 1.0);
				let mut x = area.center().x - row * 0.5;
				let top = main.bottom() + TILE_GAP;
				for (position, tile) in tiles.iter().enumerate() {
					if position == index {
						continue;
					}
					let rect = egui::Rect::from_min_size(egui::pos2(x, top), size);
					x += size.x + TILE_GAP;
					if !area.intersects(rect) {
						continue;
					}
					if let Some(focus) = self.tile(ui, state, channel, tile, rect, false, false) {
						toggle = Some(focus);
					}
				}
			}
		} else {
			// Pick the column count that makes the tiles largest inside the stage, so two
			// participants fill the width instead of sitting in a corner.
			let cap = if frameless { 148.0 } else { 620.0 };
			let (columns, mut size) = best_fit(tiles.len(), area.size(), cap);
			if frameless {
				size.y = size.y.max(132.0).min(area.height());
			}
			if size.x < 24.0 || size.y < 24.0 {
				return;
			}
			let rows = tiles.len().div_ceil(columns);
			let content = size.y * rows as f32 + TILE_GAP * (rows as f32 - 1.0);
			let top = area.top() + ((area.height() - content) * 0.5).max(0.0);
			for row in 0..rows {
				let first = row * columns;
				let in_row = (tiles.len() - first).min(columns);
				let width = size.x * in_row as f32 + TILE_GAP * (in_row as f32 - 1.0);
				let mut x = area.center().x - width * 0.5;
				let y = top + (size.y + TILE_GAP) * row as f32;
				for tile in &tiles[first..first + in_row] {
					let rect = egui::Rect::from_min_size(egui::pos2(x, y), size);
					x += size.x + TILE_GAP;
					if let Some(focus) = self.tile(ui, state, channel, tile, rect, frameless, false)
					{
						toggle = Some(focus);
					}
				}
			}
		}
		if let Some(focus) = toggle {
			self.voice_focus = (self.voice_focus != Some(focus)).then_some(focus);
		}
	}

	fn remote_texture(&self, user: Id) -> Option<&egui::TextureHandle> {
		self.voice_remote_video
			.iter()
			.find(|(id, _)| *id == user)
			.map(|(_, texture)| texture)
	}

	fn tile_has_video(&self, state: &State, channel: Id, tile: &Tile<'_>) -> bool {
		match tile {
			Tile::LocalScreen => self.screen.preview.is_some(),
			Tile::Stream(_) => true,
			Tile::Participant(entry) => {
				let own = state
					.user
					.as_ref()
					.is_some_and(|user| user.id == entry.participant.user);
				if own {
					self.voice_camera_preview.is_some()
						&& state.voice.active.as_ref().is_some_and(|call| {
							call.channel == channel && call.camera && call.phase != Phase::Failed
						})
				} else {
					entry.participant.video && self.remote_texture(entry.participant.user).is_some()
				}
			}
		}
	}

	/// One stage tile. Returns its focus key when a video tile is clicked to enlarge or restore.
	#[allow(clippy::too_many_arguments)] // Placement and framing flags of one tile.
	fn tile(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		channel: Id,
		tile: &Tile<'_>,
		rect: egui::Rect,
		frameless: bool,
		focused: bool,
	) -> Option<StageFocus> {
		let has_video = self.tile_has_video(state, channel, tile);
		let response = ui.interact(
			rect,
			ui.scope_id().with(("voice-tile", tile.key())),
			if has_video || matches!(tile, Tile::Participant(_)) {
				egui::Sense::click()
			} else {
				egui::Sense::hover()
			},
		);
		if !ui.is_rect_visible(rect) {
			return None;
		}
		let compact = rect.height() < 132.0;
		let hint = match tile {
			Tile::LocalScreen => {
				self.screen_tile(ui, rect, compact);
				self.screen
					.capture_status
					.unwrap_or("Your screen · local preview")
			}
			Tile::Stream(streamer) => {
				self.stream_tile(ui, state, rect, channel, *streamer, compact);
				egui::Popup::context_menu(&response)
					.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
					.show(|ui| self.stream_audio_controls(ui));
				"Screen share you are watching"
			}
			Tile::Participant(entry) => {
				self.participant_tile(ui, state, entry, rect, frameless, compact);
				self.voice_participant_menu(&response, state, entry);
				""
			}
		};
		if has_video {
			let label = if focused {
				"Click or press Escape to return to the grid"
			} else {
				"Click to enlarge"
			};
			response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
			let hover = if hint.is_empty() {
				label.to_owned()
			} else {
				format!("{hint} · {label}")
			};
			if response.clicked() {
				return Some(tile.focus());
			}
			response.on_hover_text(hover);
		} else if !hint.is_empty() {
			response.on_hover_text(hint);
		}
		None
	}

	fn screen_tile(&self, ui: &mut egui::Ui, rect: egui::Rect, compact: bool) {
		let content = self
			.screen
			.preview
			.as_ref()
			.map(|texture| {
				let content = fit_rect(rect, texture.size_vec2());
				ui.put(
					content,
					egui::Image::from_texture((texture.id(), content.size())).corner_radius(8),
				);
				content
			})
			.unwrap_or(rect);
		if !compact {
			name_badge(ui, content, "Your screen", None);
		}
	}

	pub fn take_voice_fullscreen_request(&mut self) -> Option<bool> {
		self.voice_stream_fullscreen_request.take()
	}

	fn exit_voice_stream_fullscreen(&mut self, ctx: &egui::Context) {
		if self.voice_stream_fullscreen {
			self.voice_stream_fullscreen = false;
			self.voice_stream_fullscreen_request = Some(self.voice_stream_fullscreen_previous);
			ctx.request_repaint();
		}
	}

	/// Full-client presentation of the currently watched screen share. The native host mirrors
	/// the viewport fullscreen state, while this overlay removes the surrounding Discord chrome.
	pub(super) fn show_voice_stream_fullscreen(
		&mut self,
		ctx: &egui::Context,
		state: &State,
	) -> bool {
		if !self.voice_stream_fullscreen {
			return false;
		}
		let valid = state.voice.active.as_ref().is_some_and(|call| {
			call.watching.is_some() && matches!(call.phase, Phase::Connected | Phase::Waiting)
		}) && self.voice_stream_view.is_some();
		if !valid || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
			self.exit_voice_stream_fullscreen(ctx);
			return false;
		}
		let screen = ctx.content_rect();
		let id = egui::Id::unique("voice-stream-fullscreen");
		let modal = egui::Modal::new(id)
			.area(
				egui::Modal::default_area(id)
					.anchor(egui::Align2::LEFT_TOP, egui::Vec2::ZERO)
					.fade_in(false),
			)
			.backdrop_color(egui::Color32::BLACK)
			.frame(egui::Frame::NONE)
			.show(ctx, |ui| {
				ui.set_min_size(screen.size());
				ui.set_max_size(screen.size());
				ui.painter()
					.rect_filled(ui.max_rect(), 0.0, egui::Color32::BLACK);
				if let Some(texture) = &self.voice_stream_view {
					let margin = 18.0;
					let stage = ui.max_rect().shrink(margin);
					let image = fit_rect(stage, texture.size_vec2());
					egui::Image::from_texture((texture.id(), image.size())).paint_at(ui, image);
				}
				let exit = egui::Rect::from_min_size(
					ui.max_rect().right_top() + egui::vec2(-142.0, 16.0),
					egui::vec2(126.0, 34.0),
				);
				if ui
					.put(
						exit,
						egui::Button::new("Exit full screen")
							.fill(egui::Color32::from_black_alpha(180)),
					)
					.clicked()
				{
					self.exit_voice_stream_fullscreen(ui.ctx());
				}
				let audio = egui::Rect::from_min_size(
					ui.max_rect().left_bottom() + egui::vec2(16.0, -50.0),
					egui::vec2(140.0, 34.0),
				);
				let response = ui.put(
					audio,
					egui::Button::new(if self.voice_stream_volume() == 0 {
						"Stream muted"
					} else {
						"Stream audio"
					})
					.fill(egui::Color32::from_black_alpha(180)),
				);
				egui::Popup::menu(&response)
					.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
					.show(|ui| self.stream_audio_controls(ui));
			});
		if modal.should_close() {
			self.exit_voice_stream_fullscreen(ctx);
		}
		true
	}

	/// The screen share this device chose to watch: the latest decoded picture or a status.
	fn stream_tile(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		rect: egui::Rect,
		channel: Id,
		streamer: Id,
		compact: bool,
	) {
		let name = participant_user(state, channel, streamer)
			.map_or_else(|| "Participant".to_owned(), |user| user.name.clone());
		let content = match &self.voice_stream_view {
			Some(texture) => {
				let content = fit_rect(rect, texture.size_vec2());
				ui.put(
					content,
					egui::Image::from_texture((texture.id(), content.size())).corner_radius(8),
				);
				content
			}
			None => {
				ui.painter().rect_filled(rect, 8, TILE_FILL);
				if !compact {
					let status = if self.voice_stream_status.is_empty() {
						"Connecting to the stream…"
					} else {
						self.voice_stream_status
					};
					let spinner = egui::Rect::from_center_size(
						rect.center() - egui::vec2(0.0, 18.0),
						egui::Vec2::splat(24.0),
					);
					ui.put(spinner, egui::Spinner::new().color(STAGE_MUTED));
					ui.painter().text(
						rect.center() + egui::vec2(0.0, 18.0),
						egui::Align2::CENTER_CENTER,
						status,
						egui::FontId::proportional(13.0),
						STAGE_MUTED,
					);
				}
				rect
			}
		};
		let audio = ui.put(
			egui::Rect::from_min_size(
				rect.left_top() + egui::vec2(8.0, 8.0),
				egui::vec2(110.0_f32.min((rect.width() - 16.0).max(0.0)), 26.0),
			),
			egui::Button::new(
				RichText::new(if self.voice_stream_volume() == 0 {
					"Stream muted"
				} else {
					"Stream audio"
				})
				.size(12.0)
				.color(egui::Color32::WHITE),
			)
			.truncate()
			.fill(egui::Color32::from_black_alpha(170))
			.corner_radius(6),
		);
		egui::Popup::menu(&audio)
			.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
			.show(|ui| self.stream_audio_controls(ui));
		if compact {
			return;
		}
		name_badge(ui, content, &format!("{name}'s screen"), None);
		let full_rect = egui::Rect::from_min_size(
			content.right_top() + egui::vec2(-118.0, 8.0),
			egui::vec2(110.0_f32.min(content.width().max(1.0)), 28.0),
		);
		if ui
			.put(
				full_rect,
				egui::Button::new("Full screen")
					.fill(egui::Color32::from_black_alpha(170))
					.corner_radius(6),
			)
			.on_hover_text("Watch this screen share in full screen")
			.clicked()
		{
			self.voice_stream_fullscreen_previous =
				ui.input(|input| input.viewport().fullscreen.unwrap_or(false));
			self.voice_stream_fullscreen = true;
			self.voice_stream_fullscreen_request = Some(true);
			ui.ctx().request_repaint();
		}
		if tile_button(
			ui,
			content,
			"Stop watching",
			egui::Color32::from_black_alpha(170),
			design::palette(ui).danger,
			"Stop receiving this screen share",
		)
		.clicked()
		{
			self.watch_request = Some(None);
		}
	}

	fn stream_audio_controls(&mut self, ui: &mut egui::Ui) {
		ui.set_width(220.0);
		ui.checkbox(&mut self.voice_stream_muted, "Mute stream audio");
		gain_slider(
			ui,
			self.voice_stream_volume.get_or_insert(100),
			"Stream volume",
		);
	}

	/// Apply a tile's watch click once the stage has mutable state again.
	fn apply_watch_request(&mut self, state: &mut State) {
		match self.watch_request.take() {
			Some(Some(user)) => {
				let _ = state.watch_stream(user);
			}
			Some(None) => state.stop_watching(),
			None => {}
		}
	}

	fn participant_tile(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		entry: &RosterEntry,
		rect: egui::Rect,
		frameless: bool,
		compact: bool,
	) {
		let size = rect.size();
		let (user, name) = resolve_member(state, entry);
		let colors = design::palette(ui);
		let own = state
			.user
			.as_ref()
			.is_some_and(|user| user.id == entry.participant.user);
		let call = state
			.voice
			.active
			.as_ref()
			.filter(|call| call.channel == entry.channel && call.phase != Phase::Failed);
		// The local preview is mirrored like a webcam; remote cameras fill the tile edge to edge.
		let video = if own && call.is_some_and(|call| call.camera) {
			self.voice_camera_preview
				.as_ref()
				.map(|texture| (texture.id(), texture.size_vec2(), true))
		} else if entry.participant.video {
			self.remote_texture(entry.participant.user)
				.map(|texture| (texture.id(), texture.size_vec2(), false))
		} else {
			None
		};
		if !frameless {
			ui.painter().rect_filled(rect, 8, TILE_FILL);
		}
		if let Some((id, image, mirror)) = video {
			cover_image(ui, rect, id, image, mirror);
		}
		let speaking = self.is_speaking(state, entry.channel, &entry.participant);
		let avatar_size = if compact {
			(size.y * 0.5).clamp(28.0, 48.0)
		} else if frameless {
			(size.y * 0.58).clamp(56.0, 112.0)
		} else {
			(size.y * 0.42).clamp(48.0, 128.0)
		};
		let offset = if compact {
			0.0
		} else if frameless {
			14.0
		} else {
			10.0
		};
		let avatar_rect = egui::Rect::from_center_size(
			rect.center() - egui::vec2(0.0, offset),
			egui::Vec2::splat(avatar_size),
		);
		let mut avatar_ui = ui.new_child(egui::UiBuilder::new().max_rect(avatar_rect));
		if video.is_some() {
			avatar_ui.set_opacity(0.0);
		}
		let avatar = if let Some(user) = user {
			self.avatars
				.show(&mut avatar_ui, user, avatar_size, state.demo)
		} else {
			design::avatar(&mut avatar_ui, name, avatar_size)
		};
		// Mute state reads as Discord's red ring plus the matching slashed glyph.
		let silenced = if entry.participant.deafened || entry.participant.server_deafened {
			Some(crate::icons::Icon::HeadphonesSlash)
		} else if entry.participant.muted || entry.participant.server_muted {
			Some(crate::icons::Icon::MicrophoneSlash)
		} else if self.voice_user_locally_muted(entry.participant.user) {
			// Silenced on this device only; the speaker glyph separates it from a microphone mute.
			Some(crate::icons::Icon::Speaker)
		} else {
			None
		};
		if let Some(icon) = silenced
			&& video.is_none()
		{
			ui.painter().circle_stroke(
				avatar.rect.center(),
				avatar.rect.width() * 0.5 + 2.0,
				egui::Stroke::new(2.0, colors.danger),
			);
			let badge = avatar.rect.right_bottom() - egui::Vec2::splat(avatar_size * 0.14);
			ui.painter().circle_filled(badge, 12.0, STAGE_FILL);
			ui.painter().circle_filled(badge, 10.0, colors.danger);
			crate::icons::paint(
				ui.painter(),
				icon,
				egui::Rect::from_center_size(badge, egui::Vec2::splat(12.0)),
				egui::Color32::WHITE,
			);
		}
		let level = self
			.voice_speaking_levels
			.get(&entry.participant.user)
			.copied()
			.unwrap_or(0);
		if silenced.is_none() {
			if video.is_none() && frameless {
				speaking_avatar(ui, &avatar, name, entry.participant.user, speaking, level);
			}
			if !frameless && speaking {
				ui.painter().rect_stroke(
					rect.shrink(1.0),
					8,
					egui::Stroke::new(2.0, colors.positive),
					egui::StrokeKind::Inside,
				);
			}
		}
		self.voice_participant_menu(&avatar, state, entry);
		if let Some(user) = user {
			self.profile.person_click(ui, &avatar, None, user);
		}
		// Discord's LIVE pill marks a streamer on every tile size; strip tiles get a small one
		// so it never covers the avatar.
		if entry.participant.streaming {
			let (pill, font) = if compact {
				(egui::vec2(30.0, 15.0), 9.0)
			} else {
				(egui::vec2(40.0, 20.0), 11.0)
			};
			let margin = if compact { 5.0 } else { 8.0 };
			let live = egui::Rect::from_min_size(rect.left_top() + egui::Vec2::splat(margin), pill);
			ui.painter().rect_filled(live, 4, colors.danger);
			ui.painter().text(
				live.center(),
				egui::Align2::CENTER_CENTER,
				"LIVE",
				egui::FontId::new(font, design::medium_family(ui.ctx())),
				egui::Color32::WHITE,
			);
		}
		if compact {
			return;
		}
		// Name label with the mute glyph: a bottom-left badge on plates, centred under the
		// avatar once the plates are gone.
		if frameless {
			let font = egui::FontId::new(13.0, design::medium_family(ui.ctx()));
			let galley =
				ui.painter()
					.layout(name.to_owned(), font, STAGE_TEXT, (size.x - 24.0).max(20.0));
			ui.painter().galley(
				egui::pos2(
					rect.center().x - galley.size().x * 0.5,
					avatar_rect.bottom() + 20.0 - galley.size().y * 0.5,
				),
				galley,
				STAGE_TEXT,
			);
		} else {
			name_badge(ui, rect, name, silenced.filter(|_| video.is_some()));
		}
		// Watching is an explicit click, never automatic.
		if entry.participant.streaming
			&& !own && let Some(call) = call
			&& matches!(call.phase, Phase::Connected | Phase::Waiting)
		{
			let watching = call.watching == Some(entry.participant.user);
			let (label, fill, hint) = if watching {
				(
					"Watching",
					egui::Color32::from_black_alpha(170),
					"Stop receiving this screen share",
				)
			} else {
				(
					"Watch stream",
					colors.accent,
					"Receive this participant's screen share",
				)
			};
			let hover = if watching {
				colors.danger
			} else {
				colors.accent.gamma_multiply(1.2)
			};
			if tile_button(ui, rect, label, fill, hover, hint).clicked() {
				self.watch_request = Some((!watching).then_some(entry.participant.user));
			}
		}
	}

	fn stage_notices(&self, state: &State, channel: Id, connected: bool) -> Vec<(String, bool)> {
		let mut notices = Vec::new();
		if let Some(call) = state.voice.active.as_ref().filter(|c| c.channel == channel) {
			if self.screen.context == Some((state.generation, channel, call.request)) {
				let status = self.screen.capture_status.unwrap_or(self.screen.status);
				if !status.is_empty() {
					notices.push((status.into(), false));
				}
			}
			if !self.voice_camera_status.is_empty() {
				notices.push((self.voice_camera_status.into(), false));
			}
			if call.watching.is_none() && !self.voice_stream_status.is_empty() {
				notices.push((self.voice_stream_status.into(), false));
			}
			if call.server_deafened {
				notices.push(("Deafened by the server".into(), false));
			} else if call.server_muted {
				notices.push(("Muted by the server".into(), false));
			}
			if !state.demo && !state.can_speak(channel) {
				notices.push((
					"Speaking is unavailable in this channel. You can still listen.".into(),
					false,
				));
			} else if !state.demo
				&& state.permission(channel, model::permissions::USE_VAD) != Some(true)
			{
				notices.push((
					"Push-to-talk is required to speak here. Enable it in Voice settings.".into(),
					false,
				));
			}
		} else if !connected {
			if state.demo {
				notices.push((
					"Synthetic participants · microphone and speakers are off.".into(),
					false,
				));
			} else if let Some(reason) = self.call_unavailable(state, channel) {
				notices.push((reason.to_owned(), false));
			}
		}
		notices
	}

	pub(crate) fn call_unavailable(&self, state: &State, channel: Id) -> Option<&'static str> {
		if state.demo {
			Some("Calls are unavailable in the offline preview. No microphone is accessed.")
		} else if !self.voice_available {
			Some("Voice is unavailable in this session.")
		} else if state.auth != AuthState::Authenticated || !state.gateway_connected {
			Some("Reconnect to Discord before calling.")
		} else if self
			.voice_switch
			.as_ref()
			.is_some_and(|switch| switch.confirmed_at.is_some())
		{
			Some("Waiting for the previous call to disconnect.")
		} else if state
			.voice
			.active
			.as_ref()
			.is_some_and(|call| call.channel == channel)
		{
			Some("You are already in this call.")
		} else if !state.can_call(channel) {
			Some("Joining this channel is unavailable with current permission information.")
		} else {
			None
		}
	}

	/// Device-free check; the caller supplies an offline synthetic call fixture.
	#[cfg(all(debug_assertions, feature = "demo"))]
	pub fn debug_call_switch_check(mut state: State) {
		state.demo = false;
		state.gateway_connected = true;
		assert!(state.start_call(Id(25), false).is_some());
		let mut view = Self {
			voice_available: true,
			..Default::default()
		};
		let mut commands = Vec::new();
		view.request_call(&mut state, Id(22), false, &mut commands);
		assert!(matches!(
			commands.as_slice(),
			[Command::Voice(client_core::voice::Command::Leave {
				channel: Id(25),
				..
			})]
		));
		let from = view
			.voice_switch
			.as_ref()
			.expect("switch leaves the current call immediately")
			.from;
		assert!(view.voice_switch.as_ref().unwrap().confirmed_at.is_some());
		assert!(state.voice.active.is_none());
		commands.clear();
		let ctx = egui::Context::default();
		state.apply_voice(client_core::voice::Event::Departed {
			channel: from.0,
			request: from.1 + 1,
		});
		view.show_call_switch(&ctx, &mut state, &mut commands);
		assert!(commands.is_empty());
		state.apply_voice(client_core::voice::Event::Departed {
			channel: from.0,
			request: from.1,
		});
		view.show_call_switch(&ctx, &mut state, &mut commands);
		assert!(commands.is_empty(), "audio teardown must complete too");
		view.voice_switch_ready = true;
		view.show_call_switch(&ctx, &mut state, &mut commands);
		assert!(matches!(
			commands.as_slice(),
			[Command::Voice(client_core::voice::Command::Join {
				channel: Id(22),
				ring: false,
				..
			})]
		));
		assert!(view.voice_switch.is_none());
		commands.clear();
		view.request_call(&mut state, Id(25), false, &mut commands);
		assert!(matches!(
			commands.as_slice(),
			[Command::Voice(client_core::voice::Command::Leave {
				channel: Id(22),
				..
			})]
		));
		state.gateway_connected = false;
		view.show_call_switch(&ctx, &mut state, &mut commands);
		assert!(view.voice_switch.is_none());
	}

	pub(crate) fn request_call(
		&mut self,
		state: &mut State,
		channel: Id,
		ring: bool,
		commands: &mut Vec<Command>,
	) {
		let _ = self.request_call_audio(state, channel, ring, None, commands);
	}

	pub(crate) fn request_call_with_audio(
		&mut self,
		state: &mut State,
		channel: Id,
		ring: bool,
		muted: bool,
		deafened: bool,
		commands: &mut Vec<Command>,
	) -> Result<(), String> {
		self.request_call_audio(state, channel, ring, Some((muted, deafened)), commands)
	}

	fn request_call_audio(
		&mut self,
		state: &mut State,
		channel: Id,
		ring: bool,
		audio: Option<(bool, bool)>,
		commands: &mut Vec<Command>,
	) -> Result<(), String> {
		if let Some(reason) = self.call_unavailable(state, channel) {
			return Err(reason.into());
		}
		if state.voice.active.is_some() {
			state.voice.follow = None;
			let from = state
				.voice
				.active
				.as_ref()
				.map(|call| (call.channel, call.request))
				.expect("active call");
			self.voice_switch = Some(CallSwitch {
				from,
				channel,
				ring,
				generation: state.generation,
				confirmed_at: Some(std::time::Instant::now()),
				audio,
			});
			if let Some(command) = state.leave_call() {
				commands.push(command);
			}
			return Ok(());
		}
		let (muted, deafened) = audio.unwrap_or((self.voice_muted, self.voice_deafened));
		let command = state
			.start_call_with_mute(channel, ring, muted, deafened)
			.ok_or("Joining this call is no longer available")?;
		if audio.is_some() {
			self.voice_muted = muted;
			self.voice_deafened = deafened;
		}
		commands.push(command);
		Ok(())
	}

	pub(super) fn show_call_switch(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		self.follow_moved_call(state, commands);
		let Some(switch) = &self.voice_switch else {
			return;
		};
		if switch.generation != state.generation
			|| !self.voice_available
			|| !state.can_call(switch.channel)
		{
			let channel = switch.channel;
			self.voice_switch = None;
			if self.voice_camera_on_join == Some(channel) {
				self.voice_camera_on_join = None;
			}
			return;
		}
		if let Some(started) = switch.confirmed_at {
			if state.voice.active.is_some() {
				self.voice_switch = None;
			} else if state.voice.departed == Some(switch.from) && self.voice_switch_ready {
				let switch = self.voice_switch.take().expect("pending switch");
				let (muted, deafened) = switch
					.audio
					.unwrap_or((self.voice_muted, self.voice_deafened));
				if let Some(command) =
					state.start_call_with_mute(switch.channel, switch.ring, muted, deafened)
				{
					if switch.audio.is_some() {
						self.voice_muted = muted;
						self.voice_deafened = deafened;
					}
					commands.push(command);
				}
			} else if started.elapsed() >= std::time::Duration::from_secs(12) {
				self.voice_switch = None;
				state.status = "Call switch cancelled: previous call did not finish disconnecting. Reconnect before calling again.";
			} else {
				ctx.request_repaint_after(std::time::Duration::from_millis(100));
			}
			return;
		}
		if state
			.voice
			.active
			.as_ref()
			.map(|call| (call.channel, call.request))
			!= Some(switch.from)
		{
			self.voice_switch = None;
			return;
		}
		if let Some(command) = state.leave_call() {
			self.voice_switch
				.as_mut()
				.expect("pending switch")
				.confirmed_at = Some(std::time::Instant::now());
			commands.push(command);
		}
	}

	fn follow_moved_call(&mut self, state: &mut State, commands: &mut Vec<Command>) {
		let Some(follow) = state.voice.follow else {
			return;
		};
		if self.voice_switch.is_some() || state.voice.active.is_some() || !self.voice_switch_ready {
			return;
		}
		state.voice.follow = None;
		if let Some(command) =
			state.start_call_with_mute(follow.channel, false, follow.muted, follow.deafened)
		{
			commands.push(command);
		}
	}

	pub(super) fn call_button(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		channel: Id,
		commands: &mut Vec<Command>,
		video: bool,
	) -> egui::Response {
		let unavailable = self.call_unavailable(state, channel);
		let incoming = state.voice.incoming == Some(channel);
		let guild = state
			.channels
			.iter()
			.any(|c| c.id == channel && c.kind == 2);
		let label = if video {
			if guild {
				"Join Video"
			} else if incoming {
				"Answer with video"
			} else if state.voice.has_dm_call(channel) {
				"Join video"
			} else {
				"Start video call"
			}
		} else if guild {
			"Join Voice"
		} else if incoming {
			"Answer call"
		} else if state.voice.has_dm_call(channel) {
			"Join call"
		} else {
			"Start voice call"
		};
		let hint = unavailable.unwrap_or(if video {
			"Join the call and enable your selected camera as soon as voice is connected."
		} else if state.can_speak(channel) {
			"Join audio. Your microphone starts after the call is secured."
		} else {
			"Join to listen. Speaking is unavailable in this channel."
		});
		// Guild channels keep Discord's green Join Voice button; DM headers use an icon.
		let response = if guild {
			let colors = design::palette(ui);
			ui.add_enabled(
				unavailable.is_none(),
				egui::Button::new(design::medium(ui, label, 15.0).color(egui::Color32::WHITE))
					.fill(colors.positive)
					.stroke(egui::Stroke::NONE)
					.corner_radius(8)
					.min_size(egui::vec2(160.0, 40.0)),
			)
		} else {
			ui.add_enabled_ui(unavailable.is_none(), |ui| {
				crate::icons::button(
					ui,
					if video {
						crate::icons::Icon::Video
					} else {
						crate::icons::Icon::Phone
					},
					32.0,
					label,
				)
			})
			.inner
		}
		.on_hover_text(hint)
		.on_disabled_hover_text(hint);
		if response.clicked() {
			self.voice_camera_on_join = video.then_some(channel);
			if self
				.request_call_audio(state, channel, !guild && !incoming, None, commands)
				.is_err()
			{
				self.voice_camera_on_join = None;
				state.status = "Joining this call is no longer available.";
			}
		}
		response
	}

	pub(super) fn voice_settings(&mut self, ui: &mut egui::Ui, demo: bool, active: bool) {
		let trigger =
			crate::icons::button(ui, crate::icons::Icon::Headphones, 32.0, "Output settings");
		self.voice_settings_popup(&trigger, demo, active, false);
	}

	/// Input or output half of Discord's voice popout, matching the chevron that opened it.
	fn voice_settings_popup(
		&mut self,
		trigger: &egui::Response,
		demo: bool,
		active: bool,
		input: bool,
	) {
		let id = trigger.id.with("voice-settings-open");
		let mut open = trigger
			.ctx
			.data_mut(|data| *data.get_temp_mut_or_default::<bool>(id));

		if trigger.clicked() {
			open = !open;
		}
		// Device dropdowns use egui's popup memory; keep the parent independently open.
		let close_behavior = if egui::Popup::is_any_open(&trigger.ctx) {
			egui::PopupCloseBehavior::IgnoreClicks
		} else {
			egui::PopupCloseBehavior::CloseOnClickOutside
		};
		egui::Popup::menu(trigger)
			.open_bool(&mut open)
			.style(|_: &mut egui::Style| {})
			.width(340.0)
			.close_behavior(close_behavior)
			.frame(
				egui::Frame::popup(&trigger.ctx.style_of(trigger.ctx.theme()))
					.inner_margin(16)
					.corner_radius(12),
			)
			.show(|ui| {
				ui.set_width(308.0);
				ui.spacing_mut().item_spacing.y = 10.0;
				ui.label(design::semibold(
					ui,
					if input { "Input" } else { "Output" },
					18.0,
				));
				egui::ScrollArea::vertical()
					.max_height((ui.ctx().content_rect().height() - 180.0).clamp(180.0, 460.0))
					.show(ui, |ui| self.voice_popup_content(ui, demo, active, input));
				ui.separator();
				if ui
					.add_sized(
						[ui.available_width(), 32.0],
						egui::Button::new("All voice settings"),
					)
					.clicked()
				{
					self.open_voice_settings();
					ui.close();
				}
			});
		trigger.ctx.data_mut(|data| data.insert_temp(id, open));
	}

	/// One half of the voice popout: the device, its level and the toggles that belong to it.
	fn voice_popup_content(&mut self, ui: &mut egui::Ui, demo: bool, active: bool, input: bool) {
		let colors = design::palette(ui);
		ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
		if !demo && !self.voice_available {
			design::notice(
				ui,
				design::Level::Info,
				crate::i18n::text(
					self.language,
					"Install a voice-enabled build to use these controls.",
				),
			);
		}
		ui.add_enabled_ui(!demo && self.voice_available, |ui| {
			// Both settings surfaces share this path. Queue discovery once, without opening streams.
			if ui.is_enabled() && self.voice_device_status.is_empty() {
				self.voice_device_status = "Looking for audio devices...";
				self.voice_refresh_devices = true;
				ui.ctx().request_repaint();
			}
			let label = ui
				.horizontal(|ui| {
					crate::icons::inline(
						ui,
						if input {
							crate::icons::Icon::Microphone
						} else {
							crate::icons::Icon::Headphones
						},
						18.0,
						colors.muted,
					);
					ui.label(design::medium(
						ui,
						crate::i18n::text(
							self.language,
							if input {
								"Input device"
							} else {
								"Output device"
							},
						),
						15.0,
					))
				})
				.inner;
			if input {
				device_combo(
					ui,
					"voice-input",
					&self.voice_inputs,
					&mut self.voice_input,
					self.language,
				)
				.labelled_by(label.id);
				gain_slider(
					ui,
					&mut self.voice_gain.input_percent,
					crate::i18n::text(self.language, "Microphone gain"),
				);
			} else {
				device_combo(
					ui,
					"voice-output",
					&self.voice_outputs,
					&mut self.voice_output,
					self.language,
				)
				.labelled_by(label.id);
				gain_slider(
					ui,
					&mut self.voice_gain.output_percent,
					crate::i18n::text(self.language, "Speaker volume"),
				);
			}
			ui.horizontal_wrapped(|ui| {
				if ui
					.small_button(crate::i18n::text(self.language, "Rescan devices"))
					.clicked()
				{
					self.voice_refresh_devices = true;
				}
				if ui
					.small_button(crate::i18n::text(self.language, "Reset levels"))
					.clicked()
				{
					self.voice_gain = crate::VoiceGain::default();
				}
			});
			ui.separator();
			if input {
				ui.label(design::medium(
					ui,
					crate::i18n::text(self.language, "Noise suppression"),
					15.0,
				));
				self.noise_level_picker(ui, true);
				design::switch(
					ui,
					crate::i18n::text(self.language, "Push to talk"),
					Some(crate::i18n::text(
						self.language,
						"Hold your configured shortcut when you want to speak.",
					)),
					&mut self.voice_push_to_talk,
				);
			} else {
				ui.label(
					RichText::new(crate::i18n::text(
						self.language,
						"Deafen turns off incoming audio and mutes your microphone with it.",
					))
					.size(12.0)
					.color(colors.muted),
				);
			}
		});
		if self.voice_microphone_unavailable {
			ui.label(
				RichText::new(crate::i18n::text(
					self.language,
					"Microphone unavailable · choose another input. You are still connected.",
				))
				.size(12.0)
				.color(colors.warning),
			);
		}
		if !self.voice_device_status.is_empty() {
			ui.label(
				RichText::new(crate::i18n::text(self.language, self.voice_device_status))
					.size(12.0)
					.color(colors.muted),
			);
		}
		if active
			&& !input && let Some(code) = &self.voice_privacy_code
		{
			egui::CollapsingHeader::new(crate::i18n::text(self.language, "Voice privacy code"))
				.show(ui, |ui| {
					ui.add(
						egui::Label::new(RichText::new(code).monospace())
							.selectable(true)
							.wrap(),
					);
				});
		}
	}

	fn microphone_preview_controls(&mut self, ui: &mut egui::Ui, active: bool) {
		let colors = design::palette(ui);
		ui.label(design::medium(ui, "Microphone test", 15.0));
		ui.label(
			RichText::new(if active {
				"Leave the call to test your microphone locally."
			} else {
				"Hear yourself through your selected speakers. Use headphones to avoid feedback."
			})
			.size(13.0)
			.color(colors.muted),
		);
		ui.horizontal(|ui| {
			ui.spacing_mut().item_spacing.x = 12.0;
			ui.add_enabled_ui(!active, |ui| {
				if design::button(
					ui,
					if self.voice_preview_requested {
						"Stop testing"
					} else {
						"Start testing"
					},
					if self.voice_preview_requested {
						design::ButtonKind::Outline
					} else {
						design::ButtonKind::Primary
					},
				)
				.clicked()
				{
					self.voice_preview_requested = !self.voice_preview_requested;
					self.voice_preview_status = "";
					self.voice_preview_level = None;
					ui.ctx().request_repaint();
				}
			});
			if self.voice_preview_requested || active {
				let db = self.voice_preview_level.unwrap_or(-100.0);
				ui.label(
					RichText::new(format!("Input level {db:.0} dBFS"))
						.size(13.0)
						.color(colors.muted),
				);
			}
		});
		let (rect, _) =
			ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::hover());
		let level = ((self.voice_preview_level.unwrap_or(-100.0) + 80.0) / 80.0).clamp(0.0, 1.0);
		let bars = (rect.width() / 9.0).floor().max(1.0) as usize;
		for index in 0..bars {
			let fraction = index as f32 / bars as f32;
			let color = if fraction < level {
				if fraction > 0.9 {
					colors.danger
				} else if fraction > 0.7 {
					colors.warning
				} else {
					colors.positive
				}
			} else {
				colors.border
			};
			let left = rect.left() + index as f32 * rect.width() / bars as f32;
			ui.painter().rect_filled(
				egui::Rect::from_min_size(
					egui::pos2(left, rect.top()),
					egui::vec2((rect.width() / bars as f32 - 3.0).max(1.0), rect.height()),
				),
				2.0,
				color,
			);
		}
		if !self.voice_preview_status.is_empty() {
			ui.label(
				RichText::new(self.voice_preview_status)
					.size(13.0)
					.color(colors.muted),
			);
		}
	}

	pub(super) fn voice_settings_content(
		&mut self,
		ui: &mut egui::Ui,
		demo: bool,
		active: bool,
		compact: bool,
	) {
		let colors = design::palette(ui);
		ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
		ui.spacing_mut().item_spacing.y = if compact { 6.0 } else { 12.0 };
		if !demo && !self.voice_available {
			design::notice(
				ui,
				design::Level::Info,
				crate::i18n::text(
					self.language,
					"Install a voice-enabled build to use these controls.",
				),
			);
		}
		ui.add_enabled_ui(!demo && self.voice_available, |ui| {
			if compact {
				self.voice_audio_controls(ui);
				egui::CollapsingHeader::new(crate::i18n::text(
					self.language,
					"Voice processing & input mode",
				))
				.show(ui, |ui| self.voice_processing_controls(ui));
			} else {
				design::group(
					ui,
					crate::i18n::text(self.language, "Devices & levels"),
					|ui| {
						self.voice_audio_controls(ui);
						design::card_divider(ui);
						self.microphone_preview_controls(ui, active);
					},
				);
				design::group(
					ui,
					crate::i18n::text(self.language, "Voice processing"),
					|ui| self.voice_processing_controls(ui),
				);
			}
		});
		if !compact {
			crate::keybinds::show_voice(
				ui,
				&mut self.keybinds,
				&mut self.keybind_capture,
				self.global_keybind_status,
				self.language,
			);
		}
		design::group(
			ui,
			crate::i18n::text(self.language, "Connection recovery"),
			|ui| self.voice_auto_rejoin_setting(ui),
		);
		if !compact {
			design::group(ui, crate::i18n::text(self.language, "Camera"), |ui| {
				self.camera_settings_content(ui, demo)
			});
		} else {
			self.camera_settings_content(ui, demo);
		}
		if active && let Some(code) = &self.voice_privacy_code {
			egui::CollapsingHeader::new(crate::i18n::text(self.language, "Voice privacy code"))
				.show(ui, |ui| {
					ui.add(
						egui::Label::new(RichText::new(code).monospace())
							.selectable(true)
							.wrap(),
					);
					ui.label(
						RichText::new(crate::i18n::text(
							self.language,
							"Compare with the other participants. This code changes with the encrypted call group.",
						))
						.size(12.0)
						.color(colors.muted),
					);
				});
		}
		if !compact {
			design::hint(
				ui,
				crate::i18n::text(
					self.language,
					"Audio preferences are saved on this device. Your microphone starts only when you join a call or start testing.",
				),
			);
		}
	}

	fn camera_settings_content(&mut self, ui: &mut egui::Ui, demo: bool) {
		let colors = design::palette(ui);
		ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
		if !cfg!(any(
			target_os = "windows",
			target_os = "macos",
			target_os = "linux"
		)) {
			ui.label("Camera capture is unavailable on this platform.");
			return;
		}
		if demo && self.voice_cameras.is_empty() {
			self.voice_cameras = vec![
				(
					"synthetic-integrated".into(),
					"Integrated Camera (preview)".into(),
				),
				("synthetic-usb".into(), "USB Camera (preview)".into()),
			];
		}
		if ui.is_enabled()
			&& !demo && self.voice_camera_device_status.is_empty()
			&& !self.voice_camera_devices_loading
		{
			self.voice_camera_device_status = "Looking for cameras...";
			self.voice_refresh_cameras = true;
			ui.ctx().request_repaint();
		}
		let label = ui.label(design::medium(ui, "Camera device", 15.0));
		device_combo(
			ui,
			"voice-camera",
			&self.voice_cameras,
			&mut self.voice_camera_device,
			self.language,
		)
		.labelled_by(label.id);
		ui.horizontal(|ui| {
			ui.spacing_mut().item_spacing.x = 4.0;
			ui.add_enabled_ui(!demo && !self.voice_camera_devices_loading, |ui| {
				if design::text_action(ui, "Refresh cameras").clicked() {
					self.voice_refresh_cameras = true;
				}
			});
			if !self.voice_camera_device_status.is_empty() && !demo {
				ui.label(
					RichText::new(self.voice_camera_device_status)
						.size(12.0)
						.color(colors.muted),
				);
			}
		});
		if !demo {
			design::hint(
				ui,
				"Changing devices stops your camera and takes effect the next time you turn it on.",
			);
		}
		if self.voice_settings_open() {
			design::card_divider(ui);
			let width = ui.available_width();
			let (rect, _) = ui.allocate_exact_size(
				egui::vec2(width, (width * 9.0 / 16.0).clamp(160.0, 300.0)),
				egui::Sense::hover(),
			);
			ui.painter().rect_filled(rect, 12, colors.base);
			let texture = self
				.camera_test_texture
				.as_ref()
				.or(self.voice_camera_preview.as_ref());
			let has_picture = texture.is_some();
			if let Some(texture) = texture {
				let size = texture.size_vec2();
				let scale = (rect.width() / size.x).min(rect.height() / size.y);
				egui::Image::new(texture)
					.uv(egui::Rect::from_min_max(
						egui::pos2(1.0, 0.0),
						egui::pos2(0.0, 1.0),
					))
					.corner_radius(12)
					.paint_at(
						ui,
						egui::Rect::from_center_size(rect.center(), size * scale),
					);
			}
			if !has_picture || self.camera_test_requested {
				let center = if has_picture {
					egui::pos2(rect.center().x, rect.bottom() - 34.0)
				} else {
					rect.center()
				};
				let button_rect =
					egui::Rect::from_center_size(center, egui::vec2(184.0_f32.min(width), 44.0));
				ui.scope_builder(egui::UiBuilder::new().max_rect(button_rect), |ui| {
					ui.add_enabled_ui(!demo && self.camera_test_available, |ui| {
						let (icon, label) = if self.camera_test_requested {
							(crate::icons::Icon::VideoSlash, "Stop preview")
						} else {
							(crate::icons::Icon::Video, "Preview camera")
						};
						if design::primary_icon_button(ui, icon, label).clicked() {
							self.camera_test_requested = !self.camera_test_requested;
							self.camera_test_status = "";
						}
					});
				});
			}
			if !self.camera_test_status.is_empty() {
				design::hint(ui, self.camera_test_status);
			}
		}
	}

	fn camera_settings_popup(&mut self, trigger: &egui::Response, demo: bool) {
		let id = trigger.id.with("camera-settings-open");
		let mut open = trigger
			.ctx
			.data_mut(|data| *data.get_temp_mut_or_default::<bool>(id));
		if trigger.clicked() {
			open = !open;
		}
		let close_behavior = if egui::Popup::is_any_open(&trigger.ctx) {
			egui::PopupCloseBehavior::IgnoreClicks
		} else {
			egui::PopupCloseBehavior::CloseOnClickOutside
		};
		egui::Popup::menu(trigger)
			.open_bool(&mut open)
			.style(|_: &mut egui::Style| {})
			.width(300.0)
			.close_behavior(close_behavior)
			.show(|ui| self.camera_settings_content(ui, demo));
		trigger.ctx.data_mut(|data| data.insert_temp(id, open));
	}

	fn voice_audio_controls(&mut self, ui: &mut egui::Ui) {
		design::hint(
			ui,
			crate::i18n::text(
				self.language,
				"System default follows your operating-system choice. Select a device only when you want Nivra to stay pinned to it.",
			),
		);
		// Both settings surfaces use this path. Queue discovery once, without opening streams.
		if ui.is_enabled() && self.voice_device_status.is_empty() {
			self.voice_device_status = "Looking for audio devices...";
			self.voice_refresh_devices = true;
			ui.ctx().request_repaint();
		}
		let colors = design::palette(ui);
		let mut device = |ui: &mut egui::Ui, input: bool| {
			let label = ui
				.horizontal(|ui| {
					crate::icons::inline(
						ui,
						if input {
							crate::icons::Icon::Microphone
						} else {
							crate::icons::Icon::Headphones
						},
						18.0,
						colors.muted,
					);
					ui.label(design::medium(
						ui,
						crate::i18n::text(
							self.language,
							if input {
								"Input device"
							} else {
								"Output device"
							},
						),
						15.0,
					))
				})
				.inner;
			if input {
				device_combo(
					ui,
					"voice-input",
					&self.voice_inputs,
					&mut self.voice_input,
					self.language,
				)
				.labelled_by(label.id);
			} else {
				device_combo(
					ui,
					"voice-output",
					&self.voice_outputs,
					&mut self.voice_output,
					self.language,
				)
				.labelled_by(label.id);
			}
		};
		if ui.available_width() >= 480.0 {
			ui.columns(2, |columns| {
				device(&mut columns[0], true);
				device(&mut columns[1], false);
			});
		} else {
			device(ui, true);
			device(ui, false);
		}
		gain_controls(ui, &mut self.voice_gain, self.language);
		design::switch(
			ui,
			crate::i18n::text(self.language, "Start bots at 50% volume"),
			Some(crate::i18n::text(
				self.language,
				"Protects your hearing from bots that join very loud. Right-click a bot in the call to change its volume.",
			)),
			&mut self.voice_bot_safe_volume,
		);
		ui.horizontal(|ui| {
			ui.spacing_mut().item_spacing.x = 4.0;
			if design::text_action(ui, crate::i18n::text(self.language, "Rescan devices")).clicked()
			{
				self.voice_refresh_devices = true;
			}
			if self.voice_gain != crate::VoiceGain::default()
				&& design::text_action(ui, crate::i18n::text(self.language, "Reset levels"))
					.clicked()
			{
				self.voice_gain = crate::VoiceGain::default();
			}
			if !self.voice_device_status.is_empty() {
				ui.label(
					RichText::new(crate::i18n::text(self.language, self.voice_device_status))
						.size(12.0)
						.color(colors.muted),
				);
			}
		});
		let input_missing = self
			.voice_input
			.as_ref()
			.is_some_and(|selected| !self.voice_inputs.iter().any(|(id, _)| id == selected));
		let output_missing = self
			.voice_output
			.as_ref()
			.is_some_and(|selected| !self.voice_outputs.iter().any(|(id, _)| id == selected));
		if input_missing || output_missing {
			design::notice(
				ui,
				design::Level::Warning,
				crate::i18n::text(
					self.language,
					"One selected audio device is unavailable. Choose System default or rescan devices.",
				),
			);
		}
		if self.voice_microphone_unavailable {
			design::notice(
				ui,
				design::Level::Warning,
				crate::i18n::text(
					self.language,
					"Microphone unavailable · choose another input. You are still connected.",
				),
			);
		}
	}

	/// Called when the audio worker could not run Maximum in real time; it has already
	/// switched to Standard, and this saves that choice and tells the user why.
	pub fn voice_noise_fallback(&mut self) {
		if self.voice_processing.effective().suppression == NoiseSuppression::DeepFilter {
			self.set_noise_level(NoiseSuppression::RnNoise);
			self.voice_noise_fell_back = true;
		}
	}

	fn set_noise_level(&mut self, level: NoiseSuppression) {
		let current = self.voice_processing.effective().suppression;
		if current != NoiseSuppression::Off {
			self.voice_noise_restore = current;
		}
		if level == current {
			return;
		}
		let processing = self.voice_processing.edit();
		processing.suppression = level;
		if level == NoiseSuppression::WebRtc {
			processing.suppression_level = processing.suppression_level.max(2);
		}
		if level != NoiseSuppression::Off {
			self.voice_noise_restore = level;
		}
		self.voice_noise_fell_back = false;
	}

	/// Call-panel toggle: off, or back to the level the user chose last.
	fn toggle_noise(&mut self) {
		if self.voice_processing.effective().suppression == NoiseSuppression::Off {
			self.set_noise_level(self.voice_noise_restore);
		} else {
			self.set_noise_level(NoiseSuppression::Off);
		}
	}

	/// The four suppression levels as plain-language choices, shared by Settings, the input
	/// popup and the call button's context menu. `compact` drops the explanations.
	fn noise_level_picker(&mut self, ui: &mut egui::Ui, compact: bool) {
		let language = self.language;
		let current = self.voice_processing.effective().suppression;
		let mut chosen = None;
		ui.scope(|ui| {
			ui.spacing_mut().item_spacing.y = if compact { 2.0 } else { 4.0 };
			for level in NoiseSuppression::ALL {
				if noise_level_row(ui, level, level == current, compact, language).clicked() {
					chosen = Some(level);
				}
			}
		});
		if let Some(level) = chosen {
			self.set_noise_level(level);
		}
		if self.voice_noise_fell_back {
			design::notice(
				ui,
				design::Level::Warning,
				crate::i18n::text(
					language,
					"Your PC couldn't keep up with Maximum, so Nivra switched to Standard to keep your voice smooth.",
				),
			);
		}
	}

	fn voice_processing_controls(&mut self, ui: &mut egui::Ui) {
		let colors = design::palette(ui);
		let language = self.language;
		let t = |english: &'static str| crate::i18n::text(language, english);

		design::section(
			ui,
			t("Noise suppression"),
			Some(t(
				"Removes background noise from your microphone before anyone else hears it.",
			)),
		);
		self.noise_level_picker(ui, false);
		design::hint(
			ui,
			t(
				"Friends complaining about noise? Choose Maximum. If your voice cuts out or your PC slows down, go back to Standard.",
			),
		);

		design::card_divider(ui);
		design::switch(
			ui,
			t("Push to talk"),
			Some(t(
				"When enabled, your microphone transmits only while the configured shortcut is held.",
			)),
			&mut self.voice_push_to_talk,
		)
		.on_hover_text(t("Mute and deafen always take priority."));

		design::card_divider(ui);
		egui::CollapsingHeader::new(t("Advanced input settings"))
			.default_open(false)
			.show(ui, |ui| {
				design::hint(
					ui,
					t(
						"The defaults work for most people. Change these only if something sounds wrong.",
					),
				);
				let effective = self.voice_processing.effective();
				let mut echo = effective.echo_cancellation;
				if design::switch(
					ui,
					t("Echo cancellation"),
					Some(t(
						"Recommended when speakers can be picked up by your microphone.",
					)),
					&mut echo,
				)
				.changed()
				{
					self.voice_processing.edit().echo_cancellation = echo;
				}
				let mut gain = effective.automatic_gain;
				if design::switch(
					ui,
					t("Automatic microphone volume"),
					Some(t(
						"Keeps speech at a more consistent loudness without changing your output volume.",
					)),
					&mut gain,
				)
				.changed()
				{
					self.voice_processing.edit().automatic_gain = gain;
				}
				design::card_divider(ui);
				let effective = self.voice_processing.effective();
				let mut sensitivity = effective.sensitivity_db.is_some();
				let sensitivity_detail = if sensitivity {
					"Only transmit sound above the threshold."
				} else {
					"Open voice activity; mute and push to talk still apply."
				};
				if design::switch(
					ui,
					t("Voice activity threshold"),
					Some(t(sensitivity_detail)),
					&mut sensitivity,
				)
				.changed()
				{
					self.voice_processing.edit().sensitivity_db =
						sensitivity.then_some(effective.sensitivity_db.unwrap_or(-55));
				}
				if sensitivity {
					let mut threshold = self
						.voice_processing
						.effective()
						.sensitivity_db
						.unwrap_or(-55);
					let before = threshold;
					design::slider(ui, &mut threshold, -80..=0, " dBFS");
					if threshold != before {
						self.voice_processing.edit().sensitivity_db = Some(threshold);
					}
				}
				if let Some(level) = self.voice_preview_level {
					ui.add(
						egui::ProgressBar::new(((level + 80.0) / 80.0).clamp(0.0, 1.0))
							.text(format!("{}: {level:.0} dBFS", t("Input level")))
							.fill(colors.positive),
					);
				}
				if self.voice_processing.effective().suppression == NoiseSuppression::WebRtc {
					design::card_divider(ui);
					let labels = ["Low", "Moderate", "High", "Very high"];
					let mut strength = self.voice_processing.effective().suppression_level.min(3);
					design::row(
						ui,
						t("Light suppression strength"),
						Some(t(
							"Higher levels remove more noise but can affect natural voice detail.",
						)),
						|ui| {
							egui::ComboBox::from_id_salt("voice-suppression-strength")
								.selected_text(t(labels[usize::from(strength)]))
								.width(ui.available_width().min(160.0))
								.show_ui(ui, |ui| {
									for (index, label) in labels.iter().enumerate() {
										ui.selectable_value(&mut strength, index as u8, t(label));
									}
								});
						},
					);
					if strength != self.voice_processing.effective().suppression_level {
						self.voice_processing.edit().suppression_level = strength;
					}
				}
				design::card_divider(ui);
				ui.horizontal_wrapped(|ui| {
					if design::text_action(ui, t("Recommended defaults")).clicked() {
						self.voice_processing = Default::default();
						self.voice_noise_fell_back = false;
					}
					if design::text_action(ui, t("Raw microphone")).clicked() {
						let processing = self.voice_processing.edit();
						processing.suppression = NoiseSuppression::Off;
						processing.echo_cancellation = false;
						processing.automatic_gain = false;
						processing.sensitivity_db = None;
					}
				});
			});
	}

	/// Whether the local mute/deafen controls may emit commands for the active call.
	fn controls_enabled(&self, state: &State) -> bool {
		self.voice_available
			&& !state.demo
			&& state
				.voice
				.active
				.as_ref()
				.is_some_and(|call| call.phase != Phase::Failed)
	}

	fn queue_voice_toggle_cue(&mut self, deafen: bool, active: bool) {
		let cue = match (deafen, active) {
			(true, true) => model::notification_preferences::Sound::Deafen,
			(true, false) => model::notification_preferences::Sound::Undeafen,
			(false, true) => model::notification_preferences::Sound::Mute,
			(false, false) => model::notification_preferences::Sound::Unmute,
		};
		// This is a live client event, not the explicit Settings preview.
		// DND and per-sound preferences are applied by the desktop notification runtime.
		if self.notification_cues.len() < 4 {
			self.notification_cues.push(cue);
		}
	}

	/// Mute or deafen toggle: red slashed glyph while active, like Discord's user area.
	pub(super) fn mute_toggle(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
		deafen: bool,
		size: f32,
	) -> egui::Response {
		let colors = design::palette(ui);
		let Some(call) = state.voice.active.as_ref() else {
			let active = if deafen {
				self.voice_deafened
			} else {
				self.voice_muted
			};
			let label = match (deafen, active) {
				(true, true) => "Undeafen",
				(true, false) => "Deafen",
				(false, true) => "Unmute",
				(false, false) => "Mute",
			};
			let (rect, response) =
				ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::click());
			if response.hovered() || response.has_focus() {
				ui.painter().rect_filled(rect, 6, colors.hover);
			}
			crate::icons::paint(
				ui.painter(),
				match (deafen, active) {
					(true, true) => crate::icons::Icon::HeadphonesSlash,
					(true, false) => crate::icons::Icon::Headphones,
					(false, true) => crate::icons::Icon::MicrophoneSlash,
					(false, false) => crate::icons::Icon::Microphone,
				},
				rect.shrink(size * 0.2),
				if active { colors.danger } else { colors.muted },
			);
			response.widget_info(|| {
				egui::WidgetInfo::selected(egui::Role::Button, true, active, label)
			});
			if response.clicked() {
				if deafen {
					self.voice_deafened = !active;
					self.queue_voice_toggle_cue(true, !active);
				} else {
					self.voice_muted = !active;
					self.queue_voice_toggle_cue(false, !active);
				}
			}
			return response.on_hover_text(format!("{label}; applies to your next call."));
		};
		let channel = call.channel;
		let can_speak = state.can_speak(channel);
		let (mut muted, mut deafened) = (self.voice_muted || !can_speak, self.voice_deafened);
		let active = if deafen { deafened } else { muted };
		let enabled =
			(self.controls_enabled(state) || state.demo) && (deafen || can_speak || state.demo);
		let label = match (deafen, active) {
			(true, true) => "Undeafen",
			(true, false) => "Deafen",
			(false, true) => "Unmute",
			(false, false) => "Mute",
		};
		let response = ui
			.add_enabled_ui(enabled, |ui| {
				let (rect, response) =
					ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::click());
				if response.hovered() || response.has_focus() {
					ui.painter().rect_filled(rect, 6, colors.hover);
				}
				let icon = match (deafen, active) {
					(true, true) => crate::icons::Icon::HeadphonesSlash,
					(true, false) => crate::icons::Icon::Headphones,
					(false, true) => crate::icons::Icon::MicrophoneSlash,
					(false, false) => crate::icons::Icon::Microphone,
				};
				let color = if !enabled {
					colors.muted.gamma_multiply(0.5)
				} else if active {
					colors.danger
				} else if response.hovered() || response.has_focus() {
					colors.text_strong
				} else {
					colors.muted
				};
				crate::icons::paint(ui.painter(), icon, rect.shrink(size * 0.2), color);
				response.widget_info(|| {
					egui::WidgetInfo::selected(egui::Role::Button, enabled, active, label)
				});
				response
			})
			.inner
			.on_hover_text(if enabled {
				label
			} else if !can_speak && !deafen {
				"Speaking is unavailable in this channel."
			} else {
				"Controls are unavailable in this build or preview."
			});
		if response.clicked() {
			if deafen {
				deafened = !deafened;
				self.queue_voice_toggle_cue(true, deafened);
			} else {
				muted = !muted;
				self.queue_voice_toggle_cue(false, muted);
			}
			self.voice_muted = muted;
			self.voice_deafened = deafened;
			if let Some(command) = state.set_call_mute(muted, deafened) {
				commands.push(command);
			}
		}
		response
	}

	/// Discord's call control bar: one media pill and the red hang-up button.
	fn call_controls(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		channel: Id,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		let Some(call) = state.voice.active.as_ref() else {
			return;
		};
		let phase = call.phase;
		let camera = call.camera;
		let can_camera = self.voice_camera_available
			&& state.can_camera(channel)
			&& matches!(phase, Phase::Connected | Phase::Waiting);
		let can_speak = state.can_speak(channel);
		let (mut muted, mut deafened) = (self.voice_muted || !can_speak, self.voice_deafened);
		let controls = self.controls_enabled(state);
		let voice_toggles = controls || state.demo;
		let language = self.language;
		let t = |english: &'static str| crate::i18n::text(language, english);
		let focused = self.voice_focus.is_some();
		let pill_width = MEDIA_PILL + if focused { 48.0 } else { 0.0 };
		let width = pill_width + BAR_GAP + HANG_UP;
		let mut camera_clicked = false;
		let mut mute_clicked = false;
		let mut deafen_clicked = false;
		let mut leave = false;
		ui.horizontal(|ui| {
			ui.spacing_mut().item_spacing.x = BAR_GAP;
			ui.add_space(((ui.available_width() - width) * 0.5).max(0.0));
			pill(ui, pill_width, |ui| {
				let mic = control(
					ui,
					if muted {
						crate::icons::Icon::MicrophoneSlash
					} else {
						crate::icons::Icon::Microphone
					},
					48.0,
					voice_toggles && (can_speak || state.demo),
					if muted { colors.danger } else { STAGE_TEXT },
					if muted { t("Unmute") } else { t("Mute") },
					if !can_speak {
						t("Speaking is unavailable in this channel.")
					} else if muted {
						t("Turn on microphone")
					} else {
						t("Turn off microphone")
					},
				);
				mute_clicked = mic.clicked();
				let settings = control(
					ui,
					crate::icons::Icon::ChevronDown,
					28.0,
					true,
					STAGE_TEXT,
					t("Voice settings"),
					t("Microphone and speaker settings"),
				);
				self.voice_settings_popup(&settings, state.demo, true, true);
				deafen_clicked = control(
					ui,
					if deafened {
						crate::icons::Icon::HeadphonesSlash
					} else {
						crate::icons::Icon::Headphones
					},
					48.0,
					voice_toggles,
					if deafened { colors.danger } else { STAGE_TEXT },
					if deafened { t("Undeafen") } else { t("Deafen") },
					if deafened {
						t("Turn on incoming audio")
					} else {
						t("Turn off incoming audio")
					},
				)
				.clicked();
				camera_clicked = control(
					ui,
					if camera {
						crate::icons::Icon::Video
					} else {
						crate::icons::Icon::VideoSlash
					},
					48.0,
					controls && (camera || can_camera),
					if camera { colors.positive } else { STAGE_TEXT },
					if camera {
						t("Turn off camera")
					} else {
						t("Turn on camera")
					},
					if camera {
						t("Stop sharing your camera")
					} else if state.demo {
						"Camera is off in the offline preview"
					} else if !cfg!(any(
						target_os = "macos",
						target_os = "windows",
						target_os = "linux"
					)) {
						"Camera capture is unavailable on this platform"
					} else if !self.voice_camera_available {
						"Camera requires H264 support from the voice server"
					} else if !state.can_camera(channel) {
						"Camera is unavailable with current channel permissions"
					} else {
						t("Share your selected camera with this call")
					},
				)
				.clicked();
				let camera_settings = control(
					ui,
					crate::icons::Icon::ChevronDown,
					28.0,
					true,
					STAGE_TEXT,
					"Camera settings",
					"Choose a camera",
				);
				self.camera_settings_popup(&camera_settings, state.demo);
				self.screen_share_control(ui, state);
				if focused {
					let shown = self.voice_focus_participants;
					if control(
						ui,
						crate::icons::Icon::People,
						48.0,
						true,
						if shown { colors.accent } else { STAGE_TEXT },
						if shown {
							"Hide participants"
						} else {
							"Show participants"
						},
						if shown {
							"Hide the participant strip under the enlarged video"
						} else {
							"Show the other participants under the enlarged video"
						},
					)
					.clicked()
					{
						self.voice_focus_participants = !shown;
					}
				}
			});
			let hang_up = {
				let (rect, response) = ui
					.allocate_exact_size(egui::vec2(HANG_UP, CONTROL_HEIGHT), egui::Sense::click());
				let enabled = !state.demo;
				let fill = if !enabled {
					colors.danger.gamma_multiply(0.45)
				} else if response.hovered() || response.has_focus() {
					colors.danger.gamma_multiply(0.85)
				} else {
					colors.danger
				};
				ui.painter().rect_filled(rect, 12, fill);
				crate::icons::paint(
					ui.painter(),
					crate::icons::Icon::HangUp,
					egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(22.0)),
					egui::Color32::WHITE,
				);
				let label = if phase == Phase::Failed {
					t("Dismiss call")
				} else {
					t("Disconnect")
				};
				response
					.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
				response.on_hover_text(if enabled {
					label
				} else {
					"Leaving is unavailable in the offline preview."
				})
			};
			leave = hang_up.clicked() && !state.demo;
		});
		if camera_clicked && let Some(command) = state.set_call_camera(!camera) {
			self.voice_camera_status = "";
			commands.push(command);
		}
		if mute_clicked {
			muted = !muted;
			self.queue_voice_toggle_cue(false, muted);
		}
		if deafen_clicked {
			deafened = !deafened;
			self.queue_voice_toggle_cue(true, deafened);
		}
		if mute_clicked || deafen_clicked {
			self.voice_muted = muted;
			self.voice_deafened = deafened;
		}
		if (mute_clicked || deafen_clicked)
			&& let Some(command) = state.set_call_mute(muted, deafened)
		{
			commands.push(command);
		}
		if leave && let Some(command) = state.leave_call() {
			commands.push(command);
		}
	}

	fn screen_share_control(&mut self, ui: &mut egui::Ui, state: &State) {
		let enabled = self.screen.busy
			|| state.demo
			|| (self.screen.supported
				&& state.voice.active.as_ref().is_some_and(|call| {
					matches!(call.phase, Phase::Connected | Phase::Waiting)
						&& state.can_stream(call.channel)
				}));
		let language = self.language;
		let t = |english: &'static str| crate::i18n::text(language, english);
		let label = if self.screen.busy {
			t("Stop sharing")
		} else {
			t("Share your screen")
		};
		let color = if self.screen.busy {
			design::palette(ui).accent
		} else {
			STAGE_TEXT
		};
		if control(
			ui,
			crate::icons::Icon::ScreenShare,
			48.0,
			enabled,
			color,
			label,
			if enabled {
				self.screen
					.capture_status
					.unwrap_or(if self.screen.status.is_empty() {
						label
					} else {
						self.screen.status
					})
			} else {
				"Screen sharing requires a connected call and video permission on a supported desktop."
			},
		)
		.clicked()
		{
			self.screen.launch(state);
		}
	}

	/// Gateway backoff indicator; appears only after a short disconnect delay.
	pub(super) fn gateway_reconnect_banner(&mut self, ui: &mut egui::Ui, state: &State) {
		let Some(disconnected_at) = state.gateway_disconnected_at else {
			return;
		};
		if state.gateway_connected || state.demo {
			return;
		}
		if disconnected_at.elapsed() < std::time::Duration::from_secs(2) {
			return;
		}
		let Some(reconnect) = state.gateway_reconnect else {
			return;
		};
		let colors = design::palette(ui);
		let label = format!(
			"{} · {} {}",
			crate::i18n::text(self.language, "Reconnecting"),
			crate::i18n::text(self.language, "attempt"),
			reconnect.attempt
		);
		let retry = std::time::Duration::from_millis(reconnect.retry_in_ms.max(1));
		let progress =
			(reconnect.since.elapsed().as_secs_f32() / retry.as_secs_f32()).clamp(0.0, 1.0);
		egui::Panel::top("gateway-reconnect")
			.show_separator_line(false)
			.frame(
				egui::Frame::new()
					.fill(colors.raised)
					.inner_margin(egui::Margin::symmetric(16, 10)),
			)
			.show(ui, |ui| {
				ui.vertical(|ui| {
					ui.spacing_mut().item_spacing.y = 8.0;
					ui.label(design::semibold(ui, &label, 14.0).color(colors.text_strong));
					ui.add(
						egui::ProgressBar::new(progress)
							.desired_width(f32::INFINITY)
							.fill(colors.accent),
					);
					if disconnected_at.elapsed() >= std::time::Duration::from_secs(2)
						&& ui
							.button(crate::i18n::text(self.language, "Reconnect now"))
							.clicked()
					{
						self.gateway_reconnect_skip_requested = true;
					}
				});
			});
	}

	pub(super) fn voice_auto_rejoin_setting(&mut self, ui: &mut egui::Ui) {
		let mut enabled = self.voice_auto_rejoin_short_disconnect;
		let previous = enabled;
		design::switch(
			ui,
			crate::i18n::text(self.language, "Rejoin calls after brief disconnects"),
			Some(crate::i18n::text(
				self.language,
				"Automatically returns to the same call when Discord reconnects within 15 seconds.",
			)),
			&mut enabled,
		);
		if enabled != previous {
			if enabled {
				self.voice_auto_rejoin_confirm = true;
			} else {
				self.voice_auto_rejoin_short_disconnect = false;
			}
		}
	}

	pub(super) fn voice_auto_rejoin_dialog(&mut self, ctx: &egui::Context) {
		if !self.voice_auto_rejoin_confirm {
			return;
		}
		let t = |english: &'static str| crate::i18n::text(self.language, english);
		match crate::dialog::DoubleConfirm::new(
			"voice-auto-rejoin",
			t("Rejoin calls after brief disconnects"),
			t("This can put you back on voice without an extra tap after short outages."),
			t(
				"Only enable this if you are comfortable rejoining voice automatically on this device.",
			),
		)
		.confirm_labels(t("Continue"), t("Enable auto-rejoin"))
		.show(ctx)
		{
			Some(crate::dialog::Choice::Confirmed) => {
				self.voice_auto_rejoin_short_disconnect = true;
				self.voice_auto_rejoin_confirm = false;
			}
			Some(crate::dialog::Choice::Cancelled) | None => {
				self.voice_auto_rejoin_confirm = false;
			}
		}
	}

	/// Offer to rejoin the last call after an abrupt close. Never joins by itself.
	pub(super) fn reconnect_call_banner(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let Some((channel, _)) = self.reconnect_offer else {
			return;
		};
		if state.voice.active.is_some() {
			return;
		}
		let colors = design::palette(ui);
		let name = state
			.channel(channel)
			.map(|channel| channel.name.as_str())
			.filter(|name| !name.is_empty())
			.unwrap_or(crate::i18n::text(self.language, "Recent call"))
			.to_owned();
		let unavailable = self.call_unavailable(state, channel);
		egui::Panel::top("reconnect-call")
			.show_separator_line(false)
			.frame(
				egui::Frame::new()
					.fill(colors.raised)
					.inner_margin(egui::Margin::symmetric(16, 10)),
			)
			.show(ui, |ui| {
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 12.0;
					crate::icons::inline(ui, crate::icons::Icon::Phone, 22.0, colors.positive);
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						ui.spacing_mut().item_spacing.x = 8.0;
						let reconnect = ui
							.add_enabled(
								unavailable.is_none(),
								egui::Button::new(
									design::medium(
										ui,
										crate::i18n::text(self.language, "Reconnect to call"),
										13.0,
									)
									.color(egui::Color32::WHITE),
								)
								.fill(colors.positive)
								.min_size(egui::vec2(148.0, 36.0)),
							)
							.on_disabled_hover_text(unavailable.unwrap_or(""));
						if reconnect.clicked()
							&& self
								.request_call_audio(state, channel, false, None, commands)
								.is_ok()
						{
							self.reconnect_offer = None;
						}
						if ui
							.add(
								egui::Button::new(crate::i18n::text(self.language, "Dismiss"))
									.min_size(egui::vec2(84.0, 36.0)),
							)
							.clicked()
						{
							self.reconnect_offer = None;
							self.reconnect_dismissed = true;
						}
						ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
							ui.vertical(|ui| {
								ui.spacing_mut().item_spacing.y = 2.0;
								ui.add(
									egui::Label::new(
										design::semibold(ui, &name, 15.0).color(colors.text_strong),
									)
									.truncate(),
								);
								ui.label(
									RichText::new(crate::i18n::text(
										self.language,
										"You were in this call recently",
									))
									.size(13.0)
									.color(colors.muted),
								);
							});
						});
					});
				});
			});
	}

	/// DM call stage above the conversation, plus the incoming-call banner.
	pub(super) fn call_bar(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		let selected = state.selected;
		if let Some(channel) = state
			.voice
			.active
			.as_ref()
			.filter(|call| call.guild.is_none() && Some(call.channel) == selected)
			.map(|call| call.channel)
		{
			let height = if self.stage_shows_video(state, channel) || state.is_group_dm(channel) {
				(ui.available_height() * 0.74).clamp(320.0, 900.0)
			} else {
				(ui.available_height() * 0.42).clamp(240.0, 340.0)
			};
			egui::Panel::top("dm-call")
				.exact_size(height)
				.show_separator_line(false)
				.frame(egui::Frame::new().fill(STAGE_FILL))
				.show(ui, |ui| {
					let rect = ui.max_rect();
					let notices = self.stage_notices(state, channel, true);
					let mut notice_ui = ui.new_child(
						egui::UiBuilder::new()
							.max_rect(rect.shrink(STAGE_MARGIN))
							.layout(egui::Layout::top_down(egui::Align::Min)),
					);
					stage_notices(&mut notice_ui, &notices);
					call_failure(
						&mut notice_ui,
						state.voice.active.as_ref().and_then(|c| c.error),
						STAGE_TEXT,
					);
					let body = egui::Rect::from_min_max(
						egui::pos2(rect.left() + STAGE_MARGIN, notice_ui.cursor().top() + 8.0),
						egui::pos2(
							rect.right() - STAGE_MARGIN,
							rect.bottom() - CONTROL_HEIGHT - 2.0 * STAGE_MARGIN,
						),
					);
					let mut body_ui = ui.new_child(
						egui::UiBuilder::new()
							.max_rect(body)
							.layout(egui::Layout::top_down(egui::Align::Min)),
					);
					self.participant_tiles(
						&mut body_ui,
						state,
						channel,
						&stage_participants(state, channel),
						true,
					);
					let bar = egui::Rect::from_min_max(
						egui::pos2(rect.left(), rect.bottom() - CONTROL_HEIGHT - STAGE_MARGIN),
						egui::pos2(rect.right(), rect.bottom() - STAGE_MARGIN),
					);
					let mut bar_ui = ui.new_child(
						egui::UiBuilder::new()
							.max_rect(bar)
							.layout(egui::Layout::left_to_right(egui::Align::Center)),
					);
					self.call_controls(&mut bar_ui, state, channel, commands);
				});
			self.apply_watch_request(state);
		}
		let existing = selected.filter(|channel| {
			state.voice.has_dm_call(*channel)
				&& state.can_view(*channel)
				&& state
					.voice
					.active
					.as_ref()
					.is_none_or(|call| call.channel != *channel)
		});
		if let Some(channel) = state.voice.incoming.or(existing) {
			let incoming = state.voice.incoming == Some(channel);
			let conversation = state.channel(channel).cloned();
			let name = state
				.channels
				.iter()
				.find(|c| c.id == channel)
				.map_or("Direct message", |c| c.name.as_str())
				.to_owned();
			let unavailable = self.call_unavailable(state, channel);
			egui::Panel::top("dm-incoming")
				.show_separator_line(false)
				.frame(
					egui::Frame::new()
						.fill(colors.raised)
						.inner_margin(egui::Margin::symmetric(16, 10)),
				)
				.show(ui, |ui| {
					ui.horizontal(|ui| {
						ui.spacing_mut().item_spacing.x = 12.0;
						match conversation.as_ref() {
							Some(group) if group.kind == 3 => {
								self.avatars.show_group(ui, group, 40.0, state.demo);
							}
							Some(dm) if dm.kind == 1 && !dm.recipients.is_empty() => {
								let user = &dm.recipients[0];
								self.avatars.show(ui, user, 40.0, state.demo);
							}
							_ => {
								design::avatar(ui, &name, 40.0);
							}
						}
						ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
							ui.spacing_mut().item_spacing.x = 8.0;
							if incoming {
								let decline = round_action(
									ui,
									crate::icons::Icon::HangUp,
									colors.danger,
									!state.demo,
									"Decline",
								);
								if decline.clicked()
									&& let Some(command) = state.decline_call()
								{
									commands.push(command);
								}
							}
							let answer = if incoming {
								round_action(
									ui,
									crate::icons::Icon::Phone,
									colors.positive,
									unavailable.is_none(),
									"Answer",
								)
							} else {
								ui.add_enabled(
									unavailable.is_none(),
									egui::Button::new(
										design::medium(ui, "Join call", 13.0)
											.color(egui::Color32::WHITE),
									)
									.fill(colors.positive)
									.min_size(egui::vec2(84.0, 36.0)),
								)
							}
							.on_disabled_hover_text(unavailable.unwrap_or(""));
							if answer.clicked() {
								self.request_call(state, channel, false, commands);
							}
							ui.with_layout(
								egui::Layout::left_to_right(egui::Align::Center),
								|ui| {
									ui.vertical(|ui| {
										ui.spacing_mut().item_spacing.y = 2.0;
										ui.add(
											egui::Label::new(
												design::semibold(ui, &name, 15.0)
													.color(colors.text_strong),
											)
											.truncate(),
										);
										ui.label(
											RichText::new(if incoming {
												unavailable.unwrap_or("Incoming call…")
											} else if !state.gateway_connected {
												"Reconnect to refresh call"
											} else {
												"Call in progress"
											})
											.size(13.0)
											.color(colors.muted),
										);
									});
								},
							);
						});
					});
				});
		}
	}

	/// Frames the header may stay green after leaving the connected phases
	/// before it admits the disconnect with yellow. Device reopens flap the
	/// phase for a frame or two around member joins and leaves.
	const CONNECTED_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

	/// Call details drawn inside the account card while connected: Discord's "Voice
	/// Connected" header, then a row of quick actions above the identity row.
	pub(super) fn voice_card_section(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let Some(call) = state.voice.active.as_ref() else {
			return;
		};
		let colors = design::palette(ui);
		let phase = call.phase;
		let channel_id = call.channel;
		let camera = call.camera;
		let connected = matches!(phase, Phase::Connected | Phase::Waiting);
		// Hold the last green through sub-second flaps: device reopens flip
		// the phase for a frame or two, and the header must not blink. Only
		// a sustained disconnect turns it yellow. Failed stays red at once.
		let unconnected_id = ui.scope_id().with("call-panel-unconnected-since");
		if connected || phase == Phase::Failed {
			ui.ctx()
				.data_mut(|data| data.remove::<std::time::Instant>(unconnected_id));
		}
		let unconnected_for = if connected || phase == Phase::Failed {
			None
		} else {
			let since = ui.ctx().data_mut(|data| {
				if let Some(since) = data.get_temp::<std::time::Instant>(unconnected_id) {
					since
				} else {
					let now = std::time::Instant::now();
					data.insert_temp(unconnected_id, now);
					now
				}
			});
			Some(since.elapsed())
		};
		let settled = unconnected_for.is_none_or(|ago| ago > Self::CONNECTED_GRACE);
		let error = call.error;
		let channel = state
			.channel(call.channel)
			.map_or("Direct message", |c| c.name.as_str())
			.to_owned();
		let guild = call
			.guild
			.and_then(|id| state.guild(id))
			.map(|g| g.name.clone());
		let detail = match guild {
			Some(guild) => format!("{channel} / {guild}"),
			None => channel,
		};
		let title = if state.demo && phase != Phase::Failed {
			"Voice preview"
		} else if phase == Phase::Failed {
			"Call failed"
		} else if connected || !settled {
			"Voice Connected"
		} else {
			"Connecting…"
		};
		let color = if phase == Phase::Failed {
			colors.danger
		} else if connected || state.demo || !settled {
			colors.positive
		} else {
			colors.warning
		};
		egui::Frame::new()
			.inner_margin(egui::Margin {
				left: 8,
				right: 8,
				top: 8,
				bottom: 8,
			})
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.spacing_mut().item_spacing.y = 8.0;
				if self.voice_microphone_unavailable {
					ui.label(
						RichText::new(crate::i18n::text(
							self.language,
							"Microphone unavailable · still connected. Choose another input in Audio settings.",
						))
						.size(12.0)
						.color(colors.warning),
					);
				}
				let header = ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 10.0;
					// Square status tile like Discord's, tinted with the connection colour.
					let (tile, _) =
						ui.allocate_exact_size(egui::Vec2::splat(40.0), egui::Sense::hover());
					ui.painter().rect_filled(tile, 8, colors.base);
					crate::icons::paint(
						ui.painter(),
						crate::icons::Icon::InCall,
						tile.shrink(10.0),
						color,
					);
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						ui.spacing_mut().item_spacing.x = 2.0;
						let leave = ui
							.add_enabled_ui(!state.demo, |ui| {
								crate::icons::button(
									ui,
									crate::icons::Icon::HangUp,
									32.0,
									if phase == Phase::Failed {
										"Dismiss call"
									} else {
										"Disconnect"
									},
								)
							})
							.inner;
						if leave.clicked()
							&& let Some(command) = state.leave_call()
						{
							commands.push(command);
						}
						ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
							ui.vertical(|ui| {
								ui.spacing_mut().item_spacing.y = 0.0;
								ui.add(
									egui::Label::new(
										design::semibold(ui, title, 15.0).color(color),
									)
									.truncate()
									.selectable(false),
								);
								ui.add(
									egui::Label::new(
										RichText::new(detail).size(12.0).color(colors.muted),
									)
									.truncate()
									.selectable(false),
								);
							});
						});
					});
				});
				if connected && !state.demo {
					let ping = self.voice_ping_ms;
					let place = self.voice_server_place.clone();
					let language = self.language;
					header.response.on_hover_ui(|ui| {
						voice_connection_tip(ui, ping, &place, language);
					});
				}
				call_failure(ui, error, colors.text_strong);
				let controls = self.controls_enabled(state);
				let can_camera = self.voice_camera_available
					&& state.can_camera(channel_id)
					&& matches!(phase, Phase::Connected | Phase::Waiting);
				let can_share = self.screen.busy
					|| state.demo || (self.screen.supported
					&& matches!(phase, Phase::Connected | Phase::Waiting)
					&& state.can_stream(channel_id));
				let processing = !state.demo && self.voice_available;
				let mut camera_clicked = false;
				let mut share_clicked = false;
				let language = self.language;
				let t = |english: &'static str| crate::i18n::text(language, english);
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 8.0;
					let width = ((ui.available_width() - 3.0 * 8.0 - 24.0) / 3.0).max(24.0);
					camera_clicked = card_action(
						ui,
						width,
						if camera {
							crate::icons::Icon::Video
						} else {
							crate::icons::Icon::VideoSlash
						},
						controls && (camera || can_camera),
						camera,
						if camera {
							t("Turn off camera")
						} else {
							t("Turn on camera")
						},
						if camera {
							t("Stop sharing your camera")
						} else if state.demo {
							"Camera is off in the offline preview"
						} else if !self.voice_camera_available {
							"Camera requires H264 support from the voice server"
						} else if !state.can_camera(channel_id) {
							"Camera is unavailable with current channel permissions"
						} else {
							t("Share your selected camera with this call")
						},
					)
					.clicked();
					let camera_settings = crate::icons::button(
						ui,
						crate::icons::Icon::ChevronDown,
						24.0,
						"Camera settings",
					);
					self.camera_settings_popup(&camera_settings, state.demo);
					share_clicked = card_action(
						ui,
						width,
						crate::icons::Icon::ScreenShare,
						can_share,
						self.screen.busy,
						if self.screen.busy {
							t("Stop sharing")
						} else {
							t("Share your screen")
						},
						if can_share {
							if let Some(status) = self.screen.capture_status {
								status
							} else if self.screen.busy {
								t("Stop sharing your screen")
							} else {
								t("Share a screen or window")
							}
						} else {
							"Screen sharing requires a connected call and video permission on a supported desktop."
						},
					)
					.clicked();
					let level = self.voice_processing.effective().suppression;
					let hint = if processing {
						format!(
							"{}: {}\n{}",
							t("Noise suppression"),
							t(noise_level_title(level)),
							t("Click to turn on or off · right-click to choose the level"),
						)
					} else {
						t("Noise suppression is unavailable in this build or preview.").to_owned()
					};
					let noise = noise_action(
						ui,
						width,
						processing,
						level,
						if level == NoiseSuppression::Off {
							t("Turn on noise suppression")
						} else {
							t("Turn off noise suppression")
						},
						hint,
					);
					if noise.clicked() {
						self.toggle_noise();
					}
					if processing {
						noise.context_menu(|ui| {
							ui.set_width(260.0);
							ui.label(design::semibold(ui, t("Noise suppression"), 14.0));
							self.noise_level_picker(ui, true);
							ui.separator();
							if ui.button(t("All voice settings")).clicked() {
								self.open_voice_settings();
								ui.close();
							}
						});
					}
				});
				if camera_clicked && let Some(command) = state.set_call_camera(!camera) {
					self.voice_camera_status = "";
					commands.push(command);
				}
				if share_clicked {
					self.screen.launch(state);
				}
			});
		if connected && ui.is_rect_visible(ui.max_rect()) {
			ui.ctx()
				.request_repaint_after(std::time::Duration::from_secs(1));
		}
	}

	/// Mic or headphones toggle followed by Discord's small chevron opening voice settings.
	pub(super) fn mute_toggle_with_settings(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
		deafen: bool,
	) {
		let chevron = crate::icons::button(
			ui,
			crate::icons::Icon::ChevronDown,
			20.0,
			if deafen {
				"Output settings"
			} else {
				"Input settings"
			},
		);
		self.voice_settings_popup(&chevron, state.demo, state.voice.active.is_some(), !deafen);
		self.mute_toggle(ui, state, commands, deafen, 32.0);
	}
}

/// Quick action in the in-call account card: filled cell, accent glyph while the feature is on.
fn card_action(
	ui: &mut egui::Ui,
	width: f32,
	icon: crate::icons::Icon,
	enabled: bool,
	active: bool,
	label: &str,
	hint: &str,
) -> egui::Response {
	let colors = design::palette(ui);
	let (rect, response) = ui.allocate_exact_size(
		egui::vec2(width, 40.0),
		if enabled {
			egui::Sense::click()
		} else {
			egui::Sense::hover()
		},
	);
	let fill = if enabled && (response.hovered() || response.has_focus()) {
		colors.selected
	} else {
		colors.hover
	};
	ui.painter().rect_filled(rect, 8, fill);
	let color = if !enabled {
		colors.muted.gamma_multiply(0.5)
	} else if active {
		colors.accent
	} else {
		colors.text_strong
	};
	crate::icons::paint(
		ui.painter(),
		icon,
		egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(20.0)),
		color,
	);
	response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, active, label));
	response.on_hover_text(hint)
}

fn noise_action(
	ui: &mut egui::Ui,
	width: f32,
	enabled: bool,
	level: NoiseSuppression,
	label: &str,
	hint: String,
) -> egui::Response {
	let colors = design::palette(ui);
	let active = level != NoiseSuppression::Off;
	let (rect, response) = ui.allocate_exact_size(
		egui::vec2(width, 40.0),
		if enabled {
			egui::Sense::click()
		} else {
			egui::Sense::hover()
		},
	);
	let fill = if enabled && (response.hovered() || response.has_focus()) {
		colors.selected
	} else {
		colors.hover
	};
	ui.painter().rect_filled(rect, 8, fill);
	let color = if !enabled {
		colors.muted.gamma_multiply(0.5)
	} else if active {
		colors.accent
	} else {
		colors.text_strong
	};
	paint_noise_suppression(
		ui.painter(),
		egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(22.0)),
		color,
		fill,
		level.level(),
	);
	response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, active, label));
	response.on_hover_text(hint)
}

fn noise_level_title(level: NoiseSuppression) -> &'static str {
	match level {
		NoiseSuppression::Off => "Off",
		NoiseSuppression::WebRtc => "Light",
		NoiseSuppression::RnNoise => "Standard",
		NoiseSuppression::DeepFilter => "Maximum",
	}
}

fn noise_level_detail(level: NoiseSuppression) -> &'static str {
	match level {
		NoiseSuppression::Off => "No filter. For studio microphones or when playing music.",
		NoiseSuppression::WebRtc => {
			"Steady hum like fans or air conditioning. Lightest on your PC."
		}
		NoiseSuppression::RnNoise => {
			"Keyboard, clicks and everyday home noise. Works well for most people."
		}
		NoiseSuppression::DeepFilter => {
			"Very noisy home, or friends complain about your background noise. Uses more of your PC."
		}
	}
}

fn noise_level_usage(level: NoiseSuppression) -> &'static str {
	match level {
		NoiseSuppression::Off => "PC usage: none",
		NoiseSuppression::WebRtc => "PC usage: very low",
		NoiseSuppression::RnNoise => "PC usage: low",
		NoiseSuppression::DeepFilter => "PC usage: medium",
	}
}

/// One suppression level: glyph, plain name, engine tag, optional explanation, PC-usage
/// meter and a radio marker. The whole row is the click target.
fn noise_level_row(
	ui: &mut egui::Ui,
	level: NoiseSuppression,
	selected: bool,
	compact: bool,
	language: model::Language,
) -> egui::Response {
	let p = design::palette(ui);
	let t = |english: &'static str| crate::i18n::text(language, english);
	let enabled = ui.is_enabled();
	let width = ui.available_width();
	let glyph = if compact { 20.0 } else { 26.0 };
	let text_left = 12.0 + glyph + 12.0;
	let meter_width = 22.0;
	let text_width = (width - text_left - meter_width - 48.0).max(80.0);
	let title_color = if enabled {
		p.text_strong
	} else {
		p.text_strong.gamma_multiply(0.5)
	};
	let title = ui.painter().layout_no_wrap(
		t(noise_level_title(level)).to_owned(),
		egui::FontId::new(
			if compact { 14.0 } else { 15.0 },
			design::semibold_family(ui.ctx()),
		),
		title_color,
	);
	let engine = match level {
		NoiseSuppression::Off => None,
		NoiseSuppression::WebRtc => Some("WebRTC"),
		NoiseSuppression::RnNoise => Some("RNNoise"),
		NoiseSuppression::DeepFilter => Some("DeepFilterNet"),
	}
	.map(|name| {
		ui.painter()
			.layout_no_wrap(name.to_owned(), egui::FontId::proportional(11.0), p.muted)
	});
	let badge = (level == NoiseSuppression::default()).then(|| {
		ui.painter().layout_no_wrap(
			t("Recommended").to_owned(),
			egui::FontId::new(11.0, design::medium_family(ui.ctx())),
			p.accent,
		)
	});
	let detail = (!compact).then(|| {
		ui.painter().layout(
			t(noise_level_detail(level)).to_owned(),
			egui::FontId::proportional(12.5),
			p.muted,
			text_width,
		)
	});
	// The meter starts 62 px before the right edge. When title, engine and
	// badge do not fit ahead of it, the badge drops to its own line instead
	// of painting over the meter and the radio. Only compact rows qualify:
	// wider rows always have their detail line below the title.
	let pill_width = badge.as_ref().map_or(0.0, |pill| pill.size().x + 12.0);
	let inline_end = text_left
		+ title.size().x
		+ 8.0 + engine.as_ref().map_or(0.0, |engine| engine.size().x + 8.0)
		+ pill_width;
	let badge_below = compact && badge.is_some() && inline_end > width - 62.0 - 4.0;
	let badge_height = badge.as_ref().map_or(0.0, |pill| pill.size().y + 4.0);
	let text_height = title.size().y
		+ detail.as_ref().map_or(0.0, |d| d.size().y + 3.0)
		+ if badge_below { 3.0 + badge_height } else { 0.0 };
	let padding = if compact { 7.0 } else { 10.0 };
	let (rect, response) = ui.allocate_exact_size(
		egui::vec2(width, text_height.max(glyph) + padding * 2.0),
		egui::Sense::click(),
	);
	let usage = t(noise_level_usage(level));
	response.widget_info(|| {
		egui::WidgetInfo::selected(
			egui::Role::RadioButton,
			enabled,
			selected,
			format!("{}, {usage}", t(noise_level_title(level))),
		)
	});
	if !ui.is_rect_visible(rect) {
		return response.on_hover_text(usage);
	}
	let hot = enabled && (response.hovered() || response.has_focus());
	let painter = ui.painter();
	let fill = if selected {
		design::mix(p.selected, p.accent, 0.12)
	} else if hot {
		p.hover
	} else {
		egui::Color32::TRANSPARENT
	};
	painter.rect_filled(rect, 8, fill);
	if selected {
		painter.rect_stroke(
			rect,
			8,
			egui::Stroke::new(1.0, p.accent.gamma_multiply(0.6)),
			egui::StrokeKind::Inside,
		);
	} else if response.has_focus() {
		painter.rect_stroke(
			rect.shrink(1.0),
			8,
			egui::Stroke::new(1.0, p.accent),
			egui::StrokeKind::Inside,
		);
	}
	let glyph_color = match (enabled, selected) {
		(false, _) => p.muted.gamma_multiply(0.5),
		(true, true) => p.accent,
		(true, false) => p.text,
	};
	let glyph_rect = egui::Rect::from_center_size(
		egui::pos2(rect.left() + 12.0 + glyph * 0.5, rect.center().y),
		egui::Vec2::splat(glyph),
	);
	let glyph_bg = if fill == egui::Color32::TRANSPARENT {
		p.raised
	} else {
		fill
	};
	paint_noise_suppression(painter, glyph_rect, glyph_color, glyph_bg, level.level());

	let mut x = rect.left() + text_left;
	let top = rect.top() + padding;
	let title_width = title.size().x;
	let title_height = title.size().y;
	painter.galley(egui::pos2(x, top), title, title_color);
	x += title_width + 8.0;
	let mid = top + title_height * 0.5;
	if let Some(engine) = engine {
		let size = engine.size();
		painter.galley(egui::pos2(x, mid - size.y * 0.5), engine, p.muted);
		x += size.x + 8.0;
	}
	if let Some(badge) = badge {
		let size = badge.size();
		let pill = if badge_below {
			egui::Rect::from_min_size(
				egui::pos2(rect.left() + text_left, top + title_height + 3.0),
				size + egui::vec2(12.0, 4.0),
			)
		} else {
			egui::Rect::from_min_size(
				egui::pos2(x, mid - size.y * 0.5 - 2.0),
				size + egui::vec2(12.0, 4.0),
			)
		};
		painter.rect_filled(pill, 9, p.accent.gamma_multiply(0.16));
		painter.galley(pill.min + egui::vec2(6.0, 2.0), badge, p.accent);
	}
	if let Some(detail) = detail {
		painter.galley(
			egui::pos2(rect.left() + text_left, top + title_height + 3.0),
			detail,
			p.muted,
		);
	}

	let marker = egui::pos2(rect.right() - 20.0, rect.center().y);
	let ring = if selected {
		p.accent
	} else if hot {
		p.text
	} else {
		p.muted
	};
	let ring = ring.gamma_multiply(if enabled { 1.0 } else { 0.4 });
	painter.circle_stroke(marker, 8.0, egui::Stroke::new(2.0, ring));
	if selected {
		painter.circle_filled(marker, 4.0, ring);
	}
	// PC-usage meter: three rising bars, filled up to the level's cost.
	let meter = egui::Rect::from_min_size(
		egui::pos2(marker.x - 20.0 - meter_width, rect.center().y - 6.0),
		egui::vec2(meter_width, 12.0),
	);
	let filled = level.level();
	for index in 0..3u8 {
		let bar_w = 5.0;
		let height = 4.0 + f32::from(index) * 4.0;
		let bar = egui::Rect::from_min_size(
			egui::pos2(
				meter.left() + f32::from(index) * (bar_w + 3.0),
				meter.bottom() - height,
			),
			egui::vec2(bar_w, height),
		);
		let on = index < filled;
		let color = match (on, filled) {
			(false, _) => p.border,
			(true, 3) => p.warning,
			(true, _) => p.positive,
		};
		painter.rect_filled(
			bar,
			1.5,
			color.gamma_multiply(if enabled { 1.0 } else { 0.4 }),
		);
	}
	response.on_hover_text(usage)
}

/// Five rounded voice bars over three level pips; off adds a slash cut out of `background`.
/// Bars are snapped to physical pixels so the small button glyph stays crisp.
fn paint_noise_suppression(
	painter: &egui::Painter,
	rect: egui::Rect,
	color: egui::Color32,
	background: egui::Color32,
	level: u8,
) {
	let ppp = painter.pixels_per_point();
	let snap = |value: f32| (value * ppp).round() / ppp;
	let size = rect.width();
	let bar_w = snap((size * 0.12).max(1.5));
	let gap = snap((size * 0.075).max(1.0));
	let wave_width = bar_w * 5.0 + gap * 4.0;
	let wave_left = snap(rect.center().x - wave_width * 0.5);
	let pip_h = snap((size * 0.1).max(1.5));
	let wave_top = rect.top() + size * 0.04;
	let wave_bottom = rect.bottom() - pip_h - size * 0.14;
	let wave_mid = (wave_top + wave_bottom) * 0.5;
	let wave_h = wave_bottom - wave_top;
	for (index, height) in [0.38, 0.7, 1.0, 0.7, 0.38].into_iter().enumerate() {
		let bar_h = snap((wave_h * height).max(bar_w));
		let x = wave_left + index as f32 * (bar_w + gap);
		let y = snap(wave_mid - bar_h * 0.5);
		painter.rect_filled(
			egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(bar_w, bar_h)),
			bar_w * 0.5,
			color,
		);
	}
	let pip_gap = gap;
	let pip_w = snap((wave_width - pip_gap * 2.0) / 3.0);
	let pip_y = snap(rect.bottom() - pip_h - size * 0.02);
	for index in 0..3u8 {
		let x = wave_left + f32::from(index) * (pip_w + pip_gap);
		let on = index < level;
		painter.rect_filled(
			egui::Rect::from_min_size(egui::pos2(x, pip_y), egui::vec2(pip_w, pip_h)),
			pip_h * 0.5,
			if on { color } else { color.gamma_multiply(0.3) },
		);
	}
	if level == 0 {
		let start = egui::pos2(rect.left() + size * 0.12, rect.bottom() - size * 0.1);
		let end = egui::pos2(rect.right() - size * 0.12, rect.top() + size * 0.06);
		let thickness = (size * 0.1).max(1.5);
		painter.line_segment([start, end], egui::Stroke::new(thickness * 2.2, background));
		painter.line_segment([start, end], egui::Stroke::new(thickness, color));
		painter.circle_filled(start, thickness * 0.5, color);
		painter.circle_filled(end, thickness * 0.5, color);
	}
}

fn voice_connection_tip(
	ui: &mut egui::Ui,
	ping_ms: Option<u32>,
	place: &str,
	language: model::Language,
) {
	ui.spacing_mut().item_spacing.y = 2.0;
	let ping = ping_ms
		.map(|ms| format!("{ms} ms"))
		.unwrap_or_else(|| "…".to_owned());
	ui.label(design::semibold(ui, ping, 13.0));
	if !place.is_empty() {
		ui.label(localized_place(language, place));
	}
}

fn localized_place(language: model::Language, place: &str) -> &'static str {
	let key = match place {
		"Brazil" => "Brazil",
		"United States" => "United States",
		"Canada" => "Canada",
		"United Kingdom" => "United Kingdom",
		"Germany" => "Germany",
		"Netherlands" => "Netherlands",
		"France" => "France",
		"Spain" => "Spain",
		"Poland" => "Poland",
		"Finland" => "Finland",
		"Sweden" => "Sweden",
		"Singapore" => "Singapore",
		"Japan" => "Japan",
		"Hong Kong" => "Hong Kong",
		"Australia" => "Australia",
		"India" => "India",
		"South Africa" => "South Africa",
		"Chile" => "Chile",
		"Argentina" => "Argentina",
		"South Korea" => "South Korea",
		"Europe" => "Europe",
		"Russia" => "Russia",
		_ => return "",
	};
	crate::i18n::text(language, key)
}

/// Country or region named by a Discord voice hostname. Unknown hosts stay blank.
pub fn voice_server_place(endpoint: &str) -> Option<&'static str> {
	let host = endpoint
		.trim()
		.trim_start_matches("https://")
		.trim_start_matches("http://")
		.split(['/', ':'])
		.next()
		.unwrap_or("")
		.to_ascii_lowercase();
	let label = host.split('.').next().unwrap_or("");
	if label.is_empty() {
		return None;
	}
	if let Some(code) = label.strip_prefix("c-") {
		let airport: String = code
			.chars()
			.take_while(|character| character.is_ascii_alphabetic())
			.take(3)
			.collect();
		if let Some(place) = airport_place(&airport) {
			return Some(place);
		}
	}
	const PREFIXES: &[(&str, &str)] = &[
		("brazil", "Brazil"),
		("us-east", "United States"),
		("us-west", "United States"),
		("us-central", "United States"),
		("us-south", "United States"),
		("rotterdam", "Netherlands"),
		("amsterdam", "Netherlands"),
		("frankfurt", "Germany"),
		("singapore", "Singapore"),
		("japan", "Japan"),
		("hongkong", "Hong Kong"),
		("hong-kong", "Hong Kong"),
		("sydney", "Australia"),
		("india", "India"),
		("southafrica", "South Africa"),
		("south-africa", "South Africa"),
		("southkorea", "South Korea"),
		("south-korea", "South Korea"),
		("europe", "Europe"),
		("russia", "Russia"),
		("london", "United Kingdom"),
		("madrid", "Spain"),
		("warsaw", "Poland"),
		("finland", "Finland"),
		("sweden", "Sweden"),
		("canada", "Canada"),
		("chile", "Chile"),
		("argentina", "Argentina"),
		("france", "France"),
	];
	PREFIXES
		.iter()
		.find(|(prefix, _)| label.starts_with(prefix))
		.map(|(_, place)| *place)
}

fn airport_place(code: &str) -> Option<&'static str> {
	Some(match code {
		"gru" | "gig" | "bsb" | "cnf" | "ssa" | "rec" | "poa" | "for" | "bel" | "mao" => "Brazil",
		"iad" | "ewr" | "atl" | "ord" | "dfw" | "sjc" | "lax" | "sea" | "mia" | "den" | "phx"
		| "bos" => "United States",
		"yul" | "yyz" | "yvr" => "Canada",
		"lhr" | "man" => "United Kingdom",
		"fra" | "muc" | "ber" => "Germany",
		"ams" => "Netherlands",
		"cdg" => "France",
		"mad" => "Spain",
		"waw" => "Poland",
		"hel" => "Finland",
		"arn" => "Sweden",
		"sin" => "Singapore",
		"nrt" | "hnd" | "kix" => "Japan",
		"hkg" => "Hong Kong",
		"syd" | "mel" => "Australia",
		"bom" | "del" => "India",
		"jnb" => "South Africa",
		"scl" => "Chile",
		"eze" => "Argentina",
		"icn" => "South Korea",
		_ => return None,
	})
}

/// Discord's call stage is black in every appearance; pills and tiles sit on it in fixed greys.
const STAGE_FILL: egui::Color32 = egui::Color32::BLACK;
const TILE_FILL: egui::Color32 = egui::Color32::from_rgb(0x2b, 0x2d, 0x31);
const PILL_FILL: egui::Color32 = egui::Color32::from_rgb(0x1e, 0x1f, 0x22);
const STAGE_TEXT: egui::Color32 = egui::Color32::from_rgb(0xdb, 0xde, 0xe1);
const STAGE_MUTED: egui::Color32 = egui::Color32::from_rgb(0x9a, 0x9b, 0xa1);
const STAGE_MARGIN: f32 = 16.0;
const TILE_GAP: f32 = 8.0;
/// Height shared by every control, pill and the hang-up button in the call bar.
const CONTROL_HEIGHT: f32 = 48.0;
/// Mic, settings chevron, deafen, camera and screen share sit in one pill.
const MEDIA_PILL: f32 = 248.0;
const HANG_UP: f32 = 64.0;
const BAR_GAP: f32 = 12.0;

/// Green ring like Discord's speaking indicator; the accessible label still names the state.
/// Which stage tile is enlarged; cleared automatically when it stops showing video.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageFocus {
	LocalScreen,
	Stream(Id),
	Participant(Id),
}

enum Tile<'a> {
	LocalScreen,
	Stream(Id),
	Participant(&'a RosterEntry),
}
impl Tile<'_> {
	fn focus(&self) -> StageFocus {
		match self {
			Self::LocalScreen => StageFocus::LocalScreen,
			Self::Stream(streamer) => StageFocus::Stream(*streamer),
			Self::Participant(entry) => StageFocus::Participant(entry.participant.user),
		}
	}
	/// Stable interaction identity, so a tile keeps its hover while the layout changes.
	fn key(&self) -> (u8, u64) {
		match self.focus() {
			StageFocus::LocalScreen => (0, 0),
			StageFocus::Stream(user) => (1, user.0),
			StageFocus::Participant(user) => (2, user.0),
		}
	}
}

/// Column count and tile size that make the tiles largest inside `area`, at 16:9.
fn best_fit(count: usize, area: egui::Vec2, cap: f32) -> (usize, egui::Vec2) {
	let mut best = (1, egui::Vec2::ZERO);
	for columns in 1..=count.max(1) {
		let rows = count.div_ceil(columns);
		let width = (area.x - TILE_GAP * (columns - 1) as f32) / columns as f32;
		let height = (area.y - TILE_GAP * (rows - 1) as f32) / rows as f32;
		if width <= 0.0 || height <= 0.0 {
			continue;
		}
		let width = width.min(height * 16.0 / 9.0).min(cap);
		// Prefer side-by-side tiles when the size cap makes multiple layouts tie.
		if width >= best.1.x {
			best = (columns, egui::vec2(width, width * 9.0 / 16.0));
		}
	}
	if best.1.x <= 0.0 {
		best = (count.max(1), egui::Vec2::ZERO);
	}
	best
}

/// The largest centred rectangle of the image's aspect ratio inside `rect`.
fn fit_rect(rect: egui::Rect, image: egui::Vec2) -> egui::Rect {
	if image.x <= 0.0 || image.y <= 0.0 {
		return rect;
	}
	let size = image * (rect.width() / image.x).min(rect.height() / image.y);
	egui::Rect::from_center_size(rect.center(), size)
}

/// Fill the tile edge to edge, cropping the overflow (Discord's camera framing).
fn cover_image(
	ui: &mut egui::Ui,
	rect: egui::Rect,
	id: egui::TextureId,
	image: egui::Vec2,
	mirror: bool,
) {
	if image.x <= 0.0 || image.y <= 0.0 {
		return;
	}
	let scale = (rect.width() / image.x).max(rect.height() / image.y);
	let shown = egui::vec2(
		(rect.width() / (image.x * scale)).min(1.0),
		(rect.height() / (image.y * scale)).min(1.0),
	);
	let mut uv = egui::Rect::from_center_size(egui::pos2(0.5, 0.5), shown);
	if mirror {
		std::mem::swap(&mut uv.min.x, &mut uv.max.x);
	}
	ui.put(
		rect,
		egui::Image::from_texture((id, rect.size()))
			.uv(uv)
			.corner_radius(8),
	);
}

/// Bottom-left translucent name plate, optionally with the mute glyph.
fn name_badge(ui: &mut egui::Ui, rect: egui::Rect, name: &str, icon: Option<crate::icons::Icon>) {
	let font = egui::FontId::new(13.0, design::medium_family(ui.ctx()));
	let icon_width = if icon.is_some() { 20.0 } else { 0.0 };
	// One line, truncated with an ellipsis; the tile hover text carries the full name.
	let mut job = egui::text::LayoutJob::simple_singleline(name.to_owned(), font, STAGE_TEXT);
	job.wrap.max_width = (rect.width() * 0.6 - icon_width).max(20.0);
	job.wrap.max_rows = 1;
	job.wrap.break_anywhere = true;
	let galley = ui.painter().layout_job(job);
	let badge = egui::Rect::from_min_size(
		rect.left_bottom() + egui::vec2(8.0, -8.0 - 24.0),
		egui::vec2(galley.size().x + 16.0 + icon_width, 24.0),
	);
	ui.painter()
		.rect_filled(badge, 6, egui::Color32::from_black_alpha(160));
	ui.painter().galley(
		egui::pos2(badge.left() + 8.0, badge.center().y - galley.size().y * 0.5),
		galley,
		STAGE_TEXT,
	);
	if let Some(icon) = icon {
		crate::icons::paint(
			ui.painter(),
			icon,
			egui::Rect::from_center_size(
				egui::pos2(badge.right() - 14.0, badge.center().y),
				egui::Vec2::splat(14.0),
			),
			design::palette(ui).danger,
		);
	}
}

/// Bottom-right pill action on a tile, sized to its label with a hover fill.
fn tile_button(
	ui: &mut egui::Ui,
	rect: egui::Rect,
	label: &str,
	fill: egui::Color32,
	hover_fill: egui::Color32,
	hint: &str,
) -> egui::Response {
	let font = egui::FontId::new(12.0, design::medium_family(ui.ctx()));
	let galley = ui
		.painter()
		.layout_no_wrap(label.to_owned(), font, egui::Color32::WHITE);
	let size = egui::vec2(galley.size().x + 20.0, 26.0);
	let button = egui::Rect::from_min_size(
		egui::pos2(rect.right() - 8.0 - size.x, rect.top() + 8.0),
		size,
	);
	let response = ui.allocate_rect(button, egui::Sense::click());
	let fill = if response.hovered() || response.has_focus() {
		hover_fill
	} else {
		fill
	};
	ui.painter().rect_filled(button, 6, fill);
	ui.painter().galley(
		button.center() - galley.size() * 0.5,
		galley,
		egui::Color32::WHITE,
	);
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
	response.on_hover_text(hint)
}

/// ASCII list separator for voice hover/status text.
///
/// A middle dot (U+00B7) is the usual mark, but its UTF-8 bytes `C2 B7` display
/// as "Â·" when a Windows tooltip or AccessKit path treats the string as
/// Windows-1252. `|` cannot mojibake.
const VOICE_STATUS_SEP: &str = " | ";

fn voice_channel_hover_text(name: &str, marks: &str, connected: bool) -> String {
	let marks = marks.replace(" · ", VOICE_STATUS_SEP);
	let mut text = format!("{name}{VOICE_STATUS_SEP}View voice channel{marks}");
	if connected {
		text.push_str(VOICE_STATUS_SEP);
		text.push_str("Connected");
	}
	text
}

fn speaking_avatar(
	ui: &egui::Ui,
	avatar: &egui::Response,
	name: &str,
	user: Id,
	active: bool,
	level: u8,
) {
	let (_visibility, radius_offset, opacity) = crate::anim::speaking_ring(
		ui.ctx(),
		egui::Id::unique(("speaking-ring", user.0)),
		active,
		level,
	);
	if opacity <= 0.0 {
		return;
	}
	let colors = design::palette(ui);
	let base = avatar.rect.width() * 0.5;
	ui.painter().circle_stroke(
		avatar.rect.center(),
		base + radius_offset,
		egui::Stroke::new(2.0, colors.positive.gamma_multiply(opacity)),
	);
	if active {
		let label = format!("{name}{VOICE_STATUS_SEP}Speaking");
		avatar.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &label));
		avatar.clone().on_hover_text(label);
	}
}

fn call_failure(ui: &mut egui::Ui, error: Option<&str>, color: egui::Color32) {
	let Some(error) = error else { return };
	ui.horizontal_top(|ui| {
		ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
			if crate::icons::button(ui, crate::icons::Icon::Copy, 28.0, "Copy failure reason")
				.clicked()
			{
				ui.ctx()
					.copy_text(format!("Nivra call failed\nReason: {error}"));
			}
			ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
				ui.add(
					egui::Label::new(RichText::new(error).size(12.0).color(color))
						.wrap()
						.selectable(true),
				);
			});
		});
	});
}

fn stage_notices(ui: &mut egui::Ui, notices: &[(String, bool)]) {
	ui.spacing_mut().item_spacing.y = 2.0;
	for (text, strong) in notices {
		ui.add(
			egui::Label::new(
				RichText::new(text)
					.size(if *strong { 13.0 } else { 12.0 })
					.color(if *strong { STAGE_TEXT } else { STAGE_MUTED }),
			)
			.truncate()
			.selectable(false),
		);
	}
	if !notices.is_empty() {
		ui.add_space(6.0);
	}
	ui.spacing_mut().item_spacing.y = 8.0;
}

/// Rounded dark group holding several call controls. Every pill is exactly `CONTROL_HEIGHT`
/// tall and `width` wide so the groups and the hang-up button share one baseline.
fn pill<R>(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
	let (rect, _) = ui.allocate_exact_size(egui::vec2(width, CONTROL_HEIGHT), egui::Sense::hover());
	ui.painter().rect_filled(rect, 12, PILL_FILL);
	ui.painter().rect_stroke(
		rect,
		12,
		egui::Stroke::new(1.0, egui::Color32::from_white_alpha(18)),
		egui::StrokeKind::Inside,
	);
	let mut inner = ui.new_child(
		egui::UiBuilder::new()
			.max_rect(rect)
			.layout(egui::Layout::left_to_right(egui::Align::Center)),
	);
	inner.spacing_mut().item_spacing.x = 0.0;
	add(&mut inner)
}

/// One control inside a pill; disabled controls stay visible but inert, like Discord's.
fn control(
	ui: &mut egui::Ui,
	icon: crate::icons::Icon,
	width: f32,
	enabled: bool,
	color: egui::Color32,
	label: &str,
	hint: &str,
) -> egui::Response {
	let (rect, response) = ui.allocate_exact_size(
		egui::vec2(width, CONTROL_HEIGHT),
		if enabled {
			egui::Sense::click()
		} else {
			egui::Sense::hover()
		},
	);
	if enabled && (response.hovered() || response.has_focus()) {
		ui.painter()
			.rect_filled(rect.shrink(4.0), 8, egui::Color32::from_white_alpha(28));
	}
	let size = if width < 40.0 { 14.0 } else { 22.0 };
	crate::icons::paint(
		ui.painter(),
		icon,
		egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(size)),
		if enabled {
			color
		} else {
			STAGE_MUTED.gamma_multiply(0.45)
		},
	);
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
	response.on_hover_text(hint)
}

/// Circular filled action (answer/decline) used by the incoming-call banner.
fn round_action(
	ui: &mut egui::Ui,
	icon: crate::icons::Icon,
	fill: egui::Color32,
	enabled: bool,
	label: &str,
) -> egui::Response {
	let (rect, response) = ui.allocate_exact_size(
		egui::Vec2::splat(40.0),
		if enabled {
			egui::Sense::click()
		} else {
			egui::Sense::hover()
		},
	);
	let fill = if !enabled {
		fill.gamma_multiply(0.45)
	} else if response.hovered() || response.has_focus() {
		fill.gamma_multiply(0.85)
	} else {
		fill
	};
	ui.painter().circle_filled(rect.center(), 20.0, fill);
	crate::icons::paint(
		ui.painter(),
		icon,
		egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(20.0)),
		egui::Color32::WHITE,
	);
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
	response.on_hover_text(label)
}

/// Resolve the display name and user for a roster entry from the entry, member list or self.
/// Include the local call participant before its gateway roster update arrives.
fn stage_participants(state: &State, channel: Id) -> Vec<RosterEntry> {
	let call = state
		.voice
		.active
		.as_ref()
		.filter(|call| call.channel == channel);
	let mut entries: Vec<_> = if let Some(call) = call.filter(|call| call.guild.is_none()) {
		call.participants
			.iter()
			.map(|participant| RosterEntry {
				guild: Id(0),
				channel,
				participant: *participant,
				member: None,
			})
			.collect()
	} else {
		state
			.voice
			.roster
			.iter()
			.filter(|entry| entry.channel == channel)
			.cloned()
			.collect()
	};
	if let Some(call) = call.filter(|call| call.phase != Phase::Failed)
		&& let Some(user) = &state.user
	{
		if let Some(index) = entries
			.iter()
			.position(|entry| entry.participant.user == user.id)
		{
			entries.swap(0, index);
		} else {
			entries.insert(
				0,
				RosterEntry {
					guild: call.guild.unwrap_or(Id(0)),
					channel,
					participant: Participant {
						user: user.id,
						muted: call.muted,
						deafened: call.deafened,
						server_muted: call.server_muted,
						server_deafened: call.server_deafened,
						video: call.camera,
						streaming: false,
					},
					member: None,
				},
			);
		}
	}
	entries
}

fn resolve_member<'a>(
	state: &'a State,
	entry: &'a RosterEntry,
) -> (Option<&'a model::User>, &'a str) {
	let member = entry.member.as_ref().or_else(|| {
		state
			.members
			.as_ref()
			.filter(|list| list.guild == Some(entry.guild))
			.and_then(|list| {
				list.slots
					.iter()
					.flatten()
					.filter_map(|slot| match slot {
						model::MemberSlot::Person(m) => Some(m),
						_ => None,
					})
					.find(|m| m.user.id == entry.participant.user)
			})
	});
	let user = member
		.map(|m| &m.user)
		.or_else(|| participant_user(state, entry.channel, entry.participant.user));
	let name = member
		.and_then(|m| m.nick.as_deref())
		.or_else(|| user.map(|u| u.name.as_str()))
		.unwrap_or("Participant");
	(user, name)
}

/// The single hover tip for a call participant: nickname and username together
/// when they differ, so two elements never repeat the same name.
fn participant_tip(user: Option<&model::User>, name: &str) -> String {
	match user {
		Some(user) if name != user.name => format!("{name} ({})", user.name),
		_ => name.to_owned(),
	}
}

/// Find a call participant's user from self, DM recipients or the roster.
fn participant_user(state: &State, channel: Id, user: Id) -> Option<&model::User> {
	state
		.user
		.as_ref()
		.filter(|u| u.id == user)
		.or_else(|| {
			state
				.channels
				.iter()
				.find(|c| c.id == channel)
				.and_then(|c| c.recipients.iter().find(|u| u.id == user))
		})
		.or_else(|| {
			state
				.voice
				.roster
				.iter()
				.find(|e| e.channel == channel && e.participant.user == user)
				.and_then(|e| e.member.as_ref())
				.map(|m| &m.user)
		})
}

/// Labelled percentage slider shared by the voice popout and the settings page.
/// Hearing-protection level for bots nobody has set a volume for.
const BOT_SAFE_VOLUME: u16 = 50;

/// Small accent pill with a robot glyph, placed before a bot's name.
fn bot_badge(ui: &mut egui::Ui, language: model::Language) {
	let colors = design::palette(ui);
	let (rect, response) = ui.allocate_exact_size(egui::vec2(22.0, 16.0), egui::Sense::hover());
	ui.painter()
		.rect_filled(rect, 5, colors.accent.gamma_multiply(0.22));
	crate::icons::paint(
		ui.painter(),
		crate::icons::Icon::Robot,
		rect.shrink2(egui::vec2(4.0, 2.0)),
		colors.accent,
	);
	let label = crate::i18n::text(language, "Bot");
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, label));
	response.on_hover_text(label);
}

fn volume_step_down(value: u16) -> u16 {
	value.saturating_sub(1) / 5 * 5
}

fn volume_step_up(value: u16) -> u16 {
	((value / 5 + 1) * 5).min(200)
}

fn is_bot(user: Option<&model::User>) -> bool {
	user.is_some_and(|user| matches!(user.kind, model::AccountKind::Bot | model::AccountKind::App))
}

/// Per-person volume that is easy to set exactly: −/+ in 5% steps, a slider that lands on 5%
/// steps and sticks at the 100% "Normal" mark, and one-click presets. True when changed.
fn volume_control(
	ui: &mut egui::Ui,
	value: &mut u16,
	title: &str,
	language: model::Language,
) -> bool {
	let t = |english: &'static str| crate::i18n::text(language, english);
	let colors = design::palette(ui);
	let before = *value;
	ui.scope(|ui| {
		ui.spacing_mut().item_spacing.y = 6.0;
		ui.horizontal(|ui| {
			ui.label(RichText::new(title).size(13.0).color(colors.muted));
			ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
				let (word, color) = match *value {
					0 => (t("Silent"), colors.muted),
					100 => (t("Normal"), colors.positive),
					101.. => (t("Louder than normal"), colors.warning),
					_ => ("", colors.muted),
				};
				ui.label(RichText::new(word).size(12.0).color(color));
			});
		});
		ui.horizontal(|ui| {
			ui.spacing_mut().item_spacing.x = 4.0;
			let step = |ui: &mut egui::Ui, label: &str, hint: &str| {
				ui.add(
					egui::Button::new(RichText::new(label).size(15.0))
						.min_size(egui::vec2(26.0, 26.0)),
				)
				.on_hover_text(hint)
				.clicked()
			};
			if step(ui, "−", t("5% quieter")) {
				*value = volume_step_down(*value);
			}
			// Both 26 px steppers plus their gaps need 60 px; reserving less
			// pushes the "+" button past the menu edge, where it still
			// renders but no longer answers clicks.
			let width = (ui.available_width() - 60.0).max(80.0);
			ui.allocate_ui(egui::vec2(width, 28.0), |ui| {
				design::marked_slider(ui, value, 0..=200, "%", 5.0, &[100]);
			});
			if step(ui, "+", t("5% louder")) {
				*value = volume_step_up(*value);
			}
		});
		// Preset buttons keep their natural size below this width, so a row
		// computed narrower still paints past the menu edge, where the extra
		// buttons render but never answer clicks. Split 3 + 2 instead.
		const PRESET_MIN_WIDTH: f32 = 60.0;
		let presets = [25u16, 50, 100, 150, 200];
		let single = (ui.available_width() - 16.0) / presets.len() as f32;
		let rows: &[&[u16]] = if single >= PRESET_MIN_WIDTH {
			&[&presets[..]]
		} else {
			&[&presets[0..3], &presets[3..5]]
		};
		for row in rows {
			ui.horizontal(|ui| {
				ui.spacing_mut().item_spacing.x = 4.0;
				let width =
					(ui.available_width() - 4.0 * (row.len() - 1) as f32) / row.len() as f32;
				for preset in *row {
					if ui
						.add_sized(
							[width, 24.0],
							egui::Button::selectable(*value == *preset, format!("{preset}%")),
						)
						.clicked()
					{
						*value = *preset;
					}
				}
			});
		}
	});
	*value != before
}

fn gain_slider(ui: &mut egui::Ui, value: &mut u16, title: &str) -> egui::Response {
	ui.scope(|ui| {
		let colors = design::palette(ui);
		ui.spacing_mut().item_spacing.y = 4.0;
		let label = ui.label(RichText::new(title).size(13.0).color(colors.muted));
		design::slider(ui, value, 0..=200, "%").labelled_by(label.id)
	})
	.inner
}

fn gain_controls(
	ui: &mut egui::Ui,
	gain: &mut crate::VoiceGain,
	language: model::Language,
) -> [egui::Response; 2] {
	let t = |english: &'static str| crate::i18n::text(language, english);
	let slider = gain_slider;
	let responses = if ui.available_width() >= 480.0 {
		ui.columns(2, |columns| {
			[
				slider(
					&mut columns[0],
					&mut gain.input_percent,
					t("Microphone gain"),
				),
				slider(
					&mut columns[1],
					&mut gain.output_percent,
					t("Speaker volume"),
				),
			]
		})
	} else {
		[
			slider(ui, &mut gain.input_percent, t("Microphone gain")),
			slider(ui, &mut gain.output_percent, t("Speaker volume")),
		]
	};
	design::hint(
		ui,
		t("100% is the original level. Higher levels may distort."),
	);
	responses
}

fn elapsed_label(call: &client_core::voice::Call) -> Option<String> {
	if !matches!(call.phase, Phase::Waiting | Phase::Connected) {
		return None;
	}
	let seconds = call.connected_at?.elapsed().as_secs();
	Some(format!(
		"{:02}:{:02}:{:02}",
		seconds / 3600,
		seconds / 60 % 60,
		seconds % 60
	))
}

fn status_icon(ui: &mut egui::Ui, icon: crate::icons::Icon, color: egui::Color32, label: &str) {
	let (rect, response) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
	crate::icons::paint(ui.painter(), icon, rect.shrink(1.0), color);
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, label));
	response.on_hover_text(label);
}

/// Discord's LIVE pill, sized for the channel list row rather than a stage tile.
fn live_badge(ui: &mut egui::Ui) {
	let (rect, response) = ui.allocate_exact_size(egui::vec2(32.0, 16.0), egui::Sense::hover());
	let colors = design::palette(ui);
	ui.painter().rect_filled(rect, 4, colors.danger);
	ui.painter().text(
		rect.center(),
		egui::Align2::CENTER_CENTER,
		"LIVE",
		egui::FontId::new(9.0, design::medium_family(ui.ctx())),
		egui::Color32::WHITE,
	);
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, "Live"));
	response.on_hover_text("Streaming");
}

fn device_combo(
	ui: &mut egui::Ui,
	id: &str,
	devices: &[(String, String)],
	selected: &mut Option<String>,
	language: model::Language,
) -> egui::Response {
	let t = |english: &'static str| crate::i18n::text(language, english);
	let unavailable = t("Device unavailable — choose another");
	let label = match selected.as_ref() {
		None => t("System default (recommended)"),
		Some(id) => devices
			.iter()
			.find(|(key, _)| key == id)
			.map_or(unavailable, |(_, label)| label.as_str()),
	};
	egui::ComboBox::from_id_salt(id)
		.selected_text(label)
		.width(ui.available_width())
		.truncate()
		.height(220.0)
		.show_ui(ui, |ui| {
			ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
			ui.selectable_value(selected, None, t("System default (recommended)"));
			for (id, label) in devices.iter().take(32) {
				ui.selectable_value(selected, Some(id.clone()), label)
					.on_hover_text(label);
			}
		})
		.response
		.on_hover_text(label)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn texts(output: &egui::FullOutput) -> Vec<(String, egui::Rect)> {
		let mut found = Vec::new();
		fn walk(shape: &egui::Shape, found: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => found.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						walk(shape, found);
					}
				}
				_ => {}
			}
		}
		for shape in &output.shapes {
			walk(&shape.shape, &mut found);
		}
		found
	}

	// Press and release land on separate frames: that is how real clicks
	// arrive, and some widgets only arm across frames.
	fn press(pos: egui::Pos2) -> Vec<egui::Event> {
		vec![
			egui::Event::PointerMoved(pos),
			egui::Event::PointerButton {
				pos,
				button: egui::PointerButton::Primary,
				pressed: true,
				modifiers: egui::Modifiers::NONE,
			},
		]
	}

	fn release(pos: egui::Pos2) -> Vec<egui::Event> {
		vec![
			egui::Event::PointerMoved(pos),
			egui::Event::PointerButton {
				pos,
				button: egui::PointerButton::Primary,
				pressed: false,
				modifiers: egui::Modifiers::NONE,
			},
		]
	}

	#[test]
	fn voice_channel_hover_text_uses_ascii_separators() {
		assert_eq!(
			voice_channel_hover_text("De Borete", "", true),
			"De Borete | View voice channel | Connected"
		);
		assert_eq!(
			voice_channel_hover_text("Room", " · Muted", false),
			"Room | View voice channel | Muted"
		);
		let text = voice_channel_hover_text("De Borete", "", true);
		assert!(
			!text.contains('\u{00C2}') && !text.contains('\u{00B7}'),
			"separator must not be a UTF-8 middle dot (shows as Â· on Windows-1252 paths): {text:?}"
		);
	}

	#[test]
	fn voice_server_place_reads_country_from_the_hostname() {
		assert_eq!(
			voice_server_place("c-gru16-abc123.discord.media:443"),
			Some("Brazil")
		);
		assert_eq!(
			voice_server_place("brazil847.discord.media"),
			Some("Brazil")
		);
		assert_eq!(
			voice_server_place("us-east1.discord.media"),
			Some("United States")
		);
		assert_eq!(
			voice_server_place("https://c-fra3-node.discord.media/path"),
			Some("Germany")
		);
		assert_eq!(voice_server_place("synthetic.discord.media"), None);
		assert_eq!(
			crate::i18n::text(model::Language::PortugueseBrazil, "Brazil"),
			"Brasil"
		);
	}

	#[test]
	fn explicit_join_audio_waits_for_call_switch_and_survives_teardown() {
		let mut state = test_support::call_demo_state();
		state.demo = false;
		state.gateway_connected = true;
		let mut view = MessagingUi {
			voice_available: true,
			..Default::default()
		};
		let before = (view.voice_muted, view.voice_deafened);
		let mut commands = Vec::new();
		view.request_call_with_audio(&mut state, Id(25), false, true, true, &mut commands)
			.unwrap();
		assert_eq!((view.voice_muted, view.voice_deafened), before);
		assert!(matches!(
			commands.as_slice(),
			[Command::Voice(client_core::voice::Command::Leave { .. })]
		));
		let origin = view.voice_switch.as_ref().unwrap().from;
		assert!(view.voice_switch.as_ref().unwrap().confirmed_at.is_some());
		assert!(state.voice.active.is_none());
		commands.clear();
		state.apply_voice(client_core::voice::Event::Departed {
			channel: origin.0,
			request: origin.1,
		});
		let ctx = egui::Context::default();
		view.show_call_switch(&ctx, &mut state, &mut commands);
		assert!(commands.is_empty());
		assert_eq!((view.voice_muted, view.voice_deafened), before);
		view.voice_switch_ready = true;
		view.show_call_switch(&ctx, &mut state, &mut commands);
		assert!(matches!(
			&commands[..],
			[Command::Voice(client_core::voice::Command::Join {
				channel: Id(25),
				mute: true,
				deaf: true,
				..
			})]
		));
		assert_eq!((view.voice_muted, view.voice_deafened), (true, true));
		commands.clear();
		assert!(
			view.request_call_with_audio(
				&mut state,
				Id(999999),
				false,
				false,
				false,
				&mut commands
			)
			.is_err()
		);
		assert_eq!((view.voice_muted, view.voice_deafened), (true, true));
		assert!(commands.is_empty());
	}

	#[test]
	fn bots_start_at_half_volume_unless_chosen_or_disabled() {
		let mut state = test_support::voice_demo_state();
		let bot = state.voice.roster[2].participant.user;
		state.voice.roster[2].member.as_mut().unwrap().user.kind = model::AccountKind::Bot;
		let mut view = MessagingUi {
			voice_bot_safe_volume: true,
			..MessagingUi::default()
		};
		let mixed = |view: &MessagingUi, state: &State, user: Id| {
			let mix = view.voice_mix_volumes(state);
			mix.iter()
				.find(|(id, _)| *id == user.0)
				.map(|(_, volume)| *volume)
		};
		assert_eq!(mixed(&view, &state, bot), Some(BOT_SAFE_VOLUME));
		assert_eq!(mixed(&view, &state, Id(2)), None, "people keep 100%");
		let saved = view.voice_user_volume_overrides();
		assert!(saved.is_empty(), "the default is not saved");

		view.set_voice_user_volume_from(bot, 100, BOT_SAFE_VOLUME);
		assert_eq!(mixed(&view, &state, bot), Some(100));
		let saved = view.voice_user_volume_overrides();
		assert_eq!(saved, [(bot.0, 100)], "a chosen 100% persists");
		view.set_voice_user_volume_from(bot, BOT_SAFE_VOLUME, BOT_SAFE_VOLUME);
		assert!(view.voice_user_volume_overrides().is_empty());

		view.set_voice_user_locally_muted(bot, true);
		assert_eq!(mixed(&view, &state, bot), Some(0), "mute wins");
		view.set_voice_user_locally_muted(bot, false);
		view.voice_bot_safe_volume = false;
		assert_eq!(mixed(&view, &state, bot), None);
		state.voice.active = None;
		view.voice_bot_safe_volume = true;
		assert_eq!(mixed(&view, &state, bot), None);
	}

	#[test]
	fn volume_control_fits_narrow_menus() {
		// The menu is 260 px wide: minus, slider, plus and every preset,
		// with their gaps, must paint inside it, or the right side renders
		// yet never answers clicks.
		let ctx = egui::Context::default();
		design::apply(&ctx);
		let mut volume = 100u16;
		let frame = |volume: &mut u16, events: Vec<egui::Event>| {
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(400.0, 300.0),
					)),
					events,
					..Default::default()
				},
				|ui| {
					ui.allocate_ui(egui::vec2(260.0, 300.0), |ui| {
						volume_control(
							ui,
							volume,
							"User volume",
							model::Language::PortugueseBrazil,
						);
					});
				},
			);
			output.textures_delta.clear();
			output
		};
		let output = frame(&mut volume, vec![]);
		// Only the stepper band counts here, anchored on the "+" glyph
		// itself: the readout sits in it, while the title above and the
		// presets below are separate rows.
		let plus_y = texts(&output)
			.iter()
			.find(|(text, _)| text == "+")
			.map(|(_, rect)| rect.center().y)
			.unwrap();
		let mut right = 0.0f32;
		let mut wide = Vec::new();
		for shape in &output.shapes {
			let rect = shape.shape.visual_bounding_rect();
			if (rect.center().y - plus_y).abs() > 15.0 {
				continue;
			}
			right = right.max(rect.right());
			if rect.right() > 261.0 {
				let label = match &shape.shape {
					egui::Shape::Text(text) => text.galley.job.text.clone(),
					other => format!("{other:?}"),
				};
				wide.push((rect, label.chars().take(24).collect::<String>()));
			}
		}
		assert!(
			right <= 261.0,
			"stepper row overflows a 260 px menu: paints to {right}: {wide:?}"
		);
		for name in ["25%", "50%", "100%", "150%", "200%"] {
			let end = texts(&output)
				.iter()
				.find(|(text, _)| text == name)
				.map(|(_, rect)| rect.right())
				.unwrap();
			assert!(
				end <= 261.0,
				"preset {name} paints past a 260 px menu: ends at {end}"
			);
		}
		let plus = texts(&output)
			.iter()
			.find(|(text, _)| text == "+")
			.map(|(_, rect)| rect.center())
			.unwrap();
		frame(&mut volume, press(plus));
		frame(&mut volume, release(plus));
		assert_eq!(volume, 105, "the stepper answers inside a narrow menu");
	}

	#[test]
	fn participant_volume_menu_survives_inside_clicks_and_reopens() {
		let ctx = egui::Context::default();
		design::apply(&ctx);
		let state = test_support::voice_demo_state();
		let entry = state.voice.roster[1].clone();
		let mut view = MessagingUi::default();
		let frame = |view: &mut MessagingUi,
		             state: &State,
		             events: Vec<egui::Event>|
		 -> (egui::FullOutput, egui::Rect) {
			let mut row = egui::Rect::NOTHING;
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(400.0, 800.0),
					)),
					events,
					..Default::default()
				},
				|ui| {
					ui.set_width(360.0);
					if let Some(response) = view.voice_participant(ui, state, &entry, false) {
						row = response.rect;
					}
				},
			);
			output.textures_delta.clear();
			(output, row)
		};
		let right_click = |pos: egui::Pos2| {
			vec![
				egui::Event::PointerMoved(pos),
				egui::Event::PointerButton {
					pos,
					button: egui::PointerButton::Secondary,
					pressed: true,
					modifiers: egui::Modifiers::NONE,
				},
				egui::Event::PointerButton {
					pos,
					button: egui::PointerButton::Secondary,
					pressed: false,
					modifiers: egui::Modifiers::NONE,
				},
			]
		};
		let click = |view: &mut MessagingUi, state: &State, pos: egui::Pos2| {
			frame(view, state, vec![egui::Event::PointerMoved(pos)]);
			frame(view, state, press(pos));
			frame(view, state, release(pos))
		};
		// Settle one frame so the row rect is known.
		let (_, row) = frame(&mut view, &state, vec![]);
		assert!(row.width() > 0.0, "participant row renders");
		let open_menu = |view: &mut MessagingUi, state: &State| {
			let (_, _) = frame(view, state, right_click(row.center()));
			let (output, _) = frame(view, state, vec![]);
			texts(&output)
		};
		// Open with right-click: the menu shows its volume section.
		let labels = open_menu(&mut view, &state);
		assert!(
			labels
				.iter()
				.any(|(text, _)| text == "Mute" || text == "Unmute"),
			"right-click opens the participant menu: {labels:?}"
		);
		assert!(
			labels.iter().any(|(text, _)| text == "User volume"),
			"menu shows the volume section: {labels:?}"
		);
		// Decisive: plus first on a fresh menu, no prior drag or preset.
		let user = entry.participant.user.0;
		let mixed = |view: &MessagingUi| {
			view.voice_user_volume_overrides()
				.iter()
				.find(|(id, _)| *id == user)
				.map(|(_, volume)| *volume)
		};
		let drag = |view: &mut MessagingUi, state: &State, from: egui::Pos2, to: egui::Pos2| {
			frame(view, state, press(from));
			frame(view, state, vec![egui::Event::PointerMoved(to)]);
			frame(view, state, release(to))
		};
		let plus = labels
			.iter()
			.find(|(text, _)| text == "+")
			.map(|(_, rect)| rect.center())
			.unwrap();
		let (output, _) = click(&mut view, &state, plus);
		let after = texts(&output);
		assert_eq!(
			mixed(&view),
			Some(105),
			"stepper answers on a fresh menu: {after:?}"
		);
		assert!(
			after.iter().any(|(text, _)| text == "User volume"),
			"clicking inside keeps the menu open: {after:?}"
		);
		let (_, _) = drag(
			&mut view,
			&state,
			egui::pos2(85.0, 120.0),
			egui::pos2(95.0, 120.0),
		);
		let (output, _) = frame(&mut view, &state, vec![]);
		let dragged = mixed(&view);
		assert!(
			matches!(dragged, Some(volume) if volume != 100),
			"slider drag changes the volume: {dragged:?}"
		);
		assert!(
			texts(&output).iter().any(|(text, _)| text == "User volume"),
			"drag keeps the menu open"
		);
		// The "+" stepper adds 5 to the dragged value; the menu stays open.
		let labels = open_menu(&mut view, &state);
		let plus = labels
			.iter()
			.find(|(text, _)| text == "+")
			.map(|(_, rect)| rect.center())
			.unwrap();
		let (output, _) = click(&mut view, &state, plus);
		let after = texts(&output);
		let stepped = mixed(&view).unwrap();
		assert_eq!(
			stepped,
			volume_step_up(dragged.unwrap()),
			"stepper click adds 5 percent"
		);
		assert!(
			after.iter().any(|(text, _)| text == "User volume"),
			"clicking inside keeps the menu open: {after:?}"
		);
		// A preset button sets the volume absolutely; the menu stays open.
		let preset = after
			.iter()
			.find(|(text, _)| text == "50%")
			.map(|(_, rect)| rect.center())
			.unwrap();
		let (output, _) = click(&mut view, &state, preset);
		let after = texts(&output);
		assert_eq!(mixed(&view), Some(50), "preset click sets 50 percent");
		assert!(
			after.iter().any(|(text, _)| text == "User volume"),
			"clicking inside keeps the menu open: {after:?}"
		);
		// The last preset sets 200 percent from inside the menu too.
		let wide = after
			.iter()
			.find(|(text, _)| text == "200%")
			.map(|(_, rect)| rect.center())
			.unwrap();
		let (output, _) = click(&mut view, &state, wide);
		let after = texts(&output);
		assert_eq!(mixed(&view), Some(200), "preset click sets 200 percent");
		assert!(
			after.iter().any(|(text, _)| text == "User volume"),
			"clicking inside keeps the menu open: {after:?}"
		);
		// Close on the row itself past the popup edge, then right-click
		// opens the menu again. (Outside clicks never reach the close path
		// in a headless harness — verified with a bare egui popup — so the
		// test closes through the row, which the menu explicitly honors.)
		let (output, _) = click(&mut view, &state, egui::pos2(340.0, 17.0));
		assert!(
			!texts(&output).iter().any(|(text, _)| text == "User volume"),
			"row click closes the menu"
		);
		let labels = open_menu(&mut view, &state);
		assert!(
			labels.iter().any(|(text, _)| text == "User volume"),
			"right-click reopens the menu: {labels:?}"
		);
	}

	#[test]
	fn participant_tip_combines_nick_and_username() {
		let user = model::User {
			id: Id(2),
			name: "Robin with a rather long display name".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
		};
		assert_eq!(
			participant_tip(Some(&user), "Robin"),
			"Robin (Robin with a rather long display name)"
		);
		assert_eq!(
			participant_tip(Some(&user), "Robin with a rather long display name"),
			"Robin with a rather long display name"
		);
		assert_eq!(participant_tip(None, "Participant"), "Participant");
	}

	#[test]
	fn participant_row_shows_a_single_name_tip() {
		let ctx = egui::Context::default();
		design::apply(&ctx);
		let mut state = test_support::voice_demo_state();
		state.voice.roster[1].member.as_mut().unwrap().nick = Some("Robin".into());
		let entry = state.voice.roster[1].clone();
		let mut view = MessagingUi::default();
		let mut now = 0.0;
		// Tooltips only appear over a still pointer, so the move happens
		// once and later frames carry no pointer events at all.
		let mut hover = |view: &mut MessagingUi, state: &State, pos: Option<egui::Pos2>| {
			now += 0.3;
			let mut row = egui::Rect::NOTHING;
			let mut output = ctx.run_ui(
				egui::RawInput {
					time: Some(now),
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(400.0, 800.0),
					)),
					events: pos.map_or(vec![], |pos| vec![egui::Event::PointerMoved(pos)]),
					..Default::default()
				},
				|ui| {
					ui.set_width(360.0);
					if let Some(response) = view.voice_participant(ui, state, &entry, false) {
						row = response.rect;
					}
				},
			);
			output.textures_delta.clear();
			(output, row)
		};
		// Settle one frame so the row rect is known.
		let (_, row) = hover(&mut view, &state, None);
		assert!(row.width() > 0.0, "participant row renders");
		// The avatar carries no tip: hovering it past the tooltip delay
		// must never paint the bare username anywhere.
		let username = "Robin with a rather long display name";
		let avatar = egui::pos2(row.left() + 14.0, row.center().y);
		let (output, _) = hover(&mut view, &state, Some(avatar));
		assert!(
			!texts(&output).iter().any(|(text, _)| text == username),
			"avatar shows no name tip on arrival"
		);
		for _ in 0..5 {
			let (output, _) = hover(&mut view, &state, None);
			assert!(
				!texts(&output).iter().any(|(text, _)| text == username),
				"avatar shows no name tip"
			);
		}
		// The name shows the single combined tip instead.
		let (output, _) = hover(&mut view, &state, None);
		let name_pos = texts(&output)
			.iter()
			.find(|(text, _)| text == "Robin")
			.map(|(_, rect)| rect.center())
			.unwrap();
		let combined = format!("Robin ({username})");
		let (output, _) = hover(&mut view, &state, Some(name_pos));
		assert!(
			!texts(&output).iter().any(|(text, _)| text == &combined),
			"tip needs a still hover, not the arrival frame"
		);
		for _ in 0..6 {
			let (output, _) = hover(&mut view, &state, None);
			if texts(&output).iter().any(|(text, _)| text == &combined) {
				return;
			}
		}
		let (output, _) = hover(&mut view, &state, None);
		panic!(
			"name hover shows {combined:?}: {:?}",
			texts(&output)
				.iter()
				.map(|(text, _)| text)
				.collect::<Vec<_>>()
		);
	}

	#[test]
	fn noise_badge_stays_before_the_meter_in_compact_menus() {
		// Compact Portuguese menu, 260 px wide: title, engine and badge
		// must all end before the usage meter, or the pill paints over
		// the radio side and looks broken.
		let ctx = egui::Context::default();
		design::apply(&ctx);
		let mut view = MessagingUi {
			language: model::Language::PortugueseBrazil,
			..Default::default()
		};
		let mut output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(400.0, 600.0),
				)),
				events: vec![],
				..Default::default()
			},
			|ui| {
				ui.allocate_ui(egui::vec2(260.0, 600.0), |ui| {
					view.noise_level_picker(ui, true);
				});
			},
		);
		output.textures_delta.clear();
		// Meter bars: 5 px wide, 4 to 12 px tall, at the row's right side.
		let mut meter_left = f32::MAX;
		for shape in &output.shapes {
			if let egui::Shape::Rect(rect) = &shape.shape {
				let size = rect.rect.size();
				if (size.x - 5.0).abs() < 0.6 && (4.0..=12.0).contains(&size.y) {
					meter_left = meter_left.min(rect.rect.left());
				}
			}
		}
		assert!(
			meter_left < f32::MAX,
			"compact rows paint their usage meter"
		);
		for name in ["Padrão", "RNNoise", "Recomendado"] {
			let end = texts(&output)
				.iter()
				.find(|(text, _)| text == name)
				.map(|(_, rect)| rect.right())
				.unwrap();
			assert!(
				end < meter_left,
				"{name} paints past the meter at {meter_left}: ends at {end}"
			);
		}
	}

	#[test]
	fn call_header_holds_green_through_short_flaps() {
		// A 200 ms dip never reaches the header; an 800 ms one does. The
		// title and its color move in the same branch, so the title proves
		// which side of the 500 ms grace each frame falls on.
		let ctx = egui::Context::default();
		design::apply(&ctx);
		let mut state = test_support::voice_demo_state();
		state.demo = false;
		let call = state.voice.active.as_mut().expect("fixture call");
		call.phase = client_core::voice::Phase::OpeningAudio;
		let mut view = MessagingUi::default();
		let mut titles = Vec::new();
		let mut frame = |view: &mut MessagingUi, state: &mut State| {
			let mut commands = Vec::new();
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(400.0, 800.0),
					)),
					events: vec![],
					..Default::default()
				},
				|ui| {
					ui.set_width(360.0);
					view.voice_card_section(ui, state, &mut commands);
				},
			);
			output.textures_delta.clear();
			for shape in &output.shapes {
				if let egui::Shape::Text(text) = &shape.shape {
					let title = text.galley.job.text.clone();
					if title == "Voice Connected" || title == "Connecting…" {
						titles.push(title);
					}
				}
			}
		};
		frame(&mut view, &mut state);
		std::thread::sleep(std::time::Duration::from_millis(200));
		frame(&mut view, &mut state);
		std::thread::sleep(std::time::Duration::from_millis(650));
		frame(&mut view, &mut state);
		assert_eq!(
			titles,
			["Voice Connected", "Voice Connected", "Connecting…"],
			"200 ms holds green, 800 ms turns yellow: {titles:?}"
		);
	}

	#[test]
	fn connection_tip_shows_last_ping_or_ellipsis() {
		// The hover keeps the last measured value so reconnections never
		// flash "…"; the ellipsis only appears before the first measurement.
		let ctx = egui::Context::default();
		design::apply(&ctx);
		let tip = |ping: Option<u32>| {
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(400.0, 300.0),
					)),
					events: vec![],
					..Default::default()
				},
				|ui| {
					voice_connection_tip(ui, ping, "Brazil", model::Language::English);
				},
			);
			output.textures_delta.clear();
			texts(&output)
				.into_iter()
				.map(|(text, _)| text)
				.collect::<Vec<_>>()
		};
		let measured = tip(Some(42));
		assert!(
			measured.iter().any(|text| text == "42 ms"),
			"measured ping renders: {measured:?}"
		);
		assert!(
			measured.iter().any(|text| text == "Brazil"),
			"server place renders: {measured:?}"
		);
		let fresh = tip(None);
		assert!(
			fresh.iter().any(|text| text == "…"),
			"ellipsis only before the first measurement: {fresh:?}"
		);
		assert!(
			!fresh.iter().any(|text| text.contains("ms")),
			"no stale value without a measurement: {fresh:?}"
		);
	}

	#[test]
	fn volume_steps_land_on_five_percent() {
		assert_eq!(volume_step_down(47), 45);
		assert_eq!(volume_step_down(45), 40);
		assert_eq!(volume_step_down(0), 0);
		assert_eq!(volume_step_up(47), 50);
		assert_eq!(volume_step_up(50), 55);
		assert_eq!(volume_step_up(198), 200);
		assert_eq!(volume_step_up(200), 200);
	}

	#[test]
	fn marked_slider_drags_stick_to_marks_and_steps() {
		let ctx = egui::Context::default();
		let raw = |events| egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(300.0, 60.0),
			)),
			events,
			..Default::default()
		};
		let mut value = 0u16;
		let mut rect = egui::Rect::NOTHING;
		ctx.run_ui(raw(vec![]), |ui| {
			rect = design::marked_slider(ui, &mut value, 0..=200, "%", 5.0, &[100]).rect;
		})
		.drop_without_applying_deltas();
		// Mirrors the slider's layout: 9 px insets and a 64 px value readout.
		let (left, right) = (rect.left() + 9.0, rect.right() - 64.0 - 9.0);
		let at = |percent: f32| left + (right - left) * percent / 200.0;
		for (percent, expected) in [(103.0, 100), (61.0, 60), (33.8, 35)] {
			let pos = egui::pos2(at(percent), rect.center().y);
			for pressed in [true, false] {
				ctx.run_ui(
					raw(vec![
						egui::Event::PointerMoved(pos),
						egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					]),
					|ui| {
						design::marked_slider(ui, &mut value, 0..=200, "%", 5.0, &[100]);
					},
				)
				.drop_without_applying_deltas();
			}
			assert_eq!(value, expected, "click at {percent}%");
		}
	}

	#[test]
	fn local_mute_still_applies_with_full_volume_overrides() {
		let mut view = MessagingUi::default();
		let volumes: Vec<_> = (100..164).map(|user| (user, 150)).collect();
		view.set_voice_user_volume_overrides(&volumes);
		view.set_voice_user_volume(Id(9), 100);
		assert_eq!(view.voice_user_volume_overrides(), volumes);
		view.set_voice_user_locally_muted(Id(9), true);
		assert!(view.voice_user_volumes().contains(&(9, 0)));
		assert_eq!(view.voice_user_volume_overrides(), volumes);
		view.set_voice_user_locally_muted(Id(9), false);
		assert_eq!(view.voice_user_volumes().to_vec(), volumes);
	}

	#[test]
	fn local_mutes_zero_one_speaker_and_keep_their_stored_volume() {
		let mut view = MessagingUi::default();
		view.set_voice_user_volume_overrides(&[(7, 150)]);
		assert!(!view.voice_user_locally_muted(Id(7)));
		view.set_voice_user_locally_muted(Id(7), true);
		assert_eq!(view.voice_user_mutes(), [7]);
		assert!(view.voice_user_volumes().contains(&(7, 0)));
		// Muting is device-local and never rewrites the volume chosen for that speaker.
		assert_eq!(view.voice_user_volume_overrides(), vec![(7, 150)]);
		view.set_voice_user_locally_muted(Id(9), true);
		assert!(view.voice_user_volumes().contains(&(9, 0)));
		view.set_voice_user_locally_muted(Id(7), false);
		assert!(view.voice_user_volumes().contains(&(7, 150)));
		assert_eq!(view.voice_user_mutes(), [9]);
		view.set_voice_user_mutes(&(0..200).collect::<Vec<u64>>());
		assert_eq!(view.voice_user_mutes().len(), MAX_USER_MUTES);
		assert!(!view.voice_user_mutes().contains(&0));
	}

	#[test]
	fn voice_toggle_cues_follow_the_resulting_state_and_preferences() {
		use model::notification_preferences::Sound;

		let mut view = MessagingUi::default();
		for (deafen, active, expected) in [
			(false, true, Sound::Mute),
			(false, false, Sound::Unmute),
			(true, true, Sound::Deafen),
			(true, false, Sound::Undeafen),
		] {
			view.notification_cues.clear();
			view.queue_voice_toggle_cue(deafen, active);
			assert_eq!(view.notification_cues.last().copied(), Some(expected));
		}
		view.notification_options.mute = false;
		view.notification_cues.clear();
		view.queue_voice_toggle_cue(false, true);
		assert_eq!(
			view.notification_cues.last().copied(),
			Some(Sound::Mute),
			"sound preferences are applied when the cue is played"
		);
	}

	#[test]
	fn camera_settings_bound_layout_and_only_request_discovery_once() {
		let mut disabled = MessagingUi::default();
		egui::Context::default()
			.run_ui(Default::default(), |ui| {
				ui.add_enabled_ui(false, |ui| disabled.camera_settings_content(ui, false));
			})
			.drop_without_applying_deltas();
		assert!(!disabled.voice_refresh_cameras);
		assert!(disabled.voice_camera_device_status.is_empty());
		for demo in [false, true] {
			for theme in [egui::Theme::Dark, egui::Theme::Light] {
				let ctx = egui::Context::default();
				ctx.set_theme(theme);
				let mut messaging = MessagingUi {
					voice_cameras: vec![(
						"synthetic-camera".into(),
						"Long synthetic camera name ".repeat(20),
					)],
					voice_camera_device: Some("synthetic-camera".into()),
					..Default::default()
				};
				for width in [240.0, 640.0] {
					for frame in 0..2 {
						let mut output = ctx.run_ui(
							egui::RawInput {
								screen_rect: Some(egui::Rect::from_min_size(
									egui::Pos2::ZERO,
									egui::vec2(width, 600.0),
								)),
								..Default::default()
							},
							|ui| {
								let available = ui.available_width();
								let content =
									ui.scope(|ui| messaging.camera_settings_content(ui, demo));
								assert!(content.response.rect.width() <= available + 1.0);
							},
						);
						output.textures_delta.clear();
						assert_eq!(
							messaging.voice_camera_device.as_deref(),
							Some("synthetic-camera")
						);
						assert_eq!(
							messaging.voice_refresh_cameras,
							!demo
								&& cfg!(any(
									target_os = "windows",
									target_os = "macos",
									target_os = "linux"
								)) && width == 240.0 && frame == 0
						);
						assert!(messaging.voice_camera_preview.is_none());
						messaging.voice_refresh_cameras = false;
					}
				}
			}
		}
	}

	#[test]
	fn camera_popup_selects_second_device_without_starting_capture() {
		fn labels(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => out.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| labels(shape, out)),
				_ => {}
			}
		}
		let ctx = egui::Context::default();
		let frame = |messaging: &mut MessagingUi, events: Vec<egui::Event>| {
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(640.0, 600.0),
					)),
					events,
					..Default::default()
				},
				|ui| {
					let trigger = ui.button("Choose camera");
					messaging.camera_settings_popup(&trigger, true);
				},
			);
			output.textures_delta.clear();
			let mut text = vec![];
			for shape in output.shapes {
				labels(&shape.shape, &mut text);
			}
			text
		};
		let click = |messaging: &mut MessagingUi, pos: egui::Pos2| {
			for pressed in [true, false] {
				frame(
					messaging,
					vec![
						egui::Event::PointerMoved(pos),
						egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
		};
		let mut messaging = MessagingUi::default();
		for label in [
			"Choose camera",
			"System default (recommended)",
			"USB Camera (preview)",
		] {
			frame(&mut messaging, vec![]);
			let text = frame(&mut messaging, vec![]);
			let pos = text
				.iter()
				.find(|(text, _)| text == label)
				.unwrap_or_else(|| panic!("Missing control: {label}"))
				.1
				.center();
			click(&mut messaging, pos);
		}
		assert_eq!(
			messaging.voice_camera_device.as_deref(),
			Some("synthetic-usb")
		);
		assert!(!messaging.voice_refresh_cameras);
		assert!(messaging.voice_camera_preview.is_none());
		assert!(
			frame(&mut messaging, vec![])
				.iter()
				.any(|(text, _)| text == "Camera device"),
			"Selecting a device keeps the parent camera popup open"
		);
	}

	#[test]
	fn solo_call_stages_show_both_local_previews_without_a_roster() {
		fn textures(shape: &egui::Shape, ids: &mut Vec<egui::TextureId>) {
			match shape {
				egui::Shape::Mesh(mesh) => ids.push(mesh.texture_id),
				egui::Shape::Rect(rect) => {
					if let Some(brush) = &rect.brush {
						ids.push(brush.fill_texture_id);
					}
				}
				egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| textures(shape, ids)),
				_ => {}
			}
		}
		for guild in [false, true] {
			for width in [320.0, 900.0] {
				for dark in [false, true] {
					let mut state = if guild {
						test_support::voice_demo_state()
					} else {
						test_support::call_demo_state()
					};
					state.voice.roster.clear();
					let call = state.voice.active.as_mut().unwrap();
					call.participants.clear();
					call.phase = Phase::Waiting;
					call.camera = true;
					let (channel, request) = (call.channel, call.request);
					let ctx = egui::Context::default();
					ctx.set_visuals(if dark {
						egui::Visuals::dark()
					} else {
						egui::Visuals::light()
					});
					let mut messaging = MessagingUi::default();
					let camera = ctx.load_texture(
						"synthetic-camera",
						egui::ColorImage::filled([4, 3], egui::Color32::RED),
						Default::default(),
					);
					let screen = ctx.load_texture(
						"synthetic-screen",
						egui::ColorImage::filled([16, 9], egui::Color32::BLUE),
						Default::default(),
					);
					let expected = [camera.id(), screen.id()];
					messaging.voice_camera_preview = Some(camera);
					messaging.screen.preview = Some(screen);
					messaging.screen.busy = true;
					messaging.screen.context = Some((state.generation, channel, request));
					assert_eq!(stage_participants(&state, channel).len(), 1);
					let mut commands = vec![];
					for phase in [Phase::Waiting, Phase::Connected, Phase::Failed] {
						state.voice.active.as_mut().unwrap().phase = phase;
						let mut output = ctx.run_ui(
							egui::RawInput {
								screen_rect: Some(egui::Rect::from_min_size(
									egui::Pos2::ZERO,
									egui::vec2(width, 800.0),
								)),
								..Default::default()
							},
							|ui| {
								if guild {
									messaging.voice_channel(ui, &mut state, channel, &mut commands);
								} else {
									messaging.call_bar(ui, &mut state, &mut commands);
								}
							},
						);
						let mut rendered = vec![];
						for shape in &output.shapes {
							textures(&shape.shape, &mut rendered);
						}
						output.textures_delta.clear();
						for id in expected {
							assert_eq!(
								rendered.contains(&id),
								phase != Phase::Failed,
								"guild={guild}, width={width}, phase={phase:?}, texture={id:?}"
							);
						}
					}
					assert!(
						commands.is_empty(),
						"Synthetic previews must never start media"
					);
					state.voice.active = None;
					assert!(stage_participants(&state, channel).is_empty());
				}
			}
		}
	}

	#[test]
	fn existing_dm_call_banner_joins_without_ringing_and_disables_unavailable_actions() {
		fn frame(
			ctx: &egui::Context,
			messaging: &mut MessagingUi,
			state: &mut State,
			width: f32,
			events: Vec<egui::Event>,
		) -> (Vec<(String, egui::Rect)>, Vec<Command>) {
			fn labels(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
				match shape {
					egui::Shape::Text(text) => out.push((
						text.galley.job.text.clone(),
						text.galley.rect.translate(text.pos.to_vec2()),
					)),
					egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| labels(shape, out)),
					_ => {}
				}
			}
			let mut commands = vec![];
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(width, 480.0),
					)),
					events,
					..Default::default()
				},
				|ui| messaging.call_bar(ui, state, &mut commands),
			);
			output.textures_delta.clear();
			let mut text = vec![];
			for shape in output.shapes {
				labels(&shape.shape, &mut text);
			}
			(text, commands)
		}
		for width in [320.0, 900.0] {
			for dark in [false, true] {
				for mode in 0..5 {
					let mut state = test_support::existing_call_demo_state();
					state.demo = mode == 1;
					state.gateway_connected = mode != 2;
					if mode == 4 {
						assert!(state.start_call(Id(25), false).is_some());
					}
					state
						.channels
						.iter_mut()
						.find(|c| c.id == Id(22))
						.unwrap()
						.name = "A long synthetic caller name ".repeat(8);
					let mut messaging = MessagingUi {
						voice_available: mode != 3,
						..Default::default()
					};
					let ctx = egui::Context::default();
					ctx.set_visuals(if dark {
						egui::Visuals::dark()
					} else {
						egui::Visuals::light()
					});
					frame(&ctx, &mut messaging, &mut state, width, vec![]);
					let (text, commands) = frame(&ctx, &mut messaging, &mut state, width, vec![]);
					assert!(
						commands.is_empty(),
						"Showing an ongoing call must never join"
					);
					let join = text
						.iter()
						.find(|(label, _)| label == "Join call")
						.expect("Visible Join call button")
						.1;
					assert!(
						join.left() >= 0.0 && join.right() <= width,
						"Join must fit narrow layouts"
					);
					assert!(text.iter().any(|(label, _)| label
						== if mode == 2 {
							"Reconnect to refresh call"
						} else {
							"Call in progress"
						}));
					let mut sent = vec![];
					for pressed in [true, false] {
						let (_, commands) = frame(
							&ctx,
							&mut messaging,
							&mut state,
							width,
							vec![
								egui::Event::PointerMoved(join.center()),
								egui::Event::PointerButton {
									pos: join.center(),
									button: egui::PointerButton::Primary,
									pressed,
									modifiers: egui::Modifiers::NONE,
								},
							],
						);
						sent.extend(commands);
					}
					if mode == 0 {
						assert!(matches!(
							sent.as_slice(),
							[Command::Voice(client_core::voice::Command::Join {
								channel: Id(22),
								ring: false,
								..
							})]
						));
					} else if mode == 4 {
						assert!(
							matches!(
								sent.as_slice(),
								[Command::Voice(client_core::voice::Command::Leave {
									channel: Id(25),
									..
								})]
							),
							"Joining another call leaves the current one immediately"
						);
					} else {
						assert!(
							sent.is_empty(),
							"Demo, offline and voice-unavailable states cannot join"
						);
					}
					state.apply_voice(client_core::voice::Event::Deleted { channel: Id(22) });
					let (text, _) = frame(&ctx, &mut messaging, &mut state, width, vec![]);
					assert!(
						!text
							.iter()
							.any(|(label, _)| label == "Join call" || label == "Call in progress")
					);
				}
			}
		}
	}

	#[test]
	fn voice_popup_keeps_device_selection_open_and_demo_controls_inert() {
		fn labels(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => out.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| labels(shape, out)),
				_ => {}
			}
		}
		fn frame(
			ctx: &egui::Context,
			messaging: &mut MessagingUi,
			demo: bool,
			events: Vec<egui::Event>,
		) -> Vec<(String, egui::Rect)> {
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(800.0, 800.0),
					)),
					events,
					..Default::default()
				},
				|ui| {
					let trigger = ui.button("Open voice");
					messaging.voice_settings_popup(&trigger, demo, false, true);
				},
			);
			output.textures_delta.clear();
			let mut text = vec![];
			for shape in output.shapes {
				labels(&shape.shape, &mut text);
			}
			text
		}
		fn click(ctx: &egui::Context, messaging: &mut MessagingUi, demo: bool, pos: egui::Pos2) {
			for pressed in [true, false] {
				frame(
					ctx,
					messaging,
					demo,
					vec![
						egui::Event::PointerMoved(pos),
						egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
		}
		let position = |text: &[(String, egui::Rect)], label: &str| {
			text.iter()
				.find(|(value, _)| value == label)
				.unwrap_or_else(|| panic!("Missing visible control: {label}"))
				.1
				.center()
		};
		for demo in [false, true] {
			let ctx = egui::Context::default();
			let mut messaging = MessagingUi {
				voice_available: true,
				voice_inputs: vec![("synthetic-input".into(), "Synthetic headset".into())],
				voice_gain: crate::VoiceGain {
					input_percent: 140,
					output_percent: 80,
				},
				..Default::default()
			};
			frame(&ctx, &mut messaging, demo, vec![]);
			let text = frame(&ctx, &mut messaging, demo, vec![]);
			click(&ctx, &mut messaging, demo, position(&text, "Open voice"));
			let text = frame(&ctx, &mut messaging, demo, vec![]);
			click(
				&ctx,
				&mut messaging,
				demo,
				position(&text, "System default (recommended)"),
			);
			let text = frame(&ctx, &mut messaging, demo, vec![]);
			if demo {
				assert!(
					!egui::Popup::is_any_open(&ctx),
					"Disabled devices must not open a picker"
				);
				click(
					&ctx,
					&mut messaging,
					demo,
					position(&text, "Rescan devices"),
				);
				let text = frame(&ctx, &mut messaging, demo, vec![]);
				click(&ctx, &mut messaging, demo, position(&text, "Reset levels"));
				assert_eq!(messaging.voice_input, None);
				assert!(!messaging.voice_refresh_devices);
				assert_eq!(messaging.voice_gain.input_percent, 140);
				assert_eq!(messaging.voice_gain.output_percent, 80);
			} else {
				assert!(
					egui::Popup::is_any_open(&ctx),
					"The device picker must survive its opening frame"
				);
				click(
					&ctx,
					&mut messaging,
					demo,
					position(&text, "Synthetic headset"),
				);
				assert_eq!(messaging.voice_input.as_deref(), Some("synthetic-input"));
				assert!(!egui::Popup::is_any_open(&ctx));
			}
			let text = frame(&ctx, &mut messaging, demo, vec![]);
			assert!(
				text.iter().any(|(value, _)| value == "All voice settings"),
				"Changing settings must keep the voice popup open"
			);
			click(&ctx, &mut messaging, demo, egui::pos2(760.0, 760.0));
			let text = frame(&ctx, &mut messaging, demo, vec![]);
			assert!(
				!text.iter().any(|(value, _)| value == "All voice settings"),
				"Clicking outside must close the popup"
			);
		}
	}

	#[test]
	fn gain_sliders_accept_keyboard_input_and_reset_with_the_session() {
		let mut messaging = MessagingUi::default();
		assert_eq!(messaging.voice_gain.input_percent, 100);
		assert_eq!(messaging.voice_gain.output_percent, 100);
		let ctx = egui::Context::default();
		let raw = || egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(240.0, 260.0),
			)),
			..Default::default()
		};
		ctx.run_ui(raw(), |ui| {
			gain_controls(ui, &mut messaging.voice_gain, model::Language::English)[0]
				.request_focus();
		})
		.drop_without_applying_deltas();
		let mut input = raw();
		input.events.push(egui::Event::Key {
			key: egui::Key::ArrowRight,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::NONE,
		});
		ctx.run_ui(input, |ui| {
			let controls = gain_controls(ui, &mut messaging.voice_gain, model::Language::English);
			assert!(
				controls
					.iter()
					.all(|r| r.rect.right() <= ui.max_rect().right() + 1.0)
			);
		})
		.drop_without_applying_deltas();
		assert_eq!(messaging.voice_gain.input_percent, 101);
		assert_eq!(messaging.voice_gain.output_percent, 100);
		assert!(
			!messaging.voice_refresh_devices,
			"Gain does not enumerate devices"
		);
		messaging.voice_gain.input_percent = u16::MAX;
		messaging.voice_gain.output_percent = 0;
		ctx.run_ui(raw(), |ui| {
			gain_controls(ui, &mut messaging.voice_gain, model::Language::English);
		})
		.drop_without_applying_deltas();
		assert_eq!(messaging.voice_gain.input_percent, 200);
		assert_eq!(messaging.voice_gain.output_percent, 0);
		messaging.clear();
		assert_eq!(messaging.voice_gain.input_percent, 100);
		assert_eq!(messaging.voice_gain.output_percent, 100);
	}

	#[test]
	fn guild_voice_requires_explicit_keyboard_join_and_demo_never_emits_media() {
		let mut state = test_support::demo_state();
		state.demo = false;
		assert!(matches!(
			state.select(Id(25)),
			Some(client_core::Command::History {
				channel: Id(25),
				..
			})
		));
		assert_eq!(state.selected, Some(Id(25)));
		assert!(state.voice.active.is_none());
		let mut messaging = MessagingUi {
			voice_available: true,
			..Default::default()
		};
		let context = egui::Context::default();
		let mut commands = vec![];
		context
			.run_ui(Default::default(), |ui| {
				messaging
					.call_button(ui, &mut state, Id(25), &mut commands, false)
					.request_focus();
			})
			.drop_without_applying_deltas();
		assert!(commands.is_empty());
		assert!(state.voice.active.is_none());
		let enter = egui::RawInput {
			events: vec![egui::Event::Key {
				key: egui::Key::Enter,
				physical_key: None,
				pressed: true,
				repeat: false,
				modifiers: egui::Modifiers::NONE,
			}],
			..Default::default()
		};
		context
			.run_ui(enter, |ui| {
				messaging.call_button(ui, &mut state, Id(25), &mut commands, false);
			})
			.drop_without_applying_deltas();
		assert!(matches!(
			commands.as_slice(),
			[Command::Voice(client_core::voice::Command::Join {
				channel: Id(25),
				ring: false,
				..
			})]
		));
		let call = state.voice.active.as_mut().unwrap();
		assert!(elapsed_label(call).is_none());
		call.connected_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(3663));
		call.phase = Phase::Waiting;
		assert_eq!(elapsed_label(call).as_deref(), Some("01:01:03"));
		call.phase = Phase::Failed;
		assert!(elapsed_label(call).is_none());
		state.demo = true;
		for width in [640.0, 1120.0] {
			let mut output = context.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(width, 480.0),
					)),
					..Default::default()
				},
				|ui| {
					assert!(
						messaging
							.show(ui, &mut state)
							.iter()
							.all(|c| !matches!(c, Command::Voice(_) | Command::History { .. }))
					);
				},
			);
			output.textures_delta.clear();
		}
		assert!(messaging.take_avatar_requests().is_empty());
		state.voice.active = None;
		assert!(messaging.call_unavailable(&state, Id(25)).is_some());
		state.demo = false;
		messaging.voice_available = false;
		assert!(
			messaging
				.call_unavailable(&state, Id(25))
				.unwrap()
				.contains("Voice is unavailable")
		);
	}

	#[test]
	fn voice_roster_marks_streaming_participants_live() {
		let mut state = test_support::demo_state();
		state.voice.roster = vec![RosterEntry {
			guild: Id(10),
			channel: Id(25),
			participant: client_core::voice::Participant {
				user: Id(1),
				muted: false,
				deafened: false,
				server_muted: false,
				server_deafened: false,
				video: false,
				streaming: true,
			},
			member: Some(model::Member {
				user: model::User {
					id: Id(1),
					name: "i play baal".into(),
					avatar: None,
					webhook: false,
					kind: Default::default(),
					discriminator: 0,
					primary_guild: None,
				},
				nick: None,
				roles: vec![],
				status: None,
				custom_status: None,
				activities: vec![],
				clients: model::ClientPlatforms::default(),
			}),
		}];
		let mut messaging = MessagingUi::default();
		let ctx = egui::Context::default();
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(190.0, 120.0),
				)),
				..Default::default()
			},
			|ui| {
				messaging.voice_participant(ui, &state, &state.voice.roster[0], false);
			},
		);
		let texts: Vec<_> = output
			.shapes
			.iter()
			.filter_map(|shape| match &shape.shape {
				egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
				_ => None,
			})
			.collect();
		assert!(
			texts.contains(&"LIVE"),
			"Streamers get a LIVE pill: {texts:?}"
		);
		assert!(
			texts.iter().any(|text| text.contains("i play baal")),
			"The name stays alongside the pill: {texts:?}"
		);
		output.drop_without_applying_deltas();
		state.voice.roster[0].participant.streaming = false;
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(190.0, 120.0),
				)),
				..Default::default()
			},
			|ui| {
				messaging.voice_participant(ui, &state, &state.voice.roster[0], false);
			},
		);
		assert!(
			!output.shapes.iter().any(|shape| matches!(
				&shape.shape,
				egui::Shape::Text(text) if text.galley.job.text == "LIVE"
			)),
			"Idle participants keep a plain row"
		);
		output.drop_without_applying_deltas();
	}

	#[test]
	fn voice_roster_preserves_status_space_with_long_names_and_virtualizes() {
		let mut state = State {
			demo: true,
			selected: Some(Id(25)),
			..Default::default()
		};
		state.voice.roster = (1..=64)
			.map(|id| RosterEntry {
				guild: Id(10),
				channel: Id(25),
				participant: client_core::voice::Participant {
					user: Id(id),
					muted: true,
					deafened: true,
					server_muted: false,
					server_deafened: false,
					video: false,
					streaming: false,
				},
				member: Some(model::Member {
					user: model::User {
						id: Id(id),
						name: "Long synthetic participant name ".repeat(5),
						avatar: None,
						webhook: false,
						kind: Default::default(),
						discriminator: 0,
						primary_guild: None,
					},
					nick: None,
					roles: vec![],
					status: None,
					custom_status: None,
					activities: vec![],
					clients: model::ClientPlatforms::default(),
				}),
			})
			.collect();
		let mut messaging = MessagingUi::default();
		let ctx = egui::Context::default();
		for theme in [egui::Theme::Light, egui::Theme::Dark] {
			ctx.set_theme(theme);
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(190.0, 320.0),
					)),
					..Default::default()
				},
				|ui| {
					let width = ui.available_width();
					let row = ui.scope(|ui| {
						messaging.voice_participant(ui, &state, &state.voice.roster[0], false);
					});
					assert!(
						row.response.rect.width() <= width + 1.0,
						"Long names must not displace the mute/deafen icons"
					);
					messaging.voice_channel(ui, &mut state, Id(25), &mut vec![]);
				},
			);
			assert!(
				output.textures_delta.set.len() < 20,
				"Only visible avatars should be loaded"
			);
			output.textures_delta.clear();
		}
		assert!(messaging.take_avatar_requests().is_empty());
	}

	#[test]
	fn viewing_an_incoming_call_never_answers_and_preview_cannot_call() {
		let mut state = State {
			demo: true,
			selected: Some(Id(1)),
			auth: AuthState::Authenticated,
			gateway_connected: true,
			channels: vec![model::Channel {
				last_message: None,
				id: Id(1),
				guild: None,
				parent_id: None,
				position: 0,
				name: "Synthetic DM".into(),
				kind: 1,
				recipients: vec![model::User {
					id: Id(2),
					name: "Synthetic peer".into(),
					avatar: None,
					webhook: false,
					kind: Default::default(),
					discriminator: 0,
					primary_guild: None,
				}],
				member_list_id: None,
				message_count: None,
				icon: None,
			}],
			..Default::default()
		};
		state.voice.incoming = Some(Id(1));
		let mut messaging = MessagingUi {
			voice_available: true,
			..Default::default()
		};
		let context = egui::Context::default();
		let output = context.run_ui(Default::default(), |ui| {
			let commands = messaging.show(ui, &mut state);
			assert!(
				commands
					.iter()
					.all(|command| !matches!(command, Command::Voice(_)))
			);
		});
		output.drop_without_applying_deltas();
		assert!(state.voice.active.is_none());
		assert_eq!(state.voice.incoming, Some(Id(1)));
		assert!(messaging.call_unavailable(&state, Id(1)).is_some());
		state.demo = false;
		let output = context.run_ui(Default::default(), |ui| {
			let commands = messaging.show(ui, &mut state);
			assert!(
				commands
					.iter()
					.all(|command| !matches!(command, Command::Voice(_)))
			);
		});
		output.drop_without_applying_deltas();
		assert!(
			state.voice.active.is_none(),
			"Incoming calls require an explicit answer"
		);
		messaging.voice_available = false;
		assert!(
			messaging
				.call_unavailable(&state, Id(1))
				.unwrap()
				.contains("Voice is unavailable")
		);
		messaging.voice_available = true;
		assert!(messaging.call_unavailable(&state, Id(1)).is_none());
	}

	#[test]
	fn reconnect_banner_never_auto_joins_and_click_uses_existing_join() {
		let mut state = test_support::existing_call_demo_state();
		state.demo = false;
		state.auth = AuthState::Authenticated;
		state.gateway_connected = true;
		let mut messaging = MessagingUi {
			voice_available: true,
			reconnect_offer: Some((Id(22), None)),
			..Default::default()
		};
		let ctx = egui::Context::default();
		let frame = |messaging: &mut MessagingUi, state: &mut State, events: Vec<egui::Event>| {
			let mut commands = vec![];
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(720.0, 240.0),
					)),
					events,
					..Default::default()
				},
				|ui| messaging.reconnect_call_banner(ui, state, &mut commands),
			);
			output.textures_delta.clear();
			let mut text = vec![];
			fn labels(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
				match shape {
					egui::Shape::Text(text) => out.push((
						text.galley.job.text.clone(),
						text.galley.rect.translate(text.pos.to_vec2()),
					)),
					egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| labels(shape, out)),
					_ => {}
				}
			}
			for shape in output.shapes {
				labels(&shape.shape, &mut text);
			}
			(text, commands)
		};
		frame(&mut messaging, &mut state, vec![]);
		let (text, commands) = frame(&mut messaging, &mut state, vec![]);
		assert!(commands.is_empty(), "showing the banner must never join");
		assert!(state.voice.active.is_none());
		assert!(text.iter().any(|(label, _)| label == "Reconnect to call"));
		assert!(text.iter().any(|(label, _)| label == "Robin (synthetic)"));
		let reconnect = text
			.iter()
			.find(|(label, _)| label == "Reconnect to call")
			.expect("reconnect control")
			.1
			.center();
		let mut sent = vec![];
		for pressed in [true, false] {
			let (_, commands) = frame(
				&mut messaging,
				&mut state,
				vec![
					egui::Event::PointerMoved(reconnect),
					egui::Event::PointerButton {
						pos: reconnect,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				],
			);
			sent.extend(commands);
		}
		assert!(
			sent.iter().any(|command| matches!(
				command,
				Command::Voice(client_core::voice::Command::Join {
					channel: Id(22),
					ring: false,
					..
				})
			)),
			"reconnect must reuse the existing join command without ringing"
		);
		assert!(state.voice.active.is_some());
		assert!(messaging.reconnect_offer.is_none());
	}
}
