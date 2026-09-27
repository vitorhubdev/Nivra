//! Chat text selection. The block is the selection target, not the glyph.

use egui::{
	Color32, CursorIcon, Event, FullOutput, Id, InteractOptions, LayerId, Order, PointerButton,
	Popup, PopupAnchor, Pos2, RawInput, Rect, Response, Sense, Stroke,
	epaint::{Galley, TextShape},
	text_selection::LabelSelectionState,
};
use std::sync::Arc;

/// Inline artwork positioned inside a run's galley (custom and Unicode emoji).
pub struct Artwork {
	pub rect: Rect,
	pub image: Option<egui::Image<'static>>,
}

struct Run {
	band: egui::Id,
	galley_pos: Pos2,
	galley: Arc<Galley>,
	rect: Rect,
	/// One band per wrapped galley row, so a run never covers a neighbour's line.
	lines: Vec<Rect>,
	painted: bool,
}

struct Hole {
	rect: Rect,
	clickable: bool,
}

/// A widget that brings its own galley (fenced code), selected in body order.
struct Embed {
	/// How many runs preceded it, so `finish` replays it between them.
	after: usize,
	response: Response,
	galley_pos: Pos2,
	galley: Arc<Galley>,
}

struct Overlay {
	id: egui::Id,
	rect: Rect,
}

/// One text block's runs, in layout order.
pub struct Surface {
	base: egui::Id,
	runs: Vec<Run>,
	embeds: Vec<Embed>,
	holes: Vec<Hole>,
	overlays: Vec<Overlay>,
	cover: Option<Rect>,
}

impl Surface {
	/// `salt` distinguishes several blocks under one `Ui` id (body, forwarded preview, …).
	pub fn new(ui: &egui::Ui, salt: impl egui::AsIdSalt) -> Self {
		Self {
			base: ui.scope_id().with(salt),
			runs: Vec::new(),
			embeds: Vec::new(),
			holes: Vec::new(),
			overlays: Vec::new(),
			cover: None,
		}
	}

	/// Stretch the tiled bands to `rect` so a drag can start on empty chat chrome.
	pub fn cover(&mut self, rect: Rect) {
		if rect.is_positive() {
			self.cover = Some(self.cover.map_or(rect, |cover| cover.union(rect)));
		}
	}

	pub fn keep(&mut self, response: &Response) {
		if response.rect.is_positive() {
			self.holes.push(Hole {
				rect: response.rect,
				clickable: response.enabled() && response.sense.senses_click(),
			});
		}
	}

	pub fn exclude(&mut self, rect: Rect) {
		if rect.is_positive() {
			self.holes.push(Hole {
				rect,
				clickable: false,
			});
		}
	}

	pub fn through(&mut self, response: &Response) {
		if response.rect.is_positive() {
			self.overlays.push(Overlay {
				id: response.id,
				rect: response.rect,
			});
		}
	}

	/// Record a run, paint it like a label, and claim a later band slot.
	pub fn run(
		&mut self,
		ui: &mut egui::Ui,
		response: &Response,
		galley_pos: Pos2,
		galley: Arc<Galley>,
		artwork: Vec<Artwork>,
	) {
		let band = self.base.with(self.runs.len());
		let painted =
			!artwork.is_empty() && galley.rows.iter().all(|row| row.visuals.mesh.is_empty());
		if painted {
			ui.painter().add(TextShape::new(
				galley_pos,
				galley.clone(),
				Color32::TRANSPARENT,
			));
		}
		for art in &artwork {
			paint_artwork(ui, art);
		}
		self.runs.push(Run {
			band,
			galley_pos,
			lines: line_bands(&galley, galley_pos, response.rect),
			galley,
			rect: response.rect,
			painted,
		});
	}

	/// Record a widget that laid out its own galley (fenced code), so that `finish` registers
	/// its selection between the runs around it.
	///
	/// egui pairs the two ends of a selection by the order labels are registered in, and
	/// treats every label registered in between as fully selected. A code block registers
	/// where it is drawn, in the middle of the body, while the surrounding runs only register
	/// in `finish`: selecting into a block that way puts the whole message — and every earlier
	/// run — "between" the two ends. Deferring the block to the same pass keeps both in
	/// reading order.
	pub fn embed(&mut self, response: &Response, galley_pos: Pos2, galley: Arc<Galley>) {
		self.embeds.push(Embed {
			after: self.runs.len(),
			response: response.clone(),
			galley_pos,
			galley,
		});
	}

	/// Tile the block and register selection on the remaining bands.
	pub fn finish(self, ui: &mut egui::Ui) {
		let block = block_rect(ui, &self.runs, self.cover);
		let embeds = self.embeds;
		let mut embedded = 0;
		let mut runs = self.runs;
		if runs.is_empty() && block.is_positive() {
			runs.push(blank_run(ui, self.base, block));
		}
		let covered = self.cover.is_some();
		tile(&mut runs, block, covered);
		let pointer = ui.input(|input| input.pointer.hover_pos());
		let over_click = pointer.is_some_and(|pos| {
			self.holes
				.iter()
				.any(|hole| hole.clickable && hole.rect.contains(pos))
				|| self.overlays.iter().any(|over| over.rect.contains(pos))
		});
		let over_reserved =
			pointer.is_some_and(|pos| self.holes.iter().any(|hole| hole.rect.contains(pos)));
		let menu_open = Popup::is_any_open(ui.ctx());
		let holes: Vec<Rect> = self.holes.iter().map(|hole| hole.rect).collect();
		for (position, run) in runs.into_iter().enumerate() {
			while embeds
				.get(embedded)
				.is_some_and(|embed| embed.after <= position)
			{
				show_embed(ui, &embeds[embedded], menu_open);
				embedded += 1;
			}
			if !run.rect.is_positive() || !ui.is_rect_visible(run.rect) {
				continue;
			}
			if menu_open {
				if !run.galley.job.text.is_empty() {
					let color = if run.painted {
						Color32::TRANSPARENT
					} else {
						ui.visuals().text_color()
					};
					ui.painter()
						.add(TextShape::new(run.galley_pos, run.galley, color));
				}
				continue;
			}
			let mut response: Option<Response> = None;
			let mut index = 0;
			for line in &run.lines {
				for piece in punch(*line, &holes) {
					let id = if index == 0 {
						run.band
					} else {
						run.band.with(index)
					};
					index += 1;
					let piece = ui.interact(piece, id, band_sense());
					response = Some(match response.take() {
						Some(prev) => prev.union(piece),
						None => piece,
					});
				}
			}
			let Some(response) = response else {
				continue;
			};
			if run.galley.job.text.is_empty() {
				continue;
			}
			let color = if run.painted {
				Color32::TRANSPARENT
			} else {
				ui.visuals().text_color()
			};
			egui::text_selection::LabelSelectionState::label_text_selection(
				ui,
				&response,
				run.galley_pos,
				run.galley,
				color,
				Stroke::NONE,
			);
		}
		for embed in &embeds[embedded..] {
			show_embed(ui, embed, menu_open);
		}
		for over in self.overlays {
			ui.interact_opt(
				over.rect,
				over.id,
				Sense::click(),
				InteractOptions { move_to_top: true },
			);
		}
		if over_click {
			ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
		} else if pointer.is_some_and(|pos| block.contains(pos)) && !over_reserved {
			ui.ctx().set_cursor_icon(CursorIcon::Default);
		}
	}
}

#[derive(Default)]
struct Pointer {
	menu: bool,
	silent: bool,
	cached: String,
	/// Pending save-selection-as-txt request from the selection popup.
	save_txt: Option<Vec<u8>>,
}

impl egui::Plugin for Pointer {
	fn debug_name(&self) -> &'static str {
		"Chat selection pointer"
	}

	fn input_hook(&mut self, ctx: &egui::Context, input: &mut RawInput) {
		let selecting = ctx.plugin::<LabelSelectionState>().lock().has_selection();
		let secondary = input.events.iter().any(|event| {
			matches!(
				event,
				Event::PointerButton {
					button: PointerButton::Secondary,
					pressed: true,
					..
				}
			)
		});
		if selecting && secondary {
			input.events.retain(|event| {
				!matches!(
					event,
					Event::PointerButton {
						button: PointerButton::Secondary,
						..
					}
				)
			});
			if !input
				.events
				.iter()
				.any(|event| matches!(event, Event::Copy))
			{
				input.events.push(Event::Copy);
				self.silent = true;
			}
			self.menu = true;
		} else {
			self.menu = false;
		}
	}

	fn on_end_pass(&mut self, ui: &mut egui::Ui) {
		if !self.menu || Popup::is_any_open(ui.ctx()) {
			return;
		}
		let id = Id::unique("chat-selection-copy");
		Popup::new(
			id,
			ui.ctx().clone(),
			PopupAnchor::PointerFixed,
			LayerId::new(Order::Foreground, id),
		)
		.open_memory(Some(egui::SetOpenCommand::Bool(true)))
		.kind(egui::PopupKind::Menu)
		.show(|ui| {
			if ui.button("Copy").clicked() {
				request_copy(ui.ctx());
				ui.close();
			}
			if ui.button("Save .txt").clicked() {
				let text = selected_text(ui.ctx());
				if !text.is_empty() {
					self.save_txt = Some(text.into_bytes());
				}
				ui.close();
			}
		});
	}

	fn output_hook(&mut self, ctx: &egui::Context, output: &mut FullOutput) {
		if let Some(text) = output
			.platform_output
			.commands
			.iter()
			.find_map(|command| match command {
				egui::OutputCommand::CopyText(text) => Some(text.clone()),
				_ => None,
			}) {
			self.cached = text;
			if self.silent {
				output
					.platform_output
					.commands
					.retain(|command| !matches!(command, egui::OutputCommand::CopyText(_)));
			}
		}
		self.silent = false;
		if output.platform_output.cursor_icon == CursorIcon::Text && !hovering_edit(ctx) {
			output.platform_output.cursor_icon = CursorIcon::Default;
		}
	}
}

fn hovering_edit(ctx: &egui::Context) -> bool {
	let hovered = ctx.interaction_snapshot(|snapshot| snapshot.hovered.clone());
	hovered
		.iter()
		.any(|id| egui::text_edit::TextEditState::load(ctx, *id).is_some())
}

pub fn install(ctx: &egui::Context) {
	ctx.add_plugin(Pointer::default());
}

/// True when a label range is active.
pub fn has_selection(ctx: &egui::Context) -> bool {
	ctx.plugin::<LabelSelectionState>().lock().has_selection()
}

pub fn open_menu(ctx: &egui::Context) -> bool {
	ctx.plugin_opt::<Pointer>()
		.is_some_and(|plugin| plugin.lock().menu)
}

/// Copy the text cached from the last selected range.
pub fn request_copy(ctx: &egui::Context) {
	let text = ctx
		.plugin_opt::<Pointer>()
		.map(|plugin| plugin.lock().cached.clone())
		.unwrap_or_default();
	if !text.is_empty() {
		ctx.copy_text(text);
	}
}

pub(crate) fn band_sense() -> Sense {
	Sense::CLICK | Sense::DRAG
}

/// Edge zone around the scroll viewport that pulls a text-selection drag.
pub const SELECT_EDGE_ZONE: f32 = 48.0;
/// Edge-scroll speed at the zone's far edge, points per second.
pub const SELECT_EDGE_MAX_SPEED: f32 = 640.0;

/// Signed scroll speed (px/s, positive scrolls down) for a text-selection drag
/// at `pointer_y` against viewport [`top`, `bottom`]. Still in the middle,
/// grows with distance outside, capped at the far edge.
pub fn edge_scroll_speed(pointer_y: f32, top: f32, bottom: f32) -> f32 {
	if pointer_y <= top {
		let t = ((top - pointer_y) / SELECT_EDGE_ZONE).clamp(0.0, 1.0);
		-(0.5 + 0.5 * t) * SELECT_EDGE_MAX_SPEED
	} else if pointer_y >= bottom {
		let t = ((pointer_y - bottom) / SELECT_EDGE_ZONE).clamp(0.0, 1.0);
		(0.5 + 0.5 * t) * SELECT_EDGE_MAX_SPEED
	} else if pointer_y - top < SELECT_EDGE_ZONE {
		-((1.0 - (pointer_y - top) / SELECT_EDGE_ZONE) * 0.5 * SELECT_EDGE_MAX_SPEED)
	} else if bottom - pointer_y < SELECT_EDGE_ZONE {
		(1.0 - (bottom - pointer_y) / SELECT_EDGE_ZONE) * 0.5 * SELECT_EDGE_MAX_SPEED
	} else {
		0.0
	}
}

/// Chrome labels (headers, rails) never join text selection: dragging a
/// message selection past them keeps the range inside the messages.
pub(crate) fn chrome_label(ui: &mut egui::Ui, text: egui::RichText) -> egui::Response {
	ui.add(egui::Label::new(text).selectable(false).truncate())
}

/// Text cached from the last selected range (empty when nothing selected).
pub fn selected_text(ctx: &egui::Context) -> String {
	ctx.plugin_opt::<Pointer>()
		.map(|plugin| plugin.lock().cached.clone())
		.unwrap_or_default()
}

/// Take a pending save-selection-as-txt request from the selection popup.
pub fn take_save_txt(ctx: &egui::Context) -> Option<Vec<u8>> {
	ctx.plugin_opt::<Pointer>()
		.and_then(|plugin| plugin.lock().save_txt.take())
}

/// Paint a deferred widget galley, registering its selection unless a menu owns the pointer.
fn show_embed(ui: &mut egui::Ui, embed: &Embed, menu_open: bool) {
	if !embed.response.rect.is_positive() || !ui.is_rect_visible(embed.response.rect) {
		return;
	}
	// The galley carries its own per-token colours; the fallback only covers unstyled glyphs.
	let color = ui.visuals().text_color();
	if menu_open {
		ui.painter().add(TextShape::new(
			embed.galley_pos,
			embed.galley.clone(),
			color,
		));
		return;
	}
	LabelSelectionState::label_text_selection(
		ui,
		&embed.response,
		embed.galley_pos,
		embed.galley.clone(),
		color,
		Stroke::NONE,
	);
}

fn punch(rect: Rect, holes: &[Rect]) -> Vec<Rect> {
	let mut parts = vec![rect];
	for hole in holes {
		if !hole.is_positive() {
			continue;
		}
		let mut next = Vec::new();
		for part in parts {
			next.extend(subtract(part, *hole));
		}
		parts = next;
		if parts.is_empty() {
			break;
		}
	}
	parts
		.into_iter()
		.filter(|part| part.is_positive() && part.width() >= 1.0 && part.height() >= 1.0)
		.collect()
}

fn subtract(rect: Rect, hole: Rect) -> Vec<Rect> {
	let cut = rect.intersect(hole);
	if !cut.is_positive() {
		return vec![rect];
	}
	let mut parts = Vec::new();
	if rect.top() < cut.top() {
		parts.push(Rect::from_min_max(
			egui::pos2(rect.left(), rect.top()),
			egui::pos2(rect.right(), cut.top()),
		));
	}
	if cut.bottom() < rect.bottom() {
		parts.push(Rect::from_min_max(
			egui::pos2(rect.left(), cut.bottom()),
			egui::pos2(rect.right(), rect.bottom()),
		));
	}
	if rect.left() < cut.left() {
		parts.push(Rect::from_min_max(
			egui::pos2(rect.left(), cut.top()),
			egui::pos2(cut.left(), cut.bottom()),
		));
	}
	if cut.right() < rect.right() {
		parts.push(Rect::from_min_max(
			egui::pos2(cut.right(), cut.top()),
			egui::pos2(rect.right(), cut.bottom()),
		));
	}
	parts
}

fn block_rect(ui: &egui::Ui, runs: &[Run], cover: Option<Rect>) -> Rect {
	let from_runs = runs.first().map(|first| {
		let pad = ui.spacing().item_spacing.y / 2.0;
		let top = runs
			.iter()
			.map(|run| run.rect.top())
			.fold(first.rect.top(), f32::min);
		let bottom = runs
			.iter()
			.map(|run| run.rect.bottom())
			.fold(first.rect.bottom(), f32::max);
		Rect::from_min_max(
			egui::pos2(ui.max_rect().left(), top - pad),
			egui::pos2(ui.max_rect().right(), bottom + pad),
		)
	});
	match (from_runs, cover) {
		(Some(runs), Some(cover)) => runs.union(cover),
		(Some(runs), None) => runs,
		(None, Some(cover)) => cover,
		(None, None) => Rect::NOTHING,
	}
}

/// Screen-space band per galley row. A wrapped run's bounding rect spans several
/// lines, and its last line shares a line with whatever follows it: one band for the
/// whole run would sit on top of those neighbours and steal their selection hits.
fn line_bands(galley: &Galley, galley_pos: Pos2, rect: Rect) -> Vec<Rect> {
	if galley.rows.len() < 2 {
		return vec![rect];
	}
	let last = galley.rows.len() - 1;
	galley
		.rows
		.iter()
		.enumerate()
		.map(|(index, row)| {
			let row = row.rect().translate(galley_pos.to_vec2());
			let top = if index == 0 { rect.top() } else { row.top() };
			let bottom = if index == last {
				rect.bottom()
			} else {
				row.bottom()
			};
			Rect::from_min_max(egui::pos2(row.left(), top), egui::pos2(row.right(), bottom))
		})
		.collect()
}

fn tile(runs: &mut [Run], block: Rect, stitch: bool) {
	let mut index: Vec<(usize, usize)> = Vec::new();
	let mut lines: Vec<Rect> = Vec::new();
	for (run_index, run) in runs.iter().enumerate() {
		for (line_index, line) in run.lines.iter().enumerate() {
			index.push((run_index, line_index));
			lines.push(*line);
		}
	}
	if lines.is_empty() {
		return;
	}
	let mut rows = Vec::new();
	let mut start = 0;
	let mut top = lines[0].top();
	let mut bottom = lines[0].bottom();
	for (line_index, line) in lines.iter().enumerate().skip(1) {
		let center = line.center().y;
		if (top..=bottom).contains(&center) {
			top = top.min(line.top());
			bottom = bottom.max(line.bottom());
		} else {
			rows.push(start..line_index);
			start = line_index;
			top = line.top();
			bottom = line.bottom();
		}
	}
	rows.push(start..lines.len());

	let last = rows.len() - 1;
	let mut previous_bottom = block.top();
	for (row_index, range) in rows.into_iter().enumerate() {
		let natural_top = lines[range.clone()]
			.iter()
			.map(|line| line.top())
			.fold(f32::INFINITY, f32::min);
		let natural_bottom = lines[range.clone()]
			.iter()
			.map(|line| line.bottom())
			.fold(f32::NEG_INFINITY, f32::max);
		let row_top = if row_index == 0 {
			block.top()
		} else if stitch || natural_top <= previous_bottom + 2.0 {
			previous_bottom
		} else {
			natural_top
		};
		let row_bottom = if row_index == last {
			block.bottom()
		} else {
			natural_bottom
		};
		let first = range.start;
		let end = range.end;
		for line in &mut lines[range] {
			line.min.y = row_top;
			line.max.y = row_bottom;
		}
		lines[first].min.x = block.left();
		lines[end - 1].max.x = block.right();
		previous_bottom = row_bottom;
	}

	for ((run_index, line_index), line) in index.into_iter().zip(lines) {
		runs[run_index].lines[line_index] = line;
	}
	for run in runs {
		if let Some(rect) = run.lines.iter().copied().reduce(Rect::union) {
			run.rect = rect;
		}
	}
}

fn blank_run(ui: &egui::Ui, base: egui::Id, block: Rect) -> Run {
	let galley = ui.painter().layout_no_wrap(
		String::new(),
		egui::FontId::proportional(1.0),
		Color32::TRANSPARENT,
	);
	Run {
		band: base.with("blank"),
		galley_pos: block.min,
		galley,
		rect: block,
		lines: vec![block],
		painted: true,
	}
}

fn paint_artwork(ui: &egui::Ui, art: &Artwork) {
	if !ui.is_rect_visible(art.rect) {
		return;
	}
	let size = art.rect.width();
	if let Some(image) = &art.image {
		let painted = image.calc_size(egui::Vec2::splat(size), image.size());
		image.paint_at(ui, Rect::from_center_size(art.rect.center(), painted));
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const WIDTH: f32 = 220.0;

	/// A body like `text …link` where the leading run wraps across several rows.
	fn show(ui: &mut egui::Ui) {
		let mut surface = Surface::new(ui, "body");
		ui.allocate_ui_with_layout(
			egui::vec2(ui.available_width(), 0.0),
			egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true),
			|ui| {
				ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
				ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
				for (text, link) in [
					("alpha bravo charlie delta echo foxtrot golf hotel ", false),
					("https://example.com", true),
				] {
					let label = egui::Label::new(text).wrap().selectable(false);
					let (pos, galley, response) = label.layout_in_ui(ui);
					surface.run(ui, &response, pos, galley, Vec::new());
					if link {
						let overlay = ui.interact(
							response.rect,
							response.id.with("link"),
							egui::Sense::click(),
						);
						surface.through(&overlay);
					}
				}
			},
		);
		surface.finish(ui);
	}

	fn input(events: Vec<Event>) -> RawInput {
		RawInput {
			screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(WIDTH, 400.0))),
			events,
			..Default::default()
		}
	}

	fn press(pos: Pos2, pressed: bool) -> Vec<Event> {
		vec![
			Event::PointerMoved(pos),
			Event::PointerButton {
				pos,
				button: PointerButton::Primary,
				pressed,
				modifiers: Default::default(),
			},
		]
	}

	/// Drag from `from` to `to` and return what a copy would yield.
	fn drag(from: Pos2, to: Pos2) -> String {
		drag_with(show, from, to)
	}

	fn drag_with(show: impl Fn(&mut egui::Ui), from: Pos2, to: Pos2) -> String {
		let ctx = egui::Context::default();
		for events in [
			Vec::new(),
			press(from, true),
			vec![Event::PointerMoved(to)],
			press(to, false),
			vec![Event::Copy],
		] {
			let output = ctx.run_ui(input(events), &show);
			let copied = output
				.platform_output
				.commands
				.iter()
				.find_map(|command| match command {
					egui::OutputCommand::CopyText(text) => Some(text.clone()),
					_ => None,
				});
			output.drop_without_applying_deltas();
			if let Some(copied) = copied {
				return copied;
			}
		}
		String::new()
	}

	// Layout is "alpha … foxtrot " / "golf hotel " + "https://example.com", so the
	// wrapped first run ends on the same visual row as the link.

	#[test]
	fn a_drag_inside_the_wrapped_row_stops_before_the_link_row() {
		assert_eq!(
			drag(Pos2::new(4.0, 7.0), Pos2::new(WIDTH - 4.0, 7.0)),
			"lpha bravo charlie delta echo foxtrot"
		);
	}

	#[test]
	fn a_drag_on_the_link_row_starts_where_the_pointer_is() {
		assert_eq!(
			drag(Pos2::new(4.0, 22.0), Pos2::new(WIDTH - 4.0, 22.0)),
			"olf hotel https://example.com"
		);
	}

	#[test]
	fn a_drag_across_rows_ends_under_the_pointer() {
		assert_eq!(
			drag(Pos2::new(4.0, 7.0), Pos2::new(40.0, 22.0)),
			"lpha bravo charlie delta echo foxtrot golf ho"
		);
		assert_eq!(
			drag(Pos2::new(120.0, 22.0), Pos2::new(60.0, 7.0)),
			"o charlie delta echo foxtrot golf hotel https://exa"
		);
	}

	#[test]
	fn select_edge_scroll_is_still_in_the_middle_and_capped_outside() {
		let (top, bottom) = (100.0, 500.0);
		assert_eq!(super::edge_scroll_speed(300.0, top, bottom), 0.0);
		assert_eq!(super::edge_scroll_speed(top, top, bottom), -320.0);
		assert_eq!(super::edge_scroll_speed(top - 48.0, top, bottom), -640.0);
		assert_eq!(super::edge_scroll_speed(top - 480.0, top, bottom), -640.0);
		assert_eq!(super::edge_scroll_speed(bottom, top, bottom), 320.0);
		assert_eq!(super::edge_scroll_speed(bottom + 24.0, top, bottom), 480.0);
		assert_eq!(super::edge_scroll_speed(bottom + 48.0, top, bottom), 640.0);
		assert_eq!(super::edge_scroll_speed(top + 24.0, top, bottom), -160.0);
		assert_eq!(super::edge_scroll_speed(bottom - 24.0, top, bottom), 160.0);
		assert!(
			super::edge_scroll_speed(top - 40.0, top, bottom)
				< super::edge_scroll_speed(top - 8.0, top, bottom),
			"farther past the edge scrolls faster"
		);
		assert!(
			super::edge_scroll_speed(bottom + 40.0, top, bottom)
				> super::edge_scroll_speed(bottom + 8.0, top, bottom),
			"farther past the edge scrolls faster"
		);
		assert_eq!(super::SELECT_EDGE_ZONE, 48.0);
		assert_eq!(super::SELECT_EDGE_MAX_SPEED, 640.0);
	}

	/// Two stacked message bodies, each with its own surface like timeline rows.
	fn show_two(ui: &mut egui::Ui) {
		for (salt, text) in [("one", "first message body"), ("two", "second message body")] {
			let mut surface = Surface::new(ui, salt);
			ui.allocate_ui_with_layout(
				egui::vec2(ui.available_width(), 0.0),
				egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true),
				|ui| {
					ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
					ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
					let label = egui::Label::new(text).wrap().selectable(false);
					let (pos, galley, response) = label.layout_in_ui(ui);
					surface.run(ui, &response, pos, galley, Vec::new());
				},
			);
			surface.finish(ui);
		}
	}

	#[test]
	fn a_drag_across_messages_selects_both() {
		let selected = drag_with(show_two, Pos2::new(4.0, 7.0), Pos2::new(60.0, 25.0));
		assert!(
			selected.contains("irst message body") && selected.contains("second"),
			"selection spans messages, got {selected:?}"
		);
	}

	/// Header-style chrome above the messages, selectable only in the buggy variant.
	fn show_with_chrome(selectable_chrome: bool) -> impl Fn(&mut egui::Ui) {
		move |ui: &mut egui::Ui| {
			if selectable_chrome {
				ui.add(egui::Label::new("general"));
			} else {
				super::chrome_label(ui, egui::RichText::new("general"));
			}
			show(ui);
		}
	}

	#[test]
	fn a_drag_from_the_messages_through_the_chrome_leaves_the_header_out() {
		// Bug repro: a selectable chrome label joins the range.
		let buggy = drag_with(
			show_with_chrome(true),
			Pos2::new(4.0, 30.0),
			Pos2::new(20.0, 5.0),
		);
		assert!(
			buggy.contains("era"),
			"repro: selectable chrome joins the range, got {buggy:?}"
		);
		// Fixed: chrome labels never join.
		let fixed = drag_with(
			show_with_chrome(false),
			Pos2::new(4.0, 30.0),
			Pos2::new(20.0, 5.0),
		);
		assert!(
			!fixed.contains("general"),
			"chrome must stay out, got {fixed:?}"
		);
		assert!(!fixed.is_empty(), "message text stays selected");
	}

	#[test]
	fn selected_text_and_save_txt_round_trip_through_the_plugin() {
		let ctx = egui::Context::default();
		super::install(&ctx);
		assert_eq!(super::selected_text(&ctx), "");
		assert_eq!(super::take_save_txt(&ctx), None);
		ctx.plugin::<Pointer>()
			.lock()
			.cached = "hello".to_owned();
		assert_eq!(super::selected_text(&ctx), "hello");
		ctx.plugin::<Pointer>()
			.lock()
			.save_txt = Some(b"hello".to_vec());
		assert_eq!(super::take_save_txt(&ctx), Some(b"hello".to_vec()));
		assert_eq!(super::take_save_txt(&ctx), None);
	}
}
