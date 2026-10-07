//! Transient notices: one outcome, stated once, that then leaves on its own.
//!
//! A status line has no natural end, so a failed action strands its text in the chrome
//! until something unrelated overwrites it. Every toast here carries a deadline instead,
//! which is the whole point of the type: nothing pushed can outlive it. Hovering holds
//! the deadline open so a notice cannot expire out from under someone reading it.
//!
//! Colours, icons and severities come from [`crate::design`], so a toast and an inline
//! [`crate::design::notice`] read as the same message in two places.
use crate::{design, icons};
use egui::{Align2, RichText, Stroke};

/// How long a toast rests on screen once drawn. Problems outlast pleasantries.
fn lifetime(level: design::Level) -> f64 {
	match level {
		design::Level::Info | design::Level::Success => 4.0,
		design::Level::Warning => 6.0,
		design::Level::Error => 7.5,
	}
}
/// Trailing fade, counted inside the lifetime.
const FADE: f64 = crate::anim::FADE_SECS;
/// Older notices are dropped past this. A stack of toasts is just a status line again.
const MAX: usize = 3;
const WIDTH: f32 = 360.0;
const GAP: f32 = 8.0;

struct Toast {
	id: u64,
	level: design::Level,
	text: String,
	/// Deadline, set on the first frame this toast is actually drawn so one pushed while
	/// another view is up does not expire unseen.
	deadline: Option<f64>,
}

#[derive(Default)]
pub struct Toasts {
	items: Vec<Toast>,
	next_id: u64,
}

impl Toasts {
	/// States `text` briefly. Repeating a notice that is still on screen restarts its
	/// deadline rather than stacking a copy, so a repeated failure reads as one event.
	pub fn push(&mut self, level: design::Level, text: impl Into<String>) {
		let text = text.into();
		if let Some(shown) = self
			.items
			.iter_mut()
			.find(|toast| toast.level == level && toast.text == text)
		{
			shown.deadline = None;
			return;
		}
		self.items.push(Toast {
			id: self.next_id,
			level,
			text,
			deadline: None,
		});
		self.next_id += 1;
		if self.items.len() > MAX {
			self.items.remove(0);
		}
	}
	/// Test-only view of the texts currently queued, oldest first.
	#[cfg(test)]
	pub(crate) fn texts(&self) -> Vec<&str> {
		self.items.iter().map(|toast| toast.text.as_str()).collect()
	}
	/// Draws the live toasts stacked under the window chrome. `top` is the inset that
	/// clears whatever the caller draws above them.
	pub fn show(&mut self, ctx: &egui::Context, top: f32) {
		if self.items.is_empty() {
			return;
		}
		let now = ctx.input(|input| input.time);
		let reduced = crate::anim::reduce_motion(ctx);
		let colors = design::palette_for(ctx);
		let shadow = ctx.style_of(ctx.theme()).visuals.window_shadow;
		let mut offset = top;
		for toast in &mut self.items {
			let deadline = *toast.deadline.get_or_insert(now + lifetime(toast.level));
			let remaining = deadline - now;
			if remaining <= 0.0 {
				continue;
			}
			let (tint, icon) = match toast.level {
				design::Level::Info => (colors.accent, icons::Icon::Help),
				design::Level::Success => (colors.positive, icons::Icon::Check),
				design::Level::Warning => (colors.warning, icons::Icon::ShieldWarning),
				design::Level::Error => (colors.danger, icons::Icon::ShieldWarning),
			};
			let mut dismissed = false;
			let enter = crate::anim::popup_alpha(ctx, egui::Id::unique(("toast-in", toast.id)));
			let response = egui::Area::new(egui::Id::unique(("toast", toast.id)))
				.anchor(Align2::CENTER_TOP, egui::vec2(0.0, offset))
				.order(egui::Order::Foreground)
				.show(ctx, |ui| {
					// Reduced motion still removes the toast at its deadline, but without the tail.
					let tail = if reduced {
						1.0
					} else {
						(remaining / FADE).clamp(0.0, 1.0) as f32
					};
					ui.set_opacity(enter * tail);
					egui::Frame::new()
						.fill(colors.raised.to_opaque())
						.stroke(Stroke::new(1.0, tint.gamma_multiply(0.55)))
						.corner_radius(10)
						.shadow(shadow)
						.inner_margin(egui::Margin::symmetric(12, 8))
						.show(ui, |ui| {
							ui.set_width(WIDTH);
							ui.horizontal_top(|ui| {
								ui.spacing_mut().item_spacing.x = 9.0;
								let (rect, _) = ui.allocate_exact_size(
									egui::Vec2::splat(16.0),
									egui::Sense::hover(),
								);
								icons::paint(ui.painter(), icon, rect, tint);
								ui.with_layout(
									egui::Layout::right_to_left(egui::Align::Center),
									|ui| {
										dismissed |=
											icons::button(ui, icons::Icon::Close, 22.0, "Dismiss")
												.clicked();
										ui.with_layout(
											egui::Layout::left_to_right(egui::Align::Center),
											|ui| {
												ui.add(
													egui::Label::new(
														RichText::new(
															crate::i18n::text_str(
																crate::i18n::interface_language(
																	ui.ctx(),
																),
																&toast.text,
															)
															.as_ref(),
														)
														.size(13.0)
														.color(colors.text),
													)
													.wrap(),
												);
											},
										);
									},
								);
							});
						});
				})
				.response;
			offset += response.rect.height() + GAP;
			// Reading is not a race: rest the pointer on a toast and it waits.
			if dismissed {
				toast.deadline = Some(now);
			} else if response.contains_pointer() {
				toast.deadline = Some(now + lifetime(toast.level));
			}
		}
		self.items
			.retain(|toast| toast.deadline.is_none_or(|deadline| deadline > now));
		// A resting toast is a still image, so sleep until the soonest fade begins and
		// only then drive frames; repainting for the whole lifetime burns a CPU core to
		// redraw the same pixels. Hovering pushes its deadline out and reschedules here.
		let lead = if reduced { 0.0 } else { FADE };
		let fade_in = self
			.items
			.iter()
			.filter_map(|toast| toast.deadline)
			.map(|deadline| deadline - lead - now)
			.fold(f64::INFINITY, f64::min);
		if fade_in <= 0.0 {
			ctx.request_repaint();
		} else if fade_in.is_finite() {
			ctx.request_repaint_after(std::time::Duration::from_secs_f64(fade_in));
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn a_known_reason_is_translated_and_an_unknown_one_stays_quiet() {
		for language in [model::Language::PortugueseBrazil, model::Language::Spanish] {
			let _ = crate::i18n::drain_untranslated_keys();
			// A toast paints whatever `text_str` returns for its stored reason.
			assert_eq!(
				crate::i18n::text_str(language, "Sign in before calling"),
				crate::i18n::text(language, "Sign in before calling"),
				"{language:?}"
			);
			assert_ne!(
				crate::i18n::text_str(language, "Sign in before calling"),
				"Sign in before calling",
				"{language:?} still in English"
			);
			// An unknown runtime reason passes through untouched and is not reported.
			let unknown = "A runtime reason nobody translated";
			assert_eq!(crate::i18n::text_str(language, unknown), unknown);
			let missing = crate::i18n::drain_untranslated_keys();
			assert!(missing.is_empty(), "{language:?} missing {missing:?}");
		}
	}

	#[test]
	fn toasts_keep_their_text_when_drawn() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut toasts = Toasts::default();
		toasts.push(design::Level::Error, "Sign in before calling");
		ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(640.0, 480.0),
				)),
				focused: true,
				..Default::default()
			},
			|ui| toasts.show(ui.ctx(), 8.0),
		)
		.drop_without_applying_deltas();
		assert_eq!(toasts.items.len(), 1);
		assert_eq!(toasts.items[0].text, "Sign in before calling");
	}
}
