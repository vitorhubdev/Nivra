//! Declarative native settings. Tests, engines and networking belong to the host.
use model::protect::{Action, Draft, Protocol, Scope};
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
}
impl Settings {
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
			"Choose a private route for Nivra. Your profiles and secrets stay in this session."
		));
		ui.add_space(12.0);
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
		profile(ui, &mut self.draft, "primary");
		if self.draft.protocol == Protocol::WireGuard
			&& ui
				.add_enabled(
					!demo && !self.busy,
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
				profile(ui, &mut self.reserve, "reserve");
			}
		}
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
		ui.add_enabled_ui(!demo && !self.busy, |ui| {
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
fn profile(ui: &mut egui::Ui, draft: &mut Draft, id: &str) {
	ui.push_id(id,|ui|{
		egui::ComboBox::from_id_salt("protocol").selected_text(draft.protocol.label()).show_ui(ui,|ui|{for p in Protocol::ALL {ui.selectable_value(&mut draft.protocol,p,p.label());}});
		if draft.protocol==Protocol::Socks4 && !draft.password.is_empty(){draft.password=Draft::default().password;}
		if draft.protocol==Protocol::WireGuard {
			ui.label(crate::tr_ui!(ui, "WireGuard is experimental; real provider compatibility has not been verified."));
			ui.label(crate::tr_ui!(ui,"Runs inside Nivra: no external program, no system VPN and no admin changes. Session only."));
			ui.label(crate::tr_ui!(ui,"WireGuard configuration · one peer · session only"));ui.add(egui::TextEdit::multiline(&mut *draft.wireguard).password(true).char_limit(4096).desired_rows(5));
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
