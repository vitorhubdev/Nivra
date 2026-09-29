#![allow(clippy::items_after_test_module)]
//! Native egui views; emits commands without owning transports or session credentials.
mod account_badge;
mod account_menu;
pub mod anim;
pub mod text_preview;

#[cfg(test)]
mod preview_tests {
	use super::*;

	fn sample() -> text_preview::TextPreview {
		text_preview::TextPreview {
			filename: "notes.md".into(),
			format: text_preview::PreviewFormat::Markdown,
			text: "# Title\n\nBody text".into(),
			truncated: false,
		}
	}

	#[test]
	fn preview_dialog_renders_and_closes() {
		let ctx = egui::Context::default();
		let mut view = MessagingUi::default();
		assert!(view.preview.is_none());
		view.set_preview(sample());
		ctx.run_ui(Default::default(), |_ui| {
			view.show_preview(&ctx);
		})
		.drop_without_applying_deltas();
		assert!(
			view.preview.is_some(),
			"an open preview stays until it is closed"
		);
		view.close_preview();
		assert!(view.preview.is_none());
	}
}
mod archives;
mod audio;
pub mod batch_select;
mod forwarding;
pub use audio::{AudioCommand, AudioState, AudioUi};
mod video;
pub use video::{VideoCommand, VideoState, VideoUi};
mod attachments;
pub use attachments::DownloadUi;
mod avatars;
pub use avatars::media::{Lane, MAX_FRAMES, Motion, Rendition, Size, fit_edge, is_motion_video};
pub use avatars::{EMBED_EDGE, GifFrames};
mod categories;
#[cfg(feature = "demo")]
pub use categories::debug_thread_navigation_check;
#[cfg(feature = "demo")]
pub use timeline::debug_unread_navigation_check;
mod channel_marks;
mod channel_menu;
mod channel_permissions;
#[cfg(test)]
mod channel_welcome_tests;
mod components;
mod composer_text;
pub mod design;
mod embeds;
pub use embeds::{WebMediaRequest, take_web_media_request};
mod extension_account_actions;
mod extension_actions;
mod extension_admin_actions;
mod extension_app;
mod extension_server_actions;
mod extensions_ui;
mod theme_editor;
mod thread_create;
pub use extensions_ui::{ExtensionContext, ExtensionEntry, ExtensionRequest, ExtensionUi};
pub mod emoji;
mod emoji_details;
mod emoji_picker;
pub mod fonts;
mod formatting;
mod forum;
mod friends;
mod group_menu;
mod guild_folders;
mod highlight;
pub mod i18n;
pub mod icons;
mod invites;
pub mod licenses;
mod local_time;
mod markdown;
mod member_search;
mod mentions;
mod messaging_permissions;
mod notification_settings;
mod notifications;
mod pending;
mod post_menu;
mod profiles;
mod slash_builtin;
mod slash_commands;
mod stickers;
/// Synthetic global profile used exclusively by the desktop's offline command adapter.
#[cfg(any(test, feature = "demo"))]
pub fn synthetic_own_profile(user: &model::User) -> model::UserProfile {
	profiles::synthetic(user, None)
}
mod contact_editor;
pub mod dialog;
mod join_server;
mod keybinds;
mod profile_edit;
mod reactions;
mod reading;
pub mod screen;
pub mod scroll;
mod search;
pub mod select;
mod server_admin;
mod server_audit_log;
mod server_integrations;
mod server_invite;
mod server_invites;
mod server_menu;
mod server_roles;
mod server_settings;
mod server_stickers;
mod settings;
mod shortcuts;
mod switcher;
mod thumbhash;
mod timeline;
#[cfg(test)]
mod title_bar_tests;
pub mod toasts;
mod typing;
pub mod updates;
mod user_menu;
mod verification;
mod voice;
use client_core::{Command, MAX_CONTENT, MAX_DRAFT_BYTES, NavStep, State};
use egui::{RichText, TextEdit};
use model::{Freshness, Id};
pub use verification::VerificationUi;
pub use voice::StageFocus;
pub use voice::voice_server_place;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VoiceGain {
	pub input_percent: u16,
	pub output_percent: u16,
}
impl Default for VoiceGain {
	fn default() -> Self {
		Self {
			input_percent: 100,
			output_percent: 100,
		}
	}
}

pub struct AttachmentPaste {
	pub target: egui::Id,
	pub text: Option<String>,
	pub image: Option<std::sync::Arc<egui::ColorImage>>,
}

/// Over-limit composer text waiting in the rename dialog.
pub struct LongTextDialog {
	pub channel: Id,
	pub text: String,
	pub filename: String,
}

/// Over-limit composer text staged as a `.txt` attachment. The desktop upload
/// wiring reads `bytes` when it resolves the staged filename.
pub struct LongTextAttachment {
	pub channel: Id,
	pub filename: String,
	pub bytes: Vec<u8>,
}

/// Default staged filename for over-limit text.
pub const LONG_TEXT_DEFAULT_FILENAME: &str = "message.txt";

/// Appends `.txt` when missing so the staged file always has the text extension.
/// Returns `None` for blank names.
pub fn normalize_txt_filename(name: &str) -> Option<String> {
	let trimmed = name.trim();
	if trimmed.is_empty() {
		return None;
	}
	Some(if trimmed.ends_with(".txt") {
		trimmed.to_owned()
	} else {
		format!("{trimmed}.txt")
	})
}

/// Filename rules mirrored from the send path: bounded, no directories, no
/// control characters, and always a `.txt` file with a non-empty stem.
pub(crate) fn valid_txt_filename(name: &str) -> bool {
	!name.trim().is_empty()
		&& name.len() <= 256
		&& !matches!(name, "." | "..")
		&& name.ends_with(".txt")
		&& name.trim_end_matches(".txt").trim().len() > 0
		&& name
			.chars()
			.all(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':'))
}

fn thread_member_rows<'a>(
	state: &'a State,
	list: &'a model::MemberList,
) -> Vec<(std::borrow::Cow<'a, model::MemberSlot>, Option<u64>)> {
	use std::borrow::Cow;
	let mut members: Vec<_> = list
		.slots
		.iter()
		.flatten()
		.filter_map(|slot| {
			let model::MemberSlot::Person(member) = slot else {
				return None;
			};
			let offline = !member_online(state, member, list.guild);
			let role = list
				.guild
				.and_then(|guild| state.member_roles(guild, member).0)
				.filter(|_| !offline);
			Some((slot, offline, role))
		})
		.collect();
	members.sort_by(|a, b| {
		a.1.cmp(&b.1).then_with(|| match (a.2, b.2) {
			(Some(a), Some(b)) => b.cmp_hierarchy(a),
			(Some(_), None) => std::cmp::Ordering::Less,
			(None, Some(_)) => std::cmp::Ordering::Greater,
			_ => std::cmp::Ordering::Equal,
		})
	});
	let mut rows = Vec::with_capacity(members.len() * 2);
	for group in members.chunk_by(|a, b| a.1 == b.1 && a.2.map(|r| r.id) == b.2.map(|r| r.id)) {
		let id = if group[0].1 {
			"offline".into()
		} else {
			group[0]
				.2
				.map_or_else(|| "online".into(), |role| role.id.to_string())
		};
		rows.push((
			Cow::Owned(model::MemberSlot::Group(id)),
			Some(group.len() as u64),
		));
		rows.extend(group.iter().map(|row| (Cow::Borrowed(row.0), None)));
	}
	rows
}

fn member_online(state: &State, member: &model::Member, guild: Option<Id>) -> bool {
	profiles::member_presence(state, member, guild)
		.0
		.is_some_and(|status| matches!(status, "online" | "idle" | "dnd"))
}

/// Rows still drawn while offline people are hidden. People after the Offline header who are
/// not online are skipped; every header stays, so the Offline count still shows who is hidden.
fn visible_member_rows<'a>(
	rows: impl Iterator<Item = Option<&'a model::MemberSlot>>,
	online: impl Fn(&model::Member) -> bool,
) -> Vec<usize> {
	let mut offline = false;
	rows.enumerate()
		.filter_map(|(index, slot)| {
			match slot {
				Some(model::MemberSlot::Group(id)) => offline = id == "offline",
				Some(model::MemberSlot::Person(member)) if offline && !online(member) => {
					return None;
				}
				_ => {}
			}
			Some(index)
		})
		.collect()
}

/// First absolute row and length of the Offline section in a lazy list. Each gateway group is
/// one header row followed by its members, matching how the gateway computes the list total.
fn lazy_offline_rows(groups: &[(String, u64)]) -> Option<(usize, usize)> {
	let mut header = 0usize;
	for (id, count) in groups {
		let count = usize::try_from(*count).ok()?;
		if id == "offline" {
			return (count > 0).then_some((header.saturating_add(1), count));
		}
		header = header.saturating_add(count).saturating_add(1);
	}
	None
}

#[derive(Default)]
pub struct MessagingUi {
	forwarding: forwarding::ForwardDialog,
	pub image_sharing_enabled: bool,
	pub image_share_requested: Option<model::ImageShare>,
	pub interaction_file_request: Option<String>,
	interaction_components: components::Components,
	pub verification: VerificationUi,
	pub extensions: ExtensionUi,
	friends: friends::Friends,
	account_menu: account_menu::AccountMenu,
	pub own_presence: model::OwnPresence,
	/// When the custom status clears itself, in milliseconds since the Unix epoch. The
	/// deadline is local: Discord carries it in account settings, which this client does
	/// not write, so the host enforces it and republishes the cleared presence.
	pub own_presence_expires: Option<u64>,
	pub own_presence_changed: bool,
	pub own_presence_status: &'static str,
	group_menu: group_menu::GroupMenu,
	server_menu: server_menu::ServerMenu,
	channel_menu: channel_menu::ChannelMenu,
	pub channel_preferences: model::ChannelPreferences,
	pub channel_preferences_changed: bool,
	pub channel_preferences_loaded: bool,
	pub channel_preferences_load_pending: bool,
	pub channel_preferences_reload: bool,
	pub channel_preferences_save_pending: bool,
	pub channel_preferences_status: &'static str,
	join_server: join_server::JoinDialog,
	folder_ui: guild_folders::FolderUi,
	rail_cache: notifications::RailCache,
	composer_layout: composer_text::Layout,
	channel_cache: categories::Cache,
	channel_move: Option<(Id, client_core::channel_actions::Action)>,
	voice_member_move: Option<(Id, model::server_admin::Action)>,
	search: search::SearchUi,
	settings: settings::Settings,
	server_settings: server_settings::Editor,
	server_icon_sequence: u64,
	server_emoji_sequence: u64,
	server_sticker_sequence: u64,
	server_role_icon_sequence: u64,
	switcher: switcher::Switcher,
	focus_switched_composer: bool,
	#[cfg(feature = "demo")]
	preview_slash_commands: bool,
	switcher_frame: bool,
	archives: archives::ArchivesUi,
	archive_parent: Option<Id>,
	thread_create: thread_create::ThreadCreateUi,
	forum: forum::ForumUi,
	scroll: scroll::Session,
	side_press: scroll::SidePress,
	timeline: timeline::TimelineView,
	edit_modified: Option<(Id, Id, bool)>,
	edit_undo_cleared: bool,
	edit_widget_id: Option<egui::Id>,
	edit_closed_channel: Option<Id>,
	avatars: avatars::Avatars,
	profile: profiles::ProfileSession,
	profile_image: Option<(u64, model::Attachment)>,
	user_action: Option<user_menu::Action>,
	pending_mention: Option<Id>,
	contact_editor: contact_editor::ContactEditor,
	profile_link: Option<String>,
	profile_formatted: markdown::FormatCache,
	pub reading_preferences: model::ReadingPreferences,
	pub show_hidden_channels: bool,
	/// Direct messages with bots stay visible until this is turned on.
	pub hide_bot_dms: bool,
	/// Offline people stay listed until this is turned on; the Offline header keeps its count.
	pub hide_offline_members: bool,
	pub hide_title_bar: bool,
	/// Device-local UI language. English is the migration/default locale.
	pub language: model::Language,
	/// Which GPU renders the window; the running adapter only changes on restart.
	pub gpu_preference: model::GpuPreference,
	/// Adapter currently in use, shown next to the preference for bug reports.
	pub gpu_adapter: String,
	pub reading_status: &'static str,
	pub reading_save_requested: bool,
	pub minimize_to_tray: bool,
	pub tray_available: bool,
	pub tray_status: &'static str,
	pub startup_enabled: bool,
	pub startup_minimized: bool,
	pub startup_available: bool,
	pub startup_busy: bool,
	pub startup_status: &'static str,
	pub startup_disable_requested: bool,
	pub discord_activity_sharing: Option<bool>,
	pub discord_activity_sharing_busy: bool,
	pub discord_activity_sharing_retry: bool,
	/// false checks the account setting; true explicitly enables it.
	pub discord_activity_sharing_request: Option<bool>,
	pub share_game_activity: bool,
	pub own_game: Option<String>,
	pub game_activity_status: &'static str,
	reading_sidebar_applied: Option<u16>,
	reading_sidebar_constrained: bool,
	reading_zoom_pending: bool,
	/// Slider value while the pointer is still down; zoom is applied on release so the
	/// slider does not rescale under the cursor mid-drag.
	reading_zoom_draft: Option<u16>,
	friend_removal: Option<(u64, model::User)>,
	members_narrow_open: bool,
	/// Only the open member request's revealed prefix; member data stays in the bounded core cache.
	member_extent: Option<((u64, Id, u64), usize)>,
	guild: Option<Id>,
	navigation_channel: Option<Id>,
	pub logout_requested: bool,
	/// Accounts remembered on this device, most recently used first.
	pub accounts: Vec<model::SavedAccount>,
	/// Switch to this saved account; the host ends the session and reads its saved token.
	pub switch_account_requested: Option<Id>,
	/// End the session and return to the sign-in screen to remember one more account.
	pub add_account_requested: bool,
	/// Drop one saved account: its token, cached history and drafts.
	pub forget_account_requested: Option<Id>,
	pub reconnect_requested: bool,
	pub draft_changes: Vec<Id>,
	pub draft_restore_pending: bool,
	pub attachment: Option<(String, u64)>,
	pub attachment_files: Vec<(String, u64)>,
	pub remove_attachment_index: Option<usize>,
	/// Downscaled pixels per selected file (`None` for non-images), produced off the render thread.
	pub attachment_previews: Vec<Option<std::sync::Arc<egui::ColorImage>>>,
	/// GPU copies of `attachment_previews`, keyed by the pixel buffer they were uploaded from.
	attachment_textures: Vec<Option<(usize, egui::TextureHandle)>>,
	pub attach_requested: bool,
	pub attachment_paste_requested: Option<AttachmentPaste>,
	pub pasted_text: Option<(Id, egui::Id, String)>,
	paste_key_handled: bool,
	pasted_text_frame: Option<u64>,
	pub remove_attachment_requested: bool,
	pub cancel_upload_requested: bool,
	pub upload_busy: bool,
	/// Over-limit text staged as a `.txt` attachment after the rename dialog. The
	/// bytes stay here so tests and the desktop upload wiring can read them next
	/// to the optimistic row; a rejected send puts the text back in the draft.
	pub long_text_attachment: Option<LongTextAttachment>,
	/// Rename dialog for over-limit text; closed on send or cancel.
	pub long_text_dialog: Option<LongTextDialog>,
	/// Bounded text, markdown or code preview opened from a file card.
	pub preview: Option<text_preview::TextPreview>,
	/// Send was pressed with over-limit text; the dialog opens once the fresh
	/// buffer is in scope. Private: only the composer sets and consumes this.
	long_text_send_requested: bool,
	/// Transient problem and progress notices. Nothing here outlives its deadline.
	pub toasts: toasts::Toasts,
	pending_upload: Option<pending::Upload>,
	pub clear_cache_requested: bool,
	pub voice_available: bool,
	/// Recent call the owner may explicitly rejoin after an abrupt close.
	pub reconnect_offer: Option<(Id, Option<Id>)>,
	/// Clear the persisted reconnect hint after an explicit dismiss.
	pub reconnect_dismissed: bool,
	pub voice_switch_ready: bool,
	voice_switch: Option<voice::CallSwitch>,
	pub screen: screen::ScreenUi,
	pub voice_camera_available: bool,
	/// Channel whose next successful join should immediately enable the selected camera.
	voice_camera_on_join: Option<Id>,
	pub voice_camera_status: &'static str,
	pub voice_cameras: Vec<(String, String)>,
	pub voice_camera_device: Option<String>,
	pub voice_refresh_cameras: bool,
	pub voice_camera_device_status: &'static str,
	pub voice_camera_devices_loading: bool,
	pub voice_camera_preview: Option<egui::TextureHandle>,
	pub camera_test_requested: bool,
	pub camera_test_available: bool,
	pub camera_test_status: &'static str,
	pub camera_test_texture: Option<egui::TextureHandle>,
	/// Decoded remote cameras by user; the desktop bounds and replaces them.
	pub voice_remote_video: Vec<(Id, egui::TextureHandle)>,
	/// Latest picture of the screen share this device chose to watch.
	pub voice_stream_view: Option<egui::TextureHandle>,
	pub voice_stream_status: &'static str,
	/// Session-only stream playback level; unset is 100%. Mute preserves the level.
	voice_stream_volume: Option<u16>,
	voice_stream_muted: bool,
	/// Enlarged stage tile; cleared when it stops showing video or on Escape.
	pub voice_focus: Option<voice::StageFocus>,
	/// Full-client watched-stream presentation and the native fullscreen transition it requests.
	pub(crate) voice_stream_fullscreen: bool,
	pub(crate) voice_stream_fullscreen_previous: bool,
	pub(crate) voice_stream_fullscreen_request: Option<bool>,
	/// Whether the other participants stay visible as a strip under the enlarged tile.
	pub voice_focus_participants: bool,
	/// Session-only visibility of the selected guild voice channel's chat.
	pub voice_chat_open: bool,
	/// Tile click to start (`Some(user)`) or stop (`None`) watching, applied by the stage.
	watch_request: Option<Option<Id>>,
	pub voice_inputs: Vec<(String, String)>,
	pub voice_outputs: Vec<(String, String)>,
	pub voice_input: Option<String>,
	pub voice_output: Option<String>,
	pub voice_gain: VoiceGain,
	voice_user_volumes: Option<Box<[(u64, u16); 64]>>,
	/// Hearing protection: bots without a chosen volume are mixed at half volume.
	pub voice_bot_safe_volume: bool,
	/// Speakers silenced on this device only; never sent to Discord.
	voice_user_muted: Vec<u64>,
	pub voice_refresh_devices: bool,
	pub voice_device_status: &'static str,
	pub voice_microphone_unavailable: bool,
	pub voice_push_to_talk: bool,
	/// Device-local voice intent, retained between calls and restarts.
	pub voice_muted: bool,
	pub voice_deafened: bool,
	pub voice_processing: model::voice_settings::VoiceProcessing,
	/// Level the call-panel button restores when noise suppression is turned back on.
	voice_noise_restore: model::voice_settings::NoiseSuppression,
	/// Maximum suppression could not keep up on this machine, so Standard took over.
	voice_noise_fell_back: bool,
	pub voice_preview_requested: bool,
	pub voice_preview_level: Option<f32>,
	pub voice_preview_status: &'static str,
	/// Device-local application shortcuts; the desktop host mirrors the global binding.
	pub keybinds: model::Keybinds,
	keybind_capture: Option<model::KeybindAction>,
	pub global_keybind_status: &'static str,
	/// Server folders the owner left open; restored from device preferences at startup.
	pub expanded_folders: Vec<u64>,
	pub voice_ptt_active: bool,
	pub voice_privacy_code: Option<String>,
	/// Latest voice-server heartbeat round trip, when a call is connected.
	pub voice_ping_ms: Option<u32>,
	/// English place name for the connected voice server, translated at draw time.
	pub voice_server_place: String,
	/// Latest media activity, capped at 64 IDs (512 bytes) by the voice host.
	pub voice_speaking: Vec<Id>,
	pub notifications_enabled: bool,
	pub notification_options: model::notification_preferences::Device,
	/// Automatic live-client cues, at most four, so a join and a leave in one update both play.
	pub notification_cues: Vec<model::notification_preferences::Sound>,
	/// Explicit settings preview; intentionally allowed to bypass automatic mute policy.
	pub notification_preview: Option<model::notification_preferences::Sound>,
	pub notification_sound_status: &'static str,
	pub notification_test_available: bool,
	pub notification_test_requested: bool,
	pub notification_status: &'static str,
	pub storage_status: &'static str,
	/// Release channel and version shown in the title bar.
	pub build: design::Build,
	pub updates: updates::Updates,
	pub updates_save_failed: bool,
	/// Header pin button rect while the pins popout is open.
	pins_anchor: Option<egui::Rect>,
	editing: Option<(Id, Id, String)>,
	composer_edit: Option<(Id, Id)>,
	edit_sent: bool,
	deleting: Option<(Id, Id)>,
	/// One confirmed delete action over at most five messages authored by this account.
	deleting_batch: Option<(Id, Vec<Id>)>,
	/// Own-message deletes waiting to be sent one at a time.
	batch_delete_queue: Vec<(Id, Id)>,
	batch_delete_total: usize,
	batch_delete_sent: usize,
	batch_delete_next: Option<f64>,
	ime_active: bool,
	mention_menu: mentions::Menu,
	slash_commands: slash_commands::Menu,
	slash_direct: Option<slash_builtin::PendingMessage>,
	mention_lookup: member_search::Search,
	emoji_picker: emoji_picker::Picker,
	reaction_picker: emoji_picker::Picker,
	/// Set when the user picks a theme preset; the host persists it.
	pub theme_variant_changed: Option<design::Variant>,
	pub primary_color: Option<[u8; 3]>,
	pub transparency_blur: bool,
	pub transparency: u8,
	pub blur: u8,
	pub transparent_all: bool,
}

/// Context strip (reply/edit) drawn as the rounded top of the composer block.
fn composer_cap(
	ui: &mut egui::Ui,
	colors: &design::Palette,
	add_contents: impl FnOnce(&mut egui::Ui),
) -> egui::Rect {
	egui::Frame::new()
		.fill(design::mix(colors.raised, colors.base, 0.45))
		.corner_radius(egui::CornerRadius {
			nw: 8,
			ne: 8,
			sw: 0,
			se: 0,
		})
		.inner_margin(egui::Margin {
			left: 16,
			right: 8,
			top: 5,
			bottom: 5,
		})
		.show(ui, |ui| {
			ui.set_min_width((ui.available_width()).max(0.0));
			ui.horizontal(|ui| add_contents(ui));
		})
		.response
		.rect
}

fn mention_switch(ui: &mut egui::Ui, colors: &design::Palette, on: &mut bool) {
	let font = egui::FontId::new(12.0, design::semibold_family(ui.ctx()));
	let padding = egui::vec2(6.0, 3.0);
	let hit =
		ui.painter()
			.layout_no_wrap("@ OFF".to_owned(), font.clone(), colors.muted)
			.size() + 2.0 * padding;
	let (rect, mut response) = ui.allocate_exact_size(hit, egui::Sense::click());
	if response.clicked() {
		*on = !*on;
		response.mark_changed();
	}
	let label = if *on { "@ ON" } else { "@ OFF" };
	let color = if *on { colors.link } else { colors.muted };
	let hover = if *on {
		"Click to disable pinging the original author."
	} else {
		"Click to enable pinging the original author."
	};
	if response.hovered() || response.has_focus() {
		ui.painter().rect_filled(rect, 6, colors.hover);
	}
	let galley = ui.painter().layout_no_wrap(label.to_owned(), font, color);
	ui.painter()
		.galley(rect.center() - galley.size() / 2.0, galley, color);
	response.widget_info(|| {
		egui::WidgetInfo::selected(
			egui::Role::CheckBox,
			ui.is_enabled(),
			*on,
			"Ping the original author",
		)
	});
	response.on_hover_text(hover);
}

enum MentionWrite {
	Inserted,
	DidNotFit,
}

impl MessagingUi {
	pub fn interaction_modal_id(&self) -> Option<Id> {
		self.interaction_components.modal_id()
	}
	pub fn set_interaction_files(&mut self, custom_id: &str, files: Vec<(usize, String)>) {
		self.interaction_components.set_files(custom_id, files);
	}

	/// Feed the frame's middle button before `show`. Never fed means never pressed.
	pub fn middle_button(&mut self, middle: scroll::Middle) {
		self.scroll.middle(middle);
	}

	/// Feed mouse 4 / mouse 5 edge presses before `show`. Flags OR so a second feed cannot clear a press.
	pub fn side_buttons(&mut self, side: scroll::SidePress) {
		self.side_press.back |= side.back;
		self.side_press.forward |= side.forward;
	}

	fn drain_side_press(&mut self) -> scroll::SidePress {
		let press = self.side_press;
		self.side_press = scroll::SidePress::default();
		press
	}

	/// True while a drive needs the cursor position, including outside the window.
	pub fn tracking_pointer(&self) -> bool {
		self.scroll.tracking()
	}

	/// Returns whether the configured push-to-talk chord is held in the focused window.
	pub fn push_to_talk_down(&self, ctx: &egui::Context) -> bool {
		ctx.input(|input| {
			input.focused
				&& !ctx.egui_wants_keyboard_input()
				&& crate::keybinds::down(
					input,
					self.keybinds.chord(model::KeybindAction::PushToTalk),
				)
		})
	}

	/// Returns focused-window mute/deafen presses that were not claimed by a native global hotkey.
	pub fn voice_toggle_pressed(&self, ctx: &egui::Context, global_mask: u8) -> u8 {
		if !ctx.input(|input| input.focused) || ctx.egui_wants_keyboard_input() {
			return 0;
		}
		ctx.input_mut(|input| {
			let mut toggles = 0;
			if global_mask & 1 == 0
				&& crate::keybinds::pressed(
					input,
					self.keybinds.chord(model::KeybindAction::ToggleMute),
				) {
				toggles |= 1;
			}
			if global_mask & 2 == 0
				&& crate::keybinds::pressed(
					input,
					self.keybinds.chord(model::KeybindAction::ToggleDeafen),
				) {
				toggles |= 2;
			}
			toggles
		})
	}

	/// Pending profile picture selection: (generation, own user, editor revision).
	pub fn take_profile_picture_request(&mut self) -> Option<(u64, Id, u64)> {
		self.settings.editor.avatar_request.take()
	}
	pub fn accept_profile_picture(
		&mut self,
		ctx: &egui::Context,
		request: (u64, Id, u64),
		result: Result<Option<(String, egui::ColorImage)>, &'static str>,
	) {
		if let Err(error) = self.settings.editor.accept_avatar(ctx, request, result) {
			self.toasts.push(design::Level::Error, error);
		}
	}
	pub fn take_group_icon_request(&mut self) -> Option<(u64, Id, u64)> {
		self.group_menu.icon_request.take()
	}
	pub fn accept_group_icon(
		&mut self,
		ctx: &egui::Context,
		request: (u64, Id, u64),
		result: Result<Option<(String, egui::ColorImage)>, &'static str>,
	) {
		self.group_menu.accept_icon(ctx, request, result);
	}
	pub fn take_create_server_icon_request(&mut self) -> Option<(u64, Id, u64)> {
		self.join_server.take_icon_request()
	}
	pub fn accept_create_server_icon(
		&mut self,
		ctx: &egui::Context,
		request: (u64, Id, u64),
		result: Result<Option<(String, egui::ColorImage)>, &'static str>,
	) {
		self.join_server.accept_icon(ctx, request, result);
	}
	pub fn timeline_reflows(&self) -> (u64, u64) {
		(
			self.timeline.reflow_frames,
			self.timeline.consecutive_reflows,
		)
	}
	/// Fixture-only: keep the synthetic slash picker visible without window focus.
	#[cfg(feature = "demo")]
	pub fn preview_slash_commands(&mut self) {
		self.preview_slash_commands = true;
	}
	/// Fixture-only: open the Threads dialog for `parent` at startup, as the header control would.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_threads(&mut self, parent: Id) {
		self.archive_parent = Some(parent);
	}
	/// Fixture-only entry point: opens People and the profile card for `user` as if clicked.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_profile(&mut self, user: model::User) {
		self.members_narrow_open = true;
		self.profile.command_open(user);
	}
	/// Fixture-only entry point: opens the emoji popout as if the composer button was clicked.
	/// Fixture-only entry point: stage a synthetic attachment as if it had been selected.
	/// Repeated calls build up a batch, like choosing several files.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_attachment(
		&mut self,
		filename: &str,
		bytes: u64,
		preview: Option<egui::ColorImage>,
	) {
		if self.attachment.is_none() {
			self.attachment = Some((filename.to_owned(), bytes));
		}
		self.attachment_files.push((filename.to_owned(), bytes));
		self.attachment_previews
			.resize(self.attachment_files.len() - 1, None);
		self.attachment_previews
			.push(preview.map(std::sync::Arc::new));
	}
	/// Every selected file; older callers and tests may stage only the first one.
	fn selected_files(&self) -> Vec<(String, u64)> {
		if self.attachment_files.is_empty() {
			self.attachment.iter().cloned().collect()
		} else {
			self.attachment_files.clone()
		}
	}
	/// Textures for `attachment_previews`, uploaded once per pixel buffer and reused per frame.
	fn attachment_textures(&mut self, ctx: &egui::Context) -> Vec<Option<egui::TextureHandle>> {
		let count = self.selected_files().len();
		self.attachment_textures.resize(count, None);
		(0..count)
			.map(|index| {
				let image = self.attachment_previews.get(index).cloned().flatten();
				let Some(image) = image else {
					self.attachment_textures[index] = None;
					return None;
				};
				let key = std::sync::Arc::as_ptr(&image) as usize;
				if self.attachment_textures[index]
					.as_ref()
					.is_none_or(|(k, _)| *k != key)
				{
					let handle = ctx.load_texture(
						format!("attachment-preview-{index}"),
						egui::ImageData::Color(image),
						egui::TextureOptions::LINEAR,
					);
					self.attachment_textures[index] = Some((key, handle));
				}
				self.attachment_textures[index]
					.as_ref()
					.map(|(_, handle)| handle.clone())
			})
			.collect()
	}
	/// Moves staged files and thumbnails into the optimistic row for `command`.
	pub fn stage_pending_upload(&mut self, ctx: &egui::Context, command: &Command) {
		let files = self.selected_files();
		if let Command::Send { nonce, .. } = command
			&& !files.is_empty()
		{
			let textures = self.attachment_textures(ctx);
			self.pending_upload = Some(pending::Upload {
				nonce: nonce.clone(),
				files: files
					.into_iter()
					.zip(textures)
					.map(|((_, bytes), preview)| pending::UploadFile { bytes, preview })
					.collect(),
				progress: None,
			});
			self.attachment = None;
			self.attachment_files.clear();
			self.attachment_previews.clear();
			self.attachment_textures.clear();
			self.upload_busy = true;
		}
	}
	/// Synthetic pending state only; no command is dispatched or file uploaded.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_sending(&mut self, ctx: &egui::Context, state: &mut State) {
		if !state.demo {
			return;
		}
		let Some(channel) = state.selected else {
			return;
		};
		state
			.drafts
			.insert(channel, "Here’s the attachment — sending it now.".into());
		let files = self.selected_files();
		let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
		if let Some(command) = state.prepare_send_with_attachments(&names) {
			self.stage_pending_upload(ctx, &command);
			if let Some(upload) = &mut self.pending_upload {
				let bytes = upload.bytes();
				upload.progress = Some((bytes * 42 / 100, bytes));
			}
		}
	}
	/// Byte counts describe data supplied to HTTP, never message delivery.
	pub fn update_upload_progress(&mut self, progress: Option<(u64, u64)>, sending: bool) {
		if let Some(upload) = &mut self.pending_upload {
			upload.progress = if sending {
				Some((upload.bytes(), upload.bytes()))
			} else {
				progress
			};
		}
	}
	/// A local Rich Presence client asked to show an invite. It is only prefilled here:
	/// the user still confirms the lookup and the join.
	pub fn open_rpc_invite(&mut self, generation: u64, code: String) {
		if invites::input_code(&code).is_some() {
			self.join_server.open_with(generation, code);
		}
	}
	/// Fixture-only: open the join-server dialog at startup.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_join_server(&mut self, generation: u64) {
		self.join_server.open(generation);
	}
	/// Fixture-only: show the native verification flow without contacting a provider.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_verification(&mut self, state: &mut State) {
		if !state.demo {
			return;
		}
		let code = "synthetic-verification".to_owned();
		state.invites.insert(
			code.clone(),
			(
				std::time::Instant::now(),
				Some(Ok(model::InvitePreview {
					guild: Id(424242),
					embed: model::Embed {
						title: Some("Nivra community".into()),
						..Default::default()
					},
				})),
			),
		);
		state.invite_join = Default::default();
		state.invite_join.code = code;
		state.invite_join.pending = true;
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::InviteChallenge {
				request: 0,
				challenge: Box::new(
					client_core::captcha::Challenge::new(
						"synthetic-preview".into(),
						None,
						None,
						None,
						false,
					)
					.expect("valid synthetic challenge"),
				),
			},
		});
	}
	/// Fixture-only: open the screen-share picker for the fixture call at startup.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_screen_share(&mut self, state: &State) {
		self.screen.launch(state);
	}
	/// Fixture-only: open the pinned messages popout on the next frame.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_pins(&mut self) {
		self.search.preview_pins();
	}
	/// Fixture-only: open the account popout above the footer card at startup.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_account_menu(&mut self, generation: u64) {
		self.account_menu.preview(generation);
	}
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_custom_status(&mut self, generation: u64) {
		let draft = self.own_presence.custom_status.clone();
		self.account_menu.preview_editor(generation, draft);
	}
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_sticker_picker(&mut self) {
		self.emoji_picker.open_stickers(None);
	}
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_emoji_picker(&mut self) {
		self.emoji_picker.preview();
	}
	/// Fixture-only: open the media viewer on `attachment` of `message` at startup.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_image_viewer(&mut self, message: model::Id, attachment: model::Id) {
		self.timeline.preview_image_viewer(message, attachment);
	}
	/// Fixture-only: opens the GIFs tab at `section` (`""`, `favorites`, `trending` or a query).
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_gif_picker(&mut self, section: &str) {
		self.emoji_picker.preview_gifs(section);
	}
	/// Fixture-only entry point: opens the search pane and submits `query` on the first frame.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_search(&mut self, query: &str) {
		self.search.preview(query);
	}
	pub fn downloads(&mut self) -> &mut DownloadUi {
		&mut self.timeline.download
	}
	/// Desktop bridge: a file card asked to read a text, markdown or code body.
	pub fn take_preview_request(&mut self) -> Option<model::Attachment> {
		self.timeline.download.preview_request.take()
	}
	/// Desktop bridge: the bounded body was fetched and decoded; show it.
	pub fn set_preview(&mut self, preview: text_preview::TextPreview) {
		self.preview = Some(preview);
	}
	/// Report a preview failure through the shared toast channel.
	pub fn preview_failed(&mut self, reason: &str) {
		self.toasts.push(design::Level::Error, reason);
	}
	/// Close the open preview dialog.
	pub fn close_preview(&mut self) {
		self.preview = None;
	}
	/// Desktop bridge: folder-pick + sequential download request. Attachments are
	/// bounded (at most 15) by the floating bar before they reach here.
	pub fn take_batch_download_request(&mut self) -> Option<Vec<model::Attachment>> {
		if self.timeline.batch_download_requested {
			self.timeline.batch_download_requested = false;
			Some(std::mem::take(
				&mut self.timeline.batch_download_attachments,
			))
		} else {
			None
		}
	}
	/// Desktop bridge: save-selection-as-txt request (filename + bytes).
	pub fn take_save_txt_request(&mut self) -> Option<(String, Vec<u8>)> {
		self.timeline.save_txt_request.take()
	}
	/// Desktop bridge: publish the batch transfer snapshot rendered by the manager.
	pub fn set_batch_download_view(
		&mut self,
		folder: Option<String>,
		files: Vec<crate::batch_select::BatchFileView>,
		open: bool,
	) {
		self.timeline.batch_mgr_folder = folder;
		self.timeline.batch_files = files;
		if open {
			self.timeline.batch_mgr_open = true;
		}
	}
	/// Desktop bridge: close the manager (folder pick cancelled or batch forgotten).
	pub fn close_batch_download_view(&mut self) {
		self.timeline.batch_mgr_open = false;
		self.timeline.batch_files.clear();
		self.timeline.batch_mgr_folder = None;
	}
	/// Desktop bridge: take manager actions (cancel, retry, open folder, dismiss).
	pub fn take_batch_manager_flags(&mut self) -> (bool, bool, bool, bool) {
		(
			std::mem::take(&mut self.timeline.batch_mgr_cancel),
			std::mem::take(&mut self.timeline.batch_mgr_retry),
			std::mem::take(&mut self.timeline.batch_mgr_open_folder),
			std::mem::take(&mut self.timeline.batch_mgr_dismiss),
		)
	}
	pub fn audio(&mut self) -> &mut AudioUi {
		&mut self.timeline.audio
	}
	pub fn video(&mut self) -> &mut VideoUi {
		&mut self.timeline.video
	}
	pub fn clear_avatars(&mut self) {
		self.avatars = avatars::Avatars::default();
	}
	pub fn take_avatar_requests(&mut self) -> Vec<String> {
		self.avatars.take_requests()
	}
	pub fn accept_gif_animation(&mut self, key: String, frames: GifFrames) {
		self.avatars.accept_animation(key, frames);
	}
	pub fn accept_avatar(
		&mut self,
		ctx: &egui::Context,
		key: String,
		image: Option<egui::ColorImage>,
	) {
		self.avatars.accept(ctx, key, image);
	}
	pub fn clear(&mut self) {
		// Window preferences belong to the application, not the account being cleared.
		*self = Self {
			build: self.build,
			// The switcher roster belongs to the device, not to the account being cleared.
			accounts: std::mem::take(&mut self.accounts),
			updates: std::mem::take(&mut self.updates),
			minimize_to_tray: self.minimize_to_tray,
			tray_available: self.tray_available,
			tray_status: self.tray_status,
			startup_enabled: self.startup_enabled,
			startup_minimized: self.startup_minimized,
			startup_available: self.startup_available,
			startup_busy: self.startup_busy,
			startup_status: self.startup_status,
			startup_disable_requested: self.startup_disable_requested,
			..Self::default()
		};
	}
	pub fn restore_channel_preferences(&mut self, preferences: model::ChannelPreferences) {
		if !self.channel_preferences_changed && preferences.is_valid() {
			self.channel_preferences = preferences;
			self.channel_preferences_loaded = true;
			self.channel_preferences_status = "";
			self.channel_cache.invalidate();
		}
	}
	pub fn has_edit(&self) -> bool {
		self.editing.is_some()
	}
	pub fn has_edit_in(&self, channel: Option<Id>) -> bool {
		self.editing
			.as_ref()
			.is_some_and(|(edit_channel, _, _)| Some(*edit_channel) == channel)
	}
	pub fn messages_deleted(&mut self, ctx: &egui::Context, channel: Id, ids: &[Id]) {
		if ids.len() > 100 {
			return;
		}
		if self
			.deleting
			.is_some_and(|(c, id)| c == channel && ids.contains(&id))
		{
			self.deleting = None;
		}
		self.timeline.batch_delete.retain(|id| !ids.contains(id));
		// Freed rows vanish at once; hold the viewport by their cached heights.
		let compensation = self.timeline.take_removal_compensation(ids);
		self.timeline.compensate_scroll(compensation);
		let clear_batch = if let Some((batch_channel, selected)) = &mut self.deleting_batch {
			if *batch_channel == channel {
				selected.retain(|id| !ids.contains(id));
				selected.is_empty()
			} else {
				false
			}
		} else {
			false
		};
		if clear_batch {
			self.deleting_batch = None;
		}
		let Some((edit_channel, message, _)) = &self.editing else {
			return;
		};
		if *edit_channel != channel || !ids.contains(message) {
			return;
		}
		let untouched = self
			.edit_modified
			.is_some_and(|(c, id, modified)| c == channel && id == *message && !modified);
		if let Some(id) = self.edit_widget_id {
			if untouched {
				egui::text_edit::TextEditState::default().store(ctx, id);
			} else if let Some(mut editor) = egui::text_edit::TextEditState::load(ctx, id) {
				editor.clear_undoer();
				editor.store(ctx, id);
			}
		}
		self.edit_sent = false;
		self.edit_undo_cleared = true;
		if untouched {
			self.editing = None;
			self.edit_modified = None;
			self.edit_widget_id = None;
			self.edit_closed_channel = Some(channel);
		}
	}
	fn reconcile_edit(&mut self, state: &State) {
		let Some((channel, id, content)) = &self.editing else {
			self.edit_modified = None;
			self.edit_undo_cleared = false;
			return;
		};
		if self
			.edit_modified
			.is_none_or(|(c, message, _)| c != *channel || message != *id)
		{
			self.edit_undo_cleared = false;
			let modified = state
				.timeline
				.get(*id)
				.filter(|message| message.channel == *channel)
				.is_none_or(|message| message.content != *content);
			self.edit_modified = Some((*channel, *id, modified));
		}
		if state.selected == Some(*channel)
			&& state.timeline.get(*id).is_none()
			&& self.edit_modified.is_some_and(|(_, _, modified)| !modified)
		{
			self.editing = None;
			self.edit_modified = None;
			self.edit_sent = false;
		}
	}
	/// Window title strip: traffic-light inset, centred context title and session state.
	fn title_bar(&mut self, ui: &mut egui::Ui, state: &State, title: &str) {
		let colors = design::palette(ui);
		egui::Panel::top("title-bar")
			.exact_size(36.0)
			.show_separator_line(false)
			.frame(egui::Frame::new().fill(design::section_surface(
				ui,
				design::window_palette(ui).base,
				design::ImageSection::TopBar,
			)))
			.show(ui, |ui| {
				// Caption text belongs to the window drag region, not text selection.
				ui.style_mut().interaction.selectable_labels = false;
				let rect = ui.max_rect();
				design::window_drag(ui, rect);
				let title_rect = egui::Rect::from_center_size(
					rect.center(),
					egui::vec2(rect.width() * 0.3, rect.height()),
				);
				ui.scope_builder(
					egui::UiBuilder::new().max_rect(title_rect).layout(
						egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
					),
					|ui| {
						ui.add(
							egui::Label::new(design::semibold(ui, title, 13.0).color(colors.text))
								.truncate(),
						);
					},
				);

				ui.scope_builder(
					egui::UiBuilder::new()
						.max_rect(egui::Rect::from_min_max(
							egui::pos2(title_rect.right() + 10.0, rect.top()),
							rect.right_bottom() - egui::vec2(12.0, 0.0),
						))
						.layout(egui::Layout::right_to_left(egui::Align::Center)),
					|ui| {
						design::window_controls(ui);
						ui.spacing_mut().item_spacing.x = 10.0;
						if self.updates.available || self.updates.ready {
							let (label, icon) = if self.updates.ready {
								("Restart to update", icons::Icon::Reload)
							} else if self.updates.busy {
								("Updating…", icons::Icon::Download)
							} else {
								("Update available", icons::Icon::Download)
							};
							let font = egui::FontId::new(12.0, design::medium_family(ui.ctx()));
							let galley =
								ui.painter()
									.layout_no_wrap(label.to_owned(), font, colors.accent);
							let icon_size = 13.0;
							let gap = 5.0;
							let pad = egui::vec2(6.0, 2.0);
							let size = egui::vec2(
								galley.size().x + icon_size + gap + pad.x * 2.0,
								galley.size().y.max(icon_size) + pad.y * 2.0,
							);
							let (rect, response) =
								ui.allocate_exact_size(size, egui::Sense::click());
							ui.painter().rect_filled(
								rect,
								255,
								colors.accent.gamma_multiply(if response.hovered() {
									0.24
								} else {
									0.16
								}),
							);
							let icon_rect = egui::Rect::from_center_size(
								egui::pos2(rect.left() + pad.x + icon_size / 2.0, rect.center().y),
								egui::Vec2::splat(icon_size),
							);
							icons::paint(ui.painter(), icon, icon_rect, colors.accent);
							ui.painter().galley(
								egui::pos2(
									icon_rect.right() + gap,
									rect.center().y - galley.size().y / 2.0,
								),
								galley,
								colors.accent,
							);
							if response.on_hover_text(&self.updates.status).clicked() {
								self.open_update_settings();
							}
						} else {
							design::build_badge(ui, self.build);
						}
						if state.demo && !self.updates.available && !self.updates.ready {
							egui::Frame::new()
								.stroke(egui::Stroke::new(1.0, colors.border))
								.corner_radius(10)
								.inner_margin(egui::Margin::symmetric(8, 3))
								.show(ui, |ui| {
									ui.label(
										design::semibold(ui, "OFFLINE PREVIEW", 10.0)
											.color(colors.muted),
									);
								})
								.response
								.on_hover_text("Synthetic data · no network or local storage");
						}
						if !state.demo
							&& state.auth != client_core::auth::AuthState::Authenticated
							&& ui.small_button("Sign in again").clicked()
						{
							self.reconnect_requested = true;
						}
					},
				);
			});
	}
	fn member_rows(&mut self, ui: &mut egui::Ui, state: &mut State, commands: &mut Vec<Command>) {
		let colors = design::palette(ui);
		let cached = state.members_cached();
		let Some(list) = state
			.members
			.as_ref()
			.filter(|list| Some(list.channel) == state.selected)
		else {
			ui.add_space(8.0);
			ui.label(RichText::new("Choose a conversation to see its people.").color(colors.muted));
			return;
		};
		let has_entry = list.slots.iter().any(|slot| slot.is_some()) || cached;
		if !has_entry {
			ui.add_space(8.0);
			if list.freshness != Freshness::Fresh {
				let text = if list.freshness == Freshness::Unavailable {
					"People aren't available in this conversation."
				} else {
					"Loading people…"
				};
				ui.label(RichText::new(text).small().color(colors.muted));
			} else {
				ui.label(
					RichText::new("No people returned for this view.")
						.small()
						.color(colors.muted),
				);
			}
		}
		// Thread snapshots contain people only; paged channel lists supply their own headers.
		let thread_rows = (!list.lazy
			&& state
				.channel(list.channel)
				.is_some_and(|channel| matches!(channel.kind, 10..=12)))
		.then(|| thread_member_rows(state, list));
		let channel = list.channel;
		let lazy = list.lazy;
		let start = list.start;
		let guild = list.guild;
		let guild_or_channel = guild.unwrap_or(channel);
		let member_key = (state.generation, guild_or_channel, list.request);
		let reset_scroll = self.member_extent.is_none_or(|(key, _)| key != member_key);
		if reset_scroll {
			self.member_extent = Some((member_key, 100));
		}
		let total = list.total.min(250_000) as usize;
		// Hidden offline people are removed from the drawn rows; headers and counts stay.
		// Lazy lists skip the gateway's Offline section, whose rows may not be loaded yet.
		let shown_rows = (!lazy && self.hide_offline_members).then(|| {
			let online = |member: &model::Member| member_online(state, member, guild);
			match &thread_rows {
				Some(rows) => {
					visible_member_rows(rows.iter().map(|row| Some(row.0.as_ref())), online)
				}
				None => visible_member_rows(list.slots.iter().map(Option::as_ref), online),
			}
		});
		let hidden_offline = lazy
			.then_some(self.hide_offline_members)
			.filter(|hide| *hide)
			.and_then(|_| lazy_offline_rows(&list.groups))
			.filter(|(first, _)| *first < total)
			.map(|(first, count)| (first, count.min(total - first)));
		let lazy_row = |row: usize| match hidden_offline {
			Some((first, count)) if row >= first => row + count,
			_ => row,
		};
		let row_count = if lazy {
			let extent = self.member_extent.unwrap().1.min(total);
			match hidden_offline {
				Some((first, count)) if extent > first => first.max(extent.saturating_sub(count)),
				_ => extent,
			}
		} else if let Some(rows) = &shown_rows {
			rows.len()
		} else {
			thread_rows.as_ref().map_or(list.slots.len(), Vec::len)
		};
		let row_spacing = ui.spacing().item_spacing.y;
		ui.spacing_mut().item_spacing.y = 0.0;
		let mut visible = 0usize..0;
		let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
		if reset_scroll {
			area = area.vertical_scroll_offset(0.0);
		}
		let output = self
			.scroll
			.attach(ui, ("people", guild_or_channel, list.request), area)
			.show_rows(ui, 42.0, row_count, |ui, range| {
				visible = range.clone();
				for row in range {
					let index = if lazy {
						lazy_row(row)
					} else {
						shown_rows.as_ref().map_or(row, |rows| rows[row])
					};
					let slot = if lazy {
						state.member_slot(index).or_else(|| {
							if index >= start {
								list.slots.get(index - start).and_then(|slot| slot.as_ref())
							} else {
								None
							}
						})
					} else if let Some(rows) = &thread_rows {
						rows.get(index).map(|row| row.0.as_ref())
					} else {
						list.slots.get(index).and_then(|s| s.as_ref())
					};
					match slot {
						Some(model::MemberSlot::Group(id)) => {
							let name = match id.as_str() {
								"online" => "Online".to_owned(),
								"offline" => "Offline".to_owned(),
								_ => {
									let role = id.parse::<u64>().ok().and_then(|role_id| {
										guild.and_then(|guild| {
											state.guild_roles(guild).and_then(|roles| {
												roles.iter().find(|role| role.id == Id(role_id))
											})
										})
									});
									match role {
										Some(role) if !role.name.is_empty() => role.name.clone(),
										_ => "Role".to_owned(),
									}
								}
							};
							let text = list
								.groups
								.iter()
								.find(|(group_id, _)| group_id == id)
								.map(|(_, count)| *count)
								.or_else(|| thread_rows.as_ref().and_then(|rows| rows[index].1))
								.map(|count| format!("{name} - {count}"))
								.unwrap_or(name);
							let (rect, _) = ui.allocate_exact_size(
								egui::vec2(ui.available_width(), 42.0),
								egui::Sense::hover(),
							);
							let mut header = ui.new_child(
								egui::UiBuilder::new()
									.max_rect(egui::Rect::from_min_max(
										rect.left_top() + egui::vec2(8.0, 16.0),
										rect.right_bottom() - egui::vec2(8.0, 0.0),
									))
									.layout(egui::Layout::left_to_right(egui::Align::Center)),
							);
							header
								.add(
									egui::Label::new(
										design::medium(ui, &text, 12.0).color(colors.muted),
									)
									.truncate(),
								)
								.on_hover_text(&text);
							if id == "offline" {
								let hidden = self.hide_offline_members;
								let button = egui::Rect::from_min_size(
									egui::pos2(rect.right() - 34.0, rect.center().y - 9.0),
									egui::vec2(26.0, 18.0),
								);
								let response = ui.interact(
									button,
									ui.scope_id().with("hide-offline-members"),
									egui::Sense::click(),
								);
								let hot = response.hovered() || response.has_focus() || hidden;
								ui.painter().rect_filled(
									button,
									9.0,
									if hidden {
										colors.accent.gamma_multiply(0.22)
									} else if hot {
										colors.hover
									} else {
										colors.raised
									},
								);
								let ink = if hidden {
									colors.accent
								} else if hot {
									colors.text
								} else {
									colors.muted
								};
								crate::icons::paint(
									ui.painter(),
									if hidden {
										crate::icons::Icon::EyeSlash
									} else {
										crate::icons::Icon::Eye
									},
									egui::Rect::from_min_size(
										egui::pos2(
											button.center().x - 6.0,
											button.center().y - 6.0,
										),
										egui::Vec2::splat(12.0),
									),
									ink,
								);
								let hint = if hidden {
									crate::i18n::text(self.language, "Show offline members")
								} else {
									crate::i18n::text(self.language, "Hide offline members")
								};
								if response.on_hover_text(hint).clicked() {
									self.hide_offline_members = !hidden;
								}
							}
						}
						Some(model::MemberSlot::Person(member)) => {
							let name = member
								.nick
								.as_deref()
								.filter(|nick| !nick.is_empty())
								.unwrap_or_else(|| state.user_display_name(&member.user));
							let (status, custom, activities, clients) =
								profiles::member_presence(state, member, guild);
							let subtitle = profiles::subtitle(custom, activities);
							let online =
								status.is_some_and(|s| matches!(s, "online" | "idle" | "dnd"));
							let (rect, response) = ui.allocate_exact_size(
								egui::vec2(ui.available_width(), 42.0),
								egui::Sense::click(),
							);
							response.widget_info(|| {
								egui::WidgetInfo::labeled(
									egui::Role::Button,
									true,
									format!(
										"{name}, {}, {}",
										status.map_or("presence unknown", profiles::presence_label),
										subtitle.as_deref().unwrap_or_default()
									),
								)
							});
							let row = rect.shrink2(egui::vec2(0.0, 1.0));
							if response.hovered() || response.has_focus() {
								ui.painter().rect_filled(
									row,
									6,
									design::row_highlight(ui, colors.hover, 1.0),
								);
							}
							let mut inner = ui.new_child(
								egui::UiBuilder::new()
									.max_rect(row.shrink2(egui::vec2(8.0, 4.0)))
									.layout(egui::Layout::left_to_right(egui::Align::Center)),
							);
							inner.spacing_mut().item_spacing.x = 12.0;
							let animate_avatar = ui.rect_contains_pointer(rect);
							inner.push_id(member.user.id.0, |ui| {
								let avatar = self
									.avatars
									.with_avatar_animation(animate_avatar, |avatars| {
										avatars.show(ui, &member.user, 32.0, state.demo)
									});
								user_menu::show(
									&avatar,
									state,
									&member.user,
									&mut self.profile,
									&mut self.user_action,
								);
								if let Some(status) = status {
									profiles::presence_badge(
										ui,
										avatar.rect,
										status,
										clients,
										colors.sidebar,
									);
								}
								let text_color = if online {
									let role_color = guild.and_then(|guild| {
										state.member_roles(guild, member).1.map(|role| role.color)
									});
									let background = if response.hovered() || response.has_focus() {
										colors.hover
									} else {
										colors.sidebar
									};
									role_color.map_or(colors.text, |rgb| {
										design::role_name_color(rgb, background, colors.text)
									})
								} else {
									colors.muted
								};
								let mut show_name = |ui: &mut egui::Ui| {
									ui.allocate_ui_with_layout(
										egui::vec2(ui.available_width(), 18.0),
										egui::Layout::left_to_right(egui::Align::Center),
										|ui| {
											ui.spacing_mut().item_spacing.x = 5.0;
											let server_tag = member.user.primary_guild.as_deref();
											let trailing =
												profiles::server_tag_width(ui, server_tag)
													+ if server_tag.is_some() { 5.0 } else { 0.0 };
											account_badge::name(
												ui,
												&member.user,
												name,
												15.0,
												text_color,
												egui::Sense::hover(),
												trailing,
											);
											if let Some(tag) = server_tag {
												profiles::server_tag(
													ui,
													tag,
													&mut self.avatars,
													state.demo,
												);
											}
										},
									);
								};
								if let Some(subtitle) = subtitle {
									ui.vertical(|ui| {
										ui.spacing_mut().item_spacing.y = 1.0;
										ui.spacing_mut().interact_size.y = 0.0;
										show_name(ui);
										ui.horizontal(|ui| {
											ui.spacing_mut().item_spacing.x = 4.0;
											if activities.first().is_some_and(profiles::is_spotify)
											{
												icons::inline(
													ui,
													icons::Icon::Spotify,
													12.0,
													colors.positive,
												);
											}
											ui.add(
												egui::Label::new(
													RichText::new(subtitle)
														.size(12.0)
														.color(colors.muted),
												)
												.truncate()
												.selectable(false),
											);
										});
									});
								} else {
									show_name(ui);
								}
								user_menu::show(
									&response,
									state,
									&member.user,
									&mut self.profile,
									&mut self.user_action,
								);
								self.profile.person_click(
									ui,
									&response,
									Some(&avatar),
									&member.user,
								);
							});
						}
						None => {
							ui.allocate_exact_size(
								egui::vec2(ui.available_width(), 42.0),
								egui::Sense::hover(),
							);
						}
					}
				}
			});
		ui.spacing_mut().item_spacing.y = row_spacing;
		if lazy && !visible.is_empty() {
			let first = visible.start;
			let mut last = visible.end.saturating_sub(1);
			if row_count < total
				&& output.state.offset.y > 0.0
				&& output.state.offset.y + output.inner_rect.height() >= output.content_size.y - 1.0
			{
				// Keep requesting the next chunk while at the bottom, without exposing
				// another page of empty rows before its first entry arrives.
				last = row_count;
				if state.member_slot(row_count).is_some()
					|| row_count
						.checked_sub(start)
						.is_some_and(|index| list.slots.get(index).is_some_and(Option::is_some))
				{
					self.member_extent = Some((member_key, (row_count + 100).min(total)));
					ui.ctx().request_repaint();
				}
			}
			if let Some(cmd) = state.focus_member_ranges(first, last) {
				commands.push(cmd);
			}
		}
	}
	/// Channel sidebar: context header, scrolling list and the account card.
	fn sidebar(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		title: &str,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		egui::Panel::top("sidebar-header")
			.exact_size(48.0)
			.show_separator_line(false)
			.frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(16, 0)))
			.show(ui, |ui| {
				let rect = ui.max_rect();
				ui.horizontal_centered(|ui| {
					if let Some(guild) = self.guild {
						self.server_menu
							.header(ui, state, guild, title, self.language);
						return;
					}
					ui.add(
						egui::Label::new(
							design::semibold(ui, title, 15.0).color(colors.text_strong),
						)
						.truncate()
						.selectable(false),
					);
				});
				ui.painter().hline(
					rect.x_range().expand(16.0),
					rect.bottom(),
					egui::Stroke::new(1.0, colors.border),
				);
			});
		egui::Frame::new()
			.inner_margin(egui::Margin {
				left: 8,
				right: 8,
				top: 8,
				bottom: 0,
			})
			.show(ui, |ui| {
				let ctx = ui.ctx().clone();
				if self.guild.is_none() {
					// Search and Friends share one row; Friends is the compact glyph beside it.
					ui.horizontal(|ui| {
						ui.spacing_mut().item_spacing.x = 6.0;
						let find = ui
							.add_enabled_ui(
								!self.ime_active
									&& !ctx.input(|input| {
										input
											.events
											.iter()
											.any(|event| matches!(event, egui::Event::Ime(_)))
									}),
								|ui| {
									ui.add_sized(
										[(ui.available_width() - 36.0).max(60.0), 30.0],
										egui::Button::new("Find conversation").truncate(),
									)
								},
							)
							.inner
							.on_hover_text("Search loaded conversations (Ctrl/Cmd+K)");
						find.widget_info(|| {
							egui::WidgetInfo::labeled(
								egui::Role::Button,
								ui.is_enabled(),
								"Find conversation, Ctrl or Command K",
							)
						});
						if find.clicked() {
							self.switcher.open(&ctx);
						}
						let friends = state.selected.is_none();
						let (rect, response) =
							ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::click());
						let hovered = response.hovered() || response.has_focus();
						if friends || hovered {
							ui.painter().rect_filled(
								rect,
								6,
								if friends {
									colors.selected
								} else {
									colors.hover
								},
							);
						}
						icons::paint(
							ui.painter(),
							icons::Icon::People,
							rect.shrink(7.0),
							if friends || hovered {
								colors.text_strong
							} else {
								colors.muted
							},
						);
						response.widget_info(|| {
							egui::WidgetInfo::selected(
								egui::Role::Button,
								true,
								friends,
								crate::i18n::text(self.language, "Friends"),
							)
						});
						if response
							.on_hover_text(crate::i18n::text(self.language, "Friends"))
							.clicked()
						{
							state.open_home();
							self.guild = None;
							self.search.open = false;
						}
					});
					ui.add_space(8.0);
				}
				if let Some(guild) = self.guild
					&& state.can_open_member_settings(guild)
					&& state.server_members_shortcut(guild)
					&& ui
						.add_sized(
							[ui.available_width(), 32.0],
							egui::Button::new("Members").frame(false),
						)
						.clicked() && let Some(command) =
					self.preview_server_admin(state, guild, "members")
				{
					commands.push(command);
				}
				if !self.channel_preferences_status.is_empty() {
					ui.colored_label(design::palette(ui).warning, self.channel_preferences_status);
					if ui.button("Retry shortcuts").clicked() {
						if self.channel_preferences_loaded {
							self.channel_preferences_changed = true;
						} else {
							self.channel_preferences_reload = true;
						}
					}
				}
				let select = self.channel_list(ui, state);
				if let Some((channel, action)) = self.channel_move.take()
					&& let Some(command) = state.request_channel_action(channel, action)
				{
					commands.push(command);
				}
				if let Some((guild, action)) = self.voice_member_move.take()
					&& let Some(command) = state.request_server_admin(guild, action)
				{
					commands.push(command);
				}
				if let Some(id) = select {
					let join_voice = state
						.channel(id)
						.is_some_and(|channel| channel.guild.is_some() && channel.kind == 2);
					if let Some(command) = state.select(id) {
						commands.push(command);
					}
					if join_voice
						&& state
							.voice
							.active
							.as_ref()
							.is_none_or(|call| call.channel != id)
					{
						self.request_call(state, id, false, commands);
					}
				}
				if let Some(parent) = self.archive_parent.take()
					&& let Some(command) =
						state.request_archives(parent, model::archives::Kind::Public, None)
				{
					self.search.open = false;
					self.archives.focus = true;
					commands.push(command);
				}
			});
	}
	/// Account card; while in a call it grows upward with the call header and quick actions.
	fn account_card(&mut self, ui: &mut egui::Ui, state: &mut State, commands: &mut Vec<Command>) {
		let colors = design::palette(ui);
		let mut anchor = None;
		let in_call = state.voice.active.is_some();
		egui::Frame::new()
			.fill(colors.raised)
			.corner_radius(8)
			.inner_margin(0)
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.spacing_mut().item_spacing.y = 0.0;
				let divider = |ui: &mut egui::Ui| {
					let (line, _) = ui.allocate_exact_size(
						egui::vec2(ui.available_width(), 1.0),
						egui::Sense::hover(),
					);
					ui.painter().rect_filled(line, 0, colors.border);
				};
				if self.shows_update_banner() {
					self.update_banner(ui);
					divider(ui);
				}
				if in_call {
					self.voice_card_section(ui, state, commands);
					divider(ui);
				}
				egui::Frame::new()
					.inner_margin(egui::Margin::symmetric(8, 6))
					.show(ui, |ui| {
						ui.set_width(ui.available_width());
						ui.horizontal(|ui| {
							ui.spacing_mut().item_spacing.x = 8.0;
							if let Some(user) = &state.user {
								let avatar = self.avatars.show(ui, user, 32.0, state.demo);
								avatar.widget_info(|| {
									egui::WidgetInfo::labeled(
										egui::Role::Button,
										true,
										"Profile and status",
									)
								});
								if avatar.has_focus() {
									ui.painter().rect_stroke(
										avatar.rect.expand(2.0),
										4,
										egui::Stroke::new(1.0, colors.accent),
										egui::StrokeKind::Inside,
									);
								}
								user_menu::show(
									&avatar,
									state,
									user,
									&mut self.profile,
									&mut self.user_action,
								);
								design::presence_dot(
									ui,
									avatar.rect,
									profiles::presence_color(self.own_presence.status.wire()),
									colors.raised,
								);
								anchor = Some(avatar.on_hover_text("Profile and status"));
							}
							ui.with_layout(
								egui::Layout::right_to_left(egui::Align::Center),
								|ui| {
									ui.spacing_mut().item_spacing.x = 2.0;
									let settings =
										icons::button(ui, icons::Icon::Gear, 32.0, "User settings");
									if settings.clicked() {
										self.settings.open = true;
										egui::Popup::close_all(ui.ctx());
									}
									self.mute_toggle_with_settings(ui, state, commands, true);
									ui.add_space(4.0);
									self.mute_toggle_with_settings(ui, state, commands, false);
									ui.with_layout(
										egui::Layout::left_to_right(egui::Align::Center),
										|ui| {
											let identity = ui
												.vertical(|ui| {
													ui.spacing_mut().item_spacing.y = 0.0;
													ui.add(
														egui::Label::new(
															design::semibold(
																ui,
																state
																	.user
																	.as_ref()
																	.map_or("Your account", |u| {
																		u.name.as_str()
																	}),
																14.0,
															)
															.color(colors.text_strong),
														)
														.truncate()
														.selectable(false),
													);
													ui.add(
														egui::Label::new(
															RichText::new(
																if !self
																	.own_presence
																	.custom_status
																	.is_empty()
																{
																	self.own_presence
																		.custom_status
																		.clone()
																} else if let Some(game) = self
																	.own_game
																	.as_deref()
																	.filter(|_| {
																		self.share_game_activity
																	}) {
																	game.to_owned()
																} else if state.demo {
																	"Offline preview".to_owned()
																} else if state.gateway_connected {
																	self.own_presence
																		.status
																		.label()
																		.to_owned()
																} else {
																	"Reconnecting…".to_owned()
																},
															)
															.size(12.0)
															.color(colors.muted),
														)
														.truncate()
														.selectable(false),
													);
												})
												.response;
											let identity = ui
												.interact(
													identity.rect,
													ui.scope_id().with("account-identity"),
													egui::Sense::click(),
												)
												.on_hover_text("Profile and status");
											identity.widget_info(|| {
												egui::WidgetInfo::labeled(
													egui::Role::Button,
													true,
													"Profile and status",
												)
											});
											if let Some(avatar) = anchor.take() {
												if identity.has_focus() {
													ui.painter().rect_stroke(
														identity.rect.expand(2.0),
														4,
														egui::Stroke::new(1.0, colors.accent),
														egui::StrokeKind::Inside,
													);
												}
												anchor = Some(avatar.union(identity));
											}
										},
									);
								},
							);
						});
					});
			});
		if let Some(anchor) = anchor {
			self.account_menu(&anchor, state, commands);
		}
	}
	/// Conversation header: channel identity on the left, tools and search on the right.
	fn channel_header(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		selected_voice: bool,
		show_members: bool,
		wide_members: bool,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		egui::Panel::top("channel-header")
			.exact_size(48.0)
			.show_separator_line(false)
			.frame(
				egui::Frame::new()
					.fill(design::section_surface(
						ui,
						design::window_palette(ui).chat,
						design::ImageSection::TopBar,
					))
					.inner_margin(egui::Margin::symmetric(16, 0)),
			)
			.show(ui, |ui| {
				let rect = ui.max_rect();
				ui.painter().hline(
					rect.x_range().expand(16.0),
					rect.bottom(),
					egui::Stroke::new(1.0, colors.border),
				);
				let channel = state.selected.and_then(|id| state.channel(id)).cloned();
				let dm = channel
					.as_ref()
					.is_some_and(|c| c.kind == 1 && c.guild.is_none());
				let shortcuts_available = self.shortcuts_available(state);
				ui.horizontal_centered(|ui| {
					ui.spacing_mut().item_spacing.x = 8.0;
					match channel.as_ref() {
						Some(c) if c.guild.is_none() && c.kind == 3 => {
							let avatar = self.avatars.show_group(ui, c, 24.0, state.demo);
							self.group_menu.context(
								&avatar,
								state,
								c,
								shortcuts::ShortcutView::new(
									&self.channel_preferences,
									shortcuts_available,
								),
							);
						}
						Some(c) if c.guild.is_none() => {
							if let Some(user) = c.recipients.first() {
								// Header avatar identifies the conversation; the profile is a
								// context-menu action, not a click target.
								let avatar = self.avatars.show_plain(ui, user, 24.0, state.demo);
								if dm {
									user_menu::show_with_pin(
										&avatar,
										state,
										user,
										&mut self.profile,
										&mut self.user_action,
										Some(shortcuts::ShortcutView::new(
											&self.channel_preferences,
											shortcuts_available,
										)),
									);
								}
								let (status, _, _, clients) =
									profiles::presence(state, user.id, None);
								if dm && let Some(status) = status {
									profiles::presence_badge(
										ui,
										avatar.rect,
										status,
										clients,
										colors.sidebar,
									);
								}
							} else {
								icons::inline(ui, icons::Icon::People, 22.0, colors.muted);
							}
						}
						Some(c) => {
							let icon = match c.kind {
								2 | 13 => icons::Icon::Speaker,
								5 => icons::Icon::Megaphone,
								15 | 16 => icons::Icon::Forum,
								10..=12 => icons::Icon::Thread,
								_ => icons::Icon::Hash,
							};
							icons::inline(ui, icon, 22.0, colors.muted);
						}
						None => {}
					}
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						ui.spacing_mut().item_spacing.x = 4.0;
						if selected_voice
							&& icons::toggle(
								ui,
								icons::Icon::Forum,
								32.0,
								self.voice_chat_open,
								if self.voice_chat_open {
									"Hide chat"
								} else {
									"Show chat"
								},
							)
							.clicked()
						{
							self.voice_chat_open = !self.voice_chat_open;
							self.focus_switched_composer = self.voice_chat_open;
						}
						if let Some(c) = channel
							.as_ref()
							.filter(|c| c.guild.is_none() && c.kind == 3)
						{
							self.group_menu.dropdown(
								ui,
								state,
								c,
								shortcuts::ShortcutView::new(
									&self.channel_preferences,
									shortcuts_available,
								),
							);
						}
						if state.selected.is_some() && !selected_voice {
							if self.search.open && !self.search.pins() {
								ui.allocate_ui_with_layout(
									egui::vec2(
										240.0_f32.min(ui.available_width() * 0.5).max(120.0),
										28.0,
									),
									egui::Layout::left_to_right(egui::Align::Center),
									|ui| self.search.header_input(ui, state, commands),
								);
							} else {
								// Search pill.
								let (pill, response) = ui.allocate_exact_size(
									egui::vec2(144.0, 28.0),
									egui::Sense::click(),
								);
								let enabled = state.can_search();
								response.widget_info(|| {
									egui::WidgetInfo::labeled(egui::Role::Button, enabled, "Search")
								});
								ui.painter().rect_filled(pill, 6, colors.raised);
								let pill_text = if enabled {
									colors.muted
								} else {
									colors.muted.gamma_multiply(0.5)
								};
								ui.painter().text(
									pill.left_center() + egui::vec2(10.0, 0.0),
									egui::Align2::LEFT_CENTER,
									"Search",
									egui::FontId::proportional(13.0),
									pill_text,
								);
								icons::paint(
									ui.painter(),
									icons::Icon::Search,
									egui::Rect::from_center_size(
										pill.right_center() - egui::vec2(14.0, 0.0),
										egui::Vec2::splat(16.0),
									),
									pill_text,
								);
								if enabled
									&& response.on_hover_text("Search this conversation").clicked()
								{
									if state.archives.is_some() {
										commands.push(state.clear_archives());
									}
									self.search.toggle(false);
								}
							}
							ui.add_space(4.0);
							if !dm
								&& icons::toggle(
									ui,
									icons::Icon::People,
									32.0,
									show_members,
									"Show member list",
								)
								.clicked()
							{
								if wide_members {
									self.reading_preferences.show_members =
										!self.reading_preferences.show_members;
								} else {
									self.members_narrow_open = !self.members_narrow_open;
								}
							}
							if state.selected.is_some() && !selected_voice {
								let select_label = if self.timeline.select_mode {
									crate::i18n::text(self.language, "Cancel")
								} else {
									crate::i18n::text(self.language, "Select")
								};
								if ui.button(select_label).clicked() {
									if self.timeline.select_mode {
										self.timeline.exit_select_mode();
									} else {
										self.timeline.enter_select_mode(None);
									}
								}
							}
							let pins_open = self.search.open && self.search.pins();
							let pins = ui
								.add_enabled_ui(state.can_search() || pins_open, |ui| {
									icons::toggle(
										ui,
										icons::Icon::Pin,
										32.0,
										pins_open,
										"Pinned messages",
									)
								})
								.inner;
							if pins.clicked()
								&& self.search.toggle(true)
								&& let Some(command) = state.request_pins()
							{
								commands.push(command);
							}
							self.pins_anchor =
								(self.search.open && self.search.pins()).then_some(pins.rect);
							if let Some(c) = channel
								.as_ref()
								.filter(|c| c.guild.is_some() && matches!(c.kind, 0 | 5 | 15 | 16))
							{
								let allowed =
									state.can_archive(c.id, model::archives::Kind::Public);
								let archive = ui
									.add_enabled_ui(allowed, |ui| {
										icons::button(ui, icons::Icon::Thread, 32.0, "Threads")
									})
									.inner;
								if archive.clicked() {
									self.archive_parent = Some(c.id);
								}
							}
							let reload = ui
								.add_enabled_ui(
									state.freshness != Freshness::Loading
										&& state
											.selected
											.is_some_and(|id| state.can_read_history(id)),
									|ui| {
										icons::button(
											ui,
											icons::Icon::Reload,
											32.0,
											"Reload history",
										)
									},
								)
								.inner;
							if reload.clicked() {
								commands.push(state.history(None));
								self.timeline.follow_latest(state);
							}
						}
						if let Some(channel) = state.selected.filter(|_| {
							channel
								.as_ref()
								.is_some_and(|c| c.guild.is_none() && matches!(c.kind, 1 | 3))
						}) {
							self.voice_settings(ui, state.demo, state.voice.active.is_some());
							self.call_button(ui, state, channel, commands, false);
						}
						ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
							// Centre the name block in the fixed-height header even without a subtitle.
							let name = channel
								.as_ref()
								.map_or(crate::i18n::text(self.language, "Direct Messages"), |c| {
									state.conversation_name(c)
								});
							let subtitle = channel
								.as_ref()
								.filter(|_| dm)
								.and_then(|c| c.recipients.first())
								.and_then(|user| {
									let (_, custom, activities, _) =
										profiles::presence(state, user.id, None);
									profiles::subtitle(custom, activities)
								});
							let name_height = ui
								.painter()
								.layout_no_wrap(
									name.to_owned(),
									egui::FontId::new(16.0, design::semibold_family(ui.ctx())),
									colors.text_strong,
								)
								.size()
								.y;
							let subtitle_height = subtitle.as_ref().map_or(0.0, |text| {
								1.0 + ui
									.painter()
									.layout_no_wrap(
										text.clone(),
										egui::FontId::proportional(12.0),
										colors.muted,
									)
									.size()
									.y
							});
							ui.vertical(|ui| {
								ui.add_space(
									((ui.available_height() - name_height - subtitle_height) / 2.0)
										.max(0.0),
								);
							ui.spacing_mut().item_spacing.y = 1.0;
							// Chrome never joins text selection: a drag past the
							// viewport keeps its range inside the messages.
							crate::select::chrome_label(
								ui,
								design::semibold(ui, name, 16.0).color(colors.text_strong),
							);
								{
								if let Some(text) = subtitle {
									crate::select::chrome_label(
										ui,
										RichText::new(&text).size(12.0).color(colors.muted),
									)
									.on_hover_text(text);
								}
							}});
							let in_call = state.voice.active.as_ref().is_some_and(|call| {
								Some(call.channel) == state.selected
									&& matches!(
										call.phase,
										client_core::voice::Phase::Connected
											| client_core::voice::Phase::Waiting
									)
							});
							if in_call {
								ui.add_space(4.0);
								icons::inline(ui, icons::Icon::InCall, 16.0, colors.positive);
								ui.add(
									egui::Label::new(
										design::medium(ui, "In a call", 14.0)
											.color(colors.positive),
									)
									.selectable(false),
								);
							}
						});
					});
				});
			});
	}
	fn clear_draft(&mut self, state: &mut State, channel: Id) {
		if self.draft_restore_pending {
			// Preserve an explicit clear until the outstanding disk snapshot is merged.
			state.drafts.insert(channel, String::new());
		} else {
			state.drafts.remove(&channel);
		}
		self.draft_changes.push(channel);
	}
	fn restore_pending(&mut self, state: &mut State, channel: Id, nonce: &str) {
		if let Some(index) = state.pending.iter().position(|p| {
			p.nonce == nonce && p.channel == channel && p.delivery != model::Delivery::Sending
		}) {
			if !state.drafts.contains_key(&channel) && state.drafts.len() >= 64 {
				state.status =
					"Draft budget full. Clear an existing draft before restoring pending text";
			} else if state.drafts.get(&channel).is_none_or(String::is_empty) {
				let pending = state.pending.remove(index);
				if !pending.attachments.is_empty() {
					state.status = "Text restored; reselect the attachment before sending again";
				}
				state.drafts.insert(channel, pending.content);
				self.draft_changes.push(channel);
			} else {
				state.status = "Keep or clear the existing draft before restoring pending text";
			}
		}
	}
	fn composer(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		channel: Id,
		ctx: &egui::Context,
		commands: &mut Vec<Command>,
	) {
		let had_edit = self.editing.is_some();
		self.reconcile_edit(state);
		let closed_here = self.edit_closed_channel.take() == Some(channel);
		if closed_here || (had_edit && self.editing.is_none()) {
			egui::text_edit::TextEditState::default()
				.store(ctx, ui.make_persistent_id("message-edit"));
			ui.weak("Message deleted. The unchanged edit was closed.");
			ctx.request_repaint();
			return;
		}
		let colors = crate::design::palette(ui);
		let keyboard_enabled = !self.switcher_frame
			&& !self.switcher.is_open()
			&& ctx.memory(|memory| memory.top_modal_layer().is_none());
		fn ime_updates_text(event: &egui::Event) -> bool {
			match event {
				egui::Event::Ime(
					egui::ImeEvent::Preedit { text, .. } | egui::ImeEvent::Commit(text),
				) => !text.is_empty(),
				egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { .. }) => true,
				_ => false,
			}
		}
		if keyboard_enabled
			&& self.editing.is_none()
			&& !self.ime_active
			&& !egui::Popup::is_any_open(ctx)
			&& ctx.memory(|memory| memory.has_focus(ui.make_persistent_id("message-input")))
			&& state.drafts.get(&channel).is_none_or(String::is_empty)
			&& ctx.input_mut(|input| {
				!input.events.iter().any(ime_updates_text)
					&& crate::keybinds::pressed_exact(
						input,
						self.keybinds.chord(model::KeybindAction::EditLastMessage),
					)
			}) && let Some(message) = state
			.timeline
			.iter()
			.rev()
			.find(|message| !message.unsupported && state.can_edit(channel, message.id))
		{
			self.editing = Some((channel, message.id, message.content.clone()));
			self.edit_modified = None;
			self.composer_edit = None;
		}
		let editing_key = self
			.editing
			.as_ref()
			.filter(|(c, _, _)| *c == channel)
			.map(|(c, id, _)| (*c, *id));
		let editing_here = editing_key.is_some();
		if let Some((_, id)) = editing_key {
			if state.timeline.get(id).is_some() {
				self.edit_undo_cleared = false;
			} else if !self.edit_undo_cleared {
				let editor_id = ui.make_persistent_id("message-edit");
				if let Some(mut editor) = egui::text_edit::TextEditState::load(ctx, editor_id) {
					editor.clear_undoer();
					editor.store(ctx, editor_id);
				}
				self.edit_undo_cleared = true;
			}
		}
		if !editing_here && !state.can_compose(channel) {
			if self.pending_mention.take().is_some() {
				state.status = "You don't have permission to mention anyone in this channel.";
			}
			// Returning to an edit must restore its focus after visiting a read-only channel.
			self.composer_edit = None;
			self.mention_menu = mentions::Menu::default();
			self.slash_commands.suspend(state, channel);
			self.emoji_picker = emoji_picker::Picker::default();
			self.ime_active = false;
			self.focus_switched_composer = false;
			egui::Frame::new()
				.fill(colors.raised)
				.corner_radius(8)
				.inner_margin(12)
				.show(ui, |ui| {
					let mut hint =
						"You don't have permission to send messages in this channel.".to_owned();
					ui.add_enabled(
						false,
						TextEdit::singleline(&mut hint)
							.desired_width(f32::INFINITY)
							.frame(egui::Frame::NONE),
					);
				});
			return;
		}
		let focus_edit =
			keyboard_enabled && editing_key.is_some() && self.composer_edit != editing_key;
		let focus_composer = keyboard_enabled
			&& (std::mem::take(&mut self.focus_switched_composer)
				|| (self.composer_edit != editing_key
					&& (editing_here || self.composer_edit.is_some_and(|(c, _)| c == channel))));
		if self.composer_edit != editing_key {
			self.composer_edit = editing_key;
			self.edit_sent = false;
			self.ime_active = false;
			self.mention_menu = mentions::Menu::default();
			self.emoji_picker = emoji_picker::Picker::default();
		}
		let mut cancel_edit = false;
		if ctx.input(|input| !input.raw.hovered_files.is_empty()) {
			let available = state.can_attach(channel) && !self.upload_busy && !editing_here;
			egui::Frame::new()
				.fill(colors.accent.gamma_multiply(0.12))
				.stroke(egui::Stroke::new(1.5, colors.accent))
				.corner_radius(12)
				.inner_margin(20)
				.show(ui, |ui| {
					ui.set_min_width((ui.available_width() - 2.0).max(0.0));
					ui.label(design::semibold(
						ui,
						if available {
							"Drop files to attach"
						} else {
							"Attachments unavailable right now"
						},
						18.0,
					));
					ui.add_space(6.0);
					ui.label(
						egui::RichText::new(if available {
							"Up to 10 files · 500 MB max · Account limit applies"
						} else {
							"Return to an available conversation after the current operation finishes"
						})
						.color(colors.muted),
					);
				});
			ui.add_space(8.0);
		}
		// Reply/edit context sits flush on top of the input as one rounded block, like Discord.
		let mut cap_top: Option<f32> = None;
		if editing_here {
			let unavailable = editing_key.is_some_and(|(_, id)| state.timeline.get(id).is_none());
			let cap = composer_cap(ui, &colors, |ui| {
				ui.label(
					RichText::new(if unavailable {
						"Message unavailable · unsent edit"
					} else {
						"Editing message"
					})
					.size(13.0)
					.color(colors.muted),
				);
				if self.edit_sent {
					ui.label(
						RichText::new("· Save requested, check the connection before retrying")
							.size(12.0)
							.color(colors.muted),
					);
				}
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					cancel_edit =
						icons::button(ui, icons::Icon::Close, 22.0, "Cancel edit").clicked();
					if unavailable
						&& ui
							.add(
								egui::Button::new(
									RichText::new("Copy edit text")
										.size(12.0)
										.color(colors.muted),
								)
								.frame(false),
							)
							.clicked() && let Some((_, _, text)) = &self.editing
					{
						ui.ctx().copy_text(text.clone());
					}
				});
			});
			cap_top = Some(cap.top());
		} else if let Some(reply) = state.reply {
			let author = state
				.timeline
				.get(reply.target())
				.map_or("an earlier message", |message| message.author.name.as_str())
				.to_owned();
			let cap = composer_cap(ui, &colors, |ui| {
				ui.spacing_mut().item_spacing.x = 0.0;
				ui.label(RichText::new("Replying to ").size(13.0).color(colors.muted));
				ui.label(design::semibold(ui, author.as_str(), 13.0).color(colors.text_strong));
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					ui.spacing_mut().item_spacing.x = 6.0;
					if icons::button(ui, icons::Icon::Close, 22.0, "Cancel reply").clicked() {
						state.reply = None;
					}
					if let Some(reply) = state.reply.as_mut() {
						mention_switch(ui, &colors, &mut reply.mention);
					}
					if ui
						.add_enabled(
							state.can_open_reply_target(reply.target()),
							egui::Button::new(
								RichText::new("View original")
									.size(12.0)
									.color(colors.muted),
							)
							.frame(false),
						)
						.on_disabled_hover_text(if state.timeline.is_deleted(reply.target()) {
							"The original message was deleted"
						} else {
							"Wait for readable, current message history"
						})
						.clicked()
					{
						self.timeline.request_reply_target(reply.target());
					}
				});
			});
			cap_top = Some(cap.top());
		}
		let full = state.draft_bytes() >= MAX_DRAFT_BYTES
			|| (!state.drafts.contains_key(&channel) && state.drafts.len() >= 64);
		if full && !editing_here {
			ui.label("Draft budget full. Clear an existing draft to continue.");
			if state.drafts.contains_key(&channel) && ui.button("Clear this draft").clicked() {
				self.clear_draft(state, channel);
			}
			return;
		}
		// Wayland/Fcitx can repeatedly emit empty preedits while idle. Only real
		// composition blocks shortcuts; dismissing an active composition still does.
		let ime_this_frame = keyboard_enabled
			&& (self.ime_active || ctx.input(|i| i.events.iter().any(ime_updates_text)));
		if keyboard_enabled {
			ctx.input(|i| {
				for event in &i.events {
					if let egui::Event::Ime(event) = event {
						match event {
							egui::ImeEvent::Preedit { text, .. } => {
								self.ime_active = !text.is_empty()
							}
							egui::ImeEvent::Commit(_) => self.ime_active = false,
							_ => {}
						}
					}
				}
			});
		}
		let composer_id = ui.make_persistent_id(if editing_here {
			"message-edit"
		} else {
			"message-input"
		});
		if editing_here {
			self.edit_widget_id = Some(composer_id);
		}
		if focus_composer {
			ctx.memory_mut(|m| m.request_focus(composer_id));
		}
		if focus_edit {
			let mut edit_state = egui::text_edit::TextEditState::default();
			let count = self
				.editing
				.as_ref()
				.map_or(0, |(_, _, content)| content.chars().count());
			edit_state
				.cursor
				.set_char_range(Some(egui::text::CCursorRange::one(
					egui::text::CCursor::new(count),
				)));
			edit_state.store(ctx, composer_id);
		}
		let pasted_text = self.pasted_text.take();
		let paste_key_released = ctx.input(|input| {
			input.events.iter().any(|event| {
				matches!(
					event,
					egui::Event::Key {
						key: egui::Key::V,
						pressed: false,
						..
					}
				)
			})
		});
		let paste_enabled = keyboard_enabled
			&& !editing_here
			&& self.slash_commands.active.is_none()
			&& !self.ime_active
			&& !ime_this_frame
			&& ctx.memory(|m| m.has_focus(composer_id));
		if let Some((paste_channel, target, text)) = pasted_text {
			if paste_enabled && paste_channel == channel && target == composer_id {
				ctx.input_mut(|i| i.events.push(egui::Event::Paste(text)));
				self.pasted_text_frame = Some(ctx.cumulative_frame_nr());
			} else {
				state.status = "Paste cancelled; focus the message and paste again";
			}
		} else if paste_enabled && self.pasted_text_frame != Some(ctx.cumulative_frame_nr()) {
			let mut request = AttachmentPaste {
				target: composer_id,
				text: None,
				image: None,
			};
			let mut requested = false;
			ctx.input_mut(|input| {
				// eframe consumes native paste key-down. File-only clipboards can emit
				// no Paste event, so the matching key-up is also a paste trigger.
				let shortcut = input.events.iter().any(|event| {
					matches!(event,
					egui::Event::Key { key: egui::Key::V, pressed, repeat: false, modifiers, .. }
					if !modifiers.shift && (modifiers.ctrl || modifiers.command || modifiers.alt)
						&& (*pressed || !self.paste_key_handled))
				});
				let has_paste = input.events.iter().any(|event| {
					matches!(event, egui::Event::Paste(_) | egui::Event::PasteImage(_))
				});
				if has_paste || shortcut {
					requested = true;
					self.paste_key_handled = true;
					input.events.retain_mut(|event| match event {
						egui::Event::Paste(text) => {
							request.text = Some(std::mem::take(text));
							false
						}
						egui::Event::PasteImage(image) => {
							request.image = Some(image.clone());
							false
						}
						egui::Event::Key {
							key: egui::Key::V, ..
						} => false,
						// Option+V can also produce a platform text character.
						egui::Event::Text(_) if shortcut => false,
						_ => true,
					});
				}
			});
			if requested {
				if request
					.text
					.as_ref()
					.is_some_and(|text| text.len() > MAX_DRAFT_BYTES)
				{
					state.status = "Pasted text exceeds the draft limit";
				} else if request
					.image
					.as_ref()
					.is_some_and(|image| image.pixels.len() > 4 * 1024 * 1024)
				{
					state.status = "Paste an image with at most 4 million pixels";
				} else {
					self.attachment_paste_requested = Some(request);
					self.upload_busy = true;
				}
			}
		}
		if paste_key_released {
			self.paste_key_handled = false;
		}
		if !editing_here
			&& state
				.drafts
				.get(&channel)
				.is_some_and(|draft| draft.starts_with('/'))
			&& let Some(command) = state.request_application_commands(channel, false)
		{
			commands.push(command);
		}
		let composer_content = if editing_here {
			self.editing
				.as_ref()
				.map_or("", |(_, _, text)| text.as_str())
		} else {
			state.drafts.get(&channel).map_or("", String::as_str)
		};
		let count_before = composer_content.chars().count();
		// Suggestion rows can take focus on press; keep the editor alive until release
		// so the shared member/channel/emoji popup can finish the click.
		let suggestion_pointer = self.mention_menu.pointer_interacting(ctx, channel)
			|| (self.slash_commands.active.is_none()
				&& self.slash_commands.pointer_interacting(ctx, channel));
		if keyboard_enabled && !self.ime_active && !ime_this_frame && suggestion_pointer {
			ctx.memory_mut(|memory| memory.request_focus(composer_id));
		}
		let mention_enabled = keyboard_enabled
			&& !self.ime_active
			&& !ime_this_frame
			&& ctx.memory(|m| m.has_focus(composer_id));
		self.slash_commands.refresh(
			state,
			channel,
			composer_content,
			!editing_here
				&& keyboard_enabled
				&& !self.ime_active
				&& !ime_this_frame
				&& (mention_enabled || self.slash_commands.active.is_some()),
		);
		let slash_pick = if !editing_here
			&& keyboard_enabled
			&& !self.ime_active
			&& !ime_this_frame
			&& (mention_enabled || self.slash_commands.form_has_focus(ctx))
		{
			self.slash_commands.keys(ctx)
		} else {
			None
		};
		let mention_users = if mention_enabled || composer_content.contains("<@") {
			mentions::known_users(state, channel)
		} else {
			Vec::new()
		};
		let mass_mentions =
			state.permission(channel, model::permissions::MENTION_EVERYONE) == Some(true);
		let cursor = egui::text_edit::TextEditState::load(ctx, composer_id)
			.and_then(|s| s.cursor.char_range())
			.filter(|r| r.is_empty())
			.map(|r| r.primary.index.0);
		self.mention_menu.refresh(
			state,
			channel,
			composer_content,
			cursor.filter(|_| mention_enabled),
			&mention_users,
		);
		let mention_pick = if mention_enabled {
			self.mention_menu.keys(ctx)
		} else {
			None
		};
		let mut editing = if editing_here {
			self.editing.take()
		} else {
			None
		};
		let demo = state.demo;
		let enter = keyboard_enabled
			&& !self.ime_active
			&& !ime_this_frame
			&& ctx.memory(|m| m.has_focus(composer_id))
			&& ctx.input_mut(|i| {
				crate::keybinds::pressed_exact(
					i,
					self.keybinds.chord(model::KeybindAction::SendMessage),
				)
			});
		let message_label = crate::i18n::text(self.language, "Message");
		let placeholder = state.channel(channel).map_or_else(
			|| message_label.to_owned(),
			|c| {
				if c.guild.is_some() {
					format!("{message_label} #{}", c.name)
				} else {
					format!("{message_label} @{}", state.conversation_name(c))
				}
			},
		);
		let can_attach = !editing_here
			&& self.slash_commands.active.is_none()
			&& state.can_attach(channel)
			&& !self.upload_busy
			&& self.attachment_files.len() < 10;
		let application_command = !editing_here && self.slash_commands.active.is_some();
		let attachment_sizes: Vec<u64> = self.selected_files().iter().map(|(_, b)| *b).collect();
		let attachments_within_limit =
			state.attachment_sizes_allowed(channel, &attachment_sizes);
		let can_send = if let Some((edit_channel, message)) = editing_key {
			state.freshness == Freshness::Fresh
				&& state.can_edit(edit_channel, message)
				&& count_before > 0
		} else if application_command {
			self.slash_commands.can_submit(state, channel)
		} else {
			state.can_send(channel)
				&& (self.attachment.is_none() || state.can_attach(channel))
				&& !self.upload_busy
				&& !(state.demo && self.attachment.is_some())
				&& (count_before > 0
					|| self.attachment.is_some()
					|| !self.attachment_files.is_empty())
				&& attachments_within_limit
		};
		if cap_top.is_some() {
			// The cap and the input form one block: undo the automatic vertical item gap.
			ui.add_space(-ui.spacing().item_spacing.y);
		}
		egui::Frame::new()
            .fill(colors.raised)
            .corner_radius(if cap_top.is_some() {
                egui::CornerRadius { nw: 0, ne: 0, sw: 8, se: 8 }
            } else {
                egui::CornerRadius::same(8)
            })
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                // Outer frame bounds for the autocomplete popout: undo the inner margin and
                // include the reply/edit cap so the popout never covers it.
                let composer_anchor = egui::Rect::from_min_max(
                    egui::pos2(ui.max_rect().left() - 10.0, cap_top.unwrap_or(ui.max_rect().top() - 6.0)),
                    egui::pos2(ui.max_rect().right() + 10.0, ui.max_rect().top()),
                );
                if !editing_here
                    && (!self.attachment_files.is_empty() || self.attachment.is_some())
                {
                    self.attachment_tray(ui, state, channel);
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    // An active application command shows its app in place of the attach button.
                    let attach = if !editing_here && self.slash_commands.composer_badge(ui, state, &mut self.avatars) {
                        None
                    } else {
                        Some(ui
                            .add_enabled_ui(can_attach, |ui| {
                                icons::button(ui, icons::Icon::Attach, 28.0, "Attach files")
                            })
                            .inner
                            .on_hover_text(crate::i18n::text(
                                self.language,
                                "Choose, drop, or paste files (Ctrl/Cmd/Option+V). Up to 10 files; each file must fit your upload limit. Send starts the upload.",
                            )))
                    };
                    if !editing_here { self.extensions.composer_menu(ui, state, self.language); }
                    if attach.is_some_and(|attach| attach.clicked()) {
                        self.attach_requested = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let send_button = ui
                            .add_enabled_ui(can_send, |ui| {
                                icons::button(
                                    ui,
                                    icons::Icon::Send,
                                    28.0,
                                    if editing_here { "Save edit" } else if application_command { "Send command" } else { "Send message" },
                                )
                            })
                            .inner;
                        let send = can_send && (enter || send_button.clicked());
                        let limit = state.message_char_limit();
                        if count_before + 200 >= limit {
                            ui.label(
                                RichText::new(format!("{}", limit.saturating_sub(count_before)))
                                    .size(11.0)
                                    .color(if count_before >= limit { colors.danger } else { colors.muted }),
                            );
                        }
                        let pick = ui
                            .add_enabled_ui(!application_command && !self.ime_active && !ime_this_frame, |ui| {
                                self.emoji_picker.image_sharing_enabled = self.image_sharing_enabled;
                                self.emoji_picker
                                    .show(ui, state, channel, &mut self.avatars, commands)
                            })
                            .inner;
                        // A chosen GIF is its own message; the typed draft stays untouched.
                        if self.emoji_picker.is_open() {
                            self.reaction_picker.dismiss(state, commands);
                        }
                        let pick = match pick {
                            Some(emoji_picker::Pick::Insert(text)) => {
                                self.reaction_picker.record(&text);
                                Some(text)
                            },
                            Some(emoji_picker::Pick::React(_, _)) => None,
                            Some(emoji_picker::Pick::Image(asset)) => {
                                if editing_here { state.status = "Finish or cancel the edit before attaching an image."; }
                                else if self.upload_busy { state.status = "Wait for the upload before attaching an image."; }
                                else if self.image_sharing_enabled && state.can_send(channel) && state.can_attach(channel) { self.image_share_requested = Some(asset); }
                                None
                            },
                            Some(emoji_picker::Pick::Sticker(sticker)) => {
                                if editing_here { state.status = "Finish or cancel the edit before sending a sticker."; }
                                else if self.upload_busy { state.status = "Wait for the upload before sending a sticker."; }
                                else if let Some(command) = state.prepare_sticker_send(&sticker) { self.timeline.follow_latest(state); commands.push(command); }
                                None
                            },
                            Some(emoji_picker::Pick::Send(url)) => {
                                if editing_here {
                                    state.status = "Finish or cancel the edit before sending a GIF.";
                                } else if self.upload_busy || !state.can_send(channel) {
                                    state.status = "Sending is unavailable with the current connection or permissions";
                                } else {
                                    let saved = state.drafts.insert(channel, url);
                                    let command = state.prepare_send();
                                    match saved {
                                        Some(saved) => {
                                            state.drafts.insert(channel, saved);
                                        }
                                        None => {
                                            state.drafts.remove(&channel);
                                        }
                                    }
                                    if let Some(command) = command {
                                        self.timeline.follow_latest(state);
                                        commands.push(command);
                                    }
                                }
                                None
                            }
                            None => None,
                        };
                        let edit = ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.vertical(|ui| {
                                ui.set_width(ui.available_width());
                        if application_command {
                            return ui.scope(|ui| {
                                ui.add_enabled_ui(keyboard_enabled, |ui| {
                                    self.slash_commands.composer(ui, state, channel, self.keybinds.chord(model::KeybindAction::SendMessage));
                                });
                                if self.slash_commands.active.is_none() {
                                    ctx.memory_mut(|memory| memory.request_focus(composer_id));
                                }
                                self.slash_commands.show(ui, composer_anchor, state, channel, &mut self.avatars);
                            }).response;
                        }
                        let composer_escape = keyboard_enabled
                            && !self.ime_active
                            && !ime_this_frame
                            && ctx.memory(|m| {
                                m.has_focus(composer_id) || m.had_focus_last_frame(composer_id)
                            });
                        cancel_edit |= editing_here
                            && composer_escape
                            && ctx.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                            });
                        if !editing_here
                            && state.reply.is_some()
                            && composer_escape
                            && ctx.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                            })
                        {
                            state.reply = None;
                        }
                        let remaining = if editing_here { MAX_CONTENT * 4 } else { MAX_DRAFT_BYTES.saturating_sub(state.draft_bytes()) };
                        // Temporarily own the buffer so suggestions can borrow the current
                        // permission state without cloning the draft or server catalogs.
                        let restore_empty_draft = self.draft_restore_pending && state.drafts.contains_key(&channel);
                        let mut new_draft = if editing_here { String::new() } else {
                            state.drafts.remove(&channel).unwrap_or_default()
                        };
                        let draft = if let Some((_, _, content)) = &mut editing {
                            content
                        } else {
                            &mut new_draft
                        };
						let mut mention_changed = false;
						match self.apply_pending_mention(ctx, composer_id, draft, remaining) {
							None => {}
							Some(MentionWrite::Inserted) => mention_changed = true,
							Some(MentionWrite::DidNotFit) => {
								state.status = "Mention will not fit. Shorten this message or free draft space.";
							}
						}
						if let Some(pick) = pick {
                            let mut edit_state =
                                egui::text_edit::TextEditState::load(ctx, composer_id).unwrap_or_default();
                            let range = edit_state.cursor.char_range();
                            if let Some(cursor) = emoji_picker::insert(draft, &pick, range, remaining) {
                                edit_state
                                    .cursor
                                    .set_char_range(Some(egui::text::CCursorRange::one(
                                        egui::text::CCursor::new(cursor),
                                    )));
                                edit_state.store(ctx, composer_id);
                                mention_changed = true;
                            } else {
                                state.status =
                                    "Emoji will not fit. Shorten this message or free draft space.";
                            }
                        }
                        if let Some(pick) = slash_pick
                            && let Some(cursor) = self.slash_commands.accept(pick, draft, remaining) {
                            let mut edit_state = egui::text_edit::TextEditState::load(ctx, composer_id).unwrap_or_default();
                            edit_state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(cursor))));
                            edit_state.store(ctx, composer_id);
                            mention_changed = true;
                        }
                        if let Some(pick) = mention_pick
                            && let Some(cursor) = mentions::insert(draft, pick)
                        {
                            let mut edit_state =
                                egui::text_edit::TextEditState::load(ctx, composer_id).unwrap_or_default();
                            edit_state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::one(
                                    egui::text::CCursor::new(cursor),
                                )));
                            edit_state.store(ctx, composer_id);
                            mention_changed = true;
                        }
						if mention_enabled
							&& let Some(cursor) = cursor
							&& let Some(cursor) =
								emoji_picker::complete_shortcode(draft, cursor, remaining)
						{
							let mut edit_state = egui::text_edit::TextEditState::load(ctx, composer_id)
								.unwrap_or_default();
							edit_state.cursor.set_char_range(Some(egui::text::CCursorRange::one(
								egui::text::CCursor::new(cursor),
							)));
							edit_state.store(ctx, composer_id);
							mention_changed = true;
						}
                        if keyboard_enabled
                            && !self.ime_active
                            && !ime_this_frame
                            && ctx.memory(|m| m.has_focus(composer_id))
							&& let Some(style) = formatting::Style::consume(ctx, &self.keybinds)
                        {
                            let mut edit_state =
                                egui::text_edit::TextEditState::load(ctx, composer_id).unwrap_or_default();
                            let range = edit_state.cursor.char_range();
                            if let Some(range) = formatting::apply(draft, style, range, remaining) {
                                edit_state.cursor.set_char_range(Some(range));
                                edit_state.store(ctx, composer_id);
                                mention_changed = true;
                            } else {
                                state.status = "Formatting will not fit. Shorten this message or free draft space.";
                            }
                        }
                        let rich_layout = &mut self.composer_layout;
                        if !self.ime_active
                            && !ime_this_frame
                            && ctx.input(|i| {
                                i.key_pressed(egui::Key::Backspace) || i.key_pressed(egui::Key::Delete)
                            })
                        {
                            rich_layout.galley(
                                ui,
                                draft,
                                ui.available_width(),
                                &mention_users,
                                mentions::known_roles(state, channel),
                                &state.channels,
                                mass_mentions,
                                &mut self.avatars,
                                demo,
                            );
                            rich_layout.select_deleted_inline(ctx, composer_id);
                        }
                        let mut output = egui::ScrollArea::vertical()
                            .id_salt((composer_id, channel, editing_key))
                            .max_height(ui.ctx().viewport_rect().height() * 0.5)
                            .min_scrolled_height(ui.ctx().viewport_rect().height() * 0.5)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, width: f32| {
                                    rich_layout.galley(
                                        ui,
                                        buffer.as_str(),
                                        width,
                                        &mention_users,
                                        mentions::known_roles(state, channel),
                                        &state.channels,
                                        mass_mentions,
                                        &mut self.avatars,
                                        demo,
                                    )
                                };
                                let output = TextEdit::multiline(draft)
                                    .interactive(keyboard_enabled)
                                    .layouter(&mut layouter)
                                    .id(composer_id)
                                    .event_filter(egui::EventFilter {
                                        horizontal_arrows: true, vertical_arrows: true, escape: editing_here,
                                        ..Default::default()
                                    })
                                    .desired_rows(1)
                                    .desired_width(f32::INFINITY)
                                    // Horizontal layouts reserve the interaction height, including around icons.
                                    .min_size(egui::vec2(0.0, ui.spacing().interact_size.y))
                                    .align(egui::Align2::LEFT_CENTER)
                                    .frame(egui::Frame::NONE)
                                    .hint_text(placeholder.as_str())
                                    .show(ui);
                                rich_layout.paint(ui, &output);
                                output
                            }).inner;
                        if !self.ime_active && !ime_this_frame {
                            rich_layout.snap_cursor(&mut output, ctx);
                        }
                        if mention_changed || (mention_enabled && suggestion_pointer) {
                            output.response.request_focus();
                        }
                        let mention_cursor = output
                            .cursor_range
                            .filter(|r| r.is_empty())
                            .map(|r| r.primary.index.0)
                            .or(cursor.filter(|_| suggestion_pointer))
                            .filter(|_| mention_enabled);
                        self.mention_menu
                            .refresh(state, channel, draft, mention_cursor, &mention_users);
                        if let Some(pick) = self.mention_menu.show(ui, composer_anchor, &mut self.avatars, demo)
                            && let Some(cursor) = mentions::insert(draft, pick)
                        {
                            output
                                .state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::one(
                                    egui::text::CCursor::new(cursor),
                                )));
                            output.state.clone().store(ctx, composer_id);
                            output.response.request_focus();
                            mention_changed = true;
                        }
                        #[cfg(not(feature = "demo"))]
                        let slash_preview = false;
                        #[cfg(feature = "demo")]
                        let slash_preview = state.demo && self.preview_slash_commands;
                        self.slash_commands.refresh(state, channel, draft,
                            !editing_here && keyboard_enabled && !self.ime_active && !ime_this_frame
                                && (output.response.has_focus() || suggestion_pointer || self.slash_commands.active.is_some() || slash_preview));
                        if let Some(pick) = self.slash_commands.show(ui, composer_anchor, state, channel, &mut self.avatars)
                            && let Some(cursor) = self.slash_commands.accept(pick, draft, remaining) {
                            output.state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(cursor))));
                            output.state.clone().store(ctx, composer_id);
                            output.response.request_focus();
                            mention_changed = true;
                        }
                        let edit = output.response;
                        let cleared = draft.is_empty();
                        if edit.changed() || mention_changed {
                            if editing_here && let Some((_, _, modified)) = &mut self.edit_modified {
                                *modified = true;
                            }
                            if editing_here {
                                self.edit_sent = false;
                            } else if cleared {
                                self.clear_draft(state, channel);
                            } else {
                                self.draft_changes.push(channel);
                            }
                        }
                        self.mention_lookup.text = self.mention_menu.member_query().to_owned();
                        self.mention_lookup.sync(ctx, state, channel, 0, commands);
                        if std::mem::take(&mut self.long_text_send_requested)
                            && !editing_here
                            && self.long_text_dialog.is_none()
                        {
                            // The fresh buffer only exists out here; the send site above
                            // merely recorded the request.
                            self.long_text_dialog = Some(LongTextDialog {
                                channel,
                                text: std::mem::take(&mut new_draft),
                                filename: LONG_TEXT_DEFAULT_FILENAME.into(),
                            });
                        }
                        if !editing_here && (!new_draft.is_empty() || restore_empty_draft) {
                            state.drafts.insert(channel, new_draft);
                        }
                        edit.response
                            }).inner
                        }).inner;
                        if std::mem::take(&mut self.slash_commands.retry)
                            && let Some(command) = state.request_application_commands(channel, true) {
                            commands.push(command);
                        }
                        let slash_run = std::mem::take(&mut self.slash_commands.run)
                            && keyboard_enabled && !self.ime_active && !ime_this_frame
                            && self.slash_commands.can_submit(state, channel);
                        if (send || slash_run) && !cancel_edit {
                            if let Some((edit_channel, message, content)) = &editing {
                                if state.freshness == Freshness::Fresh
                                    && let Some(command) = state.prepare_edit(*edit_channel, *message, content.clone())
                                {
                                    commands.push(command);
                                    self.edit_sent = true;
                                } else {
                                    state.status = "Edit kept. Wait for your current message and connection, and enter nonempty text.";
                                }
                            } else if send
                                && !application_command
                                && self.long_text_dialog.is_none()
                                && count_before > state.message_char_limit()
                                && state.can_attach(channel)
                            {
                                // Over-limit text is renamed and sent as `.txt` instead of
                                // being cut or rejected, but only where the account may
                                // attach files; the dialog opens on the next repaint once
                                // the fresh buffer is in scope.
                                self.long_text_send_requested = true;
                            } else if self.send_application_command(state, channel, commands)
                                || self.handle_builtin_slash(state, channel, ctx, commands) {
                            } else if !self.upload_busy && !(state.demo && self.attachment.is_some())
                                && let Some(command) = state.prepare_send_with_attachments(&self.selected_files().iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>()) {
                                // Consume the selection in this UI pass, before desktop dispatch.
                                // A second render or Send gesture must not enqueue it again.
                                self.stage_pending_upload(ctx, &command);
                                self.timeline.follow_latest(state);
                                commands.push(command);
                            }
                            self.show_long_text_dialog(ctx, state, channel, commands);
                            if !application_command { edit.request_focus(); }
                            if application_command && self.slash_commands.active.is_none() {
                                ctx.memory_mut(|memory| memory.request_focus(composer_id));
                            }
                        }
                    });
                });
                if let Some((edit_channel, message)) = editing_key {
                    if state.freshness != Freshness::Fresh || !state.can_edit(edit_channel, message) { ui.weak("Editing this message is unavailable. Your text is kept until you cancel."); }
                } else if !state.can_send(channel) && !application_command {
                    ui.weak("Sending messages is unavailable in this conversation. Your draft is kept.");
                } else if self.attachment.is_some() && !state.can_attach(channel) {
                    ui.weak("Attaching files is unavailable here. Remove the attachment to send only text.");
                }
            });
		if editing_here {
			self.editing = if cancel_edit { None } else { editing };
			if cancel_edit {
				self.edit_sent = false;
			}
		}
		if self.edit_sent
			&& self.editing.as_ref().is_some_and(|(channel, id, content)| {
				state.timeline.get(*id).is_some_and(|message| {
					message.channel == *channel && message.content == *content
				})
			}) {
			self.editing = None;
			self.edit_sent = false;
		}
	}
	/// Bounded, read-only preview of a text, markdown or code attachment. The
	/// body was fetched off the render thread; layout only happens here.
	pub fn show_preview(&mut self, ctx: &egui::Context) {
		if self.preview.is_none() {
			return;
		}
		let language = self.language;
		let mut close = false;
		// Move the pending external link out so the shared borrow of `preview`
		// cannot overlap a mutable borrow of the timeline.
		let mut opening = self.timeline.opening.take();
		let Some(preview) = self.preview.as_ref() else {
			return;
		};
		crate::dialog::Dialog::new("text-preview", preview.filename.clone())
			.width(640.0)
			.show(ctx, |d| {
				d.content(|ui| {
					if preview.truncated {
						ui.label(
							RichText::new(crate::i18n::text(
								language,
								"Showing the beginning of a large file.",
							))
							.size(12.0)
							.color(design::palette(ui).muted),
						);
						ui.add_space(4.0);
					}
					egui::ScrollArea::vertical()
						.max_height(420.0)
						.auto_shrink([false, false])
						.show(ui, |ui| {
							ui.set_width(ui.available_width());
							if preview.format == text_preview::PreviewFormat::Markdown {
								let mut images = crate::avatars::Avatars::default();
								let mut profile = crate::profiles::ProfileSession::default();
								markdown::Formatted::parse(&preview.text).show_with_images(
									ui,
									&mut opening,
									&[],
									None,
									&mut profile,
									(&mut images, false, &[]),
								);
							} else {
								ui.add(
									egui::Label::new(
										RichText::new(preview.text.as_str()).monospace().size(12.0),
									)
									.wrap()
									.selectable(true),
								);
							}
						});
				});
				d.footer(|ui| {
					if crate::dialog::action(
						ui,
						crate::i18n::text(language, "Close"),
						crate::dialog::Action::Neutral,
					)
					.clicked()
					{
						close = true;
					}
				});
			});
		if opening.is_some() {
			self.timeline.opening = opening;
		}
		if close {
			self.preview = None;
		}
	}
	/// Rename dialog for over-limit composer text. Enviar stages the text as a
	/// `.txt` attachment and dispatches it at once; Cancel keeps the draft.
	fn show_long_text_dialog(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		channel: Id,
		commands: &mut Vec<Command>,
	) {
		let Some(dialog) = self.long_text_dialog.as_mut() else {
			return;
		};
		if dialog.channel != channel {
			return;
		}
		let language = self.language;
		let ready = normalize_txt_filename(&dialog.filename)
			.as_deref()
			.is_some_and(valid_txt_filename);
		let mut send = false;
		let mut cancel = false;
		crate::dialog::Dialog::new(
			"long-text-rename",
			crate::i18n::text(language, "Send as text file?"),
		)
		.width(440.0)
		.show(ctx, |d| {
			d.content(|ui| {
				ui.label(
					egui::RichText::new(crate::i18n::text(
						language,
						"Your message is too long for chat, so it will be sent as a file.",
					))
					.size(13.0),
				);
				ui.add_space(4.0);
				crate::dialog::label(ui, crate::i18n::text(language, "File name"));
				if let Some(dialog) = self.long_text_dialog.as_mut() {
					crate::dialog::input(
						ui,
						egui::TextEdit::singleline(&mut dialog.filename)
							.hint_text(LONG_TEXT_DEFAULT_FILENAME),
					);
				}
				if !ready {
					ui.add_space(4.0);
					crate::dialog::notice(
						ui,
						crate::dialog::Level::Warning,
						crate::i18n::text(language, "That file name will not work."),
					);
				}
			});
			d.footer(|ui| {
				ui.add_enabled_ui(ready, |ui| {
					if crate::dialog::action(
						ui,
						crate::i18n::text(language, "Send"),
						crate::dialog::Action::Primary,
					)
					.clicked()
					{
						send = true;
					}
				});
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
		if cancel {
			if let Some(dialog) = self.long_text_dialog.take() {
				// The dialog owns the text, so cancelling returns it unless the
				// owner typed a fresh draft meanwhile.
				if !state.drafts.contains_key(&channel) {
					state.drafts.insert(channel, dialog.text);
				}
			}
		} else if send {
			self.send_long_text(ctx, state, channel, commands);
		}
	}

	/// Stages the dialog text as a `.txt` attachment and sends it through the
	/// regular attachment path. The dialog owns the text, so the composer draft
	/// is never touched here; anything that rejects the send puts the text back
	/// unless a fresh draft arrived meanwhile.
	fn send_long_text(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		channel: Id,
		commands: &mut Vec<Command>,
	) {
		let Some(dialog) = self.long_text_dialog.take() else {
			return;
		};
		let Some(filename) =
			normalize_txt_filename(&dialog.filename).filter(|name| valid_txt_filename(name))
		else {
			self.long_text_dialog = Some(dialog);
			return;
		};
		self.long_text_attachment = Some(LongTextAttachment {
			channel,
			filename: filename.clone(),
			bytes: dialog.text.clone().into_bytes(),
		});
		let mut names: Vec<String> = self
			.selected_files()
			.into_iter()
			.map(|(name, _)| name)
			.collect();
		names.push(filename);
		let refs: Vec<&str> = names.iter().map(String::as_str).collect();
		match state.prepare_send_with_attachments(&refs) {
			Some(command) => {
				self.stage_pending_upload(ctx, &command);
				self.timeline.follow_latest(state);
				commands.push(command);
			}
			None => {
				if let Some(staged) = self.long_text_attachment.take() {
					if !state.drafts.contains_key(&channel) {
						if let Ok(restored) = String::from_utf8(staged.bytes) {
							state.drafts.insert(channel, restored);
						}
					}
				}
			}
		}
	}

	/// Selected-file cards above the composer input, in the style of Discord's upload tray.
	fn attachment_tray(&mut self, ui: &mut egui::Ui, state: &State, channel: Id) {
		let colors = design::palette(ui);
		let textures = self.attachment_textures(ui.ctx());
		ui.add_space(4.0);
		let files = self.selected_files();
		let sizes: Vec<u64> = files.iter().map(|(_, bytes)| *bytes).collect();
		let limit = state.attachment_upload_limit(channel);
		let largest = sizes.iter().copied().max().unwrap_or(0);
		let over_limit = !state.attachment_sizes_allowed(channel, &sizes);
		attachments::pending_upload_meter(ui, largest, limit, over_limit);
		ui.add_space(4.0);
		egui::ScrollArea::horizontal()
			.id_salt("pending-attachments")
			.show(ui, |ui| {
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 12.0;
					for (index, (filename, bytes)) in files.iter().enumerate() {
						ui.push_id(index, |ui| {
							if attachments::pending_card(
								ui,
								filename,
								*bytes,
								textures.get(index).and_then(Option::as_ref),
								!self.upload_busy,
							) {
								self.remove_attachment_index = Some(index);
							}
						});
					}
				});
			});
		ui.add_space(8.0);
		let line = ui.max_rect().x_range();
		let y = ui.cursor().top();
		ui.painter()
			.hline(line, y, egui::Stroke::new(1.0, colors.border));
		ui.add_space(6.0);
	}
	fn shows_title_bar(&self) -> bool {
		!cfg!(target_os = "linux") && !self.hide_title_bar
	}
	fn apply_pending_mention(
		&mut self,
		ctx: &egui::Context,
		composer_id: egui::Id,
		draft: &mut String,
		remaining: usize,
	) -> Option<MentionWrite> {
		let user_id = self.pending_mention.take()?;
		if user_id == Id(0) {
			return None;
		}
		let loaded = egui::text_edit::TextEditState::load(ctx, composer_id);
		let had_editor = loaded.is_some();
		let mut edit_state = loaded.unwrap_or_default();
		let range = edit_state
			.cursor
			.char_range()
			.filter(|_| had_editor)
			.or_else(|| {
				Some(egui::text::CCursorRange::one(egui::text::CCursor::new(
					draft.chars().count(),
				)))
			});
		let start = range.map_or(draft.chars().count(), |range| {
			range
				.as_sorted_char_range()
				.start
				.0
				.min(draft.chars().count())
		});
		let glue = start > 0
			&& draft
				.chars()
				.nth(start - 1)
				.is_some_and(|c| !c.is_whitespace());
		let token = mentions::user_mention_token(user_id);
		let token = if glue { format!(" {token}") } else { token };
		let Some(cursor) = emoji_picker::insert(draft, &token, range, remaining) else {
			return Some(MentionWrite::DidNotFit);
		};
		edit_state
			.cursor
			.set_char_range(Some(egui::text::CCursorRange::one(
				egui::text::CCursor::new(cursor),
			)));
		edit_state.store(ctx, composer_id);
		Some(MentionWrite::Inserted)
	}
	fn drain_profile(&mut self, state: &mut State, commands: &mut Vec<Command>) -> bool {
		let mut cleared = false;
		for effect in self.profile.drain_effects() {
			match effect {
				profiles::ProfileEffect::ClearCore => {
					commands.push(state.clear_profile());
					self.profile_link = None;
					cleared = true;
				}
			}
		}
		cleared
	}

	fn sync_profile(
		&mut self,
		state: &mut State,
		commands: &mut Vec<Command>,
		user: &model::User,
		profile_guild: Option<model::Id>,
	) {
		if user.webhook {
			if state.profile.is_some() {
				commands.push(state.clear_profile());
			}
			return;
		}
		if state
			.profile
			.as_ref()
			.is_some_and(|view| view.user == user.id && view.guild == profile_guild)
		{
			return;
		}
		self.profile_link = None;
		#[cfg(any(test, feature = "demo"))]
		if state.demo {
			state.profile = Some(client_core::profile::ProfileView {
				user: user.id,
				guild: profile_guild,
				request: 0,
				loading: false,
				error: None,
				data: Some(
					state
						.own_profile
						.data
						.as_ref()
						.filter(|data| data.user.id == user.id)
						.cloned()
						.unwrap_or_else(|| profiles::synthetic(user, profile_guild)),
				),
			});
		}
		if !state.demo
			&& let Some(command) = state.request_profile(user.id, profile_guild)
		{
			commands.push(command);
		}
	}

	pub fn show(&mut self, ui: &mut egui::Ui, state: &mut State) -> Vec<Command> {
		crate::i18n::store_interface_language(ui.ctx(), self.language);
		crate::scroll::apply_preferences(ui.ctx(), self.reading_preferences);
		if let Some(status) = state.take_user_action_status() {
			self.toasts.push(design::Level::Error, status);
		}
		let mention_waiting = self.pending_mention.is_some();
		let composer_open = self.editing.is_some()
			|| state.selected.is_some_and(|id| {
				state
					.channel(id)
					.is_some_and(|channel| channel.supports_text())
			});
		if mention_waiting && !composer_open {
			self.pending_mention = None;
			state.status = "This view has no message box, so the mention was not added.";
		}
		if self.editing.is_none()
			&& let Some(index) = state
				.message_actions
				.failed_edits
				.iter()
				.position(|(channel, _, _)| Some(*channel) == state.selected)
		{
			let (channel, message, content) = state.message_actions.failed_edits.remove(index);
			self.editing = Some((channel, message, content));
			self.edit_modified = Some((channel, message, true));
			self.edit_sent = false;
		}

		if self
			.pending_upload
			.as_ref()
			.is_some_and(|upload| !state.pending.iter().any(|p| p.nonce == upload.nonce))
		{
			self.pending_upload = None;
		}
		self.timeline.audio.seen = false;
		self.timeline.video.seen = false;
		let side = self.drain_side_press();
		let mut commands = Vec::new();
		if self.timeline.batch_delete_cancel {
			self.timeline.batch_delete_cancel = false;
			self.batch_delete_queue.clear();
			self.batch_delete_next = None;
			self.timeline.batch_progress = None;
		}
		if let Some(due) = self.batch_delete_next
			&& ui.input(|input| input.time) >= due
			&& let Some((channel, message)) = self.batch_delete_queue.first().copied()
		{
			self.batch_delete_queue.remove(0);
			// Arm the ~180ms removal fade; the echo frees the space with
			// scroll compensation, or the fade releases on failure.
			self.timeline
				.removing
				.insert(message, ui.input(|input| input.time));
			if let Some(command) = state.prepare_delete(channel, message) {
				commands.push(command);
			}
			self.batch_delete_sent += 1;
			if self.batch_delete_queue.is_empty() {
				self.batch_delete_next = None;
				self.timeline.batch_progress = None;
			} else {
				self.batch_delete_next =
					Some(ui.input(|input| input.time) + batch_delete_gap_secs());
				self.timeline.batch_progress =
					Some((self.batch_delete_sent, self.batch_delete_total));
				ui.ctx()
					.request_repaint_after(std::time::Duration::from_millis(200));
			}
		} else if self.batch_delete_next.is_some() {
			ui.ctx()
				.request_repaint_after(std::time::Duration::from_millis(200));
		}
		if (side.back || side.forward)
			&& !self.timeline.video.is_fullscreen()
			&& !self.channel_menu.is_open()
		{
			if self.settings.open {
				if side.back {
					self.settings.open = false;
				}
			} else if self.server_settings.is_open() {
				if side.back {
					let _ = self.server_settings.navigate_away(state);
				}
			} else {
				let step = if side.back {
					state.navigate_back()
				} else {
					state.navigate_forward()
				};
				if let Some(NavStep {
					command: Some(command),
				}) = step
				{
					commands.push(command);
				}
			}
		}
		// Fullscreen media owns the whole client surface, including during native resizing.
		if self.timeline.show_fullscreen_video(ui.ctx(), state) {
			ui.painter()
				.rect_filled(ui.max_rect(), 0, egui::Color32::BLACK);
			return commands;
		}
		if self.show_voice_stream_fullscreen(ui.ctx(), state) {
			ui.painter()
				.rect_filled(ui.max_rect(), 0, egui::Color32::BLACK);
			return commands;
		}
		self.profile.disarm();
		ui.ctx().data_mut(|data| {
			data.remove::<egui::Rect>(profiles::profile_opener_id());
		});
		let ctx = ui.ctx().clone();
		self.extensions.reset_theme_shortcut(&ctx);
		if self.extensions.has_result() {
			self.settings.open = false;
		}
		if let Some(effect) = self.extensions.show_result(
			&ctx,
			state,
			&mut self.draft_changes,
			self.editing.is_some(),
			self.language,
		) && let Err(error) = self.apply_extension_effect(&ctx, state, effect, &mut commands)
		{
			self.extensions.report_error(error);
		}
		self.keybinds_shortcut(&ctx);
		self.theme_preview_navigation(ui);
		let settings_open =
			self.settings.open || self.server_settings.is_open() || self.channel_menu.is_open();
		self.extensions.begin_theme_editor_frame();
		if self.settings.open {
			self.show_settings(&ctx, state, &mut commands);
			if self.extensions.previewing_theme() {
				self.settings.open = false;
			}
			ui.disable();
		}
		if self.server_settings.is_open() {
			self.extensions.stop_theme_preview(&ctx);
			if self.profile.open_user().is_some()
				&& ctx
					.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
			{
				self.profile.close();
			}
			self.server_settings.show(
				&ctx,
				state,
				&mut self.avatars,
				&mut self.profile,
				&mut commands,
				self.language,
			);
			if self.profile.open_user().is_some() {
				ctx.set_sublayer(
					egui::LayerId::new(
						egui::Order::Foreground,
						egui::Id::unique("server-settings"),
					),
					egui::LayerId::new(
						egui::Order::Foreground,
						egui::Id::unique("user-profile-popout"),
					),
				);
			}
			if let Some(action) = self.server_settings.admin.user_action.take() {
				self.user_action = Some(action);
			}
			if let Some(channel) = self.server_settings.admin.message.take()
				&& self.server_settings.navigate_away(state)
				&& let Some(command) = state.select(channel)
			{
				commands.push(command);
			}
			ui.disable();
		}
		// Foreground confirmation handles Escape before background search/archive shortcuts.
		self.confirm_friend_removal(&ctx, state, &mut commands);
		if let Some(link) = self
			.timeline
			.opening
			.as_deref()
			.and_then(markdown::discord_chat_link)
		{
			self.timeline.opening = None;
			match state.open_chat_link(link.guild, link.channel, link.message) {
				Ok(Some(command)) => commands.push(command),
				Ok(None) => {}
				Err(message) => state.status = message,
			}
		}
		markdown::confirm_external_link(
			&ctx,
			&mut self.timeline.browser_opening,
			self.reading_preferences.confirm_external_links,
		);
		markdown::confirm_external_link(
			&ctx,
			&mut self.timeline.opening,
			self.reading_preferences.confirm_external_links,
		);
		let colors = crate::design::palette(ui);
		let background = crate::design::window_palette(ui);
		if !settings_open
			&& !self.switcher.is_open()
			&& !self.ime_active
			&& ctx.input(|input| {
				input.focused
					&& !input
						.events
						.iter()
						.any(|event| matches!(event, egui::Event::Ime(_)))
			}) && ctx.input_mut(|input| {
			crate::keybinds::pressed(
				input,
				self.keybinds
					.chord(model::KeybindAction::SwitchConversation),
			)
		}) {
			self.switcher.open(&ctx);
		}
		self.switcher_frame = self.switcher.is_open();
		if let Some(command) = state.select_opened_dm() {
			commands.push(command);
		}
		self.finish_slash_direct(state);
		if let Some(target) = self.switcher.show(&ctx, state, &mut self.avatars) {
			match target {
				switcher::Target::Channel(channel) => {
					if state.selected != Some(channel)
						&& let Some(command) = state.select(channel)
					{
						commands.push(command);
					}
					self.focus_switched_composer = state.selected == Some(channel)
						&& state
							.channel(channel)
							.is_some_and(|known| known.supports_text());
				}
				switcher::Target::Friend(user) => {
					if let Some(command) = state.open_friend_dm(user) {
						commands.push(command);
					}
				}
			}
		}
		if self.navigation_channel != state.selected {
			self.focus_switched_composer = state.selected.is_some();
			self.navigation_channel = state.selected;
			self.guild = state
				.selected
				.and_then(|id| state.channel(id))
				.and_then(|channel| channel.guild);
		}
		let title = self.guild.and_then(|id| state.guild(id)).map_or_else(
			|| crate::i18n::text(self.language, "Direct Messages").to_owned(),
			|g| g.name.clone(),
		);
		if self.shows_title_bar() {
			self.title_bar(ui, state, &title);
		}
		// Server rail and channel list share one resizable column so the account card can
		// span both, like Discord's bottom-left user pill.
		let rail = notifications::RAIL_WIDTH;
		let sidebar_max = self.prepare_reading_sidebar(ui, "navigation", rail);
		let navigation = egui::Panel::left("navigation")
			.resizable(true)
			.show_separator_line(!design::has_window_background(ui))
			.default_size(rail + f32::from(self.reading_preferences.sidebar_width).min(sidebar_max))
			.size_range(rail + 190.0..=rail + sidebar_max)
			.frame(
				egui::Frame::new()
					.fill(if design::has_window_background(ui) {
						egui::Color32::TRANSPARENT
					} else {
						background.base
					})
					.inner_margin(0),
			)
			.show(ui, |ui| {
				egui::Panel::bottom("account-footer")
					.show_separator_line(false)
					.frame(
						egui::Frame::new()
							// The card's strip continues the channel list, so a window image
							// shows through it at the same opacity instead of full strength.
							.fill(if design::has_window_background(ui) {
								design::section_surface(
									ui,
									background.sidebar,
									design::ImageSection::ChannelList,
								)
							} else {
								egui::Color32::TRANSPARENT
							})
							.inner_margin(egui::Margin {
								left: 8,
								right: 8,
								top: 8,
								bottom: 8,
							}),
					)
					.show(ui, |ui| self.account_card(ui, state, &mut commands));
				self.notification_rail(ui, state, &mut commands);
				// The lists sit on their own rounded surface beside the rail, above the card.
				ui.painter().rect_filled(
					ui.available_rect_before_wrap(),
					if design::has_window_background(ui) {
						egui::CornerRadius::ZERO
					} else {
						egui::CornerRadius {
							nw: 8,
							sw: 8,
							..Default::default()
						}
					},
					design::section_surface(
						ui,
						background.sidebar,
						design::ImageSection::ChannelList,
					),
				);
				self.sidebar(ui, state, &title, &mut commands);
			});
		self.record_reading_sidebar(navigation.response.rect.width() - rail);
		if self.channel_preferences_changed {
			self.channel_cache.invalidate();
		}
		self.channel_menu
			.show(&ctx, state, self.guild, &mut self.avatars, &mut commands);
		if let Some((guild, channel)) = self.channel_menu.invite_requested.take() {
			self.server_menu.open_invite(state, guild, channel);
		}
		self.server_menu.show(
			&ctx,
			state,
			self.guild,
			&mut commands,
			&mut self.avatars,
			self.language,
		);
		if let Some(guild) = self.server_menu.settings_requested.take()
			&& let Some(command) = self.preview_server_settings(state, guild)
		{
			commands.push(command);
		}
		if let Some(guild) = self.server_menu.mark_read_requested.take()
			&& let Some(command) = state.prepare_mark_guild_read(guild)
		{
			commands.push(command);
		}
		let selected_voice = state
			.selected
			.and_then(|id| state.channel(id))
			.is_some_and(|c| c.kind == 2);
		let selected_forum = state.selected.is_some_and(|id| state.is_forum(id));
		if let Some(id) = state.posting.created.take()
			&& let Some(command) = state.select(id)
		{
			commands.push(command);
		}
		let wide_members = ui.available_width() >= 720.0;
		self.search.sync(&ctx, state, &mut commands);
		let search_open =
			self.search.results_visible(state) && state.selected.is_some() && !selected_voice;
		let direct_message = state
			.selected
			.and_then(|id| state.channel(id))
			.is_some_and(|channel| channel.kind == 1 && channel.guild.is_none());
		let show_members = !direct_message
			&& !selected_voice
			&& !search_open
			&& state.selected.is_some()
			&& if wide_members {
				self.reading_preferences.show_members
			} else {
				self.members_narrow_open
			};
		if search_open {
			self.channel_header(
				ui,
				state,
				selected_voice,
				show_members,
				wide_members,
				&mut commands,
			);
			let width = if wide_members {
				search::PANE_WIDTH.min(ui.available_width() * 0.45)
			} else {
				(ui.available_width() * 0.6).max(240.0)
			};
			egui::Panel::right("search-pane")
				.resizable(false)
				.show_separator_line(!design::has_window_background(ui))
				.exact_size(width)
				.frame(
					egui::Frame::new()
						.fill(design::section_surface(
							ui,
							background.sidebar,
							design::ImageSection::MemberList,
						))
						.inner_margin(egui::Margin::same(12)),
				)
				.show(ui, |ui| {
					self.search.pane(
						ui,
						state,
						&mut commands,
						&mut self.avatars,
						search::MediaUi {
							download: &mut self.timeline.download,
							audio: &mut self.timeline.audio,
							video: &mut self.timeline.video,
						},
						&mut self.profile,
					);
				});
		}
		if show_members {
			if state
				.members
				.as_ref()
				.is_none_or(|list| Some(list.channel) != state.selected)
				&& let Some(command) = state.request_members()
			{
				commands.push(command);
			}
			if wide_members {
				egui::Panel::right("people-pane")
					.resizable(false)
					.show_separator_line(!design::has_window_background(ui))
					.exact_size(240.0)
					.frame(
						egui::Frame::new()
							.fill(design::section_surface(
								ui,
								background.sidebar,
								design::ImageSection::MemberList,
							))
							.inner_margin(egui::Margin {
								left: 8,
								right: 8,
								top: 8,
								bottom: 8,
							}),
					)
					.show(ui, |ui| {
						self.member_rows(ui, state, &mut commands);
					});
			} else {
				let response = dialog::Dialog::new("members-narrow", "Members")
					.subtitle("Everyone with access to this conversation.")
					.width(360.0)
					.show(&ctx, |d| {
						let max = (d.available_height() - 180.0).clamp(120.0, 620.0);
						d.content(|ui| {
							ui.set_max_height(max);
							self.member_rows(ui, state, &mut commands);
						});
					});
				if response.close {
					self.members_narrow_open = false;
				}
			}
		} else if state.members.is_some() {
			commands.push(state.close_members());
		}
		self.reaction_picker.sync(state, state.selected);
		egui::CentralPanel::default()
			.frame(
				egui::Frame::new()
					.fill(if design::has_section_background(ui) {
						egui::Color32::TRANSPARENT
					} else {
						background.chat
					})
					.inner_margin(0),
			)
			.show(ui, |ui| {
				let message_surface =
					design::section_surface(ui, background.chat, design::ImageSection::MessageList);
				if design::has_section_background(ui)
					&& (state.selected.is_none() || selected_voice || selected_forum)
				{
					ui.painter().rect_filled(ui.max_rect(), 0, message_surface);
				}
				let warnings = state.startup_warnings;
				if !warnings.presence {
					self.friends.presence_warning_dismissed = None;
				}
				let unavailable: Vec<_> = [
					(warnings.read_state, "read status"),
					(warnings.notifications, "notification settings"),
					(warnings.sessions, "session status"),
					(warnings.emojis, "some server emoji"),
					(warnings.stickers, "some server stickers"),
				]
				.into_iter()
				.filter_map(|(unavailable, label)| unavailable.then_some(label))
				.collect();
				if !unavailable.is_empty() {
					egui::Frame::new()
						.inner_margin(egui::Margin::symmetric(16, 6))
						.show(ui, |ui| {
							ui.colored_label(
								colors.warning,
								format!(
									"Connected with some data unavailable: {}. Reconnect to retry.",
									unavailable.join(", ")
								),
							);
						});
				}
				self.reconnect_call_banner(ui, state, &mut commands);
				if state.selected.is_none() && self.guild.is_none() {
					self.call_bar(ui, state, &mut commands);
					self.timeline.download.show_status(ui);
					self.friends_page(ui, state, &mut commands);
					return;
				}
				if !search_open {
					self.channel_header(
						ui,
						state,
						selected_voice,
						show_members,
						wide_members,
						&mut commands,
					);
				}
				let message_fill = (design::has_section_background(ui)
					&& state.selected.is_some()
					&& !selected_voice
					&& !selected_forum)
					.then(|| {
						(
							ui.painter().add(egui::Shape::Noop),
							ui.available_rect_before_wrap().top(),
						)
					});
				self.call_bar(ui, state, &mut commands);
				self.timeline.download.show_status(ui);
				let Some(channel) = state.selected else {
					ui.add_space((ui.available_height() * 0.32).max(24.0));
					ui.vertical_centered(|ui| {
						ui.label(
							design::semibold(ui, "No conversation selected", 20.0)
								.color(colors.text_strong),
						);
						ui.add_space(8.0);
						ui.label(
							RichText::new("Pick a channel or direct message from the list.")
								.color(colors.muted),
						);
					});
					return;
				};
				if selected_voice {
					if !self.voice_chat_open {
						self.voice_channel(ui, state, channel, &mut commands);
						return;
					}
					// Keep the existing conversation renderer beside the stage. On narrow
					// windows chat takes the body; Hide chat returns to the full stage.
					if ui.available_width() >= 720.0 {
						let chat_width = (ui.available_width() * 0.4).clamp(320.0, 440.0);
						egui::Panel::left("voice-stage")
							.exact_size(ui.available_width() - chat_width)
							.resizable(false)
							.frame(egui::Frame::NONE)
							.show(ui, |ui| {
								self.voice_channel(ui, state, channel, &mut commands);
							});
					}
				}
				if selected_forum {
					// A post's first message uses the same upload tray as the composer.
					let textures = self.attachment_textures(ui.ctx());
					let files = self.selected_files();
					let view = shortcuts::ShortcutView::new(
						&self.channel_preferences,
						self.shortcuts_available(state),
					);
					let mut staged = forum::Staged {
						files: &files,
						textures: &textures,
						choose: &mut self.attach_requested,
						remove: &mut self.remove_attachment_index,
						clear: &mut self.remove_attachment_requested,
						busy: self.upload_busy,
					};
					self.forum.show(
						ui,
						state,
						channel,
						&mut commands,
						(&mut self.scroll, &mut staged),
						(&mut self.channel_menu, view),
						self.language,
					);
					return;
				}
				egui::Panel::bottom("composer")
					.show_separator_line(false)
					.frame(
						egui::Frame::new()
							.fill(design::section_surface(
								ui,
								background.chat,
								design::ImageSection::Composer,
							))
							// The bottom inset matches the account card's, so the input and the
							// account pill sit on one line across the window.
							.inner_margin(egui::Margin {
								left: 16,
								right: 16,
								top: 2,
								bottom: 8,
							}),
					)
					.show(ui, |ui| {
						self.composer(ui, state, channel, &ctx, &mut commands);
					});
				if let Some((shape, top)) = message_fill {
					let remaining = ui.available_rect_before_wrap();
					ui.painter().set(
						shape,
						egui::Shape::rect_filled(
							egui::Rect::from_min_max(
								egui::pos2(remaining.left(), top),
								remaining.right_bottom(),
							),
							0.0,
							message_surface,
						),
					);
				}
				let notices: Vec<String> = [
					match state.freshness {
						Freshness::Fresh | Freshness::Loading => None,
						Freshness::Stale => Some("Cached history · awaiting sync"),
						Freshness::Unavailable => Some("Conversation unavailable"),
					}
					.map(str::to_owned),
					state.read_state.status(channel).map(str::to_owned),
					(state.archived_thread.is_some() && state.archived_thread == state.selected)
						.then(|| "Opened from archive".to_owned()),
				]
				.into_iter()
				.flatten()
				.collect();
				if !notices.is_empty() {
					egui::Frame::new()
						.inner_margin(egui::Margin::symmetric(16, 4))
						.show(ui, |ui| {
							ui.label(
								RichText::new(notices.join(" · "))
									.size(11.0)
									.color(colors.muted),
							);
						});
				}
				egui::Frame::new()
					.inner_margin(egui::Margin {
						left: 0,
						right: 0,
						top: 4,
						bottom: 0,
					})
					.show(ui, |ui| {
						design::paint_chat_background(ui, ui.available_rect_before_wrap());
						self.timeline.hide_media_links = self.reading_preferences.hide_media_links;
						self.timeline.instant_scrolling =
							!self.reading_preferences.smooth_scrolling;
						self.timeline.extension_actions = self.extensions.message_actions();
						let mut seen = std::collections::BTreeSet::new();
						let author_lookup: Vec<_> = state
							.timeline
							.iter()
							.rev()
							.filter(|message| !message.author.webhook)
							.map(|message| message.author.id)
							.filter(|id| seen.insert(*id))
							.take(client_core::member_search::LIMIT)
							.collect();
						if let Some(command) = state.request_author_members(&author_lookup) {
							commands.push(command);
						}
						self.timeline.language = self.language;
						self.timeline.show_with_scroll(
							ui,
							state,
							&mut self.editing,
							&mut self.deleting,
							(&mut self.avatars, &mut self.profile),
							self.pending_upload.as_ref(),
							&mut self.scroll,
						);
						if let Some(id) = self.timeline.sticker_request.take() {
							if let Some(command) = state.request_sticker(id) {
								commands.push(command);
							}
							if let Some(command) = state.request_sticker_packs() {
								commands.push(command);
							}
						}
						if let Some(sticker) = self.timeline.browse_sticker.take() {
							self.emoji_picker.open_stickers(Some(&sticker));
						}
						if let Some(command) =
							state.request_author_members(&self.timeline.visible_authors)
						{
							commands.push(command);
						}
						if let Some((action, text)) = self.timeline.extension_request.take() {
							self.extensions
								.invoke_message(action, text, state, ui.ctx());
						}
						if let Some((message, anchor, trigger)) =
							self.timeline.reaction_picker.take()
						{
							self.emoji_picker.dismiss(state, &mut commands);
							self.reaction_picker
								.open_reaction(state, message, anchor, trigger);
						}
						self.reaction_picker.show_reaction(
							ui,
							state,
							&mut self.avatars,
							&mut commands,
						);
						if let Some((message, emoji, open)) = self.timeline.reaction_users.take()
							&& let Some(command) =
								state.request_reaction_users(message, emoji, open)
						{
							commands.push(command);
						}
						crate::reactions::show_users(
							ui.ctx(),
							state,
							&mut self.avatars,
							&mut commands,
						);
						if let Some((channel, message)) = self.timeline.quick_delete.take()
							&& let Some(command) = state.prepare_delete(channel, message)
						{
							commands.push(command);
						}
						// In-place edit commit: same validation as the composer path.
						if let Some(command) = self.timeline.poll_inline_commit(state) {
							commands.push(command);
						}
						if self.timeline.batch_delete_requested {
							self.timeline.batch_delete_requested = false;
							if let Some(channel) = state.selected {
								let ids: Vec<_> =
									self.timeline.batch_delete.iter().copied().collect();
								if !ids.is_empty() && ids.len() <= 5 {
									self.deleting_batch = Some((channel, ids));
								}
							}
						}
						if let Some(id) = self.timeline.remove_preserved.take() {
							state.discard_preserved_deleted(id);
						}
						if let Some(nonce) = self.timeline.restore_pending.take() {
							if state
								.pending
								.iter()
								.any(|p| p.nonce == nonce && p.sticker.is_some())
							{
								state.discard_pending_sticker(&nonce);
							} else {
								self.restore_pending(state, channel, &nonce);
							}
						}
						self.cancel_upload_requested |=
							std::mem::take(&mut self.timeline.cancel_upload);
						if let Some(gif) = self.timeline.gif_favorite.take() {
							state.toggle_gif_favorite(&gif);
						}
						match self.timeline.invite_action.take() {
							Some(invites::Action::OpenGuild(guild))
								if state.guild(guild).is_some() =>
							{
								self.guild = Some(guild);
								if let Some(command) = state.select_guild(guild) {
									commands.push(command);
								}
							}
							Some(invites::Action::Join(code)) => {
								if state.invite_join.code == code
									&& state.invite_challenge().is_some()
								{
									self.verification.active = false;
								} else if let Some(command) = state.join_invite(code) {
									commands.push(command);
								}
							}
							_ => {}
						}
						for code in std::mem::take(&mut self.timeline.invite_requests) {
							if let Some(command) = state.request_invite(code) {
								commands.push(command);
							}
						}
						if std::mem::take(&mut self.timeline.edit_started) {
							self.edit_modified = None;
							self.edit_undo_cleared = false;
							self.composer_edit = None;
							self.edit_sent = false;
						}
						if std::mem::take(&mut self.timeline.reply_started) {
							self.focus_switched_composer = true;
							ctx.request_repaint();
						}
						if let Some(target) = self.timeline.reply_target.take() {
							if let Some(command) = state.open_reply_target(target) {
								commands.push(command);
							}
							ctx.request_repaint();
						} else if std::mem::take(&mut self.timeline.unread_jump) {
							if let Some(command) = state.open_unread() {
								commands.push(command);
							}
							ctx.request_repaint();
						} else if std::mem::take(&mut self.timeline.load_newer) {
							if let Some(command) = state.newer_history() {
								commands.push(command);
							}
							ctx.request_repaint();
						} else if std::mem::take(&mut self.timeline.latest) {
							commands.push(state.history(None));
						} else if std::mem::take(&mut self.timeline.load_older)
							&& let Some(command) = state.older_history()
						{
							commands.push(command);
						}
					});
			});
		if state
			.archives
			.as_ref()
			.is_some_and(|view| self.guild != Some(view.guild))
		{
			commands.push(state.clear_archives());
		}
		if let Some((channel, message, pinned)) = self.timeline.pin_request.take()
			&& let Some(command) = state.prepare_pin(channel, message, pinned)
		{
			commands.push(command);
		}
		if let Some(channel) = state.pins_changed.take()
			&& self.search.open
			&& self.search.pins()
			&& state.selected == Some(channel)
			&& let Some(command) = state.request_pins()
		{
			commands.push(command);
		}
		self.search
			.overlays(&ctx, state, &mut self.avatars, &mut commands);
		if state.invite_challenge().is_none() {
			self.join_server
				.show(&ctx, state, &mut self.avatars, &mut commands);
		}
		if let Some(anchor) = self.pins_anchor {
			let dm = state
				.channels
				.iter()
				.any(|c| Some(c.id) == state.selected && c.guild.is_none());
			self.search.pins_popout(
				ui,
				state,
				anchor,
				dm,
				&mut commands,
				&mut self.avatars,
				search::MediaUi {
					download: &mut self.timeline.download,
					audio: &mut self.timeline.audio,
					video: &mut self.timeline.video,
				},
				&mut self.profile,
			);
			if !(self.search.open && self.search.pins()) {
				self.pins_anchor = None;
			}
		}
		if let Some(link) = self.search.opening.take() {
			self.timeline.opening = Some(link);
		}
		if let Some(channel) = self.search.channel_reference.take() {
			self.timeline.channel_reference = Some(channel);
		}
		// A forum pane lists its own archived posts inline instead of the floating window.
		if !(selected_forum
			&& state
				.archives
				.as_ref()
				.is_some_and(|view| Some(view.parent) == state.selected))
		{
			self.archives
				.show(&ctx, state, &mut commands, &mut self.avatars);
		}
		if let Some(parent) = self.timeline.threads_request.take() {
			self.archive_parent = Some(parent);
		}
		if let Some((channel, message)) = self.timeline.thread_request.take() {
			state.clear_channel_action_result(channel);
			// Discord seeds the name from the starter's first line; the user can still edit it.
			let name = state
				.timeline
				.get(message)
				.map(|m| thread_create::suggested_name(&m.display_text()))
				.unwrap_or_default();
			self.thread_create.open(channel, Some(message), name);
		}
		// A thread shows the message it hangs off; read it lazily like other per-channel data.
		if let Some(command) = state.request_thread_starter() {
			commands.push(command);
		}
		if let Some(channel) = self.archives.create_requested.take() {
			// The editor replaces the Threads dialog instead of stacking on it.
			commands.push(state.clear_archives());
			state.clear_channel_action_result(channel);
			self.thread_create.open(channel, None, String::new());
		}
		self.thread_create.show(&ctx, state, &mut commands);
		self.screen.show(&ctx, state, self.language);
		if let Some(id) = self.timeline.channel_reference.take() {
			state.clear_channel_action_result(id);
			self.timeline.pending_channel_reference = Some(id);
		}
		if let Some(id) = self.timeline.pending_channel_reference {
			if let Some(target) = state
				.channel(id)
				.filter(|c| c.guild.is_some() && (c.supports_text() || matches!(c.kind, 15 | 16)))
			{
				self.timeline.pending_channel_reference = None;
				self.guild = target.guild;
				if let Some(command) = state.select(id) {
					commands.push(command);
				}
			} else if state.channel_action_status(id).is_some() {
				self.timeline.pending_channel_reference = None;
			} else if let Some(guild) = state
				.selected
				.and_then(|selected| state.channel(selected))
				.and_then(|source| source.guild)
			{
				if let Some(command) = state.request_channel_reference(guild, id) {
					commands.push(command);
				}
			} else {
				self.timeline.pending_channel_reference = None;
			}
		}
		if let Some(pending) = self.timeline.leave_read.take() {
			if ctx.will_discard() {
				self.timeline.leave_read = Some(pending);
			} else if let Some(command) = state.prepare_mark_left_channel_read(pending.0, pending.1)
			{
				commands.push(command);
			}
		}
		if let Some(channel) = self.timeline.mark_channel_read.take() {
			if ctx.will_discard() {
				self.timeline.mark_channel_read = Some(channel);
			} else {
				self.timeline.mark_read = None;
				if let Some(command) = state.prepare_mark_channel_read(channel) {
					commands.push(command);
				}
			}
		}
		if let Some(message) = self.timeline.mark_unread.take() {
			self.timeline.mark_read = None;
			if !settings_open && let Some(command) = state.prepare_mark_unread(message) {
				self.timeline.browse_away();
				commands.push(command);
			}
		} else if let Some(message) = self.timeline.mark_read.take() {
			if !settings_open
				&& state.search_target.is_none()
				&& let Some(command) = state.prepare_mark_read(message)
			{
				commands.push(command);
			} else if self.timeline.auto_read_attempt == Some(message) {
				// A suppressed action never reached the worker; allow it after the overlay closes.
				self.timeline.auto_read_attempt = None;
			}
		}

		if let Some(message) = self.timeline.forward_request.take() {
			self.forwarding.open(state, message);
		}
		self.forwarding
			.show(&ctx, state, &mut self.avatars, &mut commands);

		if let Some((message, custom_id, values)) = self.timeline.component_action.take()
			&& let Some(command) = state.prepare_component(message, &custom_id, values)
		{
			commands.push(command);
		}
		if let Some(channel) = state.selected {
			if state.interactions.modal.is_none()
				&& self.timeline.components.remote_search
				&& !self.timeline.components.search.text.is_empty()
			{
				self.timeline
					.components
					.search
					.sync(&ctx, state, channel, 0, &mut commands);
			}
			if self.interaction_components.remote_search
				&& !self.interaction_components.search.text.is_empty()
			{
				self.interaction_components
					.search
					.sync(&ctx, state, channel, 0, &mut commands);
			}
		}
		self.interaction_components.dialogs(
			&ctx,
			state,
			&mut commands,
			&mut self.avatars,
			&mut self.timeline.opening,
		);
		self.interaction_file_request = self.interaction_components.file_request.take();
		if let Some(id) = self.timeline.dismiss_ephemeral.take() {
			state.dismiss_ephemeral(id);
			self.timeline.components.forget_message(id);
		}
		if let Some((message, emoji)) = self.timeline.reaction.take() {
			if let Some(emoji) = emoji {
				if let Some(command) = state.prepare_reaction(message, emoji) {
					commands.push(command);
				}
			} else {
				state.refresh_reactions(message);
			}
		}
		self.group_menu
			.show(&ctx, state, &mut self.avatars, &mut commands);
		for intent in self
			.channel_menu
			.shortcut_requested
			.take()
			.into_iter()
			.chain(self.group_menu.pin_requested.take())
		{
			self.apply_shortcut(state, intent);
		}
		if let Some(action) = self.user_action.take().or(self.timeline.user_action.take()) {
			let command = match action {
				user_menu::Action::Note(user) => {
					self.profile.hide();
					self.contact_editor.open(user, false, state)
				}
				user_menu::Action::Nickname(user) => {
					self.profile.hide();
					self.contact_editor.open(user, true, state)
				}
				user_menu::Action::Shortcut(intent) => {
					self.apply_shortcut(state, intent);
					None
				}
				user_menu::Action::Mention(user) => {
					if user.id != Id(0) {
						self.pending_mention = Some(user.id);
						ctx.request_repaint();
					}
					None
				}
				action => user_menu::prepare(action, state),
			};
			if let Some(command) = command {
				commands.push(command);
			}
		}
		self.contact_editor.show(&ctx, state, &mut commands);
		self.drain_profile(state, &mut commands);
		if let Some(user) = self.profile.open_user().cloned() {
			let profile_guild = self.server_settings.guild().or_else(|| {
				state
					.channels
					.iter()
					.find(|channel| Some(channel.id) == state.selected)
					.and_then(|channel| channel.guild)
			});
			self.sync_profile(state, &mut commands, &user, profile_guild);
			let anchor = self.profile.anchor_or_place(&ctx, user.id);
			self.profile.ingest_opener_rect(&ctx);
			match profiles::show(
				ui,
				&user,
				state.profile.as_ref(),
				state,
				&mut self.avatars,
				&mut self.profile_link,
				&mut self.profile_formatted,
				self.reading_preferences.confirm_external_links,
				anchor,
			) {
				Some(profiles::Action::Avatar(media)) => {
					let ext = media
						.url
						.as_deref()
						.and_then(|u| u.split('?').next()?.rsplit_once('.').map(|(_, ext)| ext))
						.unwrap_or("png");
					self.profile_image = Some((
						state.generation,
						model::Attachment {
							id: Id(0),
							filename: format!("avatar.{ext}"),
							description: Some(format!("{}’s profile picture", user.name)),
							content_type: Some(format!("image/{ext}")),
							size: 0,
							media,
							spoiler: false,
							duration_ms: None,
							waveform: Vec::new(),
						},
					));
					self.profile.close();
				}
				Some(profiles::Action::Banner(media)) => {
					let ext = media
						.url
						.as_deref()
						.and_then(|u| u.split('?').next()?.rsplit_once('.').map(|(_, ext)| ext))
						.unwrap_or("png");
					self.profile_image = Some((
						state.generation,
						model::Attachment {
							id: Id(0),
							filename: format!("banner.{ext}"),
							description: Some(format!("{}’s banner", user.name)),
							content_type: Some(format!("image/{ext}")),
							size: 0,
							media,
							spoiler: false,
							duration_ms: None,
							waveform: Vec::new(),
						},
					));
					self.profile.close();
				}
				Some(profiles::Action::AddFriend(id)) => {
					if let Some(command) = state.add_profile_friend(id) {
						commands.push(command);
					}
				}
				Some(profiles::Action::AcceptFriend(id)) => {
					if let Some(command) = state.resolve_friend_request(id, true) {
						commands.push(command);
					}
				}
				Some(profiles::Action::RemoveFriend) => {
					self.friend_removal = Some((state.generation, user.clone()));
				}
				Some(profiles::Action::Menu(action)) => {
					// Dispatched with the other user menus on the next frame.
					self.user_action = Some(action);
				}
				Some(profiles::Action::Edit) => {
					self.profile.close();
					self.preview_settings("profile");
				}
				Some(profiles::Action::Profile(next)) => {
					self.profile.navigate(next);
				}
				Some(profiles::Action::Close) => {
					self.profile.close_unless_armed(&ctx);
				}
				Some(profiles::Action::Retry) => {
					if let Some(command) = state.request_profile(user.id, profile_guild) {
						commands.push(command);
					}
				}
				Some(profiles::Action::Message(channel)) => {
					self.profile.close();
					if self.server_settings.navigate_away(state)
						&& let Some(command) = state.select(channel)
					{
						commands.push(command);
					}
				}
				None => {}
			}
			if self.drain_profile(state, &mut commands)
				&& let Some(user) = self.profile.open_user().cloned()
			{
				self.sync_profile(state, &mut commands, &user, profile_guild);
			}
		} else {
			self.profile_link = None;
		}

		if let Some((generation, image)) = &self.profile_image
			&& (*generation != state.generation
				|| attachments::viewer(
					ui,
					std::slice::from_ref(image),
					image.id,
					&mut self.avatars,
					&mut self.timeline.download,
					&mut self.timeline.opening,
					state.demo,
				)
				.is_none())
		{
			self.profile_image = None;
		}

		self.reconcile_edit(state);
		if self.deleting.is_some_and(|(channel, message)| {
			state.selected != Some(channel) || state.timeline.get(message).is_none()
		}) {
			self.deleting = None;
		}
		if let Some((channel, message)) = self.deleting {
			let allowed = state.can_delete(channel, message);
			let conversation = state
				.channel(channel)
				.map_or("this conversation", |c| c.name.as_str());
			let mut confirm = dialog::Confirm::new(
				"delete-message",
				"Delete message?",
				format!(
					"This permanently removes the selected message from {conversation} for everyone."
				),
			)
			.danger()
			.confirm_label("Delete")
			.cancel_label("Keep Message")
			.enabled(allowed);
			if !allowed {
				confirm = confirm.note(
					dialog::Level::Warning,
					"Deleting this message is unavailable.",
				);
			}
			match confirm.show(&ctx) {
				Some(dialog::Choice::Confirmed) => {
					if let Some(command) = state.prepare_delete(channel, message) {
						commands.push(command);
					}
					self.deleting = None;
				}
				Some(dialog::Choice::Cancelled) => self.deleting = None,
				None => {}
			}
		}
		if let Some((channel, ids)) = self.deleting_batch.clone() {
			let own = state.user.as_ref().map(|user| user.id);
			let allowed = !ids.is_empty()
				&& ids.len() <= 5
				&& ids.iter().all(|id| {
					state.timeline.get(*id).is_some_and(|message| {
						message.channel == channel
							&& Some(message.author.id) == own
							&& state.can_delete(channel, *id)
					})
				});
			let conversation = state
				.channel(channel)
				.map_or("this conversation", |c| c.name.as_str());
			let mut confirm = dialog::Confirm::new(
				"delete-message-batch",
				format!("Delete {} messages?", ids.len()),
				format!(
					"This permanently removes the selected messages from {conversation} for everyone. Only messages authored by your account are included."
				),
			)
			.danger()
			.confirm_label("Delete selected")
			.cancel_label("Keep Messages")
			.enabled(allowed);
			if !allowed {
				confirm = confirm.note(
					dialog::Level::Warning,
					"One or more selected messages can no longer be deleted. Review the selection.",
				);
			}
			match confirm.show(&ctx) {
				Some(dialog::Choice::Confirmed) if allowed => {
					let now = ctx.input(|input| input.time);
					self.batch_delete_queue = ids.into_iter().map(|id| (channel, id)).collect();
					self.batch_delete_total = self.batch_delete_queue.len();
					self.batch_delete_sent = 0;
					self.batch_delete_next = Some(now + batch_delete_gap_secs());
					self.timeline.batch_delete.clear();
					self.timeline.batch_progress = Some((0, self.batch_delete_total));
					self.deleting_batch = None;
				}
				Some(dialog::Choice::Cancelled) => self.deleting_batch = None,
				_ => {}
			}
		}
		self.show_call_switch(&ctx, state, &mut commands);
		self.verification.show(&ctx, state);
		self.scroll.clear_if_unbound(&ctx);
		self.scroll.paint(&ctx);
		// Clear the title bar and channel header so a notice never sits on the chrome.
		self.toasts
			.show(&ctx, if self.shows_title_bar() { 96.0 } else { 60.0 });
		if !commands.is_empty() {
			ctx.request_repaint();
		}
		commands
	}
}

/// One second, plus 100–600 ms, so confirmed deletes are not sent together.
fn batch_delete_gap_secs() -> f64 {
	let nanos = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|duration| duration.subsec_nanos())
		.unwrap_or(0);
	(1000 + 100 + (nanos % 501)) as f64 / 1000.0
}

#[cfg(test)]
mod composer_tests {
	use super::*;

	fn dm_state() -> State {
		State {
			channels: vec![model::Channel {
				id: Id(1),
				guild: None,
				parent_id: None,
				kind: 1,
				name: "Synthetic DM".into(),
				position: 0,
				recipients: vec![],
				last_message: None,
				icon: None,
				member_list_id: None,
				message_count: None,
			}],
			selected: Some(Id(1)),
			auth: client_core::auth::AuthState::Authenticated,
			freshness: model::Freshness::Fresh,
			gateway_connected: true,
			..State::default()
		}
	}

	fn dialog_labels(output: &egui::FullOutput) -> Vec<(String, egui::Rect)> {
		fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => out.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
				_ => {}
			}
		}
		let mut labels = Vec::new();
		for shape in &output.shapes {
			walk(&shape.shape, &mut labels);
		}
		labels
	}

	fn click_label(
		ctx: &egui::Context,
		view: &mut MessagingUi,
		state: &mut State,
		commands: &mut Vec<Command>,
		labels: &[(String, egui::Rect)],
		label: &str,
	) {
		let pos = labels
			.iter()
			.find(|(text, _)| text == label)
			.unwrap_or_else(|| panic!("Missing {label}: {labels:?}"))
			.1
			.center();
		for pressed in [true, false] {
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1120.0, 760.0),
					)),
					events: vec![
						egui::Event::PointerMoved(pos),
						egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
					..Default::default()
				},
				|_| {
					view.show_long_text_dialog(ctx, state, Id(1), commands);
				},
			);
			output.textures_delta.clear();
		}
	}

	fn render_dialog(
		ctx: &egui::Context,
		view: &mut MessagingUi,
		state: &mut State,
		commands: &mut Vec<Command>,
	) -> Vec<(String, egui::Rect)> {
		let mut output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1120.0, 760.0),
				)),
				..Default::default()
			},
			|_| {
				view.show_long_text_dialog(ctx, state, Id(1), commands);
			},
		);
		output.textures_delta.clear();
		dialog_labels(&output)
	}

	#[test]
	fn long_text_filenames_normalize_and_validate_like_the_send_path() {
		assert_eq!(normalize_txt_filename("notes"), Some("notes.txt".into()));
		assert_eq!(
			normalize_txt_filename("notes.txt"),
			Some("notes.txt".into())
		);
		assert_eq!(
			normalize_txt_filename("  padded  "),
			Some("padded.txt".into())
		);
		assert_eq!(normalize_txt_filename(""), None);
		assert_eq!(normalize_txt_filename("   "), None);
		let max_stem = "a".repeat(252);
		for valid in [
			"message.txt".to_string(),
			"notes.txt".to_string(),
			"résumé.txt".to_string(),
			format!("{max_stem}.txt"),
		] {
			assert!(valid_txt_filename(&valid), "{valid}");
		}
		let overlong = format!("{}.txt", "a".repeat(253));
		for invalid in [
			"",
			"   ",
			".",
			"..",
			".txt",
			"notes",
			"a/b.txt",
			"a\\b.txt",
			"a:b.txt",
			"a\nb.txt",
			overlong.as_str(),
		] {
			assert!(!valid_txt_filename(invalid), "{invalid}");
		}
	}

	#[test]
	fn over_limit_text_dispatches_as_attachment_with_exact_bytes() {
		let ctx = egui::Context::default();
		let mut state = dm_state();
		let mut view = MessagingUi::default();
		let text = "x".repeat(2500);
		view.long_text_dialog = Some(LongTextDialog {
			channel: Id(1),
			text: text.clone(),
			filename: "notes".into(),
		});
		let mut commands = vec![];
		view.send_long_text(&ctx, &mut state, Id(1), &mut commands);
		assert_eq!(commands.len(), 1);
		let staged = view.long_text_attachment.as_ref().expect("staged file");
		assert_eq!(staged.filename, "notes.txt");
		assert_eq!(staged.bytes, text.into_bytes());
		assert_eq!(state.pending[0].attachments, ["notes.txt"]);
		assert!(!state.drafts.contains_key(&Id(1)));
	}

	#[test]
	fn dialog_send_dispatches_the_staged_file() {
		let ctx = egui::Context::default();
		let mut state = dm_state();
		let mut view = MessagingUi::default();
		let text = "y".repeat(2100);
		view.long_text_dialog = Some(LongTextDialog {
			channel: Id(1),
			text: text.clone(),
			filename: "notes".into(),
		});
		let mut commands = vec![];
		let labels = render_dialog(&ctx, &mut view, &mut state, &mut commands);
		click_label(&ctx, &mut view, &mut state, &mut commands, &labels, "Send");
		assert_eq!(commands.len(), 1);
		assert!(view.long_text_dialog.is_none());
		let staged = view.long_text_attachment.as_ref().expect("staged file");
		assert_eq!(staged.filename, "notes.txt");
		assert_eq!(staged.bytes, text.into_bytes());
	}

	#[test]
	fn over_limit_text_offers_file_only_where_attachments_fit() {
		let ctx = egui::Context::default();
		// Control: DM allows attachments, so the rename dialog opens.
		let mut state = dm_state();
		state.drafts.insert(Id(1), "x".repeat(2500));
		let mut view = MessagingUi::default();
		ctx.run_ui(Default::default(), |ui| {
			view.composer(ui, &mut state, Id(1), &ctx, &mut vec![]);
			ctx.memory_mut(|m| m.request_focus(ui.make_persistent_id("message-input")));
		})
		.drop_without_applying_deltas();
		edit_frame(&ctx, &mut view, &mut state, vec![edit_key(egui::Key::Enter)]);
		// The rename dialog is built on the repaint that follows the Send gesture.
		edit_frame(&ctx, &mut view, &mut state, vec![]);
		assert!(view.long_text_dialog.is_some(), "DM offers message.txt");
		// Gated: guild channel without attach permission never opens it.
		let mut denied = dm_state();
		denied.channels.push(model::Channel {
			id: Id(20),
			guild: Some(Id(9)),
			parent_id: None,
			kind: 0,
			name: "no-attach".into(),
			position: 1,
			recipients: vec![],
			last_message: None,
			icon: None,
			member_list_id: None,
			message_count: None,
		});
		denied.selected = Some(Id(20));
		denied.drafts.insert(Id(20), "x".repeat(2500));
		let mut denied_view = MessagingUi::default();
		ctx.run_ui(Default::default(), |ui| {
			denied_view.composer(ui, &mut denied, Id(20), &ctx, &mut vec![]);
			ctx.memory_mut(|m| m.request_focus(ui.make_persistent_id("message-input")));
		})
		.drop_without_applying_deltas();
		edit_frame(&ctx, &mut denied_view, &mut denied, vec![edit_key(egui::Key::Enter)]);
		edit_frame(&ctx, &mut denied_view, &mut denied, vec![]);
		assert!(
			denied_view.long_text_dialog.is_none(),
			"no attach permission, no dialog"
		);
	}

	#[test]
	fn cancel_keeps_the_draft() {
		let ctx = egui::Context::default();
		let mut state = dm_state();
		let mut view = MessagingUi::default();
		state.drafts.insert(Id(1), "Hello".into());
		view.long_text_dialog = Some(LongTextDialog {
			channel: Id(1),
			text: "Hello".into(),
			filename: "message.txt".into(),
		});
		let mut commands = vec![];
		let labels = render_dialog(&ctx, &mut view, &mut state, &mut commands);
		assert!(labels.iter().any(|(text, _)| text == "Send as text file?"));
		assert!(labels.iter().any(|(text, _)| text == "FILE NAME"));
		assert!(commands.is_empty());
		click_label(
			&ctx,
			&mut view,
			&mut state,
			&mut commands,
			&labels,
			"Cancel",
		);
		assert!(view.long_text_dialog.is_none());
		assert_eq!(state.drafts.get(&Id(1)).map(String::as_str), Some("Hello"));
		assert!(commands.is_empty());
		assert!(view.long_text_attachment.is_none());
	}

	#[test]
	fn invalid_filename_is_refused() {
		let ctx = egui::Context::default();
		let mut state = dm_state();
		let mut view = MessagingUi::default();
		view.long_text_dialog = Some(LongTextDialog {
			channel: Id(1),
			text: "x".repeat(2500),
			filename: "a/b.txt".into(),
		});
		let mut commands = vec![];
		let labels = render_dialog(&ctx, &mut view, &mut state, &mut commands);
		assert!(
			labels
				.iter()
				.any(|(text, _)| text == "That file name will not work.")
		);
		click_label(&ctx, &mut view, &mut state, &mut commands, &labels, "Send");
		assert!(commands.is_empty());
		assert!(view.long_text_dialog.is_some());
		assert!(view.long_text_attachment.is_none());
	}

	#[test]
	fn image_surface_reaches_the_channel_header_without_a_stripe() {
		let ctx = egui::Context::default();
		ctx.set_theme(egui::ThemePreference::Dark);
		let mut theme = extensions::Theme::default();
		theme.dark.background = Some(extensions::Background {
			sections: Some(extensions::SectionOpacity::default()),
			..Default::default()
		});
		design::set_extension_theme(Some(&theme));
		design::set_background_image(
			&ctx,
			Some(std::sync::Arc::new(egui::ColorImage::filled(
				[1, 1],
				egui::Color32::WHITE,
			))),
		);
		let mut state = test_support::demo_state();
		let mut view = MessagingUi::default();
		let mut output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1100.0, 800.0),
				)),
				..Default::default()
			},
			|ui| {
				view.show(ui, &mut state);
			},
		);
		let message_alpha = (75 * 255 / 100) as u8;
		let expected_top = if view.shows_title_bar() { 84.0 } else { 48.0 };
		let surface_reaches_header = output.shapes.iter().any(|shape| match &shape.shape {
			egui::Shape::Rect(rect)
				if rect.fill.a() == message_alpha && rect.rect.height() > 200.0 =>
			{
				(rect.rect.top() - expected_top).abs() <= 1.0
			}
			_ => false,
		});
		output.textures_delta.clear();
		design::set_extension_theme(None);
		assert!(
			surface_reaches_header,
			"message surface did not reach expected top {expected_top}",
		);
	}

	#[test]
	fn composer_placeholder_alignment() {
		for scale in [1.0, 1.25, 1.5, 2.0] {
			for theme in [egui::Theme::Dark, egui::Theme::Light] {
				let ctx = egui::Context::default();
				fonts::install(&ctx);
				design::apply(&ctx);
				ctx.set_theme(theme);
				ctx.set_pixels_per_point(scale);
				let mut state = edit_state();
				state.channels[0].name = "Alex".into();
				let mut view = MessagingUi::default();
				for width in [320.0, 900.0] {
					for draft in ["", "Message @Alex", "First line\nSecond line"] {
						state.drafts.insert(Id(10), draft.into());
						for _ in 0..2 {
							let mut empty_height = 0.0;
							let output = ctx.run_ui(
								egui::RawInput {
									screen_rect: Some(egui::Rect::from_min_size(
										egui::Pos2::ZERO,
										egui::vec2(width, 300.0),
									)),
									..Default::default()
								},
								|ui| {
									view.composer(ui, &mut state, Id(10), &ctx, &mut vec![]);
									empty_height = view
										.composer_layout
										.galley(
											ui,
											"",
											300.0,
											&[],
											&[],
											&[],
											false,
											&mut view.avatars,
											true,
										)
										.rect
										.height();
								},
							);
							let frame = output
								.shapes
								.iter()
								.find_map(|s| match &s.shape {
									egui::Shape::Rect(r) => Some(r.rect),
									_ => None,
								})
								.unwrap();
							let text = output
								.shapes
								.iter()
								.find_map(|s| match &s.shape {
									egui::Shape::Text(t)
										if t.galley.job.text == draft
											|| t.galley.job.text == "Message @Alex" =>
									{
										Some(t.galley.rect.translate(t.pos.to_vec2()))
									}
									_ => None,
								})
								.unwrap();
							output.drop_without_applying_deltas();
							assert!(
								(text.center().y - frame.center().y).abs() <= 1.0 / scale,
								"{draft:?}, scale {scale}, width {width}: text {text:?}, frame {frame:?}"
							);
							assert!(
								empty_height >= 15.0,
								"empty editor must retain a full-height caret"
							);
						}
					}
				}
			}
		}
	}

	#[test]
	fn download_cancel_remains_visible_without_a_text_composer() {
		for selection in [None, Some(Id(10))] {
			let mut state = edit_state();
			state.demo = true;
			state.selected = selection;
			state.channels[0].kind = 2;
			let mut messaging = MessagingUi::default();
			messaging.downloads().active = true;
			messaging.downloads().status = "Saving synthetic attachment".into();
			let ctx = egui::Context::default();
			let output = ctx.run_ui(egui::RawInput::default(), |ui| {
				messaging.show(ui, &mut state);
			});
			assert!(output.shapes.iter().any(|shape| {
				matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Cancel download")
			}));
			output.drop_without_applying_deltas();
		}
	}

	fn edit_state() -> State {
		let user = model::User {
			id: Id(1),
			name: "Alex".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
		};
		let mut state = State {
			selected: Some(Id(10)),
			channels: vec![model::Channel {
				id: Id(10),
				guild: None,
				parent_id: None,
				kind: 1,
				name: "Synthetic edit conversation".into(),
				position: 0,
				recipients: vec![],
				last_message: None,
				member_list_id: None,
				message_count: None,
				icon: None,
			}],
			user: Some(user.clone()),
			demo: true,
			freshness: Freshness::Fresh,
			auth: client_core::auth::AuthState::Authenticated,
			gateway_connected: true,
			..Default::default()
		};
		state
			.timeline
			.insert(
				model::Message {
					sticker_items: vec![],
					id: Id(20),
					channel: Id(10),
					author: user,
					content: "Original".into(),
					edited: false,
					edited_at: None,
					revision: 0,
					nonce: None,
					reply_to: None,
					kind: 0,
					reply_deleted: false,
					interaction: None,
					forwarded: false,
					unsupported: false,
					extra_content: Default::default(),
					components: vec![],
					application_id: None,
					ephemeral: false,
					flags: 0,
					embeds: vec![],
					attachments: vec![],
					author_nick: None,
					author_roles: vec![],
					mention_roles: vec![],
					mention_everyone: false,
					suppress_notifications: false,
					mentions: vec![],
					reactions: None,
					embeds_suppressed: false,
				},
				false,
				false,
			)
			.unwrap();
		state.drafts.insert(Id(10), "Unsent draft 👋".into());
		state
	}

	fn edit_frame(
		ctx: &egui::Context,
		view: &mut MessagingUi,
		state: &mut State,
		events: Vec<egui::Event>,
	) -> Vec<Command> {
		let mut commands = vec![];
		let output = ctx.run_ui(
			egui::RawInput {
				events,
				..Default::default()
			},
			|ui| {
				view.composer(ui, state, state.selected.unwrap(), ctx, &mut commands);
			},
		);
		output.drop_without_applying_deltas();
		// This helper simulates key taps; raw-input tests explicitly model held keys.
		ctx.input_mut(|i| i.keys_down.clear());
		commands
	}

	fn edit_key(key: egui::Key) -> egui::Event {
		egui::Event::Key {
			key,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::NONE,
		}
	}

	#[test]
	fn composer_enter_ignores_idle_linux_ime_updates() {
		for editing in [false, true] {
			for shift in [false, true] {
				for ime in [
					egui::ImeEvent::Preedit {
						text: String::new(),
						active_range_chars: None,
					},
					egui::ImeEvent::Preedit {
						text: String::new(),
						active_range_chars: Some(0..0),
					},
					egui::ImeEvent::Commit(String::new()),
				] {
					let ctx = egui::Context::default();
					ctx.set_os(egui::os::OperatingSystem::Nix);
					let mut state = edit_state();
					state.drafts.insert(Id(10), "Message".into());
					let mut view = MessagingUi::default();
					if editing {
						view.editing = Some((Id(10), Id(20), "Changed".into()));
					}
					ctx.run_ui(Default::default(), |ui| {
						view.composer(ui, &mut state, Id(10), &ctx, &mut vec![]);
						ctx.memory_mut(|m| {
							m.request_focus(ui.make_persistent_id(if editing {
								"message-edit"
							} else {
								"message-input"
							}))
						});
					})
					.drop_without_applying_deltas();
					let mut enter = edit_key(egui::Key::Enter);
					if let egui::Event::Key { modifiers, .. } = &mut enter {
						modifiers.shift = shift;
					}
					let commands = edit_frame(
						&ctx,
						&mut view,
						&mut state,
						vec![egui::Event::Ime(ime), enter],
					);
					if shift {
						assert!(commands.is_empty());
						let text = if editing {
							&view.editing.as_ref().unwrap().2
						} else {
							&state.drafts[&Id(10)]
						};
						assert!(text.contains('\n'));
					} else if editing {
						assert!(
							matches!(commands.as_slice(), [Command::Edit { content, .. }] if content == "Changed")
						);
					} else {
						assert!(
							matches!(commands.as_slice(), [Command::Send { content, .. }] if content == "Message")
						);
					}
				}
			}
		}
	}

	#[test]
	fn composer_enter_dismissing_composition_does_not_send() {
		for finish in [
			egui::ImeEvent::Preedit {
				text: String::new(),
				active_range_chars: None,
			},
			egui::ImeEvent::Commit(String::new()),
			egui::ImeEvent::Commit("composed".into()),
		] {
			let ctx = egui::Context::default();
			ctx.set_os(egui::os::OperatingSystem::Nix);
			let mut state = edit_state();
			let mut view = MessagingUi::default();
			ctx.run_ui(Default::default(), |ui| {
				view.composer(ui, &mut state, Id(10), &ctx, &mut vec![]);
				ctx.memory_mut(|m| m.request_focus(ui.make_persistent_id("message-input")));
			})
			.drop_without_applying_deltas();
			assert!(
				edit_frame(
					&ctx,
					&mut view,
					&mut state,
					vec![egui::Event::Ime(egui::ImeEvent::Preedit {
						text: "composition".into(),
						active_range_chars: None,
					})]
				)
				.is_empty()
			);
			assert!(view.ime_active);
			assert!(
				edit_frame(
					&ctx,
					&mut view,
					&mut state,
					vec![egui::Event::Ime(finish), edit_key(egui::Key::Enter)]
				)
				.is_empty()
			);
			assert!(!view.ime_active);
			assert!(matches!(
				edit_frame(
					&ctx,
					&mut view,
					&mut state,
					vec![
						egui::Event::Ime(egui::ImeEvent::Preedit {
							text: String::new(),
							active_range_chars: None,
						}),
						edit_key(egui::Key::Enter)
					]
				)
				.as_slice(),
				[Command::Send { .. }]
			));
		}
	}

	#[test]
	fn joined_invite_cards_open_servers_without_joining_and_preserve_drafts() {
		fn button_rect(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
			match shape {
				egui::Shape::Text(text) if text.galley.job.text == label => {
					Some(text.galley.rect.translate(text.pos.to_vec2()))
				}
				egui::Shape::Vec(shapes) => {
					shapes.iter().find_map(|shape| button_rect(shape, label))
				}
				_ => None,
			}
		}
		let ctx = egui::Context::default();
		design::apply(&ctx);
		let mut state = edit_state();
		state.demo = false;
		state.guilds.push(model::Guild {
			stickers: None,
			id: Id(100),
			name: "Synthetic invited server".into(),
			icon: None,
			emojis: None,
		premium_tier: 0
		});
		let mut channel = state.channels[0].clone();
		channel.id = Id(12);
		channel.guild = Some(Id(100));
		channel.kind = 0;
		state.channels.push(channel);
		state.last_viewed_channels.push((Id(100), Id(12)));
		state
			.permissions
			.replace(test_support::permission_snapshot(&state))
			.unwrap();
		let mut message = state.timeline.get(Id(20)).unwrap().clone();
		message.content = "https://discord.gg/synthetic".into();
		state.timeline.insert(message, true, false).unwrap();
		state.invites.insert(
			"synthetic".into(),
			(
				std::time::Instant::now(),
				Some(Ok(model::InvitePreview {
					guild: Id(100),
					embed: model::Embed {
						title: Some("Synthetic invited server".into()),
						..Default::default()
					},
				})),
			),
		);
		let mut view = MessagingUi::default();
		view.reading_preferences.show_members = false;
		let drafts = state.drafts.clone();
		let frame = |view: &mut MessagingUi, state: &mut State, events| {
			let mut commands = vec![];
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(760.0, 700.0),
					)),
					events,
					..Default::default()
				},
				|ui| commands = view.show(ui, state),
			);
			let rect = output
				.shapes
				.iter()
				.find_map(|shape| button_rect(&shape.shape, "Go To Server"));
			output.drop_without_applying_deltas();
			(rect, commands)
		};
		for _ in 0..3 {
			frame(&mut view, &mut state, vec![]);
		}
		let point = frame(&mut view, &mut state, vec![])
			.0
			.expect("Go To Server")
			.center();
		let mut commands = vec![];
		for pressed in [true, false] {
			commands.extend(
				frame(
					&mut view,
					&mut state,
					vec![
						egui::Event::PointerMoved(point),
						egui::Event::PointerButton {
							pos: point,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				)
				.1,
			);
		}
		commands.extend(frame(&mut view, &mut state, vec![]).1);
		assert_eq!((view.guild, state.selected), (Some(Id(100)), Some(Id(12))));
		assert_eq!(state.drafts, drafts);
		assert!(
			!commands
				.iter()
				.any(|command| matches!(command, Command::JoinInvite { .. }))
		);
		assert_eq!(
			commands
				.iter()
				.filter(|command| matches!(
					command,
					Command::History {
						channel: Id(12),
						..
					}
				))
				.count(),
			1
		);
	}

	#[test]
	fn discord_chat_links_navigate_from_message_text_without_opening_a_browser() {
		fn link_rect(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
			match shape {
				egui::Shape::Text(text) if text.galley.job.text.trim() == label => {
					Some(text.galley.rect.translate(text.pos.to_vec2()))
				}
				egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| link_rect(shape, label)),
				_ => None,
			}
		}
		for (source, label, target_message) in [
			(
				"https://discord.com/channels/100/11",
				"https://discord.com/channels/100/11",
				None,
			),
			(
				"[Other chat](https://discord.com/channels/100/11/25)",
				"Other chat",
				Some(Id(25)),
			),
		] {
			let ctx = egui::Context::default();
			let mut view = MessagingUi::default();
			let mut state = edit_state();
			let mut target = state.channels[0].clone();
			target.id = Id(11);
			target.guild = Some(Id(100));
			target.kind = 0;
			state.channels.push(target);
			state.guilds.push(model::Guild {
				stickers: None,
				id: Id(100),
				name: "Linked server".into(),
				icon: None,
				emojis: None,
			premium_tier: 0
			});
			state
				.permissions
				.replace(test_support::permission_snapshot(&state))
				.unwrap();
			let mut message = state.timeline.get(Id(20)).unwrap().clone();
			message.content = source.into();
			state.timeline.insert(message, true, false).unwrap();
			let drafts = state.drafts.clone();
			let frame = |view: &mut MessagingUi, state: &mut State, events| {
				let mut commands = vec![];
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(1000.0, 700.0),
						)),
						events,
						..Default::default()
					},
					|ui| commands = view.show(ui, state),
				);
				assert!(
					!output
						.platform_output
						.commands
						.iter()
						.any(|command| matches!(command, egui::OutputCommand::OpenUrl(_)))
				);
				let rect = output
					.shapes
					.iter()
					.find_map(|shape| link_rect(&shape.shape, label));
				output.drop_without_applying_deltas();
				(rect, commands)
			};
			for _ in 0..3 {
				frame(&mut view, &mut state, vec![]);
			}
			let point = frame(&mut view, &mut state, vec![])
				.0
				.expect("rendered chat link")
				.center();
			let mut commands = vec![];
			for pressed in [true, false] {
				commands.extend(
					frame(
						&mut view,
						&mut state,
						vec![
							egui::Event::PointerMoved(point),
							egui::Event::PointerButton {
								pos: point,
								button: egui::PointerButton::Primary,
								pressed,
								modifiers: egui::Modifiers::NONE,
							},
						],
					)
					.1,
				);
			}
			commands.extend(frame(&mut view, &mut state, vec![]).1);
			assert_eq!(state.selected, Some(Id(11)));
			assert_eq!(view.guild, Some(Id(100)));
			assert_eq!(state.search_target, target_message);
			assert_eq!(state.drafts, drafts);
			assert_eq!(commands.iter().filter(|command| matches!(command, Command::History { channel: Id(11), before, .. } if *before == target_message.map(|id| Id(id.0 + 1)))).count(), 1);
			assert!(!commands.iter().any(|command| matches!(
				command,
				Command::Send { .. } | Command::Edit { .. } | Command::Delete { .. }
			)));
		}
	}

	#[test]
	fn link_confirmation_escape_preserves_background_search_and_works_without_selection() {
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let mut view = MessagingUi::default();
		view.reading_preferences.confirm_external_links = true;
		let frame = |view: &mut MessagingUi, state: &mut State, events| {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1000.0, 700.0),
					)),
					events,
					..Default::default()
				},
				|ui| {
					view.show(ui, state);
				},
			);
			let emitted = output.platform_output.commands.len();
			output.drop_without_applying_deltas();
			assert_eq!(
				emitted, 0,
				"Showing or canceling a link never opens a browser"
			);
		};
		frame(&mut view, &mut state, vec![]);
		view.search.open = true;
		view.timeline.opening = Some("https://example.com/message".into());
		for _ in 0..3 {
			frame(&mut view, &mut state, vec![]);
		}
		frame(&mut view, &mut state, vec![edit_key(egui::Key::Escape)]);
		assert!(view.timeline.opening.is_none());
		assert!(
			view.search.open,
			"Escape belongs to the foreground confirmation"
		);
		state.selected = None;
		view.timeline.opening = Some("https://example.com/conversation".into());
		for _ in 0..3 {
			frame(&mut view, &mut state, vec![]);
		}
		assert!(view.timeline.opening.is_some());
		frame(&mut view, &mut state, vec![edit_key(egui::Key::Escape)]);
		assert!(view.timeline.opening.is_none());
	}

	#[test]
	fn conversation_shortcut_preserves_edits_and_drafts_and_never_sends() {
		fn frame(
			ctx: &egui::Context,
			view: &mut MessagingUi,
			state: &mut State,
			events: Vec<egui::Event>,
		) -> Vec<Command> {
			let mut commands = Vec::new();
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1200.0, 800.0),
					)),
					focused: true,
					events,
					..Default::default()
				},
				|ui| commands.extend(view.show(ui, state)),
			);
			output.drop_without_applying_deltas();
			assert!(!commands.iter().any(|command| matches!(
				command,
				Command::Send { .. } | Command::Edit { .. } | Command::Voice(_)
			)));
			commands
		}
		let shortcut = || egui::Event::Key {
			key: egui::Key::K,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::COMMAND,
		};
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let mut other = state.channels[0].clone();
		other.id = Id(11);
		other.name = "Other conversation".into();
		state.channels.push(other);
		state.drafts.insert(Id(11), "Another unsent draft".into());
		let drafts = state.drafts.clone();
		let mut view = MessagingUi {
			editing: Some((Id(10), Id(20), "Keep this unfinished edit".into())),
			..Default::default()
		};
		frame(&ctx, &mut view, &mut state, vec![]);
		view.ime_active = true;
		frame(&ctx, &mut view, &mut state, vec![shortcut()]);
		assert!(
			!view.switcher.is_open(),
			"Do not interrupt an active composition"
		);
		view.ime_active = false;
		frame(&ctx, &mut view, &mut state, vec![shortcut()]);
		assert!(view.switcher.is_open());
		frame(&ctx, &mut view, &mut state, vec![]);
		frame(
			&ctx,
			&mut view,
			&mut state,
			vec![edit_key(egui::Key::Escape)],
		);
		assert!(!view.switcher.is_open());
		assert_eq!(state.selected, Some(Id(10)));
		assert_eq!(
			view.editing.as_ref().unwrap().2,
			"Keep this unfinished edit"
		);
		frame(&ctx, &mut view, &mut state, vec![shortcut()]);
		frame(&ctx, &mut view, &mut state, vec![]);
		frame(
			&ctx,
			&mut view,
			&mut state,
			vec![edit_key(egui::Key::ArrowDown)],
		);
		let commands = frame(
			&ctx,
			&mut view,
			&mut state,
			vec![edit_key(egui::Key::Enter)],
		);
		assert_eq!(state.selected, Some(Id(11)));
		assert!(commands.iter().any(|command| matches!(
			command,
			Command::History {
				channel: Id(11),
				..
			}
		)));
		assert!(!view.switcher.is_open());
		assert_eq!(state.drafts, drafts);
		assert_eq!(
			view.editing.as_ref().unwrap().2,
			"Keep this unfinished edit"
		);
		for _ in 0..3 {
			frame(&ctx, &mut view, &mut state, vec![]);
		}
		assert!(!view.focus_switched_composer);
		frame(
			&ctx,
			&mut view,
			&mut state,
			vec![egui::Event::Text(" typed".into())],
		);
		assert_eq!(state.drafts[&Id(11)], "Another unsent draft typed");
		assert_eq!(state.drafts[&Id(10)], drafts[&Id(10)]);
		assert_eq!(
			view.editing.as_ref().unwrap().2,
			"Keep this unfinished edit"
		);
	}

	#[test]
	fn switcher_pointer_close_during_ime_cannot_commit_into_the_composer() {
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let drafts = state.drafts.clone();
		let mut view = MessagingUi {
			editing: Some((Id(10), Id(20), "Keep this edit".into())),
			..Default::default()
		};
		let frame = |view: &mut MessagingUi, state: &mut State, events| {
			let mut commands = Vec::new();
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1200.0, 800.0),
					)),
					focused: true,
					events,
					..Default::default()
				},
				|ui| commands.extend(view.show(ui, state)),
			);
			assert!(
				!commands
					.iter()
					.any(|command| matches!(command, Command::Send { .. } | Command::Edit { .. }))
			);
			output
		};
		frame(&mut view, &mut state, vec![]).drop_without_applying_deltas();
		view.switcher.open(&ctx);
		frame(&mut view, &mut state, vec![]).drop_without_applying_deltas();
		let output = frame(&mut view, &mut state, vec![]);
		let close = output
			.shapes
			.iter()
			.find_map(|shape| match &shape.shape {
				egui::Shape::Text(text) if text.galley.job.text == "Close" => {
					Some(text.pos + text.galley.size() / 2.0)
				}
				_ => None,
			})
			.expect("Rendered picker Close control");
		output.drop_without_applying_deltas();
		frame(
			&mut view,
			&mut state,
			vec![egui::Event::Ime(egui::ImeEvent::Preedit {
				text: "Query composition".into(),
				active_range_chars: None,
			})],
		)
		.drop_without_applying_deltas();
		for pressed in [true, false] {
			frame(
				&mut view,
				&mut state,
				vec![
					egui::Event::PointerMoved(close),
					egui::Event::PointerButton {
						pos: close,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				],
			)
			.drop_without_applying_deltas();
		}
		assert!(
			view.switcher.is_open(),
			"Pointer close must wait for the query composition"
		);
		frame(
			&mut view,
			&mut state,
			vec![egui::Event::Ime(egui::ImeEvent::Commit(
				"Query composition".into(),
			))],
		)
		.drop_without_applying_deltas();
		frame(&mut view, &mut state, vec![edit_key(egui::Key::Escape)])
			.drop_without_applying_deltas();
		frame(&mut view, &mut state, vec![]).drop_without_applying_deltas();
		assert!(!view.switcher.is_open());
		assert!(
			!view.ime_active,
			"Query IME state must not leak into the composer"
		);
		assert_eq!(state.selected, Some(Id(10)));
		assert_eq!(state.drafts, drafts);
		assert_eq!(view.editing.as_ref().unwrap().2, "Keep this edit");
	}

	#[test]
	fn inline_edit_uses_mentions_ime_and_preserves_draft_with_optimistic_updates() {
		// Restoring failed edits needs the full shell; keep editor IDs stable across frames.
		let edit_frame =
			|ctx: &egui::Context, view: &mut MessagingUi, state: &mut State, events| {
				let mut commands = Vec::new();
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(1000.0, 700.0),
						)),
						events,
						..Default::default()
					},
					|ui| commands.extend(view.show(ui, state)),
				);
				output.drop_without_applying_deltas();
				ctx.input_mut(|input| input.keys_down.clear());
				commands
			};
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let mut view = MessagingUi {
			editing: Some((Id(10), Id(20), "Original".into())),
			..Default::default()
		};
		view.reading_preferences.show_members = false;
		assert!(edit_frame(&ctx, &mut view, &mut state, vec![]).is_empty());
		assert!(
			edit_frame(
				&ctx,
				&mut view,
				&mut state,
				vec![egui::Event::Text(" @Al".into())]
			)
			.is_empty()
		);
		assert!(
			edit_frame(
				&ctx,
				&mut view,
				&mut state,
				vec![edit_key(egui::Key::Enter)]
			)
			.is_empty()
		);
		assert_eq!(view.editing.as_ref().unwrap().2, "Original <@1> ");
		assert!(
			edit_frame(
				&ctx,
				&mut view,
				&mut state,
				vec![
					egui::Event::Ime(egui::ImeEvent::Commit("語".into())),
					edit_key(egui::Key::Enter)
				]
			)
			.is_empty()
		);
		let commands = edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![edit_key(egui::Key::Enter)],
		);
		assert!(
			matches!(commands.as_slice(), [Command::Edit { channel: Id(10), message: Id(20), content, .. }] if content.trim_end() == "Original <@1> 語")
		);
		assert!(!view.has_edit(), "accepted edits close immediately");
		assert_eq!(
			state.timeline.get(Id(20)).unwrap().content,
			"Original <@1> 語\n"
		);
		let [Command::Edit { request, .. }] = commands.as_slice() else {
			panic!("Saving the edit emits one edit command");
		};
		state.apply_edit_result(
			Id(10),
			Id(20),
			*request,
			Err(client_core::auth::Failure::Network),
		);
		assert_eq!(state.timeline.get(Id(20)).unwrap().content, "Original");
		assert_eq!(
			state.message_actions.failed_edits[0].2,
			"Original <@1> 語\n"
		);
		assert_eq!(state.drafts[&Id(10)], "Unsent draft 👋");
		assert!(
			view.draft_changes.is_empty(),
			"edits must not overwrite unsent drafts"
		);
		assert!(edit_frame(&ctx, &mut view, &mut state, vec![]).is_empty());
		assert_eq!(view.editing.as_ref().unwrap().2, "Original <@1> 語\n");
		assert!(state.message_actions.failed_edits.is_empty());
		let commands = edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![edit_key(egui::Key::Enter)],
		);
		let [Command::Edit { request, .. }] = commands.as_slice() else {
			panic!()
		};
		assert!(!view.has_edit());
		let confirmed = state.timeline.get(Id(20)).unwrap().clone();
		view.editing = Some((Id(10), Id(20), "Newer input".into()));
		state.apply_edit_result(Id(10), Id(20), *request, Ok(confirmed));
		edit_frame(&ctx, &mut view, &mut state, vec![]);
		assert_eq!(view.editing.as_ref().unwrap().2, "Newer input");
		assert_eq!(state.drafts[&Id(10)], "Unsent draft 👋");
	}

	#[test]
	fn inline_edit_survives_navigation_blocks_invalid_saves_and_cancels_without_sending() {
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let mut view = MessagingUi {
			editing: Some((Id(10), Id(20), "Replacement".into())),
			..Default::default()
		};
		edit_frame(&ctx, &mut view, &mut state, vec![]);
		state.selected = Some(Id(11));
		edit_frame(&ctx, &mut view, &mut state, vec![]);
		assert_eq!(view.editing.as_ref().unwrap().2, "Replacement");
		assert!(!state.drafts.contains_key(&Id(11)));
		state.selected = Some(Id(10));
		edit_frame(&ctx, &mut view, &mut state, vec![]);
		state.freshness = Freshness::Stale;
		assert!(
			edit_frame(
				&ctx,
				&mut view,
				&mut state,
				vec![edit_key(egui::Key::Enter)]
			)
			.is_empty()
		);
		assert!(view.has_edit());
		state.freshness = Freshness::Fresh;
		let channels = std::mem::take(&mut state.channels);
		assert!(
			edit_frame(
				&ctx,
				&mut view,
				&mut state,
				vec![edit_key(egui::Key::Enter)]
			)
			.is_empty()
		);
		assert!(
			view.has_edit(),
			"Losing channel access keeps the unsaved edit"
		);
		state.channels = channels;
		state.user.as_mut().unwrap().id = Id(99);
		assert!(
			edit_frame(
				&ctx,
				&mut view,
				&mut state,
				vec![edit_key(egui::Key::Enter)]
			)
			.is_empty()
		);
		assert!(view.has_edit());
		assert!(
			edit_frame(
				&ctx,
				&mut view,
				&mut state,
				vec![edit_key(egui::Key::Escape)]
			)
			.is_empty()
		);
		assert!(!view.has_edit());
		assert_eq!(state.drafts[&Id(10)], "Unsent draft 👋");
		assert!(view.draft_changes.is_empty());
	}

	#[test]
	fn unread_pages_and_return_to_present_preserve_drafts_without_automatic_ack() {
		fn collect(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => labels.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels);
					}
				}
				_ => {}
			}
		}
		for (marker, width, dark) in [(None, 760.0, false), (Some(Id(10)), 1100.0, true)] {
			let ctx = egui::Context::default();
			ctx.set_visuals(if dark {
				egui::Visuals::dark()
			} else {
				egui::Visuals::light()
			});
			let mut view = MessagingUi::default();
			let mut state = edit_state();
			state.channels[0].last_message = Some(Id(20));
			state
				.apply_read_state(client_core::read_state::Event::Snapshot {
					entries: Some(vec![(Id(10), marker, 0)]),
					version: Some(1),
					partial: false,
				})
				.unwrap();
			let mut message = state.timeline.get(Id(20)).unwrap().clone();
			message.content = "Synthetic tall unread row\n\n".repeat(80);
			state.timeline.clear();
			state
				.timeline
				.insert(message.clone(), false, false)
				.unwrap();
			state.revision += 1;
			let drafts = state.drafts.clone();
			let frame = |view: &mut MessagingUi, state: &mut State, events| {
				let mut commands = vec![];
				let output = ctx.run_ui(
					egui::RawInput {
						focused: true,
						events,
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 720.0),
						)),
						..Default::default()
					},
					|ui| commands = view.show(ui, state),
				);
				assert!(output.platform_output.commands.is_empty());
				assert!(
					!commands.iter().any(|command| matches!(
						command,
						Command::Send { .. }
							| Command::Edit { .. }
							| Command::MarkRead { .. }
							| Command::MarkGuildRead { .. }
					)),
					"Browsing unread pages must not send or acknowledge"
				);
				let mut labels = vec![];
				for shape in &output.shapes {
					collect(&shape.shape, &mut labels);
				}
				output.drop_without_applying_deltas();
				(labels, commands)
			};
			let click = |view: &mut MessagingUi, state: &mut State, label: &str| {
				let (labels, _) = frame(view, state, vec![]);
				// The return to the live edge is an icon-only control with no painted text.
				let pos = if label == "Jump to present" {
					view.timeline
						.present_control
						.expect("navigation control")
						.center()
				} else {
					labels
						.iter()
						.find(|(text, _)| text == label)
						.expect("navigation control")
						.1
						.center()
				};
				let mut commands = vec![];
				for pressed in [true, false] {
					commands.extend(
						frame(
							view,
							state,
							vec![
								egui::Event::PointerMoved(pos),
								egui::Event::PointerButton {
									pos,
									button: egui::PointerButton::Primary,
									pressed,
									modifiers: egui::Modifiers::NONE,
								},
							],
						)
						.1,
					);
				}
				commands
			};
			for _ in 0..3 {
				frame(&mut view, &mut state, vec![]);
			}
			assert_eq!(state.read_marker(Id(10)), Some(marker));
			let commands = click(&mut view, &mut state, "Jump to unread");
			assert!(commands.iter().any(|command| matches!(command,
                Command::History { channel: Id(10), before: None, after: Some(after), .. } if *after == marker.unwrap_or(Id(0))
            )));
			let (labels, _) = frame(&mut view, &mut state, vec![]);
			assert!(!labels.iter().any(|(text, _)| text == "Next messages"));
			for (first, last) in [(11, 15), (16, 20)] {
				let messages = (first..=last)
					.map(|id| {
						let mut message = message.clone();
						message.id = Id(id);
						message
					})
					.collect();
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::History {
						channel: Id(10),
						request: state.request,
						older: false,
						messages,
					},
				});
				for _ in 0..3 {
					frame(&mut view, &mut state, vec![]);
				}
				if last == 15 {
					// Older history has no bar of its own; reaching its end requests the next page.
					view.timeline.load_newer = true;
					let (_, commands) = frame(&mut view, &mut state, vec![]);
					assert!(commands.iter().any(|command| matches!(
						command,
						Command::History {
							before: None,
							after: Some(Id(15)),
							..
						}
					)));
				}
			}
			let (labels, _) = frame(&mut view, &mut state, vec![]);
			assert!(!labels.iter().any(|(text, _)| text == "Next messages"));
			assert_eq!(state.read_marker(Id(10)), Some(marker));
			let commands = click(&mut view, &mut state, "Jump to present");
			assert!(commands.iter().any(|command| matches!(
				command,
				Command::History {
					before: None,
					after: None,
					..
				}
			)));
			assert_eq!(state.drafts, drafts);
		}
	}

	#[test]
	fn leaving_a_short_unread_channel_acknowledges_the_latest_message_seen() {
		let ctx = egui::Context::default();
		let mut view = MessagingUi::default();
		let mut state = edit_state();
		state.channels[0].last_message = Some(Id(20));
		state.history_pending = false;
		state.history_targeted = false;
		state.history_before = None;
		state.history_after = None;
		state.older_exhausted = true;
		state.channels.push(model::Channel {
			id: Id(11),
			guild: None,
			parent_id: None,
			kind: 1,
			name: "Synthetic next conversation".into(),
			position: 1,
			recipients: vec![],
			last_message: None,
			member_list_id: None,
			message_count: None,
			icon: None,
		});
		state
			.apply_read_state(client_core::read_state::Event::Snapshot {
				entries: Some(vec![(Id(10), Some(Id(10)), 0)]),
				version: Some(1),
				partial: false,
			})
			.unwrap();
		let frame = |view: &mut MessagingUi, state: &mut State| {
			let mut commands = vec![];
			let output = ctx.run_ui(
				egui::RawInput {
					focused: true,
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1280.0, 800.0),
					)),
					..Default::default()
				},
				|ui| commands = view.show(ui, state),
			);
			let mut saw_message = false;
			fn walk(shape: &egui::Shape, saw: &mut bool) {
				match shape {
					egui::Shape::Text(text) if text.galley.job.text.contains("Original") => {
						*saw = true;
					}
					egui::Shape::Vec(shapes) => {
						for shape in shapes {
							walk(shape, saw);
						}
					}
					_ => {}
				}
			}
			for shape in &output.shapes {
				walk(&shape.shape, &mut saw_message);
			}
			output.drop_without_applying_deltas();
			(commands, saw_message)
		};
		for _ in 0..4 {
			let (commands, saw_message) = frame(&mut view, &mut state);
			assert!(saw_message, "the open channel never painted its message");
			assert!(
				!commands
					.iter()
					.any(|command| matches!(command, Command::MarkRead { .. })),
				"a short unread channel acknowledged itself before the reader left"
			);
		}
		state.selected = Some(Id(11));
		let (commands, _) = frame(&mut view, &mut state);
		let ack = commands.iter().find_map(|command| match command {
			Command::MarkRead {
				channel,
				message,
				manual,
				..
			} => Some((*channel, *message, *manual)),
			_ => None,
		});
		assert_eq!(
			ack,
			Some((Id(10), Id(20), false)),
			"leaving the channel did not acknowledge the latest message on screen"
		);
	}

	#[test]
	fn reply_controls_are_inert_when_deleted_loading_stale_or_unavailable() {
		fn collect(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => labels.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels);
					}
				}
				_ => {}
			}
		}
		for blocked in 0..5 {
			let ctx = egui::Context::default();
			let mut view = MessagingUi::default();
			let mut state = edit_state();
			let mut source = state.timeline.get(Id(20)).unwrap().clone();
			source.reply_to = Some(Id(19));
			source.reply_deleted = blocked == 0;
			source.kind = 19;
			state.timeline.insert(source.clone(), true, false).unwrap();
			state.reply = Some(client_core::Reply::to(Id(19)));
			match blocked {
				0 => {}
				1 => {
					state.timeline.delete(Id(19)).unwrap();
				}
				2 => {
					state.freshness = Freshness::Loading;
					state.history_pending = true;
				}
				3 => {
					state.freshness = Freshness::Stale;
				}
				4 => {
					state.channels.clear();
					state.freshness = Freshness::Unavailable;
				}
				_ => unreachable!(),
			}
			let draft = state.drafts.clone();
			let request = state.request;
			let frame = |view: &mut MessagingUi, state: &mut State, events| {
				let mut commands = vec![];
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(760.0, 650.0),
						)),
						events,
						..Default::default()
					},
					|ui| commands = view.show(ui, state),
				);
				assert!(output.platform_output.commands.is_empty());
				assert!(!commands.iter().any(|command| matches!(
					command,
					Command::History { .. }
						| Command::Send { .. }
						| Command::Edit { .. }
						| Command::Delete { .. }
						| Command::MarkRead { .. }
						| Command::MarkGuildRead { .. }
				)));
				let mut labels = vec![];
				for shape in &output.shapes {
					collect(&shape.shape, &mut labels);
				}
				output.drop_without_applying_deltas();
				labels
			};
			for _ in 0..3 {
				frame(&mut view, &mut state, vec![]);
			}
			let labels = frame(&mut view, &mut state, vec![]);
			if blocked < 2 {
				assert!(labels.iter().any(|(text, _)| text == "Message deleted"));
			}
			// Activate the visible disabled controls (and the inert deleted label) with real input.
			for (_, rect) in labels.iter().filter(|(text, _)| {
				text == "View original"
					|| text == "Message deleted"
					|| text.starts_with("@Alex  ")
					|| text.starts_with("Earlier message")
			}) {
				let pos = rect.center();
				for pressed in [true, false] {
					frame(
						&mut view,
						&mut state,
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
			assert_eq!(state.request, request);
			assert!(state.search_target.is_none());
			assert_eq!(state.drafts, draft);
			assert_eq!(state.reply_target(), Some(Id(19)));
		}
	}

	#[test]
	fn reply_original_buttons_navigate_locally_or_request_one_page_without_sending() {
		fn collect(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => labels.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels);
					}
				}
				_ => {}
			}
		}
		for (loaded, composer, keyboard, width) in [
			(true, false, false, 900.0),
			(false, false, true, 760.0),
			(true, true, true, 900.0),
			(false, true, false, 760.0),
		] {
			let ctx = egui::Context::default();
			let mut view = MessagingUi::default();
			let mut state = edit_state();
			let mut source = state.timeline.get(Id(20)).unwrap().clone();
			source.content = "Reply source".into();
			source.reply_to = Some(Id(19));
			state.timeline.insert(source.clone(), true, false).unwrap();
			if loaded {
				source.id = Id(19);
				source.reply_to = None;
				source.content = "||Hidden original||".into();
				state.timeline.insert(source, false, false).unwrap();
			}
			state.reply = Some(client_core::Reply::to(Id(19)));
			let draft = state.drafts.clone();
			let frame = |view: &mut MessagingUi, state: &mut State, events| {
				let mut commands = vec![];
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 650.0),
						)),
						events,
						..Default::default()
					},
					|ui| commands = view.show(ui, state),
				);
				assert!(output.platform_output.commands.is_empty());
				assert!(!commands.iter().any(|command| matches!(
					command,
					Command::Send { .. }
						| Command::Edit { .. }
						| Command::Delete { .. }
						| Command::MarkRead { .. }
						| Command::MarkGuildRead { .. }
				)));
				let mut labels = vec![];
				for shape in &output.shapes {
					collect(&shape.shape, &mut labels);
				}
				for event in &output.platform_output.events {
					if let egui::output::OutputEvent::FocusGained(info) = event
						&& let Some(label) = &info.label
						&& let Some(response) = ctx
							.memory(|m| m.focused())
							.and_then(|id| ctx.read_response(id))
					{
						labels.push((label.clone(), response.rect));
					}
				}
				output.drop_without_applying_deltas();
				(labels, commands)
			};
			for _ in 0..3 {
				frame(&mut view, &mut state, vec![]);
			}
			let label_matches = |text: &str| {
				if composer {
					text == "View original"
				} else if loaded {
					text == "@Alex  Spoiler"
				} else {
					text == "Earlier message \u{b7} View original"
				}
			};
			let (labels, _) = frame(&mut view, &mut state, vec![]);
			assert!(
				!labels
					.iter()
					.any(|(text, _)| text.contains("Hidden original"))
			);
			let request = state.request;
			let commands = if keyboard {
				ctx.memory_mut(|memory| {
					if let Some(id) = memory.focused() {
						memory.surrender_focus(id);
					}
				});
				frame(&mut view, &mut state, vec![egui::Event::PointerGone]);
				let key = |key| egui::Event::Key {
					key,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers::NONE,
				};
				let mut activated = None;
				for _ in 0..60 {
					let (labels, _) = frame(&mut view, &mut state, vec![key(egui::Key::Tab)]);
					if ctx
						.memory(|memory| memory.focused())
						.and_then(|id| ctx.read_response(id))
						.is_some_and(|response| {
							response.rect.height() < 36.0
								&& labels.iter().any(|(text, rect)| {
									label_matches(text) && response.rect.contains(rect.center())
								})
						}) {
						activated =
							Some(frame(&mut view, &mut state, vec![key(egui::Key::Enter)]).1);
						break;
					}
				}
				activated.expect("Tab must reach the original-message button")
			} else {
				let point = labels
					.iter()
					.find(|(text, _)| label_matches(text))
					.unwrap()
					.1
					.center();
				let mut activated = vec![];
				for pressed in [true, false] {
					activated.extend(
						frame(
							&mut view,
							&mut state,
							vec![
								egui::Event::PointerMoved(point),
								egui::Event::PointerButton {
									pos: point,
									button: egui::PointerButton::Primary,
									pressed,
									modifiers: egui::Modifiers::NONE,
								},
							],
						)
						.1,
					);
				}
				activated
			};
			assert_eq!(state.reply_target(), Some(Id(19)));
			assert_eq!(state.drafts, draft);
			assert!(view.draft_changes.is_empty());
			assert_eq!(state.search_target, Some(Id(19)));
			if loaded {
				assert_eq!(state.request, request);
				assert!(
					!commands
						.iter()
						.any(|command| matches!(command, Command::History { .. }))
				);
				for _ in 0..3 {
					frame(&mut view, &mut state, vec![]);
				}
				assert!(state.search_target.is_none());
			} else {
				assert_eq!(
					commands
						.iter()
						.filter(|command| matches!(
							command,
							Command::History {
								channel: Id(10),
								before: Some(Id(20)),
								..
							}
						))
						.count(),
					1
				);
				assert!(state.history_pending);
				assert_eq!(state.freshness, Freshness::Loading);
				for _ in 0..3 {
					let (_, commands) = frame(&mut view, &mut state, vec![]);
					assert!(
						!commands
							.iter()
							.any(|command| matches!(command, Command::History { .. }))
					);
				}
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::HistoryFailed {
						channel: Id(10),
						request: state.request,
						failure: client_core::auth::Failure::Network,
					},
				});
				let failure = state.status;
				let (_, commands) = frame(&mut view, &mut state, vec![]);
				assert_eq!(
					state.status, failure,
					"Rendering must preserve the actual request error"
				);
				assert!(
					!commands
						.iter()
						.any(|command| matches!(command, Command::History { .. }))
				);
				assert_eq!(state.drafts, draft);
				ctx.memory_mut(|memory| {
					if let Some(id) = memory.focused() {
						memory.surrender_focus(id);
					}
				});
				let mut reload = None;
				for _ in 0..80 {
					let (labels, _) = frame(&mut view, &mut state, vec![edit_key(egui::Key::Tab)]);
					if let Some((_, rect)) =
						labels.iter().find(|(text, _)| text == "Reload history")
					{
						reload = Some(rect.center());
						break;
					}
				}
				let pos = reload.expect("Tab must reach Reload history");
				let mut reloads = 0;
				for pressed in [true, false] {
					let (_, commands) = frame(
						&mut view,
						&mut state,
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
					reloads += commands
						.iter()
						.filter(|command| {
							matches!(
								command,
								Command::History {
									channel: Id(10),
									before: None,
									..
								}
							)
						})
						.count();
				}
				assert_eq!(reloads, 1, "Explicit Reload returns to recent history");
			}
		}
	}

	#[test]
	fn deleted_edit_target_keeps_user_changes_but_releases_untouched_original() {
		fn collect(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => labels.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels);
					}
				}
				_ => {}
			}
		}
		let shortcut = |key| egui::Event::Key {
			key,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::COMMAND,
		};
		for (modified, reopen) in [(false, false), (true, false), (true, true)] {
			let ctx = egui::Context::default();
			let mut state = edit_state();
			let mut view = MessagingUi {
				editing: Some((Id(10), Id(20), "Original".into())),
				..Default::default()
			};
			let frame = |view: &mut MessagingUi, state: &mut State, events| {
				let mut commands = vec![];
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(900.0, 650.0),
						)),
						events,
						..Default::default()
					},
					|ui| commands = view.show(ui, state),
				);
				assert!(!commands.iter().any(|command| matches!(
					command,
					Command::Send { .. } | Command::Edit { .. } | Command::Delete { .. }
				)));
				assert!(output.platform_output.commands.is_empty());
				let mut labels = vec![];
				for shape in &output.shapes {
					collect(&shape.shape, &mut labels);
				}
				for event in &output.platform_output.events {
					if let egui::output::OutputEvent::FocusGained(info) = event
						&& let Some(label) = &info.label
						&& let Some(response) = ctx
							.memory(|m| m.focused())
							.and_then(|id| ctx.read_response(id))
					{
						labels.push((label.clone(), response.rect));
					}
				}
				output.drop_without_applying_deltas();
				labels
			};
			for _ in 0..3 {
				frame(&mut view, &mut state, vec![]);
			}
			if modified {
				frame(
					&mut view,
					&mut state,
					vec![
						shortcut(egui::Key::A),
						egui::Event::Text("Only my unsent text".into()),
					],
				);
				assert_eq!(view.editing.as_ref().unwrap().2, "Only my unsent text");
			}
			if reopen {
				let labels = frame(&mut view, &mut state, vec![]);
				let message = labels
					.iter()
					.find(|(text, _)| text.trim_end() == "Original")
					.expect("Loaded message remains visible")
					.1
					.center();
				frame(
					&mut view,
					&mut state,
					vec![egui::Event::PointerMoved(message)],
				);
				ctx.memory_mut(|memory| {
					if let Some(id) = memory.focused() {
						memory.surrender_focus(id);
					}
				});
				let mut activated = false;
				for _ in 0..80 {
					let labels = frame(&mut view, &mut state, vec![edit_key(egui::Key::Tab)]);
					if labels.iter().any(|(text, _)| text == "Edit message") {
						frame(&mut view, &mut state, vec![edit_key(egui::Key::Enter)]);
						activated = true;
						break;
					}
				}
				assert!(activated, "Tab must reach the own-message edit action");
				assert_eq!(view.editing.as_ref().unwrap().2, "Original");
			}
			let retained = view.editing.as_ref().unwrap().2.clone();
			let keep_edit = modified && !reopen;
			assert_eq!(retained != "Original", keep_edit);
			view.deleting = Some((Id(10), Id(20)));
			state.timeline.delete(Id(20)).unwrap();
			state.revision += 1;
			view.messages_deleted(&ctx, Id(10), &[Id(20)]);
			frame(&mut view, &mut state, vec![edit_key(egui::Key::Enter)]);
			assert!(view.deleting.is_none());
			assert_eq!(view.has_edit(), keep_edit);
			if keep_edit {
				frame(&mut view, &mut state, vec![shortcut(egui::Key::Z)]);
				assert_eq!(
					view.editing.as_ref().unwrap().2,
					retained,
					"Undo cannot restore the deleted original"
				);
				let labels = frame(&mut view, &mut state, vec![]);
				assert_eq!(view.editing.as_ref().unwrap().2, retained);
				assert!(labels.iter().any(|(label, _)| label == "Copy edit text"));
				assert!(
					labels
						.iter()
						.any(|(label, _)| label.contains("Message unavailable"))
				);
				frame(&mut view, &mut state, vec![edit_key(egui::Key::Escape)]);
				assert!(!view.has_edit());
			}
			assert_eq!(state.drafts[&Id(10)], "Unsent draft 👋");
			assert!(view.draft_changes.is_empty());
		}
	}

	#[test]
	fn off_channel_deletion_releases_original_and_undo_without_touching_other_input() {
		for modified in [false, true] {
			let ctx = egui::Context::default();
			let mut state = edit_state();
			let mut view = MessagingUi {
				editing: Some((Id(10), Id(20), "Original".into())),
				..Default::default()
			};
			for _ in 0..3 {
				edit_frame(&ctx, &mut view, &mut state, vec![]);
			}
			if modified {
				let select_all = egui::Event::Key {
					key: egui::Key::A,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers::COMMAND,
				};
				edit_frame(
					&ctx,
					&mut view,
					&mut state,
					vec![select_all, egui::Event::Text("User replacement".into())],
				);
			}
			let text = view.editing.as_ref().unwrap().2.clone();
			let widget = view.edit_widget_id.unwrap();
			let cursor = egui::text_edit::TextEditState::load(&ctx, widget)
				.unwrap()
				.cursor
				.char_range()
				.unwrap();
			state.selected = Some(Id(11));
			state.channels.push(model::Channel {
				id: Id(11),
				guild: None,
				kind: 1,
				name: "Other conversation".into(),
				last_message: None,
				parent_id: None,
				position: 0,
				recipients: vec![],
				member_list_id: None,
				message_count: None,
				icon: None,
			});
			edit_frame(&ctx, &mut view, &mut state, vec![]);
			view.deleting = Some((Id(10), Id(20)));
			for (channel, ids) in [
				(Id(11), vec![Id(20)]),
				(Id(10), vec![Id(21)]),
				(Id(10), vec![Id(20); 101]),
			] {
				view.messages_deleted(&ctx, channel, &ids);
				assert_eq!(view.editing.as_ref().unwrap().2, text);
				assert!(view.deleting.is_some());
			}
			view.messages_deleted(&ctx, Id(10), &[Id(20)]);
			assert!(view.deleting.is_none());
			assert_eq!(view.has_edit(), modified);
			let editor = egui::text_edit::TextEditState::load(&ctx, widget).unwrap();
			assert!(
				editor.undoer().undo(&(cursor, text.clone())).is_none(),
				"Original undo snapshots must be gone before revisiting the conversation"
			);
			if modified {
				assert_eq!(view.editing.as_ref().unwrap().2, "User replacement");
				assert_eq!(editor.cursor.char_range(), Some(cursor));
			}
			// The close notification is scoped to A; B must still accept this frame's input.
			let output = ctx.run_ui(
				egui::RawInput {
					events: vec![egui::Event::Text("B input".into())],
					..Default::default()
				},
				|ui| {
					ctx.memory_mut(|memory| {
						memory.request_focus(ui.make_persistent_id("message-input"))
					});
					view.composer(ui, &mut state, Id(11), &ctx, &mut vec![]);
				},
			);
			output.drop_without_applying_deltas();
			assert_eq!(state.drafts[&Id(11)], "B input");
			assert_eq!(state.drafts[&Id(10)], "Unsent draft 👋");
		}
	}

	#[test]
	fn hidden_offline_members_keep_their_header_and_count() {
		fn person(id: u64) -> model::MemberSlot {
			model::MemberSlot::Person(model::Member {
				roles: vec![],
				user: model::User {
					id: Id(id),
					name: format!("Synthetic {id}"),
					avatar: None,
					webhook: false,
					kind: Default::default(),
					discriminator: 0,
					primary_guild: None,
				},
				nick: None,
				status: None,
				custom_status: None,
				activities: vec![],
				clients: model::ClientPlatforms::default(),
			})
		}
		let slots = vec![
			Some(model::MemberSlot::Group("online".into())),
			Some(person(1)),
			Some(model::MemberSlot::Group("offline".into())),
			Some(person(2)),
			Some(person(3)),
		];
		let online = |member: &model::Member| member.user.id.0 != 3;
		let shown = visible_member_rows(slots.iter().map(Option::as_ref), online);
		assert_eq!(shown, vec![0, 1, 2, 3]);
		assert_eq!(
			lazy_offline_rows(&[("online".into(), 1), ("offline".into(), 2)]),
			Some((3, 2))
		);
		assert_eq!(lazy_offline_rows(&[("online".into(), 1)]), None);
		assert_eq!(lazy_offline_rows(&[("offline".into(), 0)]), None);
	}

	#[test]
	fn member_badges_and_subtitles_stay_inside_adjacent_rows() {
		for light in [false, true] {
			for width in [180.0, 240.0] {
				let ctx = egui::Context::default();
				ctx.set_theme(if light {
					egui::ThemePreference::Light
				} else {
					egui::ThemePreference::Dark
				});
				design::apply(&ctx);
				let mut state = State {
					demo: true,
					selected: Some(Id(1)),
					members: Some(model::MemberList {
						channel: Id(1),
						guild: Some(Id(2)),
						request: 1,
						total: 3,
						lazy: false,
						groups: vec![],
						ranges: vec![],
						freshness: Freshness::Fresh,
						start: 0,
						slots: (1..=3)
							.map(|id| {
								Some(model::MemberSlot::Person(model::Member {
									user: model::User {
										id: Id(id),
										name: format!("Member {id}"),
										avatar: None,
										kind: model::AccountKind::Bot,
										webhook: false,
										discriminator: 0,
										primary_guild: (id == 1).then(|| {
											Box::new(model::ClanTag {
												guild: Id(9),
												tag: "SPDY".into(),
												badge: None,
											})
										}),
									},
									nick: None,
									roles: vec![],
									status: Some("online".into()),
									custom_status: Some(format!("Activity {id}")),
									activities: vec![],
									clients: model::ClientPlatforms::default(),
								}))
							})
							.collect(),
					}),
					..Default::default()
				};
				let mut messaging = MessagingUi::default();
				for hover in [false, true] {
					let mut origin = egui::Pos2::ZERO;
					let mut output = ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(width, 260.0),
							)),
							events: if hover {
								vec![egui::Event::PointerMoved(egui::pos2(80.0, 105.0))]
							} else {
								vec![]
							},
							..Default::default()
						},
						|ui| {
							origin = ui.cursor().min;
							let mut commands = vec![];
							messaging.member_rows(ui, &mut state, &mut commands);
						},
					);
					output.textures_delta.clear();
					for id in 1..=3 {
						let row = egui::Rect::from_min_size(
							origin + egui::vec2(0.0, (id - 1) as f32 * 42.0),
							egui::vec2(width, 42.0),
						);
						let mut labels = vec![format!("Member {id}"), format!("Activity {id}")];
						if id == 1 {
							labels.push("SPDY".into());
						}
						for label in labels {
							let text = output
								.shapes
								.iter()
								.find_map(|s| match &s.shape {
									egui::Shape::Text(t) if t.galley.job.text == label => {
										Some(egui::Rect::from_min_size(t.pos, t.galley.size()))
									}
									_ => None,
								})
								.expect("visible member text");
							assert!(
								text.top() >= row.top() && text.bottom() <= row.bottom() - 1.0,
								"{label}, hover={hover}: text {text:?} exceeds row {row:?}"
							);
						}
					}
					assert!(messaging.take_avatar_requests().is_empty());
					output.drop_without_applying_deltas();
				}
			}
		}
	}
	#[test]
	fn member_pane_virtualizes_and_preview_never_requests_network() {
		fn collect_text(shape: &egui::Shape, text: &mut Vec<String>) {
			match shape {
				egui::Shape::Text(shape) => text.push(shape.galley.job.text.clone()),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect_text(shape, text);
					}
				}
				_ => {}
			}
		}

		let mut state = State {
			demo: true,
			selected: Some(Id(1)),
			channels: vec![model::Channel {
				last_message: None,
				id: Id(1),
				guild: Some(Id(2)),
				parent_id: None,
				position: 0,
				name: "Synthetic".into(),
				kind: 0,
				recipients: Vec::new(),
				member_list_id: Some("everyone".into()),
				message_count: None,
				icon: None,
			}],
			members: Some(model::MemberList {
				channel: Id(1),
				guild: Some(Id(2)),
				request: 1,
				total: 100,
				lazy: false,
				groups: vec![
					("8".into(), 2),
					("online".into(), 1),
					("offline".into(), 97),
				],
				ranges: vec![],
				freshness: Freshness::Fresh,
				start: 0,
				slots: (1..=100)
					.map(|id| {
						Some(model::MemberSlot::Person(model::Member {
							user: model::User {
								id: Id(id),
								name: format!("Synthetic {id}"),
								avatar: None,
								webhook: false,
								kind: Default::default(),
								discriminator: 0,
								primary_guild: None,
							},
							nick: None,
							roles: vec![],
							status: match id {
								1 => Some("online"),
								2 => Some("idle"),
								3 => Some("dnd"),
								4 => Some("offline"),
								5 => Some("unknown"),
								_ => None,
							}
							.map(str::to_owned),
							custom_status: None,
							activities: vec![],
							clients: model::ClientPlatforms::default(),
						}))
					})
					.collect(),
			}),
			..Default::default()
		};
		state.permissions.guilds.insert(
			Id(2),
			model::permissions::Guild {
				id: Id(2),
				owner: None,
				member: None,
				roles: Some(vec![model::permissions::Role {
					id: Id(8),
					bits: 0,
					name: "Founders".into(),
					color: 0xe78284,
					position: 1,
					hoist: true,
				}]),
			},
		);
		for member in state
			.members
			.as_mut()
			.unwrap()
			.slots
			.iter_mut()
			.flatten()
			.filter_map(|slot| match slot {
				model::MemberSlot::Person(m) => Some(m),
				_ => None,
			})
			.take(2)
		{
			member.roles.push(Id(8));
		}
		let slots = &mut state.members.as_mut().unwrap().slots;
		if let Some(model::MemberSlot::Person(m)) = slots[0].as_mut() {
			m.user.kind = model::AccountKind::Bot;
		}
		if let Some(model::MemberSlot::Person(m)) = slots[1].as_mut() {
			m.user.kind = model::AccountKind::App;
		}
		if let Some(model::MemberSlot::Person(m)) = slots[2].as_mut() {
			m.user.webhook = true;
		}
		slots.insert(0, Some(model::MemberSlot::Group("offline".into())));
		slots.insert(0, Some(model::MemberSlot::Group("online".into())));
		slots.insert(0, Some(model::MemberSlot::Group("8".into())));
		let mut messaging = MessagingUi::default();
		let context = egui::Context::default();
		let output = context.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1120.0, 760.0),
				)),
				..Default::default()
			},
			|ui| {
				assert!(messaging.show(ui, &mut state).is_empty());
			},
		);
		assert!(messaging.take_avatar_requests().is_empty());
		let mut text = Vec::new();
		for shape in &output.shapes {
			collect_text(&shape.shape, &mut text);
		}
		for heading in [
			"BOT",
			"APP",
			"WEBHOOK",
			"Founders - 2",
			"Online - 1",
			"Offline - 97",
		] {
			assert!(
				text.iter().any(|label| label == heading),
				"Missing {heading}"
			);
		}
		for id in 1..=6 {
			assert!(text.iter().any(|label| label == &format!("Synthetic {id}")));
		}
		for status in [
			"Online",
			"Away",
			"Do not disturb",
			"Offline",
			"Presence unavailable",
		] {
			assert!(
				!text.iter().any(|label| label == status),
				"Members without activity or custom status must have no subtitle: {status}"
			);
		}
		assert!(
			!output.textures_delta.set.is_empty(),
			"Preview must exercise actual image uploads"
		);
		assert!(
			output.textures_delta.set.len() < 30,
			"Offscreen members must not upload textures"
		);
		output.drop_without_applying_deltas();
	}

	#[test]
	fn incoming_custom_status_updates_people_and_open_profile_without_refetch() {
		fn collect(shape: &egui::Shape, labels: &mut Vec<String>) {
			match shape {
				egui::Shape::Text(text) => labels.push(text.galley.job.text.clone()),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels);
					}
				}
				_ => {}
			}
		}
		let mut state = test_support::demo_state();
		state.demo = false; // Exercise normal command admission using synthetic loaded data.
		state.guild_folders = Some(Default::default()); // Folder fetch is outside this presence-only scenario.
		let channel = state.selected.unwrap();
		let guild = state
			.channels
			.iter()
			.find(|entry| entry.id == channel)
			.unwrap()
			.guild
			.unwrap();
		let user = test_support::message(499, channel).author;
		// No read acknowledgement is part of this presence-only scenario: clear the page and
		// the latest-message metadata that an empty live edge would otherwise acknowledge.
		state.timeline.clear();
		if let Some(channel) = state.channels.iter_mut().find(|c| c.id == channel) {
			channel.last_message = None;
		}
		state.members = Some(model::MemberList {
			channel,
			guild: Some(guild),
			request: 7,
			total: 1,
			lazy: false,
			groups: vec![],
			ranges: vec![],
			freshness: Freshness::Fresh,
			start: 0,
			slots: vec![Some(model::MemberSlot::Person(model::Member {
				user: user.clone(),
				nick: None,
				roles: vec![],
				status: Some("online".into()),
				custom_status: Some("Initial synthetic status".into()),
				activities: vec![],
				clients: model::ClientPlatforms::default(),
			}))],
		});
		state.profile = Some(client_core::profile::ProfileView {
			user: user.id,
			guild: Some(guild),
			request: 314,
			loading: false,
			error: None,
			data: Some(profiles::synthetic(&user, Some(guild))),
		});
		state
			.drafts
			.insert(channel, "Keep this unsent draft".into());
		let mut messaging = MessagingUi {
			navigation_channel: Some(channel),
			guild: Some(guild),
			profile: {
				let mut profile = profiles::ProfileSession::default();
				profile.command_open(user.clone());
				profile
			},
			..Default::default()
		};
		let ctx = egui::Context::default();
		design::apply(&ctx);
		let frame = |messaging: &mut MessagingUi, state: &mut State| {
			let mut commands = vec![];
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1280.0, 900.0),
					)),
					..Default::default()
				},
				|ui| commands = messaging.show(ui, state),
			);
			let platform_commands_empty = output.platform_output.commands.is_empty();
			let mut labels = vec![];
			for shape in &output.shapes {
				collect(&shape.shape, &mut labels);
			}
			output.drop_without_applying_deltas();
			assert!(
				commands.is_empty(),
				"Presence rendering must not fetch a profile or emit other commands"
			);
			assert!(platform_commands_empty);
			labels
		};
		for _ in 0..3 {
			frame(&mut messaging, &mut state);
		}
		let labels = frame(&mut messaging, &mut state);
		assert_eq!(
			labels
				.iter()
				.filter(|text| text.as_str() == "Initial synthetic status")
				.count(),
			2,
			"People and the already-open profile both show the loaded status"
		);
		for custom_status in [Some("Updated synthetic status"), None] {
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::MemberPresence {
					guild,
					channel,
					request: 7,
					updates: vec![model::MemberPresence {
						user: user.id,
						status: Some("online".into()),
						custom_status: custom_status.map(str::to_owned),
						activities: vec![],
						clients: model::ClientPlatforms::default(),
					}],
				},
			});
			for _ in 0..2 {
				frame(&mut messaging, &mut state);
			}
			let labels = frame(&mut messaging, &mut state);
			assert!(!labels.iter().any(|text| text == "Initial synthetic status"));
			assert_eq!(
				labels
					.iter()
					.filter(|text| text.as_str() == "Updated synthetic status")
					.count(),
				if custom_status.is_some() { 2 } else { 0 }
			);
			assert_eq!(state.profile.as_ref().unwrap().request, 314);
			assert_eq!(messaging.profile.open_user().unwrap().id, user.id);
			assert_eq!(state.drafts[&channel], "Keep this unsent draft");
			assert!(messaging.draft_changes.is_empty());
		}
	}

	#[test]
	fn rich_presence_reaches_lists_dm_header_and_profile_then_clears() {
		fn collect(shape: &egui::Shape, output: &mut String) {
			match shape {
				egui::Shape::Text(text) => {
					output.push_str(&text.galley.job.text);
					output.push('\n');
				}
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, output);
					}
				}
				_ => {}
			}
		}
		for dm in [false, true] {
			let mut state = test_support::demo_state();
			let user = test_support::message(1, Id(22)).author;
			let channel = if dm { Id(22) } else { state.selected.unwrap() };
			let _ = state.select(channel);
			state.timeline.clear();
			state.history_pending = false;
			let guild = state
				.channels
				.iter()
				.find(|c| c.id == channel)
				.unwrap()
				.guild;
			let _ = state.request_members();
			state.members = Some(model::MemberList {
				guild,
				channel,
				request: 7,
				total: 1,
				lazy: false,
				groups: vec![],
				ranges: vec![],
				freshness: Freshness::Fresh,
				start: 0,
				slots: vec![Some(model::MemberSlot::Person(model::Member {
					roles: vec![],
					user: user.clone(),
					nick: None,
					status: Some("online".into()),
					custom_status: None,
					activities: vec![],
					clients: model::ClientPlatforms::default(),
				}))],
			});
			let mut messaging = MessagingUi {
				navigation_channel: Some(channel),
				guild,
				profile: {
					let mut profile = profiles::ProfileSession::default();
					profile.command_open(user.clone());
					profile
				},
				..Default::default()
			};
			let ctx = egui::Context::default();
			design::apply(&ctx);
			for clear in [false, true] {
				let activities = if clear {
					vec![]
				} else {
					vec![model::RichActivity {
						kind: 0,
						name: "Stardew Valley".into(),
						details: Some("Tending the farm".into()),
						state: Some("Spring Day 12".into()),
						image: Some(model::ActivityImage::Asset {
							application: Id(9001),
							asset: Id(9002),
						}),
						small_image: None,
						ends_at: None,
						started_at: None,
					}]
				};
				let event = if let Some(guild) = guild {
					client_core::Event::MemberPresence {
						guild,
						channel,
						request: 7,
						updates: vec![model::MemberPresence {
							user: user.id,
							status: Some("online".into()),
							custom_status: None,
							activities,
							clients: model::ClientPlatforms::default(),
						}],
					}
				} else {
					client_core::Event::DirectPresence(vec![client_core::presence::Update {
						user: user.id,
						status: model::Patch::Value("online".into()),
						activities: model::Patch::Value(activities),
						custom_status: model::Patch::Absent,
						clients: model::Patch::Absent,
					}])
				};
				state.apply(client_core::Envelope {
					generation: state.generation,
					event,
				});
				let mut painted = String::new();
				let mut artwork = Vec::new();
				for _ in 0..3 {
					painted.clear();
					artwork.clear();
					let output = ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(1280.0, 900.0),
							)),
							..Default::default()
						},
						|ui| {
							messaging.show(ui, &mut state);
						},
					);
					assert!(output.platform_output.commands.is_empty());
					for shape in &output.shapes {
						collect(&shape.shape, &mut painted);
					}
					// Match this activity's actual texture, not unrelated same-sized UI meshes.
					let activity_texture = messaging.avatars.texture_id(
						&model::ActivityImage::Asset {
							application: Id(9001),
							asset: Id(9002),
						}
						.key(),
					);
					artwork = ctx
						.tessellate(output.shapes.clone(), output.pixels_per_point)
						.iter()
						.filter_map(|shape| match &shape.primitive {
							egui::epaint::Primitive::Mesh(mesh)
								if Some(mesh.texture_id) == activity_texture
									&& shape
										.clip_rect
										.intersect(mesh.calc_bounds())
										.is_positive() =>
							{
								Some(mesh.calc_bounds())
							}
							_ => None,
						})
						.collect();
					output.drop_without_applying_deltas();
				}
				assert_eq!(
					painted.matches("Playing Stardew Valley").count(),
					if clear {
						0
					} else if dm {
						2
					} else {
						1
					},
					"{painted}"
				);
				if !clear {
					// The profile renders the Discord-style activity card with the
					// heading and the game name as separate labels (see profiles.rs).
					assert!(painted.contains("Playing\nStardew Valley"), "{painted}");
				}
				assert_eq!(painted.contains("Tending the farm"), !clear);
				assert_eq!(painted.contains("Spring Day 12"), !clear);
				assert_eq!(
					artwork.len(),
					usize::from(!clear),
					"Activity image appears and clears with its presence"
				);
			}
		}
	}

	#[test]
	fn startup_warning_is_visible_then_clears_on_ready_and_logout() {
		fn contains_warning(shape: &egui::Shape) -> bool {
			match shape {
				egui::Shape::Text(text) => text.galley.job.text.contains(
					"Connected with some data unavailable: read status, notification settings",
				),
				egui::Shape::Vec(shapes) => shapes.iter().any(contains_warning),
				_ => false,
			}
		}
		let ctx = egui::Context::default();
		design::apply(&ctx);
		let mut state = test_support::demo_state();
		let mut messaging = MessagingUi::default();
		let visible = |messaging: &mut MessagingUi, state: &mut State| {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1120.0, 900.0),
					)),
					..Default::default()
				},
				|ui| {
					messaging.show(ui, state);
				},
			);
			let visible = output
				.shapes
				.iter()
				.any(|shape| contains_warning(&shape.shape));
			output.drop_without_applying_deltas();
			visible
		};
		for logout in [false, true] {
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::StartupWarnings(model::account::Warnings {
					read_state: true,
					notifications: true,
					..Default::default()
				}),
			});
			visible(&mut messaging, &mut state);
			assert!(visible(&mut messaging, &mut state));
			if logout {
				state.logout();
			} else {
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::Ready {
						user: state.user.clone().unwrap(),
						guilds: state.guilds.clone(),
						channels: state.channels.clone(),
						permissions: Default::default(),
					},
				});
			}
			assert!(!visible(&mut messaging, &mut state));
		}
	}

	#[test]
	fn webhook_profile_does_not_request_a_user_profile() {
		for webhook in [false, true] {
			let mut user = test_support::message(1, Id(22)).author;
			user.webhook = webhook;
			let mut state = State {
				auth: client_core::auth::AuthState::Authenticated,
				gateway_connected: true,
				..Default::default()
			};
			let mut messaging = MessagingUi {
				profile: {
					let mut profile = profiles::ProfileSession::default();
					profile.command_open(user);
					profile
				},
				..Default::default()
			};
			let mut commands = vec![];
			let output = egui::Context::default().run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1120.0, 900.0),
					)),
					..Default::default()
				},
				|ui| commands = messaging.show(ui, &mut state),
			);
			output.drop_without_applying_deltas();
			assert_eq!(
				commands
					.iter()
					.any(|command| matches!(command, Command::Profile { .. })),
				!webhook
			);
			assert_eq!(state.profile.is_some(), !webhook);
		}
	}

	#[test]
	fn text_author_click_toggles_profile_like_voice() {
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
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::Message(test_support::message(501, channel)),
		});
		let mut messaging = MessagingUi::default();
		let frame = |messaging: &mut MessagingUi, state: &mut State, events: Vec<egui::Event>| {
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1120.0, 900.0),
					)),
					events,
					..Default::default()
				},
				|ui| {
					messaging.show(ui, state);
				},
			);
			output.textures_delta.clear();
			let mut text = vec![];
			for shape in output.shapes {
				labels(&shape.shape, &mut text);
			}
			text
		};
		let click = |messaging: &mut MessagingUi, state: &mut State, pos: egui::Pos2| {
			for pressed in [true, false] {
				frame(
					messaging,
					state,
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
		let text = frame(&mut messaging, &mut state, vec![]);
		let (_, author) = text
			.iter()
			.filter(|(label, _)| label == "Robin (synthetic)")
			.min_by(|(_, left), (_, right)| left.min.x.total_cmp(&right.min.x))
			.expect("message author");
		let pos = author.center();
		click(&mut messaging, &mut state, pos);
		assert_eq!(
			messaging.profile.open_user().map(|user| user.id),
			Some(Id(2))
		);
		click(&mut messaging, &mut state, pos);
		assert!(messaging.profile.open_user().is_none());
		assert!(
			state
				.profile
				.as_ref()
				.is_some_and(|view| view.data.is_some())
		);
	}

	#[test]
	fn profile_uses_open_conversation_not_browsed_sidebar_server() {
		for guild in [None, Some(Id(10))] {
			let user = model::User {
				id: Id(2),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
			};
			let mut state = State {
				demo: true,
				selected: Some(Id(1)),
				channels: vec![model::Channel {
					last_message: None,
					id: Id(1),
					guild,
					parent_id: None,
					position: 0,
					name: "Open conversation".into(),
					kind: if guild.is_some() { 0 } else { 1 },
					recipients: vec![user.clone()],
					member_list_id: None,
					message_count: None,
					icon: None,
				}],
				..Default::default()
			};
			let mut messaging = MessagingUi {
				navigation_channel: state.selected,
				guild: Some(Id(99)),
				profile: {
					let mut profile = profiles::ProfileSession::default();
					profile.command_open(user);
					profile
				},
				..Default::default()
			};
			let output = egui::Context::default().run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1120.0, 900.0),
					)),
					..Default::default()
				},
				|ui| {
					messaging.show(ui, &mut state);
				},
			);
			assert_eq!(state.profile.as_ref().unwrap().guild, guild);
			output.drop_without_applying_deltas();
		}
	}

	#[test]
	fn inline_edit_keeps_the_pending_draft_attachment_separate() {
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let mut view = MessagingUi {
			editing: Some((Id(10), Id(20), "Replacement".into())),
			attachment: Some(("draft.txt".into(), 32)),
			upload_busy: true,
			..Default::default()
		};
		edit_frame(&ctx, &mut view, &mut state, vec![]);
		let commands = edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![edit_key(egui::Key::Enter)],
		);
		assert!(
			matches!(commands.as_slice(), [Command::Edit { content, .. }] if content == "Replacement")
		);
		assert_eq!(state.drafts[&Id(10)], "Unsent draft 👋");
		assert_eq!(view.attachment, Some(("draft.txt".into(), 32)));
		assert!(
			!view.attach_requested
				&& !view.remove_attachment_requested
				&& !view.cancel_upload_requested
		);
	}

	#[test]
	fn pending_preview_waits_for_confirmation_and_restore_preserves_new_drafts() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		let mut view = MessagingUi::default();
		view.preview_attachment(
			"synthetic.png",
			100,
			Some(egui::ColorImage::filled([2, 2], egui::Color32::WHITE)),
		);
		view.preview_sending(&ctx, &mut state);
		let nonce = state.pending.last().unwrap().nonce.clone();
		assert!(
			view.pending_upload.as_ref().unwrap().files[0]
				.preview
				.is_some()
		);
		assert!(view.attachment.is_none());
		view.update_upload_progress(None, true);
		assert_eq!(
			view.pending_upload.as_ref().unwrap().progress,
			Some((100, 100))
		);
		assert_eq!(
			state.pending.last().unwrap().delivery,
			model::Delivery::Sending
		);
		state.pending.last_mut().unwrap().delivery = model::Delivery::Ambiguous;
		state.drafts.insert(channel, "New draft".into());
		view.restore_pending(&mut state, channel, &nonce);
		assert_eq!(state.drafts[&channel], "New draft");
		assert!(state.pending.iter().any(|p| p.nonce == nonce));
		state.drafts.remove(&channel);
		view.restore_pending(&mut state, channel, &nonce);
		assert!(state.drafts[&channel].contains("sending it now"));
		assert!(!state.pending.iter().any(|p| p.nonce == nonce));
		ctx.run_ui(Default::default(), |ui| {
			view.show(ui, &mut state);
		})
		.drop_without_applying_deltas();
		assert!(view.pending_upload.is_none());

		view.preview_attachment("synthetic.png", 100, None);
		view.preview_sending(&ctx, &mut state);
		let nonce = state.pending.last().unwrap().nonce.clone();
		let mut confirmed = test_support::message(999_999, channel);
		confirmed.author = state.user.clone().unwrap();
		confirmed.nonce = Some(nonce.clone());
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::SendResult {
				nonce,
				result: Ok(confirmed),
			},
		});
		ctx.run_ui(Default::default(), |ui| {
			view.show(ui, &mut state);
		})
		.drop_without_applying_deltas();
		assert!(view.pending_upload.is_none());
		assert!(state.timeline.get(Id(999_999)).is_some());
	}

	#[test]
	fn attachment_only_enter_sends_once_and_busy_upload_blocks_resending() {
		for (busy, allowed, modifiers, repeat) in [
			(true, true, egui::Modifiers::NONE, false),
			(false, true, egui::Modifiers::NONE, false),
			(false, false, egui::Modifiers::NONE, false),
			(false, true, egui::Modifiers::SHIFT, false),
			(false, true, egui::Modifiers::ALT, false),
			(false, true, egui::Modifiers::NONE, true),
		] {
			let ctx = egui::Context::default();
			let mut state = test_support::demo_state();
			state.demo = false;
			let channel = state.selected.unwrap();
			state.drafts.remove(&channel);
			if !allowed {
				state.channels.clear();
			}
			let mut messaging = MessagingUi {
				attachment: Some(("synthetic.txt".into(), 32)),
				upload_busy: busy,
				..Default::default()
			};
			let mut commands = Vec::new();
			let mut editor = egui::Id::NULL;
			ctx.run_ui(Default::default(), |ui| {
				editor = ui.make_persistent_id("message-input");
				messaging.composer(ui, &mut state, channel, &ctx, &mut commands);
			})
			.drop_without_applying_deltas();
			assert!(commands.is_empty(), "Selecting a file must not send it");
			ctx.memory_mut(|m| m.request_focus(editor));
			if repeat {
				// egui derives repeat from held keys, overriding the raw event flag.
				ctx.input_mut(|i| i.keys_down.insert(egui::Key::Enter));
			}
			ctx.run_ui(
				egui::RawInput {
					events: vec![egui::Event::Key {
						key: egui::Key::Enter,
						physical_key: None,
						pressed: true,
						repeat,
						modifiers,
					}],
					..Default::default()
				},
				|ui| messaging.composer(ui, &mut state, channel, &ctx, &mut commands),
			)
			.drop_without_applying_deltas();
			let sends = !busy && allowed && modifiers == egui::Modifiers::NONE && !repeat;
			assert_eq!(commands.len(), usize::from(sends));
			if modifiers == egui::Modifiers::SHIFT {
				assert_eq!(state.drafts[&channel], "\n");
			}
			if sends {
				assert!(
					matches!(&commands[0], Command::Send { content, .. } if content.is_empty())
				);
				assert_eq!(state.pending[0].attachments, ["synthetic.txt"]);
				assert!(messaging.attachment.is_none());
				assert!(messaging.upload_busy);
				let mut release = edit_key(egui::Key::Enter);
				if let egui::Event::Key { pressed, .. } = &mut release {
					*pressed = false;
				}
				assert!(
					edit_frame(
						&ctx,
						&mut messaging,
						&mut state,
						vec![release, edit_key(egui::Key::Enter)],
					)
					.is_empty(),
					"Another Send before desktop dispatch must not enqueue the attachment again"
				);
			} else {
				assert!(
					messaging.attachment.is_some(),
					"Unavailable sends retain selected metadata"
				);
			}
		}
	}

	#[test]
	fn oversize_attachment_blocks_composer_send() {
		use client_core::upload_limit;
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		state.demo = false;
		let channel = state.selected.unwrap();
		state.drafts.remove(&channel);
		let over = upload_limit::account_upload_bytes(None) + 1;
		let mut messaging = MessagingUi {
			attachment: Some(("large.bin".into(), over)),
			..Default::default()
		};
		let mut commands = Vec::new();
		let mut editor = egui::Id::NULL;
		ctx.run_ui(Default::default(), |ui| {
			editor = ui.make_persistent_id("message-input");
			messaging.composer(ui, &mut state, channel, &ctx, &mut commands);
		})
		.drop_without_applying_deltas();
		ctx.memory_mut(|m| m.request_focus(editor));
		ctx.run_ui(
			egui::RawInput {
				events: vec![egui::Event::Key {
					key: egui::Key::Enter,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers::NONE,
				}],
				..Default::default()
			},
			|ui| messaging.composer(ui, &mut state, channel, &ctx, &mut commands),
		)
		.drop_without_applying_deltas();
		assert!(commands.is_empty(), "Send must stay disabled above the upload limit");
		assert!(messaging.attachment.is_some());
	}

	#[test]
	fn attachment_under_limit_still_sends() {
		use client_core::upload_limit;
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		state.demo = false;
		let channel = state.selected.unwrap();
		state.drafts.remove(&channel);
		let within = upload_limit::account_upload_bytes(None);
		let mut messaging = MessagingUi {
			attachment: Some(("small.bin".into(), within)),
			..Default::default()
		};
		let mut commands = Vec::new();
		let mut editor = egui::Id::NULL;
		ctx.run_ui(Default::default(), |ui| {
			editor = ui.make_persistent_id("message-input");
			messaging.composer(ui, &mut state, channel, &ctx, &mut commands);
		})
		.drop_without_applying_deltas();
		ctx.memory_mut(|m| m.request_focus(editor));
		ctx.run_ui(
			egui::RawInput {
				events: vec![egui::Event::Key {
					key: egui::Key::Enter,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers::NONE,
				}],
				..Default::default()
			},
			|ui| messaging.composer(ui, &mut state, channel, &ctx, &mut commands),
		)
		.drop_without_applying_deltas();
		assert_eq!(commands.len(), 1);
		assert!(matches!(&commands[0], Command::Send { .. }));
	}

	#[test]
	fn rendering_does_not_hide_saved_drafts_and_clears_survive_hydration() {
		let mut state = State {
			selected: Some(Id(1)),
			channels: vec![model::Channel {
				last_message: None,
				id: Id(1),
				guild: None,
				parent_id: None,
				position: 0,
				name: "Synthetic".into(),
				kind: 1,
				recipients: Vec::new(),
				member_list_id: None,
				message_count: None,
				icon: None,
			}],
			..Default::default()
		};
		let mut messaging = MessagingUi {
			draft_restore_pending: true,
			..Default::default()
		};
		let mut output = egui::Context::default().run_ui(Default::default(), |ui| {
			messaging.show(ui, &mut state);
		});
		output.textures_delta.clear();
		assert!(
			state.drafts.is_empty(),
			"merely viewing must not suppress asynchronous hydration"
		);
		state
			.drafts
			.entry(Id(1))
			.or_insert("saved synthetic draft".into());
		assert_eq!(state.drafts[&Id(1)], "saved synthetic draft");
		messaging.clear_draft(&mut state, Id(1));
		for _ in 0..3 {
			egui::Context::default()
				.run_ui(Default::default(), |ui| {
					messaging.show(ui, &mut state);
				})
				.drop_without_applying_deltas();
		}
		assert!(
			state.drafts.contains_key(&Id(1)),
			"unchanged renders must retain the explicit clear until hydration completes"
		);
		state
			.drafts
			.entry(Id(1))
			.or_insert("old disk snapshot".into());
		assert!(state.drafts[&Id(1)].is_empty());

		messaging.draft_restore_pending = false;
		state.drafts.retain(|_, text| !text.is_empty());
		for channel in 1..=64 {
			state.drafts.insert(Id(channel), "synthetic".into());
		}
		messaging.clear_draft(&mut state, Id(1));
		assert_eq!(
			state.drafts.len(),
			63,
			"clearing at the slot limit must free capacity"
		);
		assert_eq!(messaging.draft_changes, [Id(1), Id(1)]);
	}
}

#[cfg(debug_assertions)]
pub fn debug_member_search_check(state: &State, channel: Id) {
	mentions::debug_member_search_check(state, channel);
}

#[cfg(debug_assertions)]
pub fn debug_suggestion_pointer_check(state: &mut State, channel: Id) {
	mentions::debug_pointer_check(state, channel);
}

#[cfg(debug_assertions)]
pub fn debug_role_mentions_check(state: &mut State) {
	mentions::debug_role_mentions_check(state);
}

#[cfg(debug_assertions)]
pub fn debug_forward_check(state: &mut State) {
	let source = state
		.timeline
		.row_ids()
		.find(|id| state.can_forward(*id))
		.expect("forwardable fixture");
	let target = state.selected.unwrap();
	let draft = state.drafts.get(&target).cloned();
	let mut dialog = forwarding::ForwardDialog::default();
	dialog.open(state, source);
	let ctx = egui::Context::default();
	let mut avatars = avatars::Avatars::default();
	for _ in 0..2 {
		ctx.run_ui(egui::RawInput::default(), |ui| {
			dialog.show(ui.ctx(), state, &mut avatars, &mut Vec::new())
		})
		.drop_without_applying_deltas();
	}
	// Exercise the shared row through its text area, not just the checkbox.
	let row_ctx = egui::Context::default();
	let mut selected = false;
	let mut row = egui::Rect::NOTHING;
	let mut draw = |events| {
		row_ctx
			.run_ui(
				egui::RawInput {
					events,
					..Default::default()
				},
				|ui| {
					ui.set_width(320.0);
					let response =
						design::selection_row(ui, selected, "Destination", "Server", |_, _| {});
					row = response.rect;
					assert_eq!(row.height(), 56.0);
					if response.clicked() {
						selected = !selected;
					}
					icons::button(ui, icons::Icon::Forward, 28.0, "Forward message");
				},
			)
			.drop_without_applying_deltas();
	};
	draw(Vec::new());
	draw(Vec::new());
	let point = egui::pos2(90.0, 22.0);
	for pressed in [true, false] {
		draw(vec![
			egui::Event::PointerMoved(point),
			egui::Event::PointerButton {
				pos: point,
				button: egui::PointerButton::Primary,
				pressed,
				modifiers: egui::Modifiers::NONE,
			},
		]);
	}
	assert!(selected, "Clicking the row label selects the destination");
	assert!(state.prepare_forward(source, &[target; 6], "").is_empty());
	assert!(
		state
			.prepare_forward(source, &[target, target], "")
			.is_empty()
	);
	let commands = state.prepare_forward(source, &[target], "Optional note");
	assert_eq!(commands.len(), 2);
	assert!(
		matches!(&commands[0], Command::Forward { message, channel, .. } if *message == source && *channel == target)
	);
	assert!(
		matches!(&commands[1], Command::Send { content, reply: None, .. } if content == "Optional note")
	);
	assert_eq!(state.drafts.get(&target).cloned(), draft);
	for command in commands {
		state.command_rejected(command);
	}
	assert!(
		state
			.pending
			.iter()
			.all(|p| p.delivery != model::Delivery::Sending)
	);
}
