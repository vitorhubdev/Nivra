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
	/// A call is live, pushed by the host each frame. Enabling then asks before reconnecting it.
	call_active: bool,
	call_confirm: bool,
	profiles: Vec<SavedProfile>,
	profile_name: String,
	profile_status: String,
	profiles_busy: bool,
	profiles_testing: bool,
	reveal: bool,
	locate: bool,
	flags: std::collections::BTreeMap<String, egui::TextureHandle>,
}
impl Settings {
	fn saved_profiles(&mut self, ui: &mut egui::Ui, demo: bool) {
		ui.group(|ui| {
			ui.strong(crate::tr_ui!(ui, "Saved profiles"));
			ui.label(crate::tr_ui!(
				ui,
				"Public copies: application data / nivra / protect. Secrets stay in the OS credential store."
			));
			ui.add_enabled_ui(!demo && !self.profiles_busy, |ui| {
				ui.checkbox(&mut self.locate, crate::tr_ui!(ui, "Include exit IP, approximate country and flag (ipwho.is / FlagCDN)"));
			});
			ui.label(crate::tr_ui!(ui, "Optional external queries use each tested route. These services receive its exit IP. Disabled by default."));
			ui.add_enabled_ui(!demo && !self.busy && !self.profiles_busy, |ui| {
				ui.horizontal_wrapped(|ui| {
					ui.add(
						egui::TextEdit::singleline(&mut self.profile_name)
							.hint_text(crate::tr_ui!(ui, "Profile name"))
							.char_limit(64)
							.desired_width(180.0),
					);
					if ui
						.add_enabled(
							!self.profile_name.trim().is_empty()
								&& self.draft.missing().is_none()
								&& self.profiles.len() < 16,
							egui::Button::new(crate::tr_ui!(ui, "Save a copy")),
						)
						.clicked()
					{
						self.action = Some(Action::Profiles(ProfileAction::Save {
							name: self.profile_name.clone(),
							draft: self.draft.clone(),
						}));
					}
					if ui
						.add_enabled(
							!self.profiles.is_empty(),
							egui::Button::new(crate::tr_ui!(ui, "Test all")),
						)
						.clicked()
					{
						self.action = Some(Action::Profiles(ProfileAction::TestAll { location: self.locate }));
					}
				});
			});
			if self.profiles_testing && ui.button(crate::tr_ui!(ui, "Cancel tests")).clicked() {
				self.action = Some(Action::Profiles(ProfileAction::CancelTests));
			}
			egui::ScrollArea::vertical()
				.id_salt("protect-saved-profiles")
				.max_height(220.0)
				.show(ui, |ui| {
					for row in &self.profiles {
						ui.push_id(&row.id, |ui| {
							ui.horizontal_wrapped(|ui| {
								ui.strong(&row.name);
								if let Some(flag) = self.flags.get(&row.id) {
									ui.image((flag.id(), egui::vec2(32.0, 24.0)));
								}
								ui.label(row.protocol.label());
								ui.label(crate::tr_ui!(ui, row.scope.label()));
								ui.add_enabled_ui(
									!demo && !self.busy && !self.profiles_busy,
									|ui| {
										if ui.button(crate::tr_ui!(ui, "Load")).clicked() {
											self.action = Some(Action::Profiles(
												ProfileAction::Load(row.id.clone()),
											));
										}
										if ui
											.button(crate::tr_ui!(ui, "Remove saved copy"))
											.clicked()
										{
											self.action = Some(Action::Profiles(
												ProfileAction::Delete(row.id.clone()),
											));
										}
									},
								);
							});
							if !row.result.is_empty() {
								if let Some((result, elapsed)) = row
									.result
									.rsplit_once(" · ")
									.filter(|(_, elapsed)| elapsed.ends_with(" ms"))
								{
									ui.label(format!("{} · {elapsed}", crate::tr_str!(ui, result)));
								} else {
									ui.label(crate::tr_str!(ui, &row.result));
								}
							}
							if !row.location.is_empty() { ui.label(crate::tr_str!(ui, &row.location)); }
							ui.separator();
						});
					}
				});
			if self.profiles.is_empty() {
				ui.label(crate::tr_ui!(
					ui,
					"No saved profiles yet. Import or configure a route, name it and save a copy."
				));
			}
			if self.profiles_busy {
				ui.spinner();
			}
			if !self.profile_status.is_empty() {
				ui.label(crate::tr_str!(ui, &self.profile_status));
			}
		});
	}
	fn request_enable(&mut self) {
		self.action = Some(Action::Enable(
			self.draft.clone(),
			(self.fallback && self.draft.protocol != Protocol::WireGuard)
				.then(|| self.reserve.clone()),
		));
		self.busy = true;
		self.status = "Applying…".into();
	}
	pub(crate) fn show(&mut self, ui: &mut egui::Ui, demo: bool) {
		ui.heading(crate::tr_ui!(ui, "PROTECT"));
		ui.label(crate::tr_ui!(
			ui,
			"Choose a private route, test it, then enable it. Loading or testing a profile keeps the active route unchanged."
		));
		ui.add_space(12.0);
		self.saved_profiles(ui, demo);
		ui.add_space(12.0);
		// A pending restore must not overwrite edits made while the OS store opens.
		// Keep the saved-profile section outside, so batch cancellation stays enabled.
		ui.add_enabled_ui(!self.profiles_busy, |ui| {
		ui.strong(crate::tr_ui!(ui, "Configure route"));
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
		match self.draft.scope {
			Scope::Videos => {
				ui.label(crate::tr_ui!(
					ui,
					"Routes attachment video and file transfers. Text, voice and camera keep their usual route."
				));
			}
			Scope::Voice => {
				ui.label(crate::tr_ui!(
					ui,
					"Voice and camera use the same Discord socket and move together."
				));
			}
			Scope::Streams => {
				ui.label(crate::tr_ui!(
					ui,
					"Routes screen streams and their audio; voice and camera keep their usual route."
				));
			}
			Scope::Complete => {
				ui.label(crate::tr_ui!(ui,"Routes account signaling, messages, downloads and live media. Official login and external apps are separate."));
			}
		}
		ui.add_space(8.0);
		profile(ui, &mut self.draft, "primary", &mut self.reveal);
		if self.draft.protocol == Protocol::WireGuard
			&& ui
				.add_enabled(
					!demo && !self.busy && !self.profiles_busy,
					egui::Button::new(crate::tr_ui!(ui, "Import WireGuard configuration")),
				)
				.clicked()
		{
			self.action = Some(Action::ImportWireGuard);
			self.busy = true;
		}
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
		ui.label(crate::tr_ui!(ui,"A failed protected route never silently switches to a direct connection. Changing live media routes requires reconnecting the call."));
		ui.add_space(12.0);
		let missing = self.draft.missing();
		let reserve_missing = (self.fallback && self.draft.protocol != Protocol::WireGuard)
			.then(|| self.reserve.missing())
			.flatten();
		let blocked = missing.or(reserve_missing);
		// The header describes what the host applied, never the draft being edited.
		match self.active.filter(|_| self.enabled) {
			Some(applied) => {
				ui.label(crate::tr_ui!(ui, "Route active"));
				ui.label(crate::tr_ui!(ui, applied.label()));
				if applied != self.draft.scope {
					ui.label(crate::tr_ui!(ui, "Changes not applied yet"));
				}
			}
			None => {
				ui.label(crate::tr_ui!(
					ui,
					"Route inactive: nothing is routed through PROTECT"
				));
			}
		}
		if let Some(reason) = blocked {
			ui.label(crate::tr_str!(ui, reason));
		}
		if self.call_confirm {
			let choice = crate::dialog::Confirm::new(
				"protect-call-reconnect",
				crate::tr_ctx!(ui.ctx(), "Reconnect the call with PROTECT on?"),
				crate::tr_ctx!(
					ui.ctx(),
					"Nivra will reconnect you to the call automatically once the route is active. Audio drops for a few seconds."
				),
			)
			.confirm_label(crate::tr_ctx!(ui.ctx(), "Enable and reconnect"))
			.cancel_label(crate::tr_ctx!(ui.ctx(), "Cancel"))
			.show(ui.ctx());
			match choice {
				Some(crate::dialog::Choice::Confirmed) => {
					self.call_confirm = false;
					self.request_enable();
				}
				Some(crate::dialog::Choice::Cancelled) => self.call_confirm = false,
				None => {}
			}
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
					if self.call_active {
						self.call_confirm = true;
					} else {
						self.request_enable();
					}
				}
				if ui
					.add_enabled(
						self.enabled,
						egui::Button::new(crate::tr_ui!(ui, "Disable")),
					)
					.clicked()
				{
					self.action = Some(Action::Disable);
					self.busy = true;
					self.status = "Stopping…".into();
				}
			});
		});
		if self.busy {
			ui.spinner();
		}
		if !self.status.is_empty() {
			ui.label(crate::tr_str!(ui, &self.status));
		}
		if demo {
			ui.label(crate::tr_ui!(
				ui,
				"Offline preview: connection tests and route activation are disabled."
			));
		}
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
	pub fn protect_draft(&self) -> Draft {
		self.settings.protect.draft.clone()
	}
	pub fn set_protect_profiles(&mut self, profiles: Vec<SavedProfile>) {
		self.settings.protect.profiles = profiles.into_iter().take(16).collect();
		self.settings.protect.flags.clear();
	}
	pub fn set_protect_profile_location(
		&mut self,
		ctx: &egui::Context,
		id: &str,
		location: &str,
		flag: Option<egui::ColorImage>,
	) {
		let protect = &mut self.settings.protect;
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
		self.settings.protect.draft = draft;
		self.settings.protect.reveal = false;
	}
	pub fn set_protect_profile_status(&mut self, status: &str, busy: bool, testing: bool) {
		self.settings.protect.profile_status = status.chars().take(512).collect();
		self.settings.protect.profiles_busy = busy;
		self.settings.protect.profiles_testing = testing;
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
		self.settings.protect.busy = false;
		Ok(())
	}
	pub fn take_protect_action(&mut self) -> Option<Action> {
		self.settings.protect.action.take()
	}
	/// The host reports the live route's scope every frame; `None` means nothing is routed.
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
	/// The host reports whether a call is live every frame, so Enable can ask before reconnecting it.
	pub fn set_protect_call_active(&mut self, active: bool) {
		self.settings.protect.call_active = active;
	}
	pub fn set_protect_status(&mut self, status: &str, enabled: bool) {
		self.set_protect_operation_status(status, enabled, false);
	}
	pub fn set_protect_operation_status(&mut self, status: &str, enabled: bool, busy: bool) {
		self.settings.protect.status = status.chars().take(512).collect();
		self.settings.protect.enabled = enabled;
		self.settings.protect.busy = busy;
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
