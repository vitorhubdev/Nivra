use local_store::AppPreferences;

#[derive(Default)]
pub struct Settings {
	pub current: AppPreferences,
	pub loaded: bool,
	pub state: crate::toggle_setting::Settings,
}

impl Settings {
	pub fn save(&mut self, cache: Option<&crate::cache::Cache>, generation: u64) -> bool {
		if !self.state.dirty || self.state.saving {
			return false;
		}
		let accepted = cache.is_some_and(|cache| {
			cache.queue(
				generation,
				model::Id(0),
				crate::cache::Operation::SaveAppPreferences(Box::new(self.current.clone())),
			)
		});
		// A full cache queue must not turn a device preference into a session-only change.
		self.state.dirty = !accepted;
		self.state.saving = accepted;
		self.state.failed = !accepted;
		accepted
	}
	pub fn set_reconnect_call(&mut self, hint: Option<local_store::ReconnectCallHint>) {
		if self.current.reconnect_call != hint {
			self.current.reconnect_call = hint;
			self.state.dirty = true;
			self.state.touched = true;
		}
	}
	pub fn remember_active_call(
		&mut self,
		channel: model::Id,
		guild: Option<model::Id>,
		user: model::Id,
		now_unix: u64,
	) {
		let Some(next) = local_store::ReconnectCallHint::new(channel, guild, user, now_unix) else {
			return;
		};
		if let Some(current) = self.current.reconnect_call
			&& current.same_target(next)
			&& now_unix.saturating_sub(current.at_unix) < local_store::ReconnectCallHint::TOUCH_SECS
		{
			return;
		}
		self.set_reconnect_call(Some(next));
	}
	pub fn observe(&mut self, ui: &ui::MessagingUi) {
		let value = AppPreferences {
			notifications_enabled: ui.notifications_enabled,
			auto_update: ui.updates.auto_update,
			update_nightly: false,
			notification_options: ui.notification_options,
			show_hidden_channels: ui.show_hidden_channels,
			hide_offline_members: ui.hide_offline_members,
			hide_bot_dms: ui.hide_bot_dms,
			hide_title_bar: ui.hide_title_bar,
			language: ui.language,
			language_chosen: self.current.language_chosen
				|| (self.loaded && ui.language != self.current.language),
			gpu_preference: ui.gpu_preference,
			primary_color: ui.primary_color,
			transparency_blur: ui.transparency_blur,
			transparency: ui.transparency,
			blur: ui.blur,
			transparent_all: ui.transparent_all,
			voice_noise_suppression: ui.voice_processing.effective().suppression
				!= model::voice_settings::NoiseSuppression::Off,
			voice_processing: Some(ui.voice_processing),
			voice_push_to_talk: ui.voice_push_to_talk,
			voice_muted: ui.voice_muted,
			voice_deafened: ui.voice_deafened,
			voice_input: ui.voice_input.clone(),
			voice_output: ui.voice_output.clone(),
			input_percent: ui.voice_gain.input_percent,
			output_percent: ui.voice_gain.output_percent,
			keybinds: ui.keybinds.clone(),
			expanded_folders: ui.expanded_folders.clone(),
			user_volumes: ui.voice_user_volume_overrides(),
			muted_users: ui.voice_user_mutes().to_vec(),
			voice_bot_safe_volume: ui.voice_bot_safe_volume,
			reconnect_call: self.current.reconnect_call,
			voice_auto_rejoin_short_disconnect: ui.voice_auto_rejoin_short_disconnect,
			notices_accepted: self.current.notices_accepted,
		};
		if value != self.current {
			self.state.touched = true;
			self.state.failed = !value.is_valid();
			if value.is_valid() {
				self.current = value;
				self.state.dirty = true;
			}
		}
	}
	pub fn apply(&mut self, ui: &mut ui::MessagingUi) {
		if !self.current.language_chosen {
			self.current.language = system_language();
		}
		let value = &self.current;
		ui.notifications_enabled = value.notifications_enabled;
		ui.updates.auto_update = value.auto_update;
		ui.updates.nightly = false;
		ui.notification_options = value.notification_options;
		ui.show_hidden_channels = value.show_hidden_channels;
		ui.hide_offline_members = value.hide_offline_members;
		ui.hide_bot_dms = value.hide_bot_dms;
		ui.hide_title_bar = value.hide_title_bar;
		ui.language = value.language;
		ui.gpu_preference = value.gpu_preference;
		ui.primary_color = value.primary_color;
		ui.transparency_blur = value.transparency_blur;
		ui.transparency = value.transparency;
		ui.blur = value.blur;
		ui.transparent_all = value.transparent_all;
		ui.voice_processing = value.voice_processing.unwrap_or_else(|| {
			model::voice_settings::VoiceProcessing::from_legacy(value.voice_noise_suppression)
		});
		ui.voice_push_to_talk = value.voice_push_to_talk;
		ui.voice_muted = value.voice_muted;
		ui.voice_deafened = value.voice_deafened;
		ui.voice_input.clone_from(&value.voice_input);
		ui.voice_output.clone_from(&value.voice_output);
		ui.voice_gain.input_percent = value.input_percent;
		ui.voice_gain.output_percent = value.output_percent;
		ui.keybinds = value.keybinds.clone();
		ui.expanded_folders.clone_from(&value.expanded_folders);
		ui.set_voice_user_volume_overrides(&value.user_volumes);
		ui.set_voice_user_mutes(&value.muted_users);
		ui.voice_bot_safe_volume = value.voice_bot_safe_volume;
		ui.voice_auto_rejoin_short_disconnect = value.voice_auto_rejoin_short_disconnect;
	}
}

/// First launch follows the operating system when that language exists in the app.
pub fn system_language() -> model::Language {
	match platform::locale::ui_language() {
		platform::locale::UiLanguage::Portuguese => model::Language::PortugueseBrazil,
		platform::locale::UiLanguage::Spanish => model::Language::Spanish,
		platform::locale::UiLanguage::English => model::Language::English,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn startup_defaults_do_not_overwrite_pending_saved_preferences() {
		let mut settings = Settings::default();
		let defaults = AppPreferences::default();
		let mut ui = ui::MessagingUi::default();
		ui.notifications_enabled = defaults.notifications_enabled;
		ui.transparency = defaults.transparency;
		ui.blur = defaults.blur;
		ui.voice_bot_safe_volume = defaults.voice_bot_safe_volume;
		settings.observe(&ui);
		assert!(
			!settings.state.touched,
			"startup defaults must not count as a user edit"
		);
		assert!(!settings.state.dirty);

		settings.current.notification_options.current_channel = true;
		settings.loaded = true;
		settings.apply(&mut ui);
		settings.observe(&ui);
		assert!(ui.notification_options.current_channel);
		assert!(!settings.state.touched);
		assert!(!settings.state.dirty);
	}

	#[test]
	fn member_list_choices_survive_observe_and_apply() {
		let mut settings = Settings::default();
		let mut ui = ui::MessagingUi::default();
		assert!(!ui.hide_offline_members);
		assert!(!ui.hide_bot_dms);
		ui.hide_offline_members = true;
		ui.hide_bot_dms = true;
		settings.observe(&ui);
		assert!(settings.current.hide_offline_members);
		assert!(settings.current.hide_bot_dms);
		let mut fresh = ui::MessagingUi::default();
		settings.apply(&mut fresh);
		assert!(fresh.hide_offline_members);
		assert!(fresh.hide_bot_dms);
	}

	#[test]
	fn legacy_preferences_without_voice_settings_keep_suppression_disabled() {
		let current: AppPreferences = serde_json::from_str("{}").unwrap();
		assert!(!current.voice_noise_suppression);
		assert!(current.voice_processing.is_none());
		let mut settings = Settings {
			current,
			..Default::default()
		};
		let mut ui = ui::MessagingUi::default();
		settings.apply(&mut ui);
		assert_eq!(
			ui.voice_processing.effective().suppression,
			model::voice_settings::NoiseSuppression::Off
		);
	}

	#[test]
	fn first_launch_follows_the_system_until_the_owner_picks_a_language() {
		let mut settings = Settings::default();
		settings.loaded = true;
		let mut ui = ui::MessagingUi::default();
		settings.apply(&mut ui);
		assert_eq!(ui.language, system_language());
		assert!(!settings.current.language_chosen);
		settings.observe(&ui);
		assert!(!settings.state.dirty);

		ui.language = match system_language() {
			model::Language::English => model::Language::PortugueseBrazil,
			_ => model::Language::English,
		};
		settings.observe(&ui);
		assert!(settings.current.language_chosen);
		assert_eq!(settings.current.language, ui.language);
		assert!(settings.state.dirty);
	}
}
