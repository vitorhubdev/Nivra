//! Inline player: one bounded texture with Discord-style overlay controls. The desktop owns
//! media decoding and playback; this module only draws frames and emits commands.
use model::{Attachment, Id, Message};

const CORNER: u8 = 8;
const MAX_WIDTH: f32 = crate::avatars::media::MEDIA_MAX_WIDTH;
const MAX_HEIGHT: f32 = crate::avatars::media::MEDIA_MAX_HEIGHT;
const BAR_HEIGHT: f32 = 60.0;
/// Short clips autoplay only when both the stated duration and size are small.
const AUTOPLAY_MAX_MILLIS: u32 = 15_000;
const AUTOPLAY_MAX_BYTES: u64 = 25 * 1024 * 1024;

/// Whether a card has enough metadata to autoplay as a short clip: a video
/// attachment (not an embed preview, whose size is unknown) with a stated
/// duration and size inside the caps.
fn short_autoplay_clip(attachment: &Attachment) -> bool {
	attachment.size > 0
		&& attachment.size <= AUTOPLAY_MAX_BYTES
		&& attachment
			.duration_ms
			.is_some_and(|millis| (1..=AUTOPLAY_MAX_MILLIS).contains(&millis))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoState {
	#[default]
	Idle,
	Loading,
	Playing,
	Paused,
	Ended,
	Failed(&'static str),
}
#[derive(Debug)]
pub enum VideoCommand {
	/// Open and play immediately. A short autoplay clip opens muted and loops.
	Play {
		attachment: Attachment,
		muted: bool,
		looping: bool,
	},
	/// Open and show the first frame paused, so the card has a poster before play.
	Preview(Attachment),
	Pause(bool),
	Seek(f64),
	Volume(f32),
	Stop,
}
pub struct VideoUi {
	pub active: Option<(Id, Id, Attachment)>,
	pub state: VideoState,
	pub position: f64,
	pub duration: f64,
	/// Where the user dragged the bar to. The decoder jumps once, on release; this
	/// keeps the bar and the clock showing the target in the meantime.
	pub seek_preview: Option<f64>,
	pub command: Option<VideoCommand>,
	pub seen: bool,
	pub volume: f32,
	/// Playback started by the short-video autoplay and is currently muted; the
	/// first click on the picture unmutes instead of pausing.
	pub muted: bool,
	/// Settings > Chat: play short visible videos automatically.
	pub autoplay_short_videos: bool,
	/// Settings > Chat: autoplay with sound instead of muted.
	pub autoplay_with_sound: bool,
	/// The current playback was started by autoplay, so a newer visible short
	/// video may take it over; a manually opened video stays until it leaves.
	autoplay_owned: bool,
	texture: Option<egui::TextureHandle>,
	/// Staging pixels for the current frame. The renderer drops its reference after the
	/// upload, so the same allocation is refilled each frame instead of reallocating up to
	/// 8 MB per frame (1080p at 60 fps churned ~500 MB/s through the allocator).
	frame: Option<std::sync::Arc<egui::ColorImage>>,
	/// Vertical transparent-to-black ramp behind the overlay controls.
	shade: Option<egui::TextureHandle>,
	/// Keyboard focus rested on an overlay control last frame, so keep the overlay visible.
	controls_focused: bool,
	/// Keep the viewport's previous mode so leaving playback restores the window.
	fullscreen: Option<(egui::Context, bool, egui::Id)>,
	/// Native window transition for the desktop to apply after this UI frame.
	fullscreen_request: Option<bool>,
}
impl Default for VideoUi {
	fn default() -> Self {
		Self {
			active: None,
			state: VideoState::Idle,
			position: 0.0,
			duration: 0.0,
			seek_preview: None,
			command: None,
			seen: false,
			volume: 1.0,
			muted: false,
			autoplay_short_videos: false,
			autoplay_with_sound: false,
			autoplay_owned: false,
			texture: None,
			frame: None,
			shade: None,
			controls_focused: false,
			fullscreen: None,
			fullscreen_request: None,
		}
	}
}
impl VideoUi {
	pub fn stop(&mut self) {
		self.exit_fullscreen();
		self.active = None;
		self.texture = None;
		self.frame = None;
		self.state = VideoState::Idle;
		self.position = 0.0;
		self.duration = 0.0;
		self.seek_preview = None;
		self.seen = false;
		self.muted = false;
		self.autoplay_owned = false;
		self.command = Some(VideoCommand::Stop);
	}
	fn exit_fullscreen(&mut self) {
		if let Some((ctx, previous, focus)) = self.fullscreen.take() {
			self.fullscreen_request = Some(previous);
			ctx.memory_mut(|memory| memory.request_focus(focus));
			ctx.request_repaint();
		}
	}
	pub fn take_fullscreen_request(&mut self) -> Option<bool> {
		self.fullscreen_request.take()
	}
	pub(super) fn is_fullscreen(&self) -> bool {
		self.fullscreen.is_some()
	}
	pub(super) fn show_fullscreen(
		&mut self,
		ctx: &egui::Context,
		message: &Message,
		attachment: &Attachment,
		download: &mut crate::DownloadUi,
		opening: &mut Option<String>,
		demo: bool,
	) {
		// A modal sizing pass is invisible; it must not stop the active decoder.
		self.seen = true;
		let screen = ctx.content_rect();
		let id = egui::Id::unique("video-fullscreen");
		let overlay = egui::Modal::new(id)
			.area(
				egui::Modal::default_area(id)
					.anchor(egui::Align2::LEFT_TOP, egui::Vec2::ZERO)
					.fade_in(false),
			)
			.backdrop_color(egui::Color32::BLACK)
			.frame(egui::Frame::NONE)
			.show(ctx, |ui| {
				ui.set_min_size(screen.size());
				ui.set_max_size(screen.size());
				let response =
					self.show_player(ui, message, attachment, true, download, opening, demo);
				crate::attachments::media_context_menu(
					&response, attachment, download, opening, demo,
				);
			});
		// The shared link confirmation is drawn before the timeline. Leave the video
		// overlay when opening an original so that confirmation remains visible.
		if overlay.should_close() || opening.is_some() {
			self.exit_fullscreen();
		}
	}
	/// The desktop rejects stale session/player frames before handing over decoded pixels.
	/// Pixels arrive already converted on the media worker; the render thread only
	/// swaps the staged buffer and uploads the texture.
	pub fn accept_frame(&mut self, ctx: &egui::Context, frame: egui::ColorImage) -> bool {
		let [width, height] = frame.size;
		if self.active.is_none()
			|| width == 0
			|| height == 0
			|| width > 1920
			|| height > 1920
			|| width * height > 1920 * 1080
			|| frame.pixels.len() != width * height
		{
			return false;
		}
		let image = self
			.frame
			.get_or_insert_with(|| std::sync::Arc::new(frame.clone()));
		// Reuses the buffer once the previous upload released it; clones only if the
		// renderer still holds the last frame.
		*std::sync::Arc::make_mut(image) = frame;
		let image = std::sync::Arc::clone(image);
		if let Some(texture) = &mut self.texture {
			texture.set(image, egui::TextureOptions::LINEAR);
		} else {
			self.texture =
				Some(ctx.load_texture("inline-video", image, egui::TextureOptions::LINEAR));
		}
		true
	}
	fn toggle(&mut self, message: &Message, attachment: &Attachment, state: VideoState) {
		// A short clip autoplayed muted; the first click gives it sound instead of
		// pausing, so one click is never lost on a silent picture.
		if self.muted && matches!(state, VideoState::Playing | VideoState::Paused) {
			self.set_muted(false);
			return;
		}
		self.command = Some(match state {
			VideoState::Loading => {
				self.stop();
				VideoCommand::Stop
			}
			VideoState::Playing => VideoCommand::Pause(true),
			VideoState::Paused => VideoCommand::Pause(false),
			VideoState::Ended => {
				self.begin(message, attachment, true);
				return;
			}
			_ => {
				self.begin(message, attachment, true);
				return;
			}
		});
	}
	/// Mutes or unmutes the running player, keeping `volume` as the user's level.
	pub fn set_muted(&mut self, muted: bool) {
		self.muted = muted;
		self.command = Some(VideoCommand::Volume(if muted { 0.0 } else { self.volume }));
	}
	/// Opens a short clip the autoplay picked: plays at once, muted (unless the
	/// sound preference is on) and loops while visible.
	pub fn begin_autoplay(&mut self, message: &Message, attachment: &Attachment, sound: bool) {
		self.begin_with(message, attachment, true, !sound, true);
		self.autoplay_owned = true;
	}
	/// Opens the inline player: `autoplay` starts the clock, the default shows the
	/// first frame as a poster and waits for the play button.
	pub fn begin(&mut self, message: &Message, attachment: &Attachment, autoplay: bool) {
		self.begin_with(message, attachment, autoplay, false, false);
		self.autoplay_owned = false;
	}
	fn begin_with(
		&mut self,
		message: &Message,
		attachment: &Attachment,
		autoplay: bool,
		muted: bool,
		looping: bool,
	) {
		self.active = Some((message.channel, message.id, attachment.clone()));
		self.texture = None;
		self.frame = None;
		self.state = VideoState::Loading;
		self.position = 0.0;
		self.duration = 0.0;
		self.seek_preview = None;
		self.seen = true;
		self.muted = muted;
		self.command = Some(if autoplay {
			VideoCommand::Play {
				attachment: attachment.clone(),
				muted,
				looping,
			}
		} else {
			VideoCommand::Preview(attachment.clone())
		});
	}
	fn shade(&mut self, ctx: &egui::Context) -> egui::TextureId {
		self.shade
			.get_or_insert_with(|| {
				let pixels = (0..16)
					.map(|row| egui::Color32::from_black_alpha((row * 200 / 15) as u8))
					.collect();
				ctx.load_texture(
					"inline-video-shade",
					egui::ColorImage {
						size: [1, 16],
						source_size: egui::vec2(1.0, 16.0),
						pixels,
					},
					egui::TextureOptions::LINEAR,
				)
			})
			.id()
	}
	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		message: &Message,
		attachment: &Attachment,
		download: &mut crate::DownloadUi,
		opening: &mut Option<String>,
		demo: bool,
	) -> egui::Response {
		self.show_player(ui, message, attachment, false, download, opening, demo)
	}
	#[allow(clippy::too_many_arguments)]
	fn show_player(
		&mut self,
		ui: &mut egui::Ui,
		message: &Message,
		attachment: &Attachment,
		fullscreen: bool,
		download: &mut crate::DownloadUi,
		opening: &mut Option<String>,
		demo: bool,
	) -> egui::Response {
		let colors = crate::design::palette(ui);
		let mut active = self.active.as_ref().is_some_and(|(channel, id, file)| {
			*channel == message.channel && *id == message.id && file == attachment
		});
		let mut state = if active { self.state } else { VideoState::Idle };
		let width = ui.available_width().clamp(1.0, MAX_WIDTH);
		let size = if fullscreen {
			ui.available_size().max(egui::Vec2::splat(1.0))
		} else {
			stage_size(attachment, width)
		};
		let (stage, mut response) = ui.allocate_exact_size(size, egui::Sense::hover());
		if active && self.is_fullscreen() && !fullscreen {
			return response;
		}
		// The card left the viewport: stop and free its texture so a scrolled-away
		// clip cannot keep a decoder and its frame buffer alive.
		if active && !fullscreen && !ui.is_rect_visible(stage) {
			self.stop();
			active = false;
			state = VideoState::Idle;
		}
		// Autoplay owns the session: losing window focus (Alt-Tab without minimizing)
		// stops the decoder and its audio instead of looping behind another app.
		if active && self.autoplay_owned && !fullscreen && !ui.input(|input| input.focused) {
			self.stop();
			active = false;
			state = VideoState::Idle;
		}
		// Short clips play on their own while visible, muted and in a loop. Reduce
		// motion, a file over the caps or a manually opened player keep the poster
		// and the play button.
		if !active
			&& !fullscreen
			&& self.autoplay_short_videos
			&& !crate::anim::reduce_motion(ui.ctx())
			&& ui.input(|input| input.focused)
			&& !ui.input(|input| input.viewport().minimized.unwrap_or(false))
			&& ui.is_rect_visible(stage)
			&& short_autoplay_clip(attachment)
		{
			// One player only: a newer visible clip takes over an older autoplay
			// one, but never a clip the user opened by hand.
			let may_take_over = self
				.active
				.as_ref()
				.is_none_or(|(_, id, _)| self.autoplay_owned && *id < message.id);
			if may_take_over {
				self.begin_autoplay(message, attachment, self.autoplay_with_sound);
				active = true;
				state = self.state;
			}
		}
		let label = match state {
			VideoState::Loading => crate::tr_ui!(ui, "Cancel"),
			VideoState::Playing => crate::tr_ui!(ui, "Pause"),
			VideoState::Paused => crate::tr_ui!(ui, "Resume"),
			VideoState::Ended => crate::tr_ui!(ui, "Replay"),
			VideoState::Failed(_) => crate::tr_ui!(ui, "Retry"),
			VideoState::Idle => crate::tr_ui!(ui, "Play"),
		};
		let painter = ui.painter().with_clip_rect(stage);
		painter.rect_filled(stage, CORNER, egui::Color32::BLACK);
		if let Some(texture) = self.texture.as_ref().filter(|_| active) {
			let size = texture.size_vec2();
			let scale = (stage.width() / size.x).min(stage.height() / size.y);
			let image_rect = egui::Rect::from_center_size(stage.center(), size * scale);
			egui::Image::new(egui::load::SizedTexture::new(
				texture.id(),
				image_rect.size(),
			))
			.corner_radius(CORNER)
			.paint_at(ui, image_rect);
		}
		let hovered = response.hovered() || ui.rect_contains_pointer(stage);
		let show_controls = active
			&& state != VideoState::Idle
			&& (hovered || state != VideoState::Playing || self.controls_focused);
		// The controls are painted over the stage. Keep playback interaction out of both
		// overlay bands so a control click cannot also become a play/pause click.
		let action_top = (stage.top() + 44.0).min(stage.bottom());
		let action_bottom =
			(stage.bottom() - if show_controls { BAR_HEIGHT } else { 0.0 }).max(action_top);
		let action = ui.interact(
			egui::Rect::from_min_max(
				egui::pos2(stage.left(), action_top),
				egui::pos2(stage.right(), action_bottom),
			),
			response.id.with("playback"),
			egui::Sense::click(),
		);
		action.widget_info(|| {
			egui::WidgetInfo::labeled(
				egui::Role::Button,
				ui.is_enabled(),
				format!("{} {}", label, attachment.filename),
			)
		});
		let center = if show_controls {
			stage.center() - egui::vec2(0.0, BAR_HEIGHT * 0.25)
		} else {
			stage.center()
		};
		let white = egui::Color32::WHITE;
		let show_center = match state {
			VideoState::Loading => {
				ui.put(
					egui::Rect::from_center_size(center, egui::Vec2::splat(36.0)),
					egui::Spinner::new().size(36.0).color(white),
				);
				false
			}
			VideoState::Playing => false,
			_ => true,
		};
		if show_center {
			let radius = 28.0;
			painter.circle_filled(
				center,
				radius,
				if hovered {
					egui::Color32::from_rgba_unmultiplied(0, 0, 0, 200)
				} else {
					egui::Color32::from_rgba_unmultiplied(0, 0, 0, 160)
				},
			);
			painter.circle_stroke(
				center,
				radius,
				egui::Stroke::new(1.5, egui::Color32::from_white_alpha(60)),
			);
			if matches!(state, VideoState::Ended | VideoState::Failed(_)) {
				crate::icons::paint(
					&painter,
					crate::icons::Icon::Reload,
					egui::Rect::from_center_size(center, egui::Vec2::splat(26.0)),
					white,
				);
			} else {
				painter.add(egui::Shape::convex_polygon(
					vec![
						center + egui::vec2(-8.0, -12.0),
						center + egui::vec2(13.0, 0.0),
						center + egui::vec2(-8.0, 12.0),
					],
					white,
					egui::Stroke::NONE,
				));
			}
		}
		if let VideoState::Failed(error) = state {
			// A real reason, translated, with the two ways out: try again, or keep
			// the file. A synthetic embed attachment has nothing to download.
			let reason = crate::tr_ui!(ui, error);
			let text_rect = egui::Rect::from_min_max(
				egui::pos2(stage.left() + 12.0, center.y + 38.0),
				egui::pos2(stage.right() - 12.0, stage.bottom()),
			);
			ui.scope_builder(
				egui::UiBuilder::new()
					.max_rect(text_rect)
					.layout(egui::Layout::top_down(egui::Align::Center)),
				|ui| {
					ui.add(
						egui::Label::new(
							egui::RichText::new(reason)
								.size(12.0)
								.color(egui::Color32::from_white_alpha(230)),
						)
						.wrap(),
					)
					.on_hover_text(reason);
					ui.horizontal(|ui| {
						if ui.small_button(crate::tr_ui!(ui, "Retry")).clicked() {
							self.toggle(message, attachment, state);
						}
						if attachment.size > 0
							&& ui
								.small_button(crate::tr_ui!(ui, "Download video"))
								.on_hover_text(crate::tr_ui!(
									ui,
									"Save this video to your computer"
								))
								.clicked()
						{
							download.request = Some(attachment.clone());
						}
						// A failed decode still has the original URL: open it in the browser
						// so a codec this build cannot play is never a dead end.
						if let Some(url) = attachment
							.media
							.url
							.as_deref()
							.and_then(crate::markdown::external_url)
							&& ui
								.small_button(crate::tr_ui!(ui, "Open original…"))
								.clicked()
						{
							*opening = Some(url);
						}
					});
				},
			);
		}
		if (show_controls && state != VideoState::Playing) || (!active && hovered) {
			let font = egui::FontId::proportional(12.0);
			let galley = painter.layout(
				attachment.filename.clone(),
				font,
				white,
				(stage.width() - 32.0).max(16.0),
			);
			let pill = egui::Rect::from_min_size(
				stage.left_top() + egui::vec2(8.0, 8.0),
				galley.size() + egui::vec2(12.0, 6.0),
			);
			painter.rect_filled(pill, 4, egui::Color32::from_black_alpha(150));
			painter.galley(pill.min + egui::vec2(6.0, 3.0), galley, white);
		}
		if state == VideoState::Idle
			&& let Some(duration) = attachment.duration_ms.filter(|ms| *ms > 0)
		{
			let galley = painter.layout_no_wrap(
				timestamp(duration as f64 / 1000.0),
				egui::FontId::proportional(12.0),
				white,
			);
			let badge = egui::Rect::from_min_size(
				egui::pos2(
					stage.left() + 8.0,
					stage.bottom() - 8.0 - galley.size().y - 6.0,
				),
				galley.size() + egui::vec2(12.0, 6.0),
			);
			painter.rect_filled(badge, 4, egui::Color32::from_black_alpha(150));
			painter.galley(badge.min + egui::vec2(6.0, 3.0), galley, white);
		}
		if action.clicked() {
			self.toggle(message, attachment, state);
		}
		response = action | response;
		let mut controls_focused = false;
		let context_click = ui.input(|i| {
			i.pointer.button_down(egui::PointerButton::Secondary)
				|| i.pointer.button_released(egui::PointerButton::Secondary)
		});
		if show_controls {
			let bar = egui::Rect::from_min_max(
				egui::pos2(stage.left(), stage.bottom() - BAR_HEIGHT),
				stage.right_bottom(),
			);
			let shade = self.shade(ui.ctx());
			egui::Image::new(egui::load::SizedTexture::new(shade, bar.size()))
				.corner_radius(egui::CornerRadius {
					nw: 0,
					ne: 0,
					sw: CORNER,
					se: CORNER,
				})
				.paint_at(ui, bar);
			let duration = if self.duration.is_finite() && self.duration > 0.0 {
				self.duration
			} else {
				1.0
			};
			let can_seek = self.duration.is_finite()
				&& self.duration > 0.0
				&& matches!(state, VideoState::Playing | VideoState::Paused);
			let inner = bar.shrink2(egui::vec2(10.0, 6.0));
			ui.scope_builder(
				egui::UiBuilder::new()
					.max_rect(inner)
					.layout(egui::Layout::top_down(egui::Align::Min)),
				|ui| {
					let visuals = ui.visuals_mut();
					visuals.selection.bg_fill = colors.accent;
					visuals.widgets.inactive.bg_fill = egui::Color32::from_white_alpha(70);
					visuals.widgets.hovered.bg_fill = egui::Color32::from_white_alpha(110);
					visuals.widgets.active.bg_fill = egui::Color32::from_white_alpha(140);
					visuals.widgets.inactive.fg_stroke.color = white;
					visuals.widgets.hovered.fg_stroke.color = white;
					visuals.widgets.active.fg_stroke.color = white;
					visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
					visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
					visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
					ui.spacing_mut().item_spacing = egui::vec2(8.0, 2.0);
					ui.spacing_mut().slider_rail_height = 4.0;
					ui.spacing_mut().interact_size.y = 18.0;
					ui.spacing_mut().slider_width = ui.available_width().max(16.0);
					let mut position = self.seek_preview.unwrap_or(self.position).max(0.0);
					let seek = ui.add_enabled(
						can_seek,
						egui::Slider::new(&mut position, 0.0..=duration)
							.show_value(false)
							.trailing_fill(true),
					);
					let seek_label = crate::tr_ui!(ui, "Seek video");
					seek.widget_info(|| egui::WidgetInfo::slider(can_seek, position, seek_label));
					controls_focused |= seek.has_focus();
					response |= seek.clone();
					if can_seek && !context_click && (seek.changed() || seek.drag_stopped()) {
						// Dragging only moves the bar; the decoder jumps once, on release,
						// so a long drag cannot queue dozens of native seeks.
						if seek.is_pointer_button_down_on() {
							self.seek_preview = Some(position);
						} else {
							self.seek_preview = None;
							self.command = Some(VideoCommand::Seek(position));
						}
					}
					ui.horizontal(|ui| {
						ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);
						let (glyph_rect, glyph) =
							ui.allocate_exact_size(egui::Vec2::splat(22.0), egui::Sense::CLICK);
						let glyph_color = if glyph.hovered() {
							white
						} else {
							egui::Color32::from_white_alpha(220)
						};
						let c = glyph_rect.center();
						match state {
							VideoState::Playing => {
								for x in [-3.5, 3.5] {
									ui.painter().rect_filled(
										egui::Rect::from_center_size(
											c + egui::vec2(x, 0.0),
											egui::vec2(3.5, 13.0),
										),
										1,
										glyph_color,
									);
								}
							}
							VideoState::Paused | VideoState::Idle => {
								ui.painter().add(egui::Shape::convex_polygon(
									vec![
										c + egui::vec2(-5.0, -7.0),
										c + egui::vec2(7.0, 0.0),
										c + egui::vec2(-5.0, 7.0),
									],
									glyph_color,
									egui::Stroke::NONE,
								));
							}
							VideoState::Loading => crate::icons::paint(
								ui.painter(),
								crate::icons::Icon::Close,
								glyph_rect.shrink(4.0),
								glyph_color,
							),
							VideoState::Ended | VideoState::Failed(_) => crate::icons::paint(
								ui.painter(),
								crate::icons::Icon::Reload,
								glyph_rect.shrink(3.0),
								glyph_color,
							),
						}
						response |= glyph.clone();
						if glyph.on_hover_text(label).clicked() {
							self.toggle(message, attachment, state);
						}
						ui.label(
							egui::RichText::new(format!(
								"{} / {}",
								timestamp(self.seek_preview.unwrap_or(self.position)),
								if self.duration > 0.0 {
									timestamp(self.duration)
								} else {
									"--:--".into()
								}
							))
							.size(12.0)
							.color(white),
						);
						ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
							let (rect, button) = ui
								.allocate_exact_size(egui::Vec2::splat(22.0), egui::Sense::click());
							let label = if fullscreen {
								crate::tr_ui!(ui, "Exit fullscreen (Esc)")
							} else {
								crate::tr_ui!(ui, "Fullscreen")
							};
							button.widget_info(|| {
								egui::WidgetInfo::labeled(
									egui::Role::Button,
									ui.is_enabled(),
									label,
								)
							});
							if fullscreen {
								crate::icons::paint(
									ui.painter(),
									crate::icons::Icon::Close,
									rect.shrink(3.0),
									white,
								);
							} else {
								for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
									let corner = rect.center() + egui::vec2(x * 7.0, y * 7.0);
									ui.painter().add(egui::Shape::line(
										vec![
											corner - egui::vec2(x * 5.0, 0.0),
											corner,
											corner - egui::vec2(0.0, y * 5.0),
										],
										egui::Stroke::new(1.5, white),
									));
								}
							}
							controls_focused |= button.has_focus();
							if button.has_focus() {
								ui.painter().rect_stroke(
									rect,
									3,
									egui::Stroke::new(2.0, colors.accent),
									egui::StrokeKind::Inside,
								);
							}
							response |= button.clone();
							let button_id = button.id;
							if button
								.on_hover_text(label)
								.on_hover_cursor(egui::CursorIcon::PointingHand)
								.clicked()
							{
								if fullscreen {
									self.exit_fullscreen();
								} else {
									let previous =
										ui.input(|i| i.viewport().fullscreen.unwrap_or(false));
									self.fullscreen = Some((ui.ctx().clone(), previous, button_id));
									self.fullscreen_request = Some(true);
								}
							}
							ui.spacing_mut().slider_width =
								(ui.available_width() - 24.0).clamp(24.0, 56.0);
							let mut volume_value = self.volume;
							let volume_label = crate::tr_ui!(ui, "Video volume");
							let volume = ui.add(
								egui::Slider::new(&mut volume_value, 0.0..=1.0)
									.show_value(false)
									.trailing_fill(true),
							);
							volume.widget_info(|| {
								egui::WidgetInfo::slider(
									ui.is_enabled(),
									volume_value as f64,
									volume_label,
								)
							});
							controls_focused |= volume.has_focus();
							response |= volume.clone();
							if volume.on_hover_text(volume_label).changed() && !context_click {
								// Moving the volume is a request to hear: leave mute.
								self.muted = false;
								self.volume = volume_value;
								self.command = Some(VideoCommand::Volume(self.volume));
							}
							let mute_label = if self.muted {
								crate::tr_ui!(ui, "Unmute video")
							} else {
								crate::tr_ui!(ui, "Mute video")
							};
							let (speaker_rect, speaker) = ui
								.allocate_exact_size(egui::Vec2::splat(22.0), egui::Sense::click());
							speaker.widget_info(|| {
								egui::WidgetInfo::labeled(
									egui::Role::Button,
									ui.is_enabled(),
									mute_label,
								)
							});
							let speaker_color = if speaker.hovered() {
								white
							} else {
								egui::Color32::from_white_alpha(220)
							};
							controls_focused |= speaker.has_focus();
							if speaker.has_focus() {
								ui.painter().rect_stroke(
									speaker_rect,
									3,
									egui::Stroke::new(2.0, colors.accent),
									egui::StrokeKind::Inside,
								);
							}
							let glyph = speaker_rect.shrink(4.0);
							crate::icons::paint(
								ui.painter(),
								crate::icons::Icon::Speaker,
								glyph,
								speaker_color,
							);
							if self.muted {
								// A slash over the same glyph is the visible muted state.
								ui.painter().line_segment(
									[
										egui::pos2(glyph.left() + 1.0, glyph.bottom() - 1.0),
										egui::pos2(glyph.right() - 1.0, glyph.top() + 1.0),
									],
									egui::Stroke::new(2.0, speaker_color),
								);
							}
							response |= speaker.clone();
							if speaker
								.on_hover_text(mute_label)
								.on_hover_cursor(egui::CursorIcon::PointingHand)
								.clicked()
							{
								self.set_muted(!self.muted);
							}
						});
					});
				},
			);
		}
		self.controls_focused = controls_focused;
		// Download/open controls stay visible regardless of load state, unlike the bottom bar.
		// Added last so Tab order still reaches the playback controls first.
		let overlay = egui::Rect::from_min_size(
			egui::pos2(stage.left() + 8.0, stage.top() + 8.0),
			egui::vec2((stage.width() - 16.0).max(0.0), 28.0),
		);
		ui.scope_builder(
			egui::UiBuilder::new()
				.max_rect(overlay)
				.layout(egui::Layout::right_to_left(egui::Align::Min)),
			|ui| {
				ui.spacing_mut().item_spacing.x = 6.0;
				let idle = !demo && !download.busy();
				let disabled_hover = if demo {
					crate::tr_ui!(ui, "Downloads are disabled for synthetic attachments")
				} else {
					crate::tr_ui!(ui, "A download is already active")
				};
				if ui
					.add_enabled_ui(idle, |ui| {
						crate::attachments::glass_button(
							ui,
							crate::icons::Icon::Download,
							28.0,
							crate::tr_ui!(ui, "Download"),
						)
					})
					.inner
					.on_disabled_hover_text(disabled_hover)
					.clicked()
				{
					download.request = Some(attachment.clone());
				}
				if let Some(url) = attachment
					.media
					.url
					.as_deref()
					.and_then(crate::markdown::external_url)
					&& crate::attachments::glass_button(
						ui,
						crate::icons::Icon::External,
						28.0,
						crate::tr_ui!(ui, "Open original…"),
					)
					.clicked()
				{
					*opening = Some(url);
				}
			},
		);
		if response.has_focus() {
			ui.painter().rect_stroke(
				stage,
				CORNER,
				egui::Stroke::new(2.0, colors.accent),
				egui::StrokeKind::Inside,
			);
		}
		if ui.is_rect_visible(stage)
			&& self.active.as_ref().is_some_and(|(channel, id, file)| {
				*channel == message.channel && *id == message.id && file == attachment
			}) {
			self.seen = true;
		}
		response
	}
}
impl Drop for VideoUi {
	fn drop(&mut self) {
		self.exit_fullscreen();
	}
}
fn stage_size(attachment: &Attachment, width: f32) -> egui::Vec2 {
	let width = width.clamp(1.0, MAX_WIDTH);
	let (native_width, native_height) = if attachment.media.width > 0 && attachment.media.height > 0
	{
		(
			attachment.media.width as f32,
			attachment.media.height as f32,
		)
	} else {
		(16.0, 9.0)
	};
	let scale = (width / native_width)
		.min(MAX_HEIGHT / native_height)
		.min(1.0);
	(egui::vec2(native_width, native_height) * scale).max(egui::vec2(1.0, 1.0))
}
/// Stage plus the spacing after each attachment; all controls are overlays or menu actions.
pub(super) fn estimated_height(attachment: &Attachment, width: f32) -> f32 {
	stage_size(attachment, width.clamp(1.0, MAX_WIDTH)).y + 6.0
}
fn timestamp(seconds: f64) -> String {
	let seconds = seconds.max(0.0) as u64;
	format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn video_controls_are_explicit_bounded_and_keyboard_operable() {
		let mut message = test_support::message(1, Id(2));
		let attachment = Attachment {
			id: Id(3),
			filename: "synthetic-clip.MOV".into(),
			description: None,
			content_type: Some("application/octet-stream".into()),
			size: 128,
			media: model::EmbedMedia {
				width: 1920,
				height: 1080,
				..Default::default()
			},
			spoiler: false,
			duration_ms: None,
			waveform: Vec::new(),
		};
		message.attachments.push(attachment.clone());
		for (width, theme) in [(220.0, egui::Theme::Dark), (420.0, egui::Theme::Light)] {
			let ctx = egui::Context::default();
			crate::design::apply(&ctx);
			ctx.set_theme(theme);
			let mut video = VideoUi::default();
			let frame = |video: &mut VideoUi, key: Option<egui::Key>| {
				ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width + 16.0, 600.0),
						)),
						events: key
							.into_iter()
							.map(|key| egui::Event::Key {
								key,
								physical_key: None,
								pressed: true,
								repeat: false,
								modifiers: egui::Modifiers::NONE,
							})
							.collect(),
						..Default::default()
					},
					|ui| {
						ui.set_width(width);
						crate::attachments::show(
							ui,
							&message,
							&mut crate::avatars::Avatars::default(),
							&mut None,
							&mut None,
							&mut crate::attachments::DownloadUi::default(),
							&mut crate::AudioUi::default(),
							video,
							false,
							&mut crate::select::Surface::new(ui, "attachment-test"),
						);
						assert!(ui.min_rect().width() <= width + 2.0);
						// Only the stage and attachment spacing drive the layout estimate;
						// overlay controls and menu actions never add height.
						assert!(
							(ui.min_rect().height() - estimated_height(&attachment, width)).abs()
								< 24.0,
							"{} vs {}",
							ui.min_rect().height(),
							estimated_height(&attachment, width)
						);
					},
				)
				.drop_without_applying_deltas();
			};
			let frame_of = |width: usize, height: usize, pixels: usize| egui::ColorImage {
				size: [width, height],
				source_size: egui::vec2(width as f32, height as f32),
				pixels: vec![egui::Color32::BLACK; pixels],
			};
			frame(&mut video, None);
			assert!(video.command.is_none() && video.active.is_none());
			assert!(!video.accept_frame(&ctx, frame_of(1, 1, 1)));
			for key in [egui::Key::Tab, egui::Key::Enter] {
				frame(&mut video, Some(key));
			}
			assert!(matches!(
				video.command.take(),
				Some(VideoCommand::Play { attachment: file, muted: false, looping: false })
					if file == attachment
			));
			assert!(video.seen);
			assert!(!video.accept_frame(&ctx, frame_of(1921, 1080, 0)));
			assert!(!video.accept_frame(&ctx, frame_of(1, 1921, 0)));
			assert!(!video.accept_frame(&ctx, frame_of(1920, 1920, 0)));
			assert!(!video.accept_frame(&ctx, frame_of(usize::MAX, usize::MAX, 0)));
			assert!(!video.accept_frame(&ctx, frame_of(1, 1, 0)));
			assert!(video.accept_frame(&ctx, frame_of(1, 1920, 1920)));
			assert!(video.accept_frame(&ctx, frame_of(1, 1, 1)));
			video.state = VideoState::Playing;
			video.duration = 12.0;
			frame(&mut video, Some(egui::Key::Enter));
			assert!(matches!(
				video.command.take(),
				Some(VideoCommand::Pause(true))
			));
			video.state = VideoState::Paused;
			frame(&mut video, Some(egui::Key::Enter));
			assert!(matches!(
				video.command.take(),
				Some(VideoCommand::Pause(false))
			));
			for key in [egui::Key::Tab, egui::Key::ArrowRight] {
				frame(&mut video, Some(key));
			}
			assert!(matches!(video.command.take(), Some(VideoCommand::Seek(value)) if value > 0.0));
			// While a control keeps keyboard focus, playback resuming must not hide the overlay.
			video.state = VideoState::Playing;
			frame(&mut video, Some(egui::Key::ArrowRight));
			assert!(matches!(video.command.take(), Some(VideoCommand::Seek(_))));
			for key in [egui::Key::Tab, egui::Key::Tab, egui::Key::ArrowLeft] {
				frame(&mut video, Some(key));
			}
			assert!(
				matches!(video.command.take(), Some(VideoCommand::Volume(value)) if value < 1.0)
			);
			video.state = VideoState::Failed("Unsupported video codec");
			frame(&mut video, None);
			video.stop();
			assert!(video.active.is_none() && video.texture.is_none());
			assert!(matches!(video.command, Some(VideoCommand::Stop)));
		}
	}

	#[test]
	fn dragging_the_bar_previews_and_jumps_once_on_release() {
		let mut message = test_support::message(1, Id(2));
		let attachment = Attachment {
			id: Id(3),
			filename: "clip.mp4".into(),
			description: None,
			content_type: Some("video/mp4".into()),
			size: 4096,
			media: model::EmbedMedia {
				width: 640,
				height: 360,
				..Default::default()
			},
			spoiler: false,
			duration_ms: None,
			waveform: Vec::new(),
		};
		message.attachments.push(attachment.clone());
		let mut video = VideoUi::default();
		video.begin(&message, &attachment, true);
		video.state = VideoState::Playing;
		video.duration = 100.0;
		video.position = 10.0;
		let width = 420.0;
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let frame = |video: &mut VideoUi, events: Vec<egui::Event>| {
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(width + 16.0, 600.0),
					)),
					focused: true,
					events,
					..Default::default()
				},
				|ui| {
					ui.set_width(width);
					video.show(
						ui,
						&message,
						&attachment,
						&mut crate::attachments::DownloadUi::default(),
						&mut None,
						false,
					);
				},
			)
			.drop_without_applying_deltas();
		};
		let y = stage_size(&attachment, width).y - BAR_HEIGHT + 15.0;
		let at = |x: f32| egui::pos2(x, y);
		let press = |pos: egui::Pos2, pressed: bool| {
			vec![egui::Event::PointerButton {
				pos,
				button: egui::PointerButton::Primary,
				pressed,
				modifiers: egui::Modifiers::NONE,
			}]
		};
		frame(&mut video, vec![egui::Event::PointerMoved(at(40.0))]);
		video.command = None;
		frame(&mut video, press(at(40.0), true));
		assert!(video.command.is_none(), "pressing the bar must not seek");
		let mut previewed = 0.;
		for x in [120.0, 200.0, 280.0] {
			frame(&mut video, vec![egui::Event::PointerMoved(at(x))]);
			previewed = video.seek_preview.expect("drag previews the target");
			assert!(video.command.is_none(), "dragging must not seek at {x}");
			assert!(previewed > 10.0, "{previewed}");
		}
		frame(&mut video, press(at(280.0), false));
		assert!(
			matches!(video.command.take(), Some(VideoCommand::Seek(target)) if (target - previewed).abs() < 1.0),
			"releasing seeks once to the dragged target"
		);
		assert!(video.seek_preview.is_none());
		for x in [60.0, 300.0, 140.0] {
			frame(&mut video, press(at(x), true));
			frame(&mut video, vec![egui::Event::PointerMoved(at(x + 40.0))]);
			assert!(video.seek_preview.is_some(), "preview at {x}");
			assert!(video.command.is_none(), "no seek while dragging at {x}");
			frame(&mut video, press(at(x + 40.0), false));
			assert!(
				matches!(video.command.take(), Some(VideoCommand::Seek(_))),
				"one jump per drag at {x}"
			);
		}
	}

	#[test]
	fn an_idle_card_plays_on_the_first_click() {
		let mut message = test_support::message(1, Id(2));
		let attachment = Attachment {
			id: Id(3),
			filename: "oobe-intro1.mp4".into(),
			description: None,
			content_type: Some("video/mp4".into()),
			size: 4096,
			media: model::EmbedMedia {
				width: 1080,
				height: 1920,
				..Default::default()
			},
			spoiler: false,
			duration_ms: Some(4000),
			waveform: Vec::new(),
		};
		message.attachments.push(attachment.clone());
		let mut video = VideoUi::default();
		// One click on the idle card starts playback instead of opening a paused
		// poster that needs a second click on play.
		video.toggle(&message, &attachment, VideoState::Idle);
		assert!(
			matches!(
				video.command,
				Some(VideoCommand::Play {
					muted: false,
					looping: false,
					..
				})
			),
			"idle plays on the first activation"
		);
		assert_eq!(video.state, VideoState::Loading);
		assert!(video.active.is_some());
		// A running clip still pauses on click.
		video.state = VideoState::Playing;
		video.toggle(&message, &attachment, VideoState::Playing);
		assert!(matches!(video.command, Some(VideoCommand::Pause(true))));
		// A paused clip resumes, and the end replays from the start (autoplay).
		video.state = VideoState::Paused;
		video.toggle(&message, &attachment, VideoState::Paused);
		assert!(matches!(video.command, Some(VideoCommand::Pause(false))));
		video.state = VideoState::Ended;
		video.toggle(&message, &attachment, VideoState::Ended);
		assert!(matches!(
			video.command,
			Some(VideoCommand::Play {
				muted: false,
				looping: false,
				..
			})
		));
	}

	#[test]
	fn clicking_a_muted_autoplay_clip_sounds_it_before_pausing() {
		let mut message = test_support::message(7, Id(2));
		let attachment = Attachment {
			id: Id(8),
			filename: "clip.mp4".into(),
			description: None,
			content_type: Some("video/mp4".into()),
			size: 4096,
			media: model::EmbedMedia {
				width: 640,
				height: 360,
				..Default::default()
			},
			spoiler: false,
			duration_ms: Some(4000),
			waveform: Vec::new(),
		};
		message.attachments.push(attachment.clone());
		let mut video = VideoUi::default();
		video.volume = 0.8;
		video.begin_autoplay(&message, &attachment, false);
		assert!(video.muted);
		assert!(matches!(
			video.command,
			Some(VideoCommand::Play {
				muted: true,
				looping: true,
				..
			})
		));
		video.command = None;
		video.state = VideoState::Playing;
		// The first click gives the silent picture sound; a second one pauses.
		video.toggle(&message, &attachment, VideoState::Playing);
		assert!(!video.muted);
		assert!(matches!(video.command, Some(VideoCommand::Volume(v)) if v == 0.8));
		video.command = None;
		video.toggle(&message, &attachment, VideoState::Playing);
		assert!(matches!(video.command, Some(VideoCommand::Pause(true))));
	}

	fn short_clip_message(message_id: u64, duration_ms: Option<u32>, size: u64) -> Message {
		let mut message = test_support::message(message_id, Id(2));
		message.attachments.push(Attachment {
			id: Id(message_id + 100),
			filename: format!("clip-{message_id}.mp4"),
			description: None,
			content_type: Some("video/mp4".into()),
			size,
			media: model::EmbedMedia {
				width: 640,
				height: 360,
				..Default::default()
			},
			spoiler: false,
			duration_ms,
			waveform: Vec::new(),
		});
		message
	}

	/// Draws one card (or two, in order) the way the timeline does, and returns the
	/// last autoplay command the frame produced.
	fn draw_cards(
		ctx: &egui::Context,
		video: &mut VideoUi,
		messages: &[Message],
		clip: Option<egui::Rect>,
	) -> Option<VideoCommand> {
		draw_cards_focused(ctx, video, messages, clip, true)
	}
	fn draw_cards_focused(
		ctx: &egui::Context,
		video: &mut VideoUi,
		messages: &[Message],
		clip: Option<egui::Rect>,
		focused: bool,
	) -> Option<VideoCommand> {
		let mut command = None;
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(436.0, 620.0),
				)),
				focused,
				..Default::default()
			},
			|ui| {
				if let Some(clip) = clip {
					ui.set_clip_rect(clip);
				}
				ui.set_width(420.0);
				for (index, message) in messages.iter().enumerate() {
					crate::attachments::show(
						ui,
						message,
						&mut crate::avatars::Avatars::default(),
						&mut None,
						&mut None,
						&mut crate::attachments::DownloadUi::default(),
						&mut crate::AudioUi::default(),
						video,
						false,
						&mut crate::select::Surface::new(ui, format!("autoplay-{index}")),
					);
				}
				command = video.command.take();
			},
		);
		output.drop_without_applying_deltas();
		command
	}

	#[test]
	fn a_visible_short_video_autoplays_muted_and_looping() {
		let message = short_clip_message(1, Some(4000), 1_000_000);
		let attachment = message.attachments[0].clone();
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut video = VideoUi::default();
		video.autoplay_short_videos = true;
		video.volume = 0.7;
		let command = draw_cards(&ctx, &mut video, std::slice::from_ref(&message), None);
		assert!(
			matches!(
				command,
				Some(VideoCommand::Play { muted: true, looping: true, attachment: ref file })
					if *file == attachment
			),
			"the visible 4 s clip starts on its own, silent and looping: {command:?}"
		);
		assert!(video.muted);
		assert_eq!(video.state, VideoState::Loading);
		assert!(
			video
				.active
				.as_ref()
				.is_some_and(|(_, id, file)| *id == message.id && *file == attachment)
		);
	}

	#[test]
	fn a_long_video_and_reduce_motion_wait_for_the_play_button() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		// Over the 15 s cap: no autoplay, but one click plays.
		let long = short_clip_message(2, Some(40_000), 1_000_000);
		let long_attachment = long.attachments[0].clone();
		let mut video = VideoUi::default();
		video.autoplay_short_videos = true;
		assert!(draw_cards(&ctx, &mut video, std::slice::from_ref(&long), None).is_none());
		assert!(video.active.is_none());
		video.toggle(&long, &long_attachment, VideoState::Idle);
		assert!(matches!(
			video.command,
			Some(VideoCommand::Play {
				muted: false,
				looping: false,
				..
			})
		));
		// Over the 25 MB cap: same, the poster stays until clicked.
		let big = short_clip_message(3, Some(4000), 26 * 1024 * 1024);
		let mut video = VideoUi::default();
		video.autoplay_short_videos = true;
		assert!(draw_cards(&ctx, &mut video, std::slice::from_ref(&big), None).is_none());
		assert!(video.active.is_none());
		// Reduce motion turns the policy off entirely.
		crate::anim::set_reduce_motion(&ctx, true);
		let short = short_clip_message(4, Some(4000), 1_000_000);
		let mut video = VideoUi::default();
		video.autoplay_short_videos = true;
		assert!(draw_cards(&ctx, &mut video, std::slice::from_ref(&short), None).is_none());
		assert!(video.active.is_none());
	}

	#[test]
	fn losing_window_focus_stops_an_autoplay_owned_clip() {
		let message = short_clip_message(8, Some(4000), 1_000_000);
		let attachment = message.attachments[0].clone();
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut video = VideoUi::default();
		video.autoplay_short_videos = true;
		let command = draw_cards(&ctx, &mut video, std::slice::from_ref(&message), None);
		assert!(matches!(command, Some(VideoCommand::Play { .. })));
		assert!(video.autoplay_owned);
		// Alt-Tab leaves the card visible but unfocused: autoplay hands the decoder back.
		let command = draw_cards_focused(
			&ctx,
			&mut video,
			std::slice::from_ref(&message),
			None,
			false,
		);
		assert!(matches!(command, Some(VideoCommand::Stop)), "{command:?}");
		assert!(video.active.is_none());
		// A manually opened clip is not autoplay-owned and keeps playing unfocused.
		video.begin(&message, &attachment, true);
		video.command = None;
		let command = draw_cards_focused(
			&ctx,
			&mut video,
			std::slice::from_ref(&message),
			None,
			false,
		);
		assert!(
			command.is_none(),
			"a manual session must not stop on focus loss"
		);
		assert!(video.active.is_some());
	}

	#[test]
	fn an_offscreen_autoplay_clip_stops_and_frees_its_texture() {
		let message = short_clip_message(5, Some(4000), 1_000_000);
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut video = VideoUi::default();
		video.autoplay_short_videos = true;
		draw_cards(&ctx, &mut video, std::slice::from_ref(&message), None);
		assert!(video.active.is_some());
		video.texture = Some(ctx.load_texture(
			"autoplay-texture",
			egui::ColorImage::filled([2, 2], egui::Color32::BLACK),
			egui::TextureOptions::LINEAR,
		));
		video.state = VideoState::Playing;
		video.command = None;
		// The card is out of the clip: the next frame stops it and drops the frame.
		let clip = egui::Rect::from_min_size(egui::pos2(4000.0, 4000.0), egui::vec2(10.0, 10.0));
		let command = draw_cards(&ctx, &mut video, std::slice::from_ref(&message), Some(clip));
		assert!(matches!(command, Some(VideoCommand::Stop)));
		assert!(video.active.is_none());
		assert!(video.texture.is_none());
		assert_eq!(video.state, VideoState::Idle);
	}

	#[test]
	fn only_the_newest_visible_short_video_plays() {
		let older = short_clip_message(6, Some(4000), 1_000_000);
		let newer = short_clip_message(7, Some(4000), 1_000_000);
		let newest = newer.attachments[0].clone();
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut video = VideoUi::default();
		video.autoplay_short_videos = true;
		let command = draw_cards(&ctx, &mut video, &[older, newer], None);
		assert!(
			matches!(
				command,
				Some(VideoCommand::Play { muted: true, looping: true, attachment: ref file })
					if *file == newest
			),
			"the newest visible clip owns the single player: {command:?}"
		);
		assert!(
			video
				.active
				.as_ref()
				.is_some_and(|(_, id, file)| *id == Id(7) && *file == newest)
		);
	}

	#[test]
	fn a_failure_shows_the_reason_with_retry_and_download() {
		let mut message = test_support::message(1, Id(2));
		let attachment = Attachment {
			id: Id(3),
			filename: "clip.mp4".into(),
			description: None,
			content_type: Some("video/mp4".into()),
			size: 4096,
			media: model::EmbedMedia {
				width: 640,
				height: 360,
				url: Some(
					"https://cdn.discordapp.com/attachments/1/2/clip.mp4?ex=68dc&is=68db&hm=abc"
						.into(),
				),
				..Default::default()
			},
			spoiler: false,
			duration_ms: None,
			waveform: Vec::new(),
		};
		message.attachments.push(attachment.clone());
		let mut video = VideoUi::default();
		video.begin(&message, &attachment, true);
		video.state =
			VideoState::Failed("This video format or codec is not supported on this system.");
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut download = crate::attachments::DownloadUi::default();
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(436.0, 600.0),
				)),
				focused: true,
				..Default::default()
			},
			|ui| {
				video.show(ui, &message, &attachment, &mut download, &mut None, false);
			},
		);
		let shown = output
			.shapes
			.iter()
			.filter_map(|shape| match &shape.shape {
				egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
				_ => None,
			})
			.collect::<Vec<_>>()
			.join(" | ");
		output.drop_without_applying_deltas();
		assert!(shown.contains("codec"), "{shown}");
		assert!(shown.contains("Retry"), "{shown}");
		assert!(shown.contains("Download video"), "{shown}");
		// A failure is never a dead end: the original URL is offered too.
		assert!(shown.contains("Open original"), "{shown}");
		assert!(download.request.is_none(), "no click, no request");
	}
}
