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
	/// The host looks for an installed engine once, when the WireGuard panel is first shown.
	engine_scanned: bool,
	engine_found: Option<(String, String)>,
	engine_pinned: Option<model::protect::EngineRelease>,
	engine_latest: Option<model::protect::EngineRelease>,
	/// Share of the sing-box download received; `None` when nothing is downloading.
	engine_progress: Option<f32>,
	/// Set by the Cancel button; the host stops the download and clears it.
	engine_cancel: bool,
	/// The host found the daily check due; the panel starts it once when it opens.
	engine_check_due: bool,
}
impl Settings {
	/// The sing-box block of the WireGuard panel: what is installed, and the download or update.
	fn engine_block(&mut self, ui: &mut egui::Ui, demo: bool) {
		let idle = !demo && !self.busy;
		if self.engine_check_due && idle {
			self.engine_check_due = false;
			self.action = Some(Action::CheckEngine);
			self.busy = true;
		}
		if let Some(fraction) = self.engine_progress {
			ui.add(egui::ProgressBar::new(fraction).show_percentage());
			if ui.button(crate::tr_ui!(ui, "Cancel download")).clicked() {
				self.engine_cancel = true;
			}
			return;
		}
		if let Some((path, version)) = self.engine_found.clone() {
			ui.label(format!(
				"{} {version}",
				crate::tr_ui!(ui, "sing-box installed by Nivra:")
			));
			if self.draft.engine != path
				&& ui
					.add_enabled(
						idle,
						egui::Button::new(crate::tr_ui!(ui, "Use the installed sing-box")),
					)
					.clicked()
			{
				self.draft.engine = path;
			}
		} else if self.engine_pinned.is_some() {
			ui.label(crate::tr_ui!(ui, "Nivra has not installed sing-box yet."));
		}
		if let Some(pinned) = self.engine_pinned.clone() {
			if self.engine_found.is_none()
				&& ui
					.add_enabled(
						idle,
						egui::Button::new(format!(
							"{} {}",
							crate::tr_ui!(ui, "Download sing-box"),
							pinned.version
						)),
					)
					.clicked()
			{
				self.action = Some(Action::InstallEngine(pinned));
				self.busy = true;
			}
			if ui
				.add_enabled(
					idle,
					egui::Button::new(crate::tr_ui!(ui, "Check for a newer sing-box")),
				)
				.clicked()
			{
				self.action = Some(Action::CheckEngine);
				self.busy = true;
			}
			if let Some(latest) = self.engine_latest.clone()
				&& ui
					.add_enabled(
						idle,
						egui::Button::new(format!(
							"{} {}",
							crate::tr_ui!(ui, "Update sing-box to"),
							latest.version
						)),
					)
					.clicked()
			{
				self.action = Some(Action::InstallEngine(latest));
				self.busy = true;
			}
			ui.label(crate::tr_ui!(ui, "Downloads come only from SagerNet's official GitHub release and must match its SHA-256. They use your direct connection, as Nivra updates do."));
		}
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
		if self.draft.protocol == Protocol::WireGuard
			&& ui
				.add_enabled(
					!demo && !self.busy,
					egui::Button::new(crate::tr_ui!(ui, "Choose the local sing-box executable")),
				)
				.clicked()
		{
			self.action = Some(Action::ImportEngine);
			self.busy = true;
		}
		if self.draft.protocol == Protocol::WireGuard {
			if !self.engine_scanned && !demo {
				self.engine_scanned = true;
				self.action = Some(Action::FindEngine);
				self.busy = true;
			}
			self.engine_block(ui, demo);
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
		ui.label(crate::tr_ui!(
			ui,
			if self.enabled {
				"Route active: the selected scope is protected"
			} else {
				"Route inactive: nothing is routed through PROTECT"
			}
		));
		if let Some(reason) = blocked {
			ui.label(crate::tr_str!(ui, reason));
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
					self.action = Some(Action::Enable(
						self.draft.clone(),
						(self.fallback && self.draft.protocol != Protocol::WireGuard)
							.then(|| self.reserve.clone()),
					));
					self.busy = true;
					self.status = "Applying…".into();
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
/// Official sing-box releases; pick the build for the system shown next to the link.
const SING_BOX_RELEASES: &str = "https://github.com/SagerNet/sing-box/releases";

fn profile(ui: &mut egui::Ui, draft: &mut Draft, id: &str) {
	ui.push_id(id,|ui|{
		egui::ComboBox::from_id_salt("protocol").selected_text(draft.protocol.label()).show_ui(ui,|ui|{for p in Protocol::ALL {ui.selectable_value(&mut draft.protocol,p,p.label());}});
		if draft.protocol==Protocol::Socks4 && !draft.password.is_empty(){draft.password=Draft::default().password;}
		if draft.protocol==Protocol::WireGuard {
			ui.label(crate::tr_ui!(ui, "WireGuard is experimental; real provider compatibility has not been verified."));
			ui.label(crate::tr_ui!(ui,"Uses sing-box 1.14. Nothing is changed in the system VPN."));
			// Official release page; the engine is never downloaded by Nivra itself.
			let system = if cfg!(target_os = "windows") {
				"Windows"
			} else if cfg!(target_os = "macos") {
				"macOS"
			} else {
				"Linux"
			};
			ui.hyperlink_to(
				format!("{} ({system})", crate::tr_ui!(ui, "Download sing-box for this system")),
				SING_BOX_RELEASES,
			);
			ui.label(crate::tr_ui!(ui,"Local sing-box executable"));ui.add(egui::TextEdit::singleline(&mut draft.engine).hint_text(crate::tr_ui!(ui,"Choose the file or paste its full path")).char_limit(1024));
			ui.label(crate::tr_ui!(ui,"WireGuard configuration · one peer · session only"));ui.add(egui::TextEdit::multiline(&mut *draft.wireguard).password(true).hint_text(crate::tr_ui!(ui,"[Interface] PrivateKey and Address, then one [Peer] with PublicKey, Endpoint and AllowedIPs")).char_limit(4096).desired_rows(5));
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
	/// Stores the sing-box path picked in the native dialog; the text field stays editable.
	/// The host's scan result: an engine already installed here, and the pinned release to offer.
	pub fn set_protect_engine_found(
		&mut self,
		installed: Option<(String, String)>,
		pinned: Option<model::protect::EngineRelease>,
		check_due: bool,
	) {
		self.settings.protect.engine_found = installed;
		self.settings.protect.engine_pinned = pinned;
		self.settings.protect.engine_check_due = check_due;
		self.settings.protect.busy = false;
	}
	/// The host's view of the download in flight, pushed each frame.
	pub fn set_protect_engine_progress(&mut self, progress: Option<f32>) {
		self.settings.protect.engine_progress = progress;
	}
	/// True once when the user pressed Cancel download; reading it clears the request.
	pub fn take_protect_engine_cancel(&mut self) -> bool {
		std::mem::take(&mut self.settings.protect.engine_cancel)
	}
	pub fn set_protect_engine_latest(&mut self, latest: Option<model::protect::EngineRelease>) {
		self.settings.protect.engine_latest = latest;
		self.settings.protect.busy = false;
	}
	/// A release was installed and verified: it is selected and no longer an update.
	pub fn set_protect_engine_installed(
		&mut self,
		path: &str,
		version: &str,
	) -> Result<(), &'static str> {
		self.set_protect_engine(path)?;
		self.settings.protect.engine_found = Some((path.to_owned(), version.to_owned()));
		self.settings.protect.engine_latest = None;
		Ok(())
	}
	pub fn set_protect_engine(&mut self, path: &str) -> Result<(), &'static str> {
		if path.is_empty() || path.len() > 1024 || path.contains('\0') {
			return Err("Choose a valid sing-box executable path");
		}
		self.settings.protect.draft.engine = path.to_owned();
		self.settings.protect.busy = false;
		Ok(())
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
