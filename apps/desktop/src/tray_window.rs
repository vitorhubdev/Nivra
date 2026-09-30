//! Shared window-close routing for all tray adapters. Exit checks remain in the desktop's rendered UI.
use super::egui;

/// What a requested window close did. The caller logs the fallback; only `Proceed`
/// lets the viewport close for real.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseDisposition {
	/// No close was requested this frame.
	None,
	/// The window hid behind the tray icon; the close was cancelled.
	Hidden,
	/// The setting wants the tray but no icon exists: the window minimized to the
	/// taskbar instead of hiding without a way back. The close was cancelled.
	MinimizedWithoutTray,
	/// The close proceeds: explicit opt-out or a real quit.
	Proceed,
}

pub struct State {
	pub hidden: bool,
	#[allow(dead_code)] // Read by desktop tray path; tray-debug includes this file without it.
	pub hide_notice_shown: bool,
	exiting: bool,
	close_after_show: bool,
	/// Compositor IPC for Wayland sessions where winit can neither hide nor minimize (Hyprland).
	compositor: Option<platform::compositor::Hider>,
}

#[allow(dead_code)] // Used by desktop; tray-debug includes this file without calling it.
pub fn should_show_hide_notice(
	tray_available: bool,
	setting_enabled: bool,
	already_shown: bool,
	just_hidden: bool,
) -> bool {
	tray_available && setting_enabled && just_hidden && !already_shown
}

impl Default for State {
	fn default() -> Self {
		Self {
			hidden: false,
			hide_notice_shown: false,
			exiting: false,
			close_after_show: false,
			compositor: platform::compositor::Hider::detect(),
		}
	}
}

impl State {
	/// Restores the window from any thread. Hyprland sends no frames to a parked window, so
	/// events that must show it cannot wait for the UI to run.
	pub fn restorer(&self) -> impl Fn() + Send + Sync + 'static {
		let compositor = self.compositor.clone();
		move || {
			if let Some(compositor) = &compositor {
				compositor.show();
			}
		}
	}
	pub fn show(&mut self, ctx: &egui::Context) {
		self.hidden = false;
		if let Some(compositor) = &self.compositor {
			compositor.show();
		}
		ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
		ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
		ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
		ctx.request_repaint();
	}
	/// Tray "minimize": hides through the compositor where minimization is ignored, otherwise
	/// restores the window first so the minimize request lands on a mapped surface.
	#[allow(dead_code)]
	pub fn minimize(&mut self, ctx: &egui::Context) {
		if let Some(compositor) = &self.compositor {
			self.hidden = true;
			compositor.hide();
			ctx.request_repaint();
			return;
		}
		self.show(ctx);
		ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
	}
	pub fn quit(&mut self, ctx: &egui::Context) {
		self.exiting = true;
		self.close_after_show = true;
		self.show(ctx);
	}
	pub fn cancel_quit(&mut self) {
		self.exiting = false;
		self.close_after_show = false;
	}
	pub fn logic(
		&mut self,
		ctx: &egui::Context,
		want_tray: bool,
		tray_ready: bool,
		can_hide: bool,
	) -> CloseDisposition {
		let was_hidden = self.hidden;
		if self.hidden && !tray_ready {
			self.show(ctx);
		}
		let (close, visible) = ctx.input(|input| {
			(
				input.viewport().close_requested(),
				input.viewport().visible(),
			)
		});
		if !close {
			return CloseDisposition::None;
		}
		if want_tray && !self.exiting {
			if tray_ready {
				// Native Wayland cannot hide surfaces through winit, and Hyprland ignores
				// minimization, so its IPC parks the window instead. Elsewhere never label a
				// possibly visible window hidden; the compositor may ignore minimization.
				if let Some(compositor) = &self.compositor {
					self.hidden = true;
					compositor.hide();
				} else {
					self.hidden = can_hide;
					ctx.send_viewport_cmd(if can_hide {
						egui::ViewportCommand::Visible(false)
					} else {
						egui::ViewportCommand::Minimized(true)
					});
				}
				self.cancel_close(ctx);
				return CloseDisposition::Hidden;
			}
			// No icon could take the window: minimizing keeps the process alive and the
			// window recoverable from the taskbar instead of hiding it without a way
			// back, or exiting from under the user.
			self.hidden = false;
			ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
			self.cancel_close(ctx);
			return CloseDisposition::MinimizedWithoutTray;
		}
		// Registration can finish while the existing exit dialog is open.
		self.exiting = true;
		if was_hidden || visible == Some(false) || self.close_after_show {
			self.close_after_show = true;
			self.show(ctx);
		} else {
			return CloseDisposition::Proceed;
		}
		self.cancel_close(ctx);
		CloseDisposition::Proceed
	}
	fn cancel_close(&self, ctx: &egui::Context) {
		ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
		ctx.input_mut(|input| {
			let viewport = input.raw.viewport_id;
			if let Some(info) = input.raw.viewports.get_mut(&viewport) {
				info.events
					.retain(|event| *event != egui::ViewportEvent::Close);
			}
		});
	}
	pub fn ui(&mut self, ctx: &egui::Context) {
		if std::mem::take(&mut self.close_after_show) {
			ctx.send_viewport_cmd(egui::ViewportCommand::Close);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{CloseDisposition, State, should_show_hide_notice};

	fn close_input() -> super::egui::RawInput {
		let mut close = super::egui::RawInput::default();
		close
			.viewports
			.get_mut(&super::egui::ViewportId::ROOT)
			.unwrap()
			.events
			.push(super::egui::ViewportEvent::Close);
		close
	}

	fn commands(output: &super::egui::LogicOutput) -> &[super::egui::ViewportCommand] {
		&output.viewport_commands[&super::egui::ViewportId::ROOT]
	}

	#[test]
	fn close_with_ready_tray_hides_and_cancels() {
		let ctx = super::egui::Context::default();
		let mut state = State::default();
		let close = close_input();
		let output = ctx.run_logic(&close, |ctx| {
			assert_eq!(state.logic(ctx, true, true, true), CloseDisposition::Hidden);
		});
		assert!(state.hidden);
		let cmds = commands(&output);
		assert!(cmds.contains(&super::egui::ViewportCommand::CancelClose));
		assert!(cmds.contains(&super::egui::ViewportCommand::Visible(false)));
		assert!(!cmds.contains(&super::egui::ViewportCommand::Close));
	}

	#[test]
	fn close_without_tray_icon_minimizes_and_survives() {
		// Setting on but no icon: hiding would strand the window with no way
		// back, and exiting would kill the session; minimize instead.
		let ctx = super::egui::Context::default();
		let mut state = State::default();
		let close = close_input();
		let output = ctx.run_logic(&close, |ctx| {
			assert_eq!(
				state.logic(ctx, true, false, true),
				CloseDisposition::MinimizedWithoutTray
			);
		});
		assert!(!state.hidden, "no icon takes the window, so do not hide");
		let cmds = commands(&output);
		assert!(cmds.contains(&super::egui::ViewportCommand::Minimized(true)));
		assert!(cmds.contains(&super::egui::ViewportCommand::CancelClose));
		assert!(!cmds.contains(&super::egui::ViewportCommand::Visible(false)));
		assert!(!cmds.contains(&super::egui::ViewportCommand::Close));
	}

	#[test]
	fn close_with_explicit_opt_out_proceeds() {
		let ctx = super::egui::Context::default();
		let mut state = State::default();
		let close = close_input();
		let output = ctx.run_logic(&close, |ctx| {
			assert_eq!(
				state.logic(ctx, false, false, true),
				CloseDisposition::Proceed
			);
		});
		assert!(!state.hidden);
		let cmds: &[super::egui::ViewportCommand] = output
			.viewport_commands
			.get(&super::egui::ViewportId::ROOT)
			.map_or(&[], Vec::as_slice);
		assert!(!cmds.contains(&super::egui::ViewportCommand::CancelClose));
		assert!(!cmds.contains(&super::egui::ViewportCommand::Close));
	}

	#[test]
	fn tray_quit_exits_for_real() {
		let ctx = super::egui::Context::default();
		let mut state = State::default();
		let close = close_input();
		let output = ctx.run_logic(&close, |ctx| {
			state.quit(ctx);
			assert_eq!(
				state.logic(ctx, true, true, true),
				CloseDisposition::Proceed
			);
		});
		assert!(!commands(&output).contains(&super::egui::ViewportCommand::Close));
		let mut output = ctx.run_ui(super::egui::RawInput::default(), |ui| state.ui(ui.ctx()));
		output.textures_delta.clear();
		assert!(
			output.viewport_output[&super::egui::ViewportId::ROOT]
				.commands
				.contains(&super::egui::ViewportCommand::Close)
		);
	}

	#[test]
	fn hide_notice_only_on_first_available_hide_with_setting_on() {
		assert!(should_show_hide_notice(true, true, false, true));
		assert!(
			!should_show_hide_notice(true, true, true, true),
			"already shown"
		);
		assert!(
			!should_show_hide_notice(false, true, false, true),
			"tray unavailable"
		);
		assert!(
			!should_show_hide_notice(true, false, false, true),
			"setting off"
		);
		assert!(
			!should_show_hide_notice(true, true, false, false),
			"not hidden"
		);
	}
}
