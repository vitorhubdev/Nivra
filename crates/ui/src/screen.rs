//! Screen selection is a local intent; the desktop owns capture and stream credentials.
use client_core::{
	State,
	screen::{Settings, Source, SourceId},
	voice::Phase,
};
use model::Id;

pub enum Request {
	Start(Settings),
	Stop,
}

pub struct ScreenUi {
	pub context: Option<(u64, Id, u64)>,
	pub open: bool,
	pub sources: Vec<Source>,
	pub selected: Option<SourceId>,
	pub refresh_requested: bool,
	pub request: Option<Request>,
	pub busy: bool,
	pub status: &'static str,
	pub capture_status: Option<&'static str>,
	pub supported: bool,
	pub preview: Option<egui::TextureHandle>,
	height: u32,
	fps: u32,
	cursor: bool,
	audio: bool,
	apps_open: bool,
	quality_open: bool,
}
impl Default for ScreenUi {
	fn default() -> Self {
		Self {
			context: None,
			open: false,
			sources: Vec::new(),
			selected: None,
			refresh_requested: false,
			request: None,
			busy: false,
			status: "",
			capture_status: None,
			supported: false,
			preview: None,
			height: if cfg!(target_os = "linux") { 720 } else { 1080 },
			fps: 30,
			cursor: true,
			audio: cfg!(target_os = "macos"),
			apps_open: false,
			quality_open: false,
		}
	}
}
impl ScreenUi {
	pub(crate) fn launch(&mut self, state: &State) {
		let Some(call) = &state.voice.active else {
			return;
		};
		if self.busy {
			self.request = Some(Request::Stop);
			return;
		}
		self.context = Some((state.generation, call.channel, call.request));
		self.open = true;
		self.apps_open = false;
		self.quality_open = false;
		self.selected = None;
		self.sources.clear();
		if state.demo {
			self.sources = vec![
				Source {
					id: SourceId::Display(1),
					name: "Display 1 · Synthetic preview".into(),
				},
				Source {
					id: SourceId::Window(2),
					name: "Project notes · Synthetic window".into(),
				},
			];
			self.selected = Some(SourceId::Display(1));
			self.status = "Offline preview · no screen is captured";
		} else {
			self.refresh_requested = true;
			self.status = "Looking for screens and windows…";
		}
	}
	fn settings(&self) -> Option<Settings> {
		let source = self
			.selected
			.filter(|id| self.sources.iter().any(|s| s.id == *id))?;
		let settings = Settings {
			source,
			width: match self.height {
				480 => 854,
				1080 => 1920,
				_ => 1280,
			},
			height: self.height,
			fps: self.fps,
			cursor: self.cursor,
			audio: self.audio,
		};
		settings.valid().then_some(settings)
	}
	pub(super) fn show(&mut self, ctx: &egui::Context, state: &State, language: model::Language) {
		let current = state
			.voice
			.active
			.as_ref()
			.filter(|c| c.phase != Phase::Failed)
			.map(|c| (state.generation, c.channel, c.request));
		if self.context.is_some() && current != self.context {
			self.preview = None;
			self.open = false;
			self.sources.clear();
			self.selected = None;
			self.request = None;
			self.refresh_requested = false;
		}
		if !self.open {
			return;
		}
		let mut cancel = false;
		let mut share = false;
		let response = crate::dialog::Dialog::new(
			"screen-share-settings",
			crate::i18n::text(language, "Share your screen"),
		)
		.subtitle(crate::i18n::text(
			language,
			"People in this call will see what you pick.",
		))
		.width(460.0)
		.show(ctx, |d| {
			d.scroll(260.0, |ui| self.body(ui, state, language));
			d.footer(|ui| {
				let allowed = !state.demo
					&& self.supported
					&& !self.busy && self.settings().is_some()
					&& state.voice.active.as_ref().is_some_and(|call| {
						matches!(call.phase, Phase::Connected | Phase::Waiting)
							&& state.can_stream(call.channel)
					});
				ui.add_enabled_ui(allowed, |ui| {
					share = crate::dialog::action(
						ui,
						crate::i18n::text(language, "Share Screen"),
						crate::dialog::Action::Primary,
					)
					.clicked();
				});
				cancel |= crate::dialog::action(
					ui,
					crate::i18n::text(language, "Cancel"),
					crate::dialog::Action::Neutral,
				)
				.clicked();
			});
		});
		cancel |= response.close;
		if share && !cancel {
			self.request = self.settings().map(Request::Start);
		}
		if cancel || share {
			self.open = false;
		}
	}

	/// Whole screens first, audio under them, apps folded to the three most recent.
	fn body(&mut self, ui: &mut egui::Ui, state: &State, language: model::Language) {
		let t = |english: &'static str| crate::i18n::text(language, english);
		let colors = crate::design::palette(ui);
		let displays: Vec<_> = self
			.sources
			.iter()
			.filter(|source| is_display(source.id))
			.map(|source| (source.id, source.name.clone()))
			.collect();
		let windows: Vec<_> = self
			.sources
			.iter()
			.filter(|source| matches!(source.id, SourceId::Window(_)))
			.map(|source| (source.id, source.name.clone()))
			.collect();
		if self
			.selected
			.is_none_or(|id| !self.sources.iter().any(|source| source.id == id))
		{
			self.selected = displays.first().map(|(id, _)| *id);
		}
		if displays.is_empty() && windows.is_empty() {
			egui::Frame::new()
				.fill(colors.base)
				.corner_radius(8)
				.inner_margin(egui::Margin::symmetric(12, 14))
				.show(ui, |ui| {
					ui.set_width(ui.available_width());
					ui.add(
						egui::Label::new(
							egui::RichText::new(t("Looking for your screens…"))
								.size(13.0)
								.color(colors.muted),
						)
						.wrap(),
					);
				});
		}
		let heading = |ui: &mut egui::Ui, text: &str| {
			ui.add_space(4.0);
			ui.label(crate::design::eyebrow(ui, text, colors.muted));
			ui.add_space(2.0);
		};
		if !displays.is_empty() {
			heading(ui, t("Screens"));
		}
		for (index, (id, name)) in displays.iter().enumerate() {
			let title = if displays.len() == 1 {
				t("Entire screen").to_owned()
			} else {
				format!("{} {}", t("Entire screen"), index + 1)
			};
			self.source_row(ui, *id, &title, name, true);
		}

		heading(ui, t("Apps"));
		let selected_app = windows
			.iter()
			.find(|(id, _)| self.selected == Some(*id))
			.map(|(_, name)| name.as_str());
		let app_summary = match (selected_app, windows.len()) {
			(Some(name), _) => name.to_owned(),
			(None, 0) => t("None open").to_owned(),
			(None, count) => format!("{} {}", count, t("open")),
		};
		if disclosure(ui, t("Share an app"), &app_summary, self.apps_open).clicked() {
			self.apps_open = !self.apps_open;
		}
		if self.apps_open {
			if windows.is_empty() {
				crate::design::hint(ui, t("No apps are open to share."));
			}
			// Every open window stays reachable; the list scrolls instead of cutting off at three.
			egui::ScrollArea::vertical()
				.id_salt("screen-apps")
				.max_height(240.0)
				.auto_shrink([false, true])
				.show(ui, |ui| {
					for (id, name) in windows.iter() {
						self.source_row(ui, *id, name, t("App"), false);
					}
				});
			ui.add_enabled_ui(!state.demo && !cfg!(target_os = "linux"), |ui| {
				if crate::design::text_action(ui, t("Refresh")).clicked() {
					self.selected = None;
					self.sources.clear();
					self.refresh_requested = true;
					self.status = "Looking for screens and windows…";
				}
			});
		}

		heading(ui, t("Options"));
		let quality = format!("{}p · {} fps", self.height, self.fps);
		if disclosure(ui, t("Quality"), &quality, self.quality_open).clicked() {
			self.quality_open = !self.quality_open;
		}
		if self.quality_open {
			ui.add_space(4.0);
			let choices = |ui: &mut egui::Ui, label: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 6.0;
					ui.add_sized(
						[84.0, 32.0],
						egui::Label::new(egui::RichText::new(label).size(13.0).color(colors.muted)),
					);
					add(ui);
				});
			};
			choices(ui, t("Resolution"), &mut |ui| {
				for height in [480, 720, 1080] {
					if segment(ui, &format!("{height}p"), self.height == height).clicked() {
						self.height = height;
					}
				}
			});
			choices(ui, t("Frame rate"), &mut |ui| {
				for fps in [15, 30, 60] {
					if segment(ui, &format!("{fps} fps"), self.fps == fps).clicked() {
						self.fps = fps;
					}
				}
			});
			crate::design::switch(
				ui,
				t("Show cursor"),
				Some(t("Include the pointer in the shared video.")),
				&mut self.cursor,
			);
		}
		if self.supported {
			crate::design::switch(
				ui,
				t("Share audio"),
				Some(t(
					"Also send sound from other apps. Your microphone stays as it is.",
				)),
				&mut self.audio,
			);
		}
		// The generic prompt is redundant once something is picked; progress and errors stay.
		let redundant = self.status == "Choose a screen or window" && self.selected.is_some();
		if !self.status.is_empty() && !redundant {
			crate::design::hint(ui, t(self.status));
		}
	}

	fn source_row(
		&mut self,
		ui: &mut egui::Ui,
		id: SourceId,
		title: &str,
		detail: &str,
		display: bool,
	) {
		let colors = crate::design::palette(ui);
		let selected = self.selected == Some(id);
		let (rect, response) =
			ui.allocate_exact_size(egui::vec2(ui.available_width(), 52.0), egui::Sense::click());
		let fill = if selected {
			colors.accent.gamma_multiply(0.18)
		} else if response.hovered() || response.has_focus() {
			colors.hover
		} else {
			colors.base
		};
		ui.painter().rect_filled(rect, 8, fill);
		if selected {
			ui.painter().rect_stroke(
				rect,
				8,
				egui::Stroke::new(1.0, colors.accent),
				egui::StrokeKind::Inside,
			);
		}
		crate::icons::paint(
			ui.painter(),
			if display {
				crate::icons::Icon::Television
			} else {
				crate::icons::Icon::ScreenShare
			},
			egui::Rect::from_center_size(
				egui::pos2(rect.left() + 26.0, rect.center().y),
				egui::Vec2::splat(20.0),
			),
			if selected {
				colors.accent
			} else {
				colors.muted
			},
		);
		let text_left = rect.left() + 46.0;
		let text_width = (rect.right() - 36.0 - text_left).max(40.0);
		let name = ui.painter().layout(
			title.to_owned(),
			egui::FontId::new(15.0, crate::design::semibold_family(ui.ctx())),
			colors.text_strong,
			text_width,
		);
		let kind = ui.painter().layout(
			detail.to_owned(),
			egui::FontId::proportional(11.0),
			colors.muted,
			text_width,
		);
		let total = name.size().y + 2.0 + kind.size().y;
		let name_height = name.size().y;
		let mut y = rect.center().y - total * 0.5;
		ui.painter()
			.galley(egui::pos2(text_left, y), name, colors.text_strong);
		y += name_height + 2.0;
		ui.painter()
			.galley(egui::pos2(text_left, y), kind, colors.muted);
		if selected {
			crate::icons::paint(
				ui.painter(),
				crate::icons::Icon::Check,
				egui::Rect::from_center_size(
					egui::pos2(rect.right() - 20.0, rect.center().y),
					egui::Vec2::splat(16.0),
				),
				colors.accent,
			);
		}
		response.widget_info(|| {
			egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, title)
		});
		if response.clicked() {
			self.selected = Some(id);
		}
		ui.add_space(4.0);
	}
}

fn is_display(id: SourceId) -> bool {
	matches!(
		id,
		SourceId::Display(_) | SourceId::X11Desktop | SourceId::Portal
	)
}

/// Card-style expandable row: label on the left, the current choice and a chevron on the
/// right, aligned with the source rows above it.
fn disclosure(ui: &mut egui::Ui, label: &str, summary: &str, open: bool) -> egui::Response {
	let colors = crate::design::palette(ui);
	let (rect, response) =
		ui.allocate_exact_size(egui::vec2(ui.available_width(), 44.0), egui::Sense::click());
	let hot = response.hovered() || response.has_focus();
	ui.painter()
		.rect_filled(rect, 8, if hot { colors.hover } else { colors.base });
	let chevron = egui::Rect::from_center_size(
		egui::pos2(rect.right() - 20.0, rect.center().y),
		egui::Vec2::splat(14.0),
	);
	crate::icons::paint(
		ui.painter(),
		if open {
			crate::icons::Icon::ChevronDown
		} else {
			crate::icons::Icon::ChevronRight
		},
		chevron,
		colors.muted,
	);
	let title = ui.painter().layout_no_wrap(
		label.to_owned(),
		egui::FontId::new(14.0, crate::design::medium_family(ui.ctx())),
		colors.text_strong,
	);
	let title_width = title.size().x;
	ui.painter().galley(
		egui::pos2(rect.left() + 16.0, rect.center().y - title.size().y * 0.5),
		title,
		colors.text_strong,
	);
	let room = (chevron.left() - 10.0 - (rect.left() + 16.0 + title_width + 16.0)).max(0.0);
	let value = ui.painter().layout(
		summary.to_owned(),
		egui::FontId::proportional(12.5),
		colors.muted,
		room,
	);
	if room > 0.0 && value.rows.len() == 1 {
		ui.painter().galley(
			egui::pos2(
				chevron.left() - 10.0 - value.size().x,
				rect.center().y - value.size().y * 0.5,
			),
			value,
			colors.muted,
		);
	}
	response.widget_info(|| {
		egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{label}, {summary}"))
	});
	ui.add_space(4.0);
	response
}

/// Compact segmented choice used by the quality and frame-rate rows.
fn segment(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
	let colors = crate::design::palette(ui);
	let galley = ui.painter().layout_no_wrap(
		label.to_owned(),
		egui::FontId::new(13.0, crate::design::medium_family(ui.ctx())),
		colors.text_strong,
	);
	let (rect, response) = ui.allocate_exact_size(
		egui::vec2(galley.size().x + 24.0, 32.0),
		egui::Sense::click(),
	);
	let fill = if selected {
		colors.accent
	} else if response.hovered() || response.has_focus() {
		colors.hover
	} else {
		colors.base
	};
	ui.painter().rect_filled(rect, 8, fill);
	let color = if selected {
		colors.accent_text
	} else {
		colors.text
	};
	ui.painter().galley(
		egui::pos2(
			rect.center().x - galley.size().x * 0.5,
			rect.center().y - galley.size().y * 0.5,
		),
		galley,
		color,
	);
	response
		.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, label));
	response
}

#[cfg(test)]
mod tests {
	use super::*;

	fn dialog_height(ctx: &egui::Context, picker: &mut ScreenUi, state: &State) -> f32 {
		let mut height = 0.0;
		for _ in 0..4 {
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1280.0, 900.0),
					)),
					..Default::default()
				},
				|ui| {
					picker.show(ui.ctx(), state, model::Language::PortugueseBrazil);
					height = ui.ctx().memory(|memory| {
						memory
							.area_rect(egui::Id::unique("screen-share-settings"))
							.map_or(0.0, |rect| rect.height())
					});
				},
			)
			.drop_without_applying_deltas();
		}
		height
	}

	#[test]
	fn dialog_is_only_as_tall_as_its_choices() {
		let state = test_support::voice_demo_state();
		let mut picker = ScreenUi {
			open: true,
			supported: true,
			sources: vec![
				Source {
					id: SourceId::Display(1),
					name: "27GL650F".into(),
				},
				Source {
					id: SourceId::Window(2),
					name: "Notes".into(),
				},
				Source {
					id: SourceId::Window(3),
					name: "Browser".into(),
				},
				Source {
					id: SourceId::Window(4),
					name: "Music".into(),
				},
			],
			status: "Choose a screen or window",
			..Default::default()
		};
		let ctx = egui::Context::default();
		let height = dialog_height(&ctx, &mut picker, &state);
		assert!(
			height > 200.0 && height < 520.0,
			"dialog is {height} px tall"
		);
		picker.apps_open = true;
		picker.quality_open = true;
		let open = dialog_height(&ctx, &mut picker, &state);
		picker.apps_open = false;
		picker.quality_open = false;
		let closed = dialog_height(&ctx, &mut picker, &state);
		assert!(open > height, "expanding grows the dialog");
		assert!(
			(closed - height).abs() < 1.0,
			"collapsing shrinks the dialog back"
		);
	}

	#[test]
	fn settings_require_a_current_source_and_offer_high_quality_without_entitlements() {
		let mut picker = ScreenUi {
			selected: Some(SourceId::Window(7)),
			..Default::default()
		};
		assert!(picker.settings().is_none());
		picker.sources.push(Source {
			id: SourceId::Window(7),
			name: "Notes".into(),
		});
		picker.height = 1080;
		picker.fps = 60;
		let settings = picker.settings().unwrap();
		assert_eq!(
			(settings.width, settings.height, settings.fps),
			(1920, 1080, 60)
		);
		assert_eq!(settings.bit_rate(), 16_000_000);
		picker.audio = true;
		assert!(picker.settings().unwrap().audio);
		picker.audio = false;
		assert!(!picker.settings().unwrap().audio);
		picker.sources.clear();
		assert!(picker.settings().is_none());
	}
}
