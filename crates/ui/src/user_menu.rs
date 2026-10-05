//! Shared user actions; rendering only records intent, dispatched after borrowed rows finish.
use crate::shortcuts::{Intent, ShortcutView};
use client_core::{Command, State};
use model::{Shortcut, User};

use crate::i18n::interface_language;

#[derive(Clone)]
pub enum Action {
	Note(User),
	Nickname(User),
	Mention(User),
	CloseDm(model::Id),
	Block {
		user: model::Id,
		blocked: bool,
	},
	Mute {
		channel: model::Id,
		muted: bool,
	},
	Shortcut(Intent),
	/// Server moderation, gated by the same rules as Server Settings. The fence
	/// inside `request_server_admin` re-checks permission before anything runs.
	Admin {
		guild: model::Id,
		action: model::server_admin::Action,
	},
}
impl PartialEq for Action {
	fn eq(&self, other: &Self) -> bool {
		match (self, other) {
			(Action::Note(a), Action::Note(b)) => a == b,
			(Action::Nickname(a), Action::Nickname(b)) => a == b,
			(Action::Mention(a), Action::Mention(b)) => a == b,
			(Action::CloseDm(a), Action::CloseDm(b)) => a == b,
			(
				Action::Block {
					user: a,
					blocked: ab,
				},
				Action::Block {
					user: b,
					blocked: bb,
				},
			) => a == b && ab == bb,
			(
				Action::Mute {
					channel: a,
					muted: am,
				},
				Action::Mute {
					channel: b,
					muted: bm,
				},
			) => a == b && am == bm,
			(Action::Shortcut(a), Action::Shortcut(b)) => a == b,
			// The inner server-admin action has no equality; tests match on it.
			(Action::Admin { .. }, Action::Admin { .. }) => false,
			_ => false,
		}
	}
}

/// Nickname being edited from the member menu; kept in egui temp storage so the
/// popup itself stays stateless.
#[derive(Clone)]
struct NickDraft {
	guild: model::Id,
	user: model::Id,
	name: String,
}
impl std::fmt::Debug for Action {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("User menu action")
	}
}

pub(super) fn prepare(action: Action, state: &mut State) -> Option<Command> {
	match action {
		Action::Note(_) | Action::Nickname(_) | Action::Shortcut(_) | Action::Mention(_) => None,
		Action::CloseDm(channel) => state.close_dm(channel),
		Action::Block { user, blocked } => state.set_user_blocked(user, blocked),
		Action::Mute { channel, muted } => state.set_dm_muted(channel, muted),
		Action::Admin { guild, action } => state.request_server_admin(guild, action),
	}
}

pub(super) fn popup(response: &egui::Response, id: egui::Id) -> egui::Popup<'_> {
	let keyboard = response.has_focus()
		&& response
			.ctx
			.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::F10));
	let passive = !response.sense.senses_click();
	let pointer_opened = if passive {
		response.container_secondary_clicked()
	} else {
		response.secondary_clicked()
	};
	let mut popup = if passive {
		egui::Popup::menu(response)
			.open_memory(if pointer_opened {
				Some(egui::SetOpenCommand::Bool(true))
			} else if response.container_clicked() {
				Some(egui::SetOpenCommand::Bool(false))
			} else {
				None
			})
			.at_pointer_fixed()
	} else {
		egui::Popup::context_menu(response)
	}
	// Admin confirmations render on later frames while the menu stays open; every
	// action below still closes the menu explicitly once it fires.
	.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
	.id(id);
	if keyboard {
		popup = popup
			.open_memory(Some(egui::SetOpenCommand::Bool(true)))
			.at_position(response.rect.right_bottom());
	} else if !pointer_opened && egui::Popup::position_of_id(&response.ctx, id).is_none() {
		// Keyboard-opened menus have no remembered pointer position.
		popup = popup.at_position(response.rect.right_bottom());
	}
	popup
}

pub(super) fn show(
	response: &egui::Response,
	state: &State,
	user: &User,
	profile: &mut crate::profiles::ProfileSession,
	action: &mut Option<Action>,
) {
	show_with_pin(response, state, user, profile, action, None);
}

/// `view` is `Some` only where the row is a direct message, so Pin DM stays off member lists.
pub(super) fn show_with_pin(
	response: &egui::Response,
	state: &State,
	user: &User,
	profile: &mut crate::profiles::ProfileSession,
	action: &mut Option<Action>,
	view: Option<ShortcutView<'_>>,
) {
	popup(response, egui::Popup::default_response_id(response)).show(|ui| {
		crate::anim::popup_motion(ui);
		contents(ui, state, user, profile, action, view)
	});
}

/// Guild of the channel the menu was opened from; `None` in DMs and elsewhere
/// without a server, where the admin section stays hidden.
fn admin_guild(state: &State) -> Option<model::Id> {
	state
		.selected
		.and_then(|id| state.channel(id))
		.and_then(|channel| channel.guild)
}

/// Guild member behind the right-clicked user, if their row is loaded.
fn guild_member(state: &State, user: model::Id) -> Option<&model::Member> {
	state
		.members
		.as_ref()?
		.slots
		.iter()
		.filter_map(|slot| slot.as_ref())
		.find_map(|slot| match slot {
			model::MemberSlot::Person(member) if member.user.id == user => Some(member),
			_ => None,
		})
}

fn kick_key(user: model::Id) -> egui::Id {
	egui::Id::unique(("member-admin-kick", user.0))
}

fn nick_key(user: model::Id) -> egui::Id {
	egui::Id::unique(("member-admin-nick", user.0))
}

/// Renders a pending kick confirmation or nickname dialog. Returns true while one
/// is open so the menu itself stays out of the way.
fn pending_admin_dialog(
	ui: &mut egui::Ui,
	state: &State,
	user: &User,
	action: &mut Option<Action>,
) -> bool {
	let ctx = ui.ctx();
	if ctx
		.data(|data| data.get_temp::<bool>(kick_key(user.id)))
		.unwrap_or(false)
	{
		let language = interface_language(ctx);
		let name = user.name.clone();
		match crate::dialog::Confirm::new(
			"member-admin-kick",
			format!("{} {name}?", crate::i18n::text(language, "Kick")),
			crate::i18n::text(
				language,
				"This removes the member from this server. They can rejoin with a new invite.",
			),
		)
		.danger()
		.confirm_label(crate::i18n::text(language, "Kick"))
		.cancel_label(crate::i18n::text(language, "Cancel"))
		.show(ctx)
		{
			Some(crate::dialog::Choice::Confirmed) => {
				ctx.data_mut(|data| data.remove_temp::<bool>(kick_key(user.id)));
				if let Some(guild) = admin_guild(state) {
					*action = Some(Action::Admin {
						guild,
						action: model::server_admin::Action::Kick { user: user.id },
					});
				}
				ui.close();
			}
			Some(crate::dialog::Choice::Cancelled) => {
				ctx.data_mut(|data| data.remove_temp::<bool>(kick_key(user.id)));
			}
			None => {}
		}
		return true;
	}
	if ctx
		.data(|data| data.get_temp::<Option<NickDraft>>(nick_key(user.id)))
		.unwrap_or(None)
		.is_some()
	{
		let mut save = false;
		let mut cancel = false;
		let language = interface_language(ctx);
		crate::dialog::Dialog::new(
			"member-admin-nickname",
			crate::i18n::text(language, "Change Nickname"),
		)
		.width(400.0)
		.show(ctx, |d| {
			d.content(|ui| {
				crate::dialog::label(ui, crate::i18n::text(language, "Nickname"));
				let mut draft: Option<NickDraft> = ctx
					.data(|data| data.get_temp(nick_key(user.id)))
					.unwrap_or(None);
				if let Some(draft) = draft.as_mut() {
					crate::dialog::input(
						ui,
						egui::TextEdit::singleline(&mut draft.name).hint_text(&user.name),
					);
					let ready = !draft.name.chars().any(char::is_control)
						&& state.can_edit_guild_nickname(draft.guild, draft.user);
					if crate::dialog::action(
						ui,
						crate::i18n::text(language, "Save"),
						crate::dialog::Action::Primary,
					)
					.clicked() && ready
					{
						save = true;
					}
					ctx.data_mut(|data| data.insert_temp(nick_key(user.id), Some(draft.clone())));
				}
			});
			d.footer(|ui| {
				if crate::dialog::action(
					ui,
					crate::i18n::text(language, "Cancel"),
					crate::dialog::Action::Neutral,
				)
				.clicked()
				{
					cancel = true;
				}
			});
		});
		if save {
			if let Some(Some(draft)) =
				ctx.data_mut(|data| data.remove_temp::<Option<NickDraft>>(nick_key(user.id)))
			{
				*action = Some(Action::Admin {
					guild: draft.guild,
					action: model::server_admin::Action::SetNickname {
						user: draft.user,
						nick: draft.name,
					},
				});
			}
			ui.close();
		} else if cancel {
			ctx.data_mut(|data| data.remove_temp::<Option<NickDraft>>(nick_key(user.id)));
		}
		return true;
	}
	false
}

pub(super) fn contents(
	ui: &mut egui::Ui,
	state: &State,
	user: &User,
	profile: &mut crate::profiles::ProfileSession,
	action: &mut Option<Action>,
	view: Option<ShortcutView<'_>>,
) {
	let colors = crate::design::palette(ui);
	let language = interface_language(ui.ctx());
	ui.set_min_width(200.0);
	ui.spacing_mut().button_padding = egui::vec2(8.0, 6.0);
	if pending_admin_dialog(ui, state, user, action) {
		return;
	}
	if ui.button(crate::i18n::text(language, "Profile")).clicked() {
		profile.command_open(user.clone());
		ui.close();
	}
	if !user.webhook
		&& state.selected.is_some_and(|id| {
			state
				.channel(id)
				.is_some_and(|channel| channel.supports_text())
		}) && ui.button(crate::i18n::text(language, "Mention")).clicked()
	{
		*action = Some(Action::Mention(user.clone()));
		ui.close();
	}
	if user.webhook || state.user.as_ref().is_some_and(|own| own.id == user.id) {
		return;
	}
	let dm = state
		.channels
		.iter()
		.find(|c| c.guild.is_none() && c.kind == 1 && c.recipients.iter().any(|u| u.id == user.id));
	let enabled = (state.demo || state.gateway_connected) && !state.user_action_pending();
	ui.separator();
	if ui
		.add_enabled(
			enabled,
			egui::Button::new(crate::i18n::text(language, "Add Note")),
		)
		.clicked()
	{
		*action = Some(Action::Note(user.clone()));
		ui.close();
	}
	if ui
		.add_enabled(
			enabled && state.friends().any(|friend| friend.id == user.id),
			egui::Button::new(if state.friend_nickname(user.id).is_some() {
				crate::i18n::text(language, "Edit Friend Nickname")
			} else {
				crate::i18n::text(language, "Add Friend Nickname")
			}),
		)
		.on_disabled_hover_text(crate::i18n::text(
			language,
			"Private nicknames are available for confirmed friends.",
		))
		.clicked()
	{
		*action = Some(Action::Nickname(user.clone()));
		ui.close();
	}
	ui.separator();
	if let Some(dm) = dm {
		if let Some(view) = view {
			let pinned = view.contains(Shortcut::Pinned, dm.id);
			if ui
				.add_enabled(
					view.available(),
					egui::Button::new(if pinned {
						crate::i18n::text(language, "Unpin DM")
					} else {
						crate::i18n::text(language, "Pin DM")
					}),
				)
				.on_hover_text(crate::i18n::text(
					language,
					"Pinned direct messages are saved on this device.",
				))
				.clicked()
			{
				*action = Some(Action::Shortcut(view.toggle(Shortcut::Pinned, dm.id)));
				ui.close();
			}
		}
		let muted = state.dm_muted(dm.id) == Some(true);
		if ui
			.add_enabled(
				enabled,
				egui::Button::new(if muted {
					crate::i18n::text(language, "Unmute Conversation")
				} else {
					crate::i18n::text(language, "Mute Conversation")
				}),
			)
			.on_hover_text(crate::i18n::text(
				language,
				"Mute this direct message's notifications until you unmute it.",
			))
			.clicked()
		{
			*action = Some(Action::Mute {
				channel: dm.id,
				muted: !muted,
			});
			ui.close();
		}
		if ui
			.add_enabled(
				enabled,
				egui::Button::new(crate::i18n::text(language, "Close DM")),
			)
			.on_hover_text(crate::i18n::text(
				language,
				"Remove this conversation from your DM list. Messages are kept.",
			))
			.clicked()
		{
			*action = Some(Action::CloseDm(dm.id));
			ui.close();
		}
	} else {
		ui.add_enabled(
			false,
			egui::Button::new(crate::i18n::text(language, "Mute Conversation")),
		)
		.on_disabled_hover_text(crate::i18n::text(
			language,
			"No open direct message with this user.",
		));
	}
	ui.separator();
	let blocked = state.user_blocked(user.id) == Some(true);
	if ui
		.add_enabled(
			enabled,
			egui::Button::new(
				egui::RichText::new(if blocked {
					crate::i18n::text(language, "Unblock")
				} else {
					crate::i18n::text(language, "Block")
				})
				.color(colors.danger),
			),
		)
		.clicked()
	{
		*action = Some(Action::Block {
			user: user.id,
			blocked: !blocked,
		});
		ui.close();
	}
	if let Some(guild) = admin_guild(state) {
		let member = guild_member(state, user.id);
		let can_nickname = state.can_edit_guild_nickname(guild, user.id);
		let roles: Vec<(model::Id, String)> = state
			.guild_roles(guild)
			.map(|roles| {
				roles
					.iter()
					.filter(|role| state.can_edit_member_role(guild, user.id, role.id))
					.map(|role| (role.id, role.name.clone()))
					.collect()
			})
			.unwrap_or_default();
		let can_kick = state.can_kick_guild_member(guild, user.id);
		if can_nickname || !roles.is_empty() || can_kick {
			ui.separator();
			if can_nickname
				&& ui
					.button(crate::i18n::text(language, "Change Nickname"))
					.clicked()
			{
				ctx_data_insert_nick(ui, guild, user.id, member.and_then(|m| m.nick.clone()));
			}
			if !roles.is_empty() {
				ui.menu_button(crate::i18n::text(language, "Roles"), |ui| {
					let assigned_roles: Vec<model::Id> =
						member.map(|m| m.roles.clone()).unwrap_or_default();
					for (role, name) in &roles {
						let mut assigned = assigned_roles.contains(role);
						if ui
							.add_enabled(
								!state.server_admin.pending,
								egui::Checkbox::new(&mut assigned, name),
							)
							.changed()
						{
							*action = Some(Action::Admin {
								guild,
								action: model::server_admin::Action::SetRole {
									user: user.id,
									role: *role,
									assigned,
								},
							});
							ui.close();
						}
					}
				});
			}
			if can_kick
				&& ui
					.button(
						egui::RichText::new(format!(
							"{} {}",
							crate::i18n::text(language, "Kick"),
							user.name
						))
						.color(colors.danger),
					)
					.clicked()
			{
				ui.ctx()
					.data_mut(|data| data.insert_temp(kick_key(user.id), true));
			}
		}
	}
}

/// Opens the nickname editor for the member menu; split out so the borrow of `ui`
/// for the temp store does not overlap the menu widgets.
fn ctx_data_insert_nick(ui: &egui::Ui, guild: model::Id, user: model::Id, nick: Option<String>) {
	ui.ctx().data_mut(|data| {
		data.insert_temp(
			nick_key(user),
			Some(NickDraft {
				guild,
				user,
				name: nick.unwrap_or_default(),
			}),
		)
	});
}

#[cfg(test)]
mod tests {
	use super::*;
	use egui::{Event, Modifiers, PointerButton, Pos2, Rect};

	fn labels(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
		match shape {
			egui::Shape::Text(t) => out.push((
				t.galley.job.text.clone(),
				t.galley.rect.translate(t.pos.to_vec2()),
			)),
			egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| labels(s, out)),
			_ => {}
		}
	}
	fn pointer(pos: Pos2, button: PointerButton, pressed: bool) -> Vec<Event> {
		vec![
			Event::PointerMoved(pos),
			Event::PointerButton {
				pos,
				button,
				pressed,
				modifiers: Modifiers::NONE,
			},
		]
	}
	fn frame(
		ctx: &egui::Context,
		state: &State,
		user: &User,
		events: Vec<Event>,
		profile: &mut crate::profiles::ProfileSession,
		action: &mut Option<Action>,
	) -> (egui::Response, Vec<(String, Rect)>) {
		let mut response = None;
		let mut output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(320.0, 420.0))),
				events,
				..Default::default()
			},
			|ui| {
				let row = ui.button(&user.name);
				show(&row, state, user, profile, action);
				response = Some(row);
			},
		);
		output.textures_delta.clear();
		let mut text = vec![];
		for shape in output.shapes {
			labels(&shape.shape, &mut text);
		}
		(response.unwrap(), text)
	}

	#[test]
	fn user_menu_mouse_keyboard_and_actions_in_both_themes() {
		for light in [false, true] {
			for label in [
				"Profile",
				"Mention",
				"Mute Conversation",
				"Close DM",
				"Block",
			] {
				let ctx = egui::Context::default();
				ctx.set_visuals(if light {
					egui::Visuals::light()
				} else {
					egui::Visuals::dark()
				});
				let state = test_support::demo_state();
				let dm = state.channels.iter().find(|c| c.kind == 1).unwrap();
				let user = &dm.recipients[0];
				let (mut profile, mut action) = (crate::profiles::ProfileSession::default(), None);
				let (row, _) = frame(&ctx, &state, user, vec![], &mut profile, &mut action);
				if light {
					row.request_focus();
					frame(
						&ctx,
						&state,
						user,
						vec![Event::Key {
							key: egui::Key::F10,
							physical_key: None,
							pressed: true,
							repeat: false,
							modifiers: Modifiers::SHIFT,
						}],
						&mut profile,
						&mut action,
					);
				} else {
					for pressed in [true, false] {
						frame(
							&ctx,
							&state,
							user,
							pointer(row.rect.center(), PointerButton::Secondary, pressed),
							&mut profile,
							&mut action,
						);
					}
				}
				let (_, text) = frame(&ctx, &state, user, vec![], &mut profile, &mut action);
				assert!(profile.open_user().is_none() && action.is_none());
				for expected in [
					"Profile",
					"Mention",
					"Add Note",
					"Add Friend Nickname",
					"Mute Conversation",
					"Close DM",
					"Block",
				] {
					let rect = text
						.iter()
						.find(|(s, _)| s == expected)
						.unwrap_or_else(|| panic!("Missing {expected}: {text:?}"))
						.1;
					assert!(
						Rect::from_min_size(Pos2::ZERO, egui::vec2(320.0, 420.0))
							.contains_rect(rect)
					);
				}
				let pos = text.iter().find(|(s, _)| s == label).unwrap().1.center();
				for pressed in [true, false] {
					frame(
						&ctx,
						&state,
						user,
						pointer(pos, PointerButton::Primary, pressed),
						&mut profile,
						&mut action,
					);
				}
				match label {
					"Profile" => assert_eq!(profile.open_user().unwrap().id, user.id),
					"Mention" => assert_eq!(action, Some(Action::Mention(user.clone()))),
					"Mute Conversation" => assert_eq!(
						action,
						Some(Action::Mute {
							channel: dm.id,
							muted: true
						})
					),
					"Close DM" => assert_eq!(action, Some(Action::CloseDm(dm.id))),
					_ => assert_eq!(
						action,
						Some(Action::Block {
							user: user.id,
							blocked: true
						})
					),
				}
				assert!(!egui::Popup::is_any_open(&ctx));
			}
		}
	}

	fn admin_state(target_position: i32) -> (State, model::Id, model::User) {
		use model::permissions as p;
		let guild = model::Id(10);
		let role = |id: u64, bits: u128, position: i32| p::Role {
			id: model::Id(id),
			bits,
			name: format!("role{id}"),
			color: 0,
			position,
			hoist: false,
		};
		let target = model::User {
			id: model::Id(3),
			name: "Target".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
		};
		let mut state = test_support::demo_state();
		state.user = Some(model::User {
			id: model::Id(2),
			name: "Moderator".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
		});
		state.selected = Some(model::Id(20));
		state.permissions.guilds.insert(
			guild,
			p::Guild {
				id: guild,
				owner: Some(model::Id(1)),
				member: Some(p::Member {
					roles: vec![model::Id(30)],
					timeout_until: None,
				}),
				roles: Some(vec![
					role(10, 0, 0),
					role(
						30,
						p::KICK_MEMBERS | p::MANAGE_ROLES | p::MANAGE_NICKNAMES,
						5,
					),
					role(31, 0, target_position),
				]),
			},
		);
		state.server_admin.guild = Some(guild);
		state.server_admin.members = Some(model::server_admin::Members {
			items: vec![model::server_admin::Member {
				user: target.clone(),
				nick: None,
				roles: vec![model::Id(31)],
				joined_at: None,
				join_source: None,
				invite_code: None,
				flags: None,
				unusual_dm_until: None,
				timeout_until: None,
			}],
			roles: vec![
				model::server_admin::Role {
					role: role(30, p::KICK_MEMBERS | p::MANAGE_ROLES, 5),
					managed: false,
				},
				model::server_admin::Role {
					role: role(31, 0, target_position),
					managed: false,
				},
			],
			total: 1,
			..Default::default()
		});
		(state, guild, target)
	}

	fn open_menu(
		ctx: &egui::Context,
		state: &State,
		user: &model::User,
	) -> (
		crate::profiles::ProfileSession,
		Option<Action>,
		Vec<(String, Rect)>,
	) {
		let (mut profile, mut action) = (crate::profiles::ProfileSession::default(), None);
		let (row, _) = frame(ctx, state, user, vec![], &mut profile, &mut action);
		for pressed in [true, false] {
			frame(
				ctx,
				state,
				user,
				pointer(row.rect.center(), PointerButton::Secondary, pressed),
				&mut profile,
				&mut action,
			);
		}
		let (_, text) = frame(ctx, state, user, vec![], &mut profile, &mut action);
		(profile, action, text)
	}

	fn click_label(
		ctx: &egui::Context,
		state: &State,
		user: &model::User,
		profile: &mut crate::profiles::ProfileSession,
		action: &mut Option<Action>,
		text: &[(String, Rect)],
		label: &str,
	) {
		let pos = text
			.iter()
			.find(|(s, _)| s == label)
			.unwrap_or_else(|| panic!("Missing {label}: {text:?}"))
			.1
			.center();
		for pressed in [true, false] {
			frame(
				ctx,
				state,
				user,
				pointer(pos, PointerButton::Primary, pressed),
				profile,
				action,
			);
		}
	}

	#[test]
	fn member_without_permission_sees_no_admin_section() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		state.selected = Some(model::Id(20));
		let target = model::User {
			id: model::Id(3),
			name: "Target".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
		};
		let (_, action, text) = open_menu(&ctx, &state, &target);
		assert!(action.is_none());
		for hidden in ["Roles", "Change Nickname"] {
			assert!(
				text.iter().all(|(s, _)| s != hidden),
				"unexpected {hidden}: {text:?}"
			);
		}
		assert!(
			text.iter().all(|(s, _)| !s.starts_with("Kick ")),
			"unexpected kick entry: {text:?}"
		);
	}

	#[test]
	fn kick_respects_role_hierarchy() {
		let ctx = egui::Context::default();
		let (state, _, target) = admin_state(1);
		let (_, action, text) = open_menu(&ctx, &state, &target);
		assert!(action.is_none());
		assert!(text.iter().any(|(s, _)| s == "Roles"));
		assert!(text.iter().any(|(s, _)| s == "Change Nickname"));
		assert!(text.iter().any(|(s, _)| s == "Kick Target"));
		let (higher, _, _) = admin_state(10);
		let (_, action, text) = open_menu(&ctx, &higher, &target);
		assert!(action.is_none());
		assert!(
			text.iter().all(|(s, _)| !s.starts_with("Kick ")),
			"hierarchy ignored: {text:?}"
		);
	}

	#[test]
	fn kick_requires_confirmation_before_dispatch() {
		let ctx = egui::Context::default();
		let (state, guild, target) = admin_state(1);
		let (mut profile, mut action) = (crate::profiles::ProfileSession::default(), None);
		let (row, _) = frame(&ctx, &state, &target, vec![], &mut profile, &mut action);
		for pressed in [true, false] {
			frame(
				&ctx,
				&state,
				&target,
				pointer(row.rect.center(), PointerButton::Secondary, pressed),
				&mut profile,
				&mut action,
			);
		}
		let (_, text) = frame(&ctx, &state, &target, vec![], &mut profile, &mut action);
		click_label(
			&ctx,
			&state,
			&target,
			&mut profile,
			&mut action,
			&text,
			"Kick Target",
		);
		assert!(action.is_none(), "kick dispatched without confirmation");
		let (_, text) = frame(&ctx, &state, &target, vec![], &mut profile, &mut action);
		assert!(text.iter().any(|(s, _)| s == "Kick Target?"));
		click_label(
			&ctx,
			&state,
			&target,
			&mut profile,
			&mut action,
			&text,
			"Kick",
		);
		assert!(
			matches!(
				action,
				Some(Action::Admin {
					guild: g,
					action: model::server_admin::Action::Kick { user },
				}) if g == guild && user == target.id
			),
			"expected confirmed kick, got {action:?}"
		);
	}

	#[test]
	fn admin_menu_renders_fully_translated_in_portuguese() {
		let ctx = egui::Context::default();
		let _ = crate::i18n::drain_untranslated_keys();
		crate::i18n::store_interface_language(&ctx, model::Language::PortugueseBrazil);
		let (state, _, target) = admin_state(1);
		let (_, action, text) = open_menu(&ctx, &state, &target);
		assert!(action.is_none());
		let rendered: Vec<&str> = text.iter().map(|(label, _)| label.as_str()).collect();
		for expected in [
			"Perfil",
			"Mudar apelido",
			"Cargos",
			"Expulsar Target",
			"Bloquear",
		] {
			assert!(
				rendered.contains(&expected),
				"missing {expected}: {rendered:?}"
			);
		}
		assert!(
			!rendered.contains(&"Kick Target"),
			"untranslated label leaked: {rendered:?}"
		);
		let missing = crate::i18n::drain_untranslated_keys();
		assert!(missing.is_empty(), "untranslated menu keys: {missing:?}");
	}

	fn pin_frame(
		ctx: &egui::Context,
		state: &State,
		user: &User,
		events: Vec<Event>,
		profile: &mut crate::profiles::ProfileSession,
		action: &mut Option<Action>,
		prefs: &model::ChannelPreferences,
	) -> (egui::Response, Vec<(String, Rect)>) {
		let mut response = None;
		let mut output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(320.0, 420.0))),
				events,
				..Default::default()
			},
			|ui| {
				let row = ui.button(&user.name);
				show_with_pin(
					&row,
					state,
					user,
					profile,
					action,
					Some(crate::shortcuts::ShortcutView::new(prefs, true)),
				);
				response = Some(row);
			},
		);
		output.textures_delta.clear();
		let mut text = vec![];
		for shape in output.shapes {
			labels(&shape.shape, &mut text);
		}
		(response.unwrap(), text)
	}

	#[test]
	fn every_plain_option_closes_the_menu() {
		let ctx = egui::Context::default();
		let state = test_support::demo_state();
		let peer = state
			.channels
			.iter()
			.find(|c| c.kind == 1)
			.expect("dm")
			.recipients[0]
			.clone();
		let friend = state.friends().next().expect("friend").clone();
		let prefs = model::ChannelPreferences {
			favorites: vec![],
			pinned: vec![],
			collapsed_categories: vec![],
		};
		for (label, target, pin) in [
			("Profile", &peer, false),
			("Mention", &peer, false),
			("Add Note", &peer, false),
			("Add Friend Nickname", &friend, false),
			("Pin DM", &peer, true),
			("Mute Conversation", &peer, false),
			("Close DM", &peer, false),
			("Block", &peer, false),
		] {
			let mut profile = crate::profiles::ProfileSession::default();
			let mut action = None;
			let mut open = |events: Vec<Event>| {
				if pin {
					pin_frame(
						&ctx,
						&state,
						target,
						events,
						&mut profile,
						&mut action,
						&prefs,
					)
					.0
				} else {
					frame(&ctx, &state, target, events, &mut profile, &mut action).0
				}
			};
			let row = open(vec![]);
			for pressed in [true, false] {
				open(pointer(
					row.rect.center(),
					PointerButton::Secondary,
					pressed,
				));
			}
			let (_, text) = if pin {
				pin_frame(
					&ctx,
					&state,
					target,
					vec![],
					&mut profile,
					&mut action,
					&prefs,
				)
			} else {
				frame(&ctx, &state, target, vec![], &mut profile, &mut action)
			};
			let pos = text
				.iter()
				.find(|(s, _)| s == label)
				.unwrap_or_else(|| panic!("Missing {label}: {text:?}"))
				.1
				.center();
			for pressed in [true, false] {
				if pin {
					pin_frame(
						&ctx,
						&state,
						target,
						pointer(pos, PointerButton::Primary, pressed),
						&mut profile,
						&mut action,
						&prefs,
					);
				} else {
					frame(
						&ctx,
						&state,
						target,
						pointer(pos, PointerButton::Primary, pressed),
						&mut profile,
						&mut action,
					);
				}
			}
			if label == "Profile" {
				assert_eq!(profile.open_user().unwrap().id, target.id);
			} else {
				assert!(action.is_some(), "{label} click did nothing");
			}
			assert!(
				!egui::Popup::is_any_open(&ctx),
				"{label} left the menu open"
			);
		}
	}

	#[test]
	fn dm_row_and_avatar_open_menu_without_navigation_and_dispatch_once() {
		for avatar in [false, true] {
			let ctx = egui::Context::default();
			let mut view = crate::MessagingUi::default();
			let mut state = test_support::demo_state();
			let selected = state.selected;
			let render = |view: &mut crate::MessagingUi, state: &mut State, events| {
				let mut commands = vec![];
				let mut output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(Rect::from_min_size(
							Pos2::ZERO,
							egui::vec2(1000.0, 700.0),
						)),
						events,
						..Default::default()
					},
					|ui| {
						view.channel_list(ui, state);
						if let Some(action) = view.user_action.take()
							&& let Some(command) = prepare(action, state)
						{
							commands.push(command);
						}
					},
				);
				output.textures_delta.clear();
				let mut text = vec![];
				for shape in output.shapes {
					labels(&shape.shape, &mut text);
				}
				(commands, text)
			};
			render(&mut view, &mut state, vec![]);
			let (_, text) = render(&mut view, &mut state, vec![]);
			let name = text
				.iter()
				.find(|(s, _)| s == "Robin (synthetic)")
				.unwrap()
				.1;
			let pos = if avatar {
				egui::pos2(name.left() - 28.0, name.center().y)
			} else {
				name.center()
			};
			for pressed in [true, false] {
				let (commands, _) = render(
					&mut view,
					&mut state,
					pointer(pos, PointerButton::Secondary, pressed),
				);
				assert!(commands.is_empty());
			}
			let (_, text) = render(&mut view, &mut state, vec![]);
			assert_eq!(state.selected, selected);
			assert!(view.profile.open_user().is_none());
			let pos = text
				.iter()
				.find(|(s, _)| s == "Close DM")
				.unwrap()
				.1
				.center();
			let mut writes = 0;
			for pressed in [true, false] {
				let (commands, _) = render(
					&mut view,
					&mut state,
					pointer(pos, PointerButton::Primary, pressed),
				);
				writes += commands
					.iter()
					.filter(|c| {
						matches!(
							c,
							Command::UserAction {
								action: client_core::user_actions::Action::CloseDm(model::Id(22)),
								..
							}
						)
					})
					.count();
			}
			assert_eq!(writes, 1);
			assert!(
				state.channels.iter().any(|c| c.id == model::Id(22)),
				"Wait for transport confirmation"
			);
			assert!(render(&mut view, &mut state, vec![]).0.is_empty());
		}
	}
}
