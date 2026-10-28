//! Declarative native settings. Tests, engines and networking belong to the host.
use model::protect::{Action, Draft, ProfileAction, Protocol, SavedProfile, Scope};
#[derive(Default)]
pub(crate) struct Settings {
	draft: Draft,
	reserve: Draft,
	fallback: bool,
	action: Option<Action>,
	status: String,
	busy: bool,
	enabled: bool,
	/// Scope of the live route, pushed by the host each frame; drives the PROTECT markers.
	active: Option<Scope>,
	profiles: Vec<SavedProfile>,
	profile_status: String,
	profiles_busy: bool,
	profiles_testing: bool,
	/// How many saved profiles the running test has finished, and how many it will test.
	test_done: usize,
	test_total: usize,
	reveal: bool,
	/// Exit country, ping and flags are collected by default. This switches them off and
	/// removes what was fetched.
	flags_removed: bool,
	flags: std::collections::BTreeMap<String, egui::TextureHandle>,
	/// A row was clicked: enable that profile as soon as the host has loaded it.
	activate_on_load: bool,
	activation_scope: Option<Scope>,
	login_blocked: bool,
	pending_remove: Option<String>,
	/// The profile being connected from the list, then the one that is live.
	activating: Option<String>,
	active_profile: Option<String>,
	/// A row clicked while tests were running: tests stop first, then it loads.
	activate_after_tests: Option<String>,
}
impl Settings {
	/// One click: load the profile and enable it; the host reports an error if that fails.
	fn activate(&mut self, id: &str) {
		if self.profiles_testing {
			self.activate_after_tests = Some(id.to_owned());
			self.action = Some(Action::Profiles(ProfileAction::CancelTests));
			self.status = "Connecting…".into();
			return;
		}
		self.activate_on_load = true;
		self.activation_scope = Some(self.draft.scope);
		self.activating = Some(id.to_owned());
		self.status = "Connecting…".into();
		self.action = Some(Action::Profiles(ProfileAction::Load(id.to_owned())));
	}
	fn request_enable(&mut self) {
		if let Some(reason) = self.draft.missing() {
			self.status = reason.into();
			self.activating = None;
			return;
		}
		self.action = Some(Action::Enable(
			self.draft.clone(),
			(self.fallback && self.draft.protocol != Protocol::WireGuard)
				.then(|| self.reserve.clone()),
		));
		self.busy = true;
		self.status = "Applying…".into();
	}
	/// What is routed now, and the one control that turns it off.
	fn status_header(&mut self, ui: &mut egui::Ui, demo: bool) {
		let colors = crate::design::palette(ui);
		crate::design::card(ui, |ui| {
			ui.strong(crate::tr_ui!(ui, "Protected connection"));
			ui.horizontal_wrapped(|ui| {
				if self.busy {
					ui.spinner();
				}
				match self.active.filter(|_| self.enabled) {
					Some(applied) => {
						let name = self
							.active_profile
							.as_deref()
							.and_then(|id| self.profiles.iter().find(|row| row.id == id))
							.map(|row| row.name.clone())
							.unwrap_or_else(|| crate::tr_ui!(ui, "Route active").to_string());
						ui.label(
							egui::RichText::new(format!(
								"{} · {name}",
								crate::tr_ui!(ui, "Connected")
							))
							.strong()
							.color(colors.positive),
						);
						ui.label(crate::tr_ui!(ui, applied.label()));
					}
					None => {
						ui.label(crate::tr_ui!(ui, "Not connected"));
					}
				}
			});
			// A failed profile switch must stay visible even while the previous route is active.
			if !self.status.is_empty() {
				ui.add(egui::Label::new(crate::tr_str!(ui, &self.status)).wrap());
			}
			if self.login_blocked {
				crate::design::notice(
					ui,
					crate::design::Level::Warning,
					crate::tr_ui!(
						ui,
						"Discord is paused until protection is ready. Retry a profile or choose to connect without PROTECT."
					),
				);
			}
			if (self.enabled || self.login_blocked)
				&& ui
					.add_enabled(
						!demo && !self.busy,
						egui::Button::new(crate::tr_ui!(
							ui,
							if self.enabled {
								"Disable"
							} else {
								"Connect without PROTECT"
							}
						))
						.min_size(egui::vec2(0.0, 32.0)),
					)
					.clicked()
			{
				self.action = Some(Action::Disable);
				self.busy = true;
				self.status = "Stopping…".into();
			}
		});
		if demo {
			ui.label(crate::tr_ui!(
				ui,
				"Offline preview: connection tests and route activation are disabled."
			));
		}
	}
	/// Saved routes as the main control: click one and it connects.
	fn profile_list(&mut self, ui: &mut egui::Ui, demo: bool) {
		let colors = crate::design::palette(ui);
		ui.horizontal(|ui| {
			ui.strong(crate::tr_ui!(ui, "Saved profiles"));
			if self.profiles_busy {
				ui.spinner();
			}
			if self.profiles_testing && ui.button(crate::tr_ui!(ui, "Cancel tests")).clicked() {
				self.action = Some(Action::Profiles(ProfileAction::CancelTests));
			}
		});
		if self.profiles_testing && self.test_total > 0 {
			let fraction = self.test_done as f32 / self.test_total as f32;
			ui.add(
				egui::ProgressBar::new(fraction)
					.animate(true)
					.desired_width(ui.available_width())
					.text(format!("{} / {}", self.test_done, self.test_total)),
			);
		}
		if self.profiles.is_empty() {
			ui.label(crate::tr_ui!(
				ui,
				"No profiles yet. Import a WireGuard .conf and it appears here, ready to connect."
			));
		}
		let rows = self.profiles.clone();
		egui::ScrollArea::vertical()
			.id_salt("protect-saved-profiles")
			.max_height(340.0)
			.show(ui, |ui| {
				for row in &rows {
					ui.push_id(&row.id, |ui| {
						let live = self.enabled && self.active_profile.as_deref() == Some(&row.id);
						let connecting = self.activating.as_deref() == Some(&row.id);
						ui.horizontal(|ui| {
							match self.flags.get(&row.id) {
								Some(flag) if !self.flags_removed => {
									ui.image((flag.id(), egui::vec2(24.0, 18.0)));
								}
								_ => {
									ui.add_space(24.0);
								}
							}
							let mark = if live {
								"●"
							} else if connecting {
								"…"
							} else {
								"○"
							};
							let mut label = egui::RichText::new(format!("{mark}  {}", row.name));
							if live {
								label = label.strong().color(colors.accent_text);
							}
							let button = egui::Button::new(label)
								.truncate()
								.min_size(egui::vec2((ui.available_width() - 40.0).max(1.0), 36.0))
								.fill(if live { colors.accent } else { colors.raised });
							let clickable = !demo
								&& !self.busy && (!self.profiles_busy
								|| self.profiles_testing)
								&& (!live || self.active != Some(self.draft.scope));
							if ui
								.add_enabled(clickable, button)
								.on_hover_text(format!(
									"{} · {}",
									crate::tr_ui!(ui, "Connect this profile"),
									row.name
								))
								.clicked()
							{
								self.activate(&row.id);
							}
							if ui
								.add_enabled_ui(!demo && !self.busy && !self.profiles_busy, |ui| {
									crate::icons::button(
										ui,
										crate::icons::Icon::Trash,
										32.0,
										crate::tr_ui!(ui, "Remove saved copy"),
									)
								})
								.inner
								.clicked()
							{
								self.pending_remove = Some(row.id.clone());
							}
						});
						if self.pending_remove.as_deref() == Some(row.id.as_str()) {
							ui.label(crate::tr_ui!(
								ui,
								"Remove only this saved profile? The active connection is not changed."
							));
							ui.horizontal_wrapped(|ui| {
								if ui
									.add_enabled(
										!demo && !self.busy && !self.profiles_busy,
										egui::Button::new(crate::tr_ui!(ui, "Remove")),
									)
									.clicked()
								{
									self.action = Some(Action::Profiles(ProfileAction::Delete(
										row.id.clone(),
									)));
									self.pending_remove = None;
								}
								if ui.button(crate::tr_ui!(ui, "Cancel")).clicked() {
									self.pending_remove = None;
								}
							});
						}
						let detail = Self::row_detail(ui, row, self.flags_removed);
						if !detail.is_empty() {
							ui.label(egui::RichText::new(detail).small().color(colors.muted));
						}
					});
				}
			});
		if !self.profile_status.is_empty() {
			ui.label(
				egui::RichText::new(crate::tr_str!(ui, &self.profile_status))
					.small()
					.color(colors.muted),
			);
		}
	}
	/// Protocol, test result and (unless removed) the exit country, in one muted line.
	fn row_detail(ui: &egui::Ui, row: &SavedProfile, flags_removed: bool) -> String {
		// WireGuard is the default; naming it on every row is noise.
		let mut parts = Vec::new();
		if row.protocol != Protocol::WireGuard {
			parts.push(row.protocol.label().to_owned());
		}
		if !row.result.is_empty() {
			if let Some((result, elapsed)) = row
				.result
				.rsplit_once(" · ")
				.filter(|(_, elapsed)| elapsed.ends_with(" ms"))
			{
				parts.push(format!("{} · {elapsed}", crate::tr_str!(ui, result)));
			} else {
				parts.push(crate::tr_str!(ui, &row.result).to_string());
			}
		}
		if !flags_removed && !row.location.is_empty() {
			parts.push(crate::tr_str!(ui, &row.location).to_string());
		}
		parts.join("  ·  ")
	}
	fn import_row(&mut self, ui: &mut egui::Ui, demo: bool) {
		ui.horizontal_wrapped(|ui| {
			if ui
				.add_enabled(
					!demo && !self.busy && !self.profiles_busy,
					egui::Button::new(crate::tr_ui!(ui, "Import WireGuard configuration")),
				)
				.clicked()
			{
				self.action = Some(Action::ImportWireGuard);
				self.busy = true;
			}
			if ui
				.add_enabled(
					!demo && !self.busy && !self.profiles.is_empty() && !self.profiles_busy,
					egui::Button::new(crate::tr_ui!(ui, "Test all")),
				)
				.clicked()
			{
				self.action = Some(Action::Profiles(ProfileAction::TestAll {
					location: !self.flags_removed,
				}));
			}
		});
	}
	/// What to route, chosen before connecting: it decides whether the call is touched at all.
	fn scope_row(&mut self, ui: &mut egui::Ui) {
		ui.add_enabled_ui(!self.busy && !self.profiles_busy, |ui| {
			ui.horizontal_wrapped(|ui| {
				ui.label(crate::tr_ui!(ui, "What to protect"));
				egui::ComboBox::from_id_salt("protect_scope")
					.selected_text(crate::tr_ui!(ui, self.draft.scope.label()))
					.show_ui(ui, |ui| {
						for scope in Scope::ALL {
							ui.selectable_value(
								&mut self.draft.scope,
								scope,
								crate::tr_ui!(ui, scope.label()),
							);
						}
					});
			});
			let colors = crate::design::palette(ui);
			let explanation = match self.draft.scope {
				Scope::Videos => crate::tr_ui!(ui, "Routes attachment video and file transfers. Text, voice and camera keep their usual route.").to_string(),
				Scope::Voice => crate::tr_ui!(ui, "Voice and camera use the same Discord socket and move together.").to_string(),
				Scope::Streams => crate::tr_ui!(ui, "Routes screen streams and their audio; voice and camera keep their usual route.").to_string(),
				Scope::Complete => crate::tr_ui!(ui, "Routes account signaling, messages, downloads and live media. Official login and external apps are separate.").to_string(),
			};
			ui.label(egui::RichText::new(explanation).small().color(colors.muted));
			ui.label(egui::RichText::new(crate::tr_ui!(ui, if self.draft.scope.protects_login() {
				"Startup: PROTECT first, then Discord. No automatic direct fallback."
			} else {
				"Startup: Discord first, then PROTECT. Messages stay connected during media route changes."
			})).small().color(colors.muted));
			if self
				.active
				.filter(|_| self.enabled)
				.is_some_and(|applied| applied != self.draft.scope)
			{
				ui.label(crate::tr_ui!(ui, "Changes not applied yet"));
			}
		});
	}
	/// What most people never need, folded away: manual proxies, reserve, keys.
	fn advanced(&mut self, ui: &mut egui::Ui, demo: bool) {
		ui.add_enabled_ui(!self.busy && !self.profiles_busy, |ui| {
			profile(ui, &mut self.draft, "primary", &mut self.reveal);
			if self.draft.protocol != Protocol::WireGuard {
				ui.checkbox(
					&mut self.fallback,
					crate::tr_ui!(
						ui,
						"Automatically try one reserve profile when reconnecting"
					),
				);
				if self.fallback {
					self.reserve.scope = self.draft.scope;
					profile(ui, &mut self.reserve, "reserve", &mut self.reveal);
				}
			}
		});
		let missing = self.draft.missing();
		let reserve_missing = (self.fallback && self.draft.protocol != Protocol::WireGuard)
			.then(|| self.reserve.missing())
			.flatten();
		let blocked = missing.or(reserve_missing);
		if let Some(reason) = blocked {
			ui.label(crate::tr_str!(ui, reason));
		}
		ui.add_enabled_ui(!demo && !self.busy && !self.profiles_busy, |ui| {
			ui.horizontal(|ui| {
				if ui
					.add_enabled(
						blocked.is_none(),
						egui::Button::new(crate::tr_ui!(ui, "Test connection")),
					)
					.clicked()
				{
					self.action = Some(Action::Test(self.draft.clone()));
					self.busy = true;
					self.status = "Testing…".into();
				}
				if ui
					.add_enabled(
						blocked.is_none(),
						egui::Button::new(crate::tr_ui!(ui, "Enable")),
					)
					.clicked()
				{
					self.request_enable();
				}
			});
		});
		ui.add_space(6.0);
		if !self.flags_removed
			&& ui
				.button(crate::tr_ui!(ui, "Remove flags and exit country"))
				.clicked()
		{
			self.flags_removed = true;
			self.flags.clear();
		}
		ui.label(crate::tr_ui!(ui,"A failed protected route never silently switches to a direct connection. Only media covered by the changed route is affected."));
	}
	pub(crate) fn show(&mut self, ui: &mut egui::Ui, demo: bool) {
		// The window already carries the PROTECT title.
		self.status_header(ui, demo);
		ui.add_space(6.0);
		self.scope_row(ui);
		ui.add_space(12.0);
		self.profile_list(ui, demo);
		ui.add_space(8.0);
		self.import_row(ui, demo);
		ui.add_space(10.0);
		egui::CollapsingHeader::new(crate::tr_ui!(ui, "Advanced"))
			.id_salt("protect-advanced")
			.show(ui, |ui| self.advanced(ui, demo));
	}
}
fn profile(ui: &mut egui::Ui, draft: &mut Draft, id: &str, reveal: &mut bool) {
	ui.push_id(id,|ui|{
		egui::ComboBox::from_id_salt("protocol").selected_text(draft.protocol.label()).show_ui(ui,|ui|{for p in Protocol::ALL {ui.selectable_value(&mut draft.protocol,p,p.label());}});
		if draft.protocol==Protocol::Socks4 && !draft.password.is_empty(){draft.password=Draft::default().password;}
		if draft.protocol==Protocol::WireGuard {
			ui.label(crate::tr_ui!(ui, "WireGuard is experimental; real provider compatibility has not been verified."));
			ui.label(crate::tr_ui!(ui,"Runs inside Nivra: no external program, no system VPN and no admin changes."));
			let public = model::protect::public_wireguard(&draft.wireguard);
			if !public.is_empty() {
				ui.group(|ui| { ui.label(egui::RichText::new(&public).monospace()); });
			}
			if public.lines().any(|line| line.starts_with("DNS =")) {
				ui.label(crate::tr_ui!(ui,"DNS shown above comes from the .conf. This field is not applied; the tunnel resolver uses 1.1.1.1. System DNS stays unchanged."));
			}
			ui.checkbox(reveal, crate::tr_ui!(ui,"Reveal and edit configuration, including private keys"));
			if *reveal {
				ui.add(egui::TextEdit::multiline(&mut *draft.wireguard).char_limit(4096).desired_rows(7).code_editor());
			}
		} else {
			ui.label(crate::tr_ui!(ui,"Server"));ui.add(egui::TextEdit::singleline(&mut draft.host).hint_text(crate::tr_ui!(ui,"proxy.example.com")).char_limit(253));
			ui.horizontal(|ui|{ui.label(crate::tr_ui!(ui,"Port"));ui.add(egui::DragValue::new(&mut draft.port).range(1..=65535));});
			ui.label(crate::tr_ui!(ui,"Username (optional)"));ui.add(egui::TextEdit::singleline(&mut *draft.username).char_limit(255));
			if draft.protocol!=Protocol::Socks4 {ui.label(crate::tr_ui!(ui,"Password (optional)"));ui.add(egui::TextEdit::singleline(&mut *draft.password).password(true).char_limit(255));}
			if draft.protocol==Protocol::Http || draft.protocol==Protocol::Socks4 {ui.label(crate::tr_ui!(ui,"TCP only. Use Videos and files; live voice, camera and streams require SOCKS5 UDP or WireGuard."));}
		}
	});
}
impl crate::MessagingUi {
	/// Recovery is available before login, even when the complete route failed closed.
	pub fn show_protect_recovery(&mut self, ui: &mut egui::Ui) {
		ui.push_id("startup-protect-recovery", |ui| {
			self.settings.protect.show(ui, false)
		});
	}
	pub fn protect_draft(&self) -> Draft {
		self.settings.protect.draft.clone()
	}
	pub fn set_protect_profiles(&mut self, profiles: Vec<SavedProfile>) {
		let protect = &mut self.settings.protect;
		let previous = std::mem::take(&mut protect.profiles);
		protect.profiles = profiles
			.into_iter()
			.take(16)
			.map(|mut row| {
				// A refresh of the list must not erase what was already tested or located.
				if let Some(old) = previous.iter().find(|old| old.id == row.id) {
					if row.result.is_empty() {
						row.result = old.result.clone();
					}
					if row.location.is_empty() {
						row.location = old.location.clone();
					}
				}
				row
			})
			.collect();
		let kept: Vec<String> = protect.profiles.iter().map(|row| row.id.clone()).collect();
		protect.flags.retain(|id, _| kept.contains(id));
	}
	pub fn set_protect_profile_location(
		&mut self,
		ctx: &egui::Context,
		id: &str,
		location: &str,
		flag: Option<egui::ColorImage>,
	) {
		let protect = &mut self.settings.protect;
		if protect.flags_removed {
			return;
		}
		if let Some(row) = protect.profiles.iter_mut().find(|row| row.id == id) {
			row.location = location.chars().take(512).collect();
			protect.flags.remove(id);
			if let Some(flag) = flag {
				protect.flags.insert(
					id.to_owned(),
					ctx.load_texture(
						format!("protect-flag-{id}"),
						flag,
						egui::TextureOptions::LINEAR,
					),
				);
			}
		}
	}
	pub fn set_protect_draft(&mut self, draft: Draft) {
		let protect = &mut self.settings.protect;
		let mut draft = draft;
		if protect.activate_on_load
			&& let Some(scope) = protect.activation_scope.take()
		{
			draft.scope = scope;
		}
		protect.draft = draft;
		protect.reveal = false;
		// A row click loaded this profile to connect it: enabling is the next step.
		if std::mem::take(&mut protect.activate_on_load) {
			protect.request_enable();
		}
	}
	pub fn set_protect_profile_status(&mut self, status: &str, busy: bool, testing: bool) {
		let protect = &mut self.settings.protect;
		protect.profile_status = status.chars().take(512).collect();
		protect.profiles_busy = busy;
		protect.profiles_testing = testing;
		if busy {
			return;
		}
		// The load finished without producing a profile: say why instead of staying silent.
		if std::mem::take(&mut protect.activate_on_load) {
			protect.activating = None;
			protect.activation_scope = None;
			protect.status = status.chars().take(512).collect();
		}
		// A row clicked during a test run connects as soon as the tests have stopped.
		if let Some(id) = protect.activate_after_tests.take() {
			protect.activate(&id);
		}
	}
	/// Progress of the running profile test, for the bar in the list.
	pub fn set_protect_profile_progress(&mut self, done: usize, total: usize) {
		let protect = &mut self.settings.protect;
		protect.test_done = done.min(total);
		protect.test_total = total;
	}
	pub fn set_protect_profile_result(&mut self, id: &str, result: &str) {
		if let Some(row) = self
			.settings
			.protect
			.profiles
			.iter_mut()
			.find(|row| row.id == id)
		{
			row.result = result.chars().take(512).collect();
		}
	}
	pub(super) fn open_protect_scope(&mut self, scope: Scope) {
		self.settings.protect.draft.scope = scope;
		self.open_protect_settings();
	}
	pub fn clear_protect(&mut self) {
		self.settings.protect = Settings::default();
	}
	pub fn set_protect_wireguard(&mut self, text: &str) -> Result<(), &'static str> {
		if text.len() > 4096 {
			return Err("WireGuard configuration exceeds 4 KiB");
		};
		let mut replacement = Draft::default().wireguard;
		replacement.push_str(text);
		self.settings.protect.draft.wireguard = replacement;
		self.settings.protect.draft.protocol = Protocol::WireGuard;
		self.settings.protect.busy = false;
		Ok(())
	}
	pub fn take_protect_action(&mut self) -> Option<Action> {
		self.settings.protect.action.take()
	}
	/// The host reports the live route's scope every frame; `None` means nothing is routed.
	pub fn set_protect_login_blocked(&mut self, blocked: bool) {
		self.settings.protect.login_blocked = blocked;
	}
	pub fn set_protect_active_scope(&mut self, scope: Option<Scope>) {
		self.settings.protect.active = scope;
	}
	/// True when the live route covers `scope`, so its PROTECT marker may be shown.
	pub(crate) fn protect_covers(&self, scope: Scope) -> bool {
		self.settings
			.protect
			.active
			.is_some_and(|active| active.contains(scope))
	}
	/// Whether exit country, ping and flags are collected (they are, until removed).
	pub fn protect_locate(&self) -> bool {
		!self.settings.protect.flags_removed
	}
	/// The host confirmed the profile clicked in the list is now the live route.
	pub fn mark_protect_active(&mut self) {
		let protect = &mut self.settings.protect;
		protect.active_profile = protect.activating.take();
	}
	/// A PROTECT problem the person must see even with the settings page closed.
	pub fn protect_alert(&mut self, text: &str) {
		self.toasts.push(crate::design::Level::Error, text);
	}
	pub fn set_protect_status(&mut self, status: &str, enabled: bool) {
		self.set_protect_operation_status(status, enabled, false);
	}
	pub fn set_protect_operation_status(&mut self, status: &str, enabled: bool, busy: bool) {
		let protect = &mut self.settings.protect;
		protect.status = status.chars().take(512).collect();
		protect.enabled = enabled;
		protect.busy = busy;
		if !enabled {
			protect.active_profile = None;
		}
		// Done without the host confirming the profile: the attempt failed, nothing is live.
		if !busy {
			protect.activating = None;
		}
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn batch_cancellation_remains_clickable_while_configuration_fields_are_locked() {
		fn label_rect(shape: &egui::Shape) -> Option<egui::Rect> {
			match shape {
				egui::Shape::Text(text) if text.galley.job.text == "Cancel tests" => {
					Some(text.galley.rect.translate(text.pos.to_vec2()))
				}
				egui::Shape::Vec(shapes) => shapes.iter().find_map(label_rect),
				_ => None,
			}
		}
		let ctx = egui::Context::default();
		let mut settings = super::Settings {
			profiles_busy: true,
			profiles_testing: true,
			..Default::default()
		};
		let draw = |settings: &mut super::Settings, events| {
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(900.0, 1100.0),
					)),
					events,
					..Default::default()
				},
				|ui| settings.show(ui, false),
			)
		};
		for _ in 0..3 {
			draw(&mut settings, vec![]).drop_without_applying_deltas();
		}
		let output = draw(&mut settings, vec![]);
		let pos = output
			.shapes
			.iter()
			.find_map(|shape| label_rect(&shape.shape))
			.expect("visible cancellation button")
			.center();
		output.drop_without_applying_deltas();
		for pressed in [true, false] {
			draw(
				&mut settings,
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
			.drop_without_applying_deltas();
		}
		assert!(matches!(
			settings.action,
			Some(model::protect::Action::Profiles(
				model::protect::ProfileAction::CancelTests
			))
		));
	}

	fn saved(id: &str) -> model::protect::SavedProfile {
		model::protect::SavedProfile {
			id: id.into(),
			name: "US Houston".into(),
			protocol: model::protect::Protocol::WireGuard,
			scope: model::protect::Scope::Complete,
			result: String::new(),
			location: String::new(),
		}
	}

	#[test]
	fn refreshing_the_list_keeps_what_was_already_tested_and_located() {
		let ctx = egui::Context::default();
		let mut ui = crate::MessagingUi::default();
		ui.set_protect_profiles(vec![saved("a")]);
		ui.set_protect_profile_result("a", "Reachable · 42 ms");
		ui.set_protect_profile_location(&ctx, "a", "203.0.113.7 · United States (US)", None);
		// Importing another profile makes the host send the whole list again, without results.
		ui.set_protect_profiles(vec![saved("a"), saved("b")]);
		let rows = &ui.settings.protect.profiles;
		assert_eq!(rows[0].result, "Reachable · 42 ms");
		assert_eq!(rows[0].location, "203.0.113.7 · United States (US)");
		assert!(rows[1].result.is_empty() && rows[1].location.is_empty());
		// A removed profile takes its result with it.
		ui.set_protect_profiles(vec![saved("b")]);
		assert_eq!(ui.settings.protect.profiles.len(), 1);
	}

	#[test]
	fn the_test_bar_follows_the_host_progress_and_never_overshoots() {
		let mut ui = crate::MessagingUi::default();
		ui.set_protect_profile_progress(1, 3);
		assert_eq!(
			(
				ui.settings.protect.test_done,
				ui.settings.protect.test_total
			),
			(1, 3)
		);
		ui.set_protect_profile_progress(9, 3);
		assert_eq!(ui.settings.protect.test_done, 3);
	}

	#[test]
	fn clicking_a_saved_profile_loads_then_enables_it_without_asking() {
		let mut ui = crate::MessagingUi::default();
		ui.set_protect_profiles(vec![saved("a")]);
		ui.settings.protect.activate("a");
		assert!(matches!(
			ui.take_protect_action(),
			Some(model::protect::Action::Profiles(
				model::protect::ProfileAction::Load(id)
			)) if id == "a"
		));
		// The host loaded the profile: connecting is the next step, with no dialog between.
		ui.set_protect_draft(model::protect::Draft {
			protocol: model::protect::Protocol::Socks5,
			host: "127.0.0.1".into(),
			..model::protect::Draft::default()
		});
		assert!(matches!(
			ui.take_protect_action(),
			Some(model::protect::Action::Enable(..))
		));
		// Only the host's confirmation makes it the live profile.
		ui.set_protect_operation_status("Closing the previous connection…", false, true);
		assert!(ui.settings.protect.active_profile.is_none());
		ui.mark_protect_active();
		ui.set_protect_operation_status("PROTECT enabled", true, false);
		assert_eq!(ui.settings.protect.active_profile.as_deref(), Some("a"));
	}

	#[test]
	fn a_failed_connect_marks_nothing_live_and_says_why() {
		let mut ui = crate::MessagingUi::default();
		ui.set_protect_profiles(vec![saved("a")]);
		ui.settings.protect.activate("a");
		ui.take_protect_action();
		ui.set_protect_draft(model::protect::Draft {
			protocol: model::protect::Protocol::Socks5,
			host: "127.0.0.1".into(),
			..model::protect::Draft::default()
		});
		ui.take_protect_action();
		ui.set_protect_operation_status("WireGuard handshake timed out", false, false);
		assert!(ui.settings.protect.active_profile.is_none());
		assert!(ui.settings.protect.activating.is_none());
		assert_eq!(ui.settings.protect.status, "WireGuard handshake timed out");
	}

	#[test]
	fn a_profile_that_cannot_be_loaded_reports_the_reason() {
		let mut ui = crate::MessagingUi::default();
		ui.set_protect_profiles(vec![saved("a")]);
		ui.settings.protect.activate("a");
		ui.take_protect_action();
		// The load ended without producing a configuration.
		ui.set_protect_profile_status("Protected profile store is unavailable", false, false);
		assert!(ui.take_protect_action().is_none(), "nothing is enabled");
		assert_eq!(
			ui.settings.protect.status,
			"Protected profile store is unavailable"
		);
	}

	#[test]
	fn a_click_during_tests_stops_them_then_connects() {
		let mut ui = crate::MessagingUi::default();
		ui.set_protect_profiles(vec![saved("a")]);
		ui.set_protect_profile_status("Testing saved profiles…", true, true);
		ui.settings.protect.activate("a");
		assert!(matches!(
			ui.take_protect_action(),
			Some(model::protect::Action::Profiles(
				model::protect::ProfileAction::CancelTests
			))
		));
		ui.set_protect_profile_status("Tests cancelled; active route unchanged", false, false);
		assert!(matches!(
			ui.take_protect_action(),
			Some(model::protect::Action::Profiles(
				model::protect::ProfileAction::Load(id)
			)) if id == "a"
		));
	}

	#[test]
	fn removed_flags_stay_removed_and_tests_stop_collecting_them() {
		let ctx = egui::Context::default();
		let mut ui = crate::MessagingUi::default();
		ui.set_protect_profiles(vec![saved("a")]);
		assert!(
			ui.protect_locate(),
			"country and flags are collected by default"
		);
		ui.settings.protect.flags_removed = true;
		assert!(!ui.protect_locate());
		ui.set_protect_profile_location(&ctx, "a", "1.2.3.4 · Brazil (BR)", None);
		assert!(ui.settings.protect.profiles[0].location.is_empty());
	}

	#[test]
	fn chosen_scope_survives_loading_a_saved_profile() {
		let mut ui = crate::MessagingUi::default();
		ui.settings.protect.draft.scope = model::protect::Scope::Streams;
		ui.settings.protect.activate("fixture");
		ui.take_protect_action();
		ui.set_protect_draft(model::protect::Draft {
			protocol: model::protect::Protocol::Socks5,
			host: "127.0.0.1".into(),
			scope: model::protect::Scope::Complete,
			..model::protect::Draft::default()
		});
		assert!(
			matches!(ui.take_protect_action(), Some(model::protect::Action::Enable(draft, _)) if draft.scope == model::protect::Scope::Streams)
		);
	}
	#[test]
	fn invalid_drafts_do_not_queue_an_enable_operation() {
		let mut settings = super::Settings::default();
		settings.request_enable();
		assert!(settings.action.is_none());
		assert!(!settings.status.is_empty());
		assert!(!settings.busy);
	}
	#[test]
	fn failed_switch_remains_visible_while_old_route_is_active() {
		let mut settings = super::Settings {
			enabled: true,
			active: Some(model::protect::Scope::Videos),
			status: "Synthetic route failure".into(),
			..Default::default()
		};
		let ctx = egui::Context::default();
		fn contains(shape: &egui::Shape) -> bool {
			match shape {
				egui::Shape::Text(text) => text.galley.job.text.contains("Synthetic route failure"),
				egui::Shape::Vec(shapes) => shapes.iter().any(contains),
				_ => false,
			}
		}
		let output = ctx.run_ui(Default::default(), |ui| settings.status_header(ui, false));
		assert!(output.shapes.iter().any(|shape| contains(&shape.shape)));
		output.drop_without_applying_deltas();
	}

	#[test]
	fn importing_a_configuration_selects_wireguard() {
		let mut ui = crate::MessagingUi::default();
		ui.settings.protect.draft.protocol = model::protect::Protocol::Socks5;
		ui.set_protect_wireguard("[Interface]\nPrivateKey = x\n")
			.unwrap();
		assert_eq!(
			ui.settings.protect.draft.protocol,
			model::protect::Protocol::WireGuard
		);
		assert_eq!(
			model::protect::Draft::default().protocol,
			model::protect::Protocol::WireGuard,
			"WireGuard is the default connection type"
		);
	}

	#[test]
	fn transition_status_keeps_controls_busy_until_host_acknowledgement() {
		let mut ui = crate::MessagingUi::default();
		ui.set_protect_operation_status("Closing previous transport", false, true);
		assert!(ui.settings.protect.busy);
		assert!(!ui.settings.protect.enabled);
		ui.set_protect_operation_status("Shutdown uncertain", false, true);
		assert!(
			ui.settings.protect.busy,
			"an intermediate status cannot unlock controls"
		);
		ui.set_protect_operation_status("Route activated after shutdown", true, false);
		assert!(!ui.settings.protect.busy);
		assert!(ui.settings.protect.enabled);
	}
}
