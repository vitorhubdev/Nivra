use super::*;
use client_core::{GatewayReconnect, State};
use egui::{Event as InputEvent, PointerButton, ViewportCommand};
use std::time::{Duration, Instant};

fn frame_output(
	ctx: &egui::Context,
	view: &mut MessagingUi,
	state: &State,
	width: f32,
	events: Vec<InputEvent>,
) -> egui::FullOutput {
	ctx.run_ui(
		egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(width, 600.0),
			)),
			focused: true,
			events,
			..Default::default()
		},
		|ui| view.title_bar(ui, state, "Synthetic context title"),
	)
}

fn frame(
	ctx: &egui::Context,
	view: &mut MessagingUi,
	width: f32,
	events: Vec<InputEvent>,
) -> Vec<ViewportCommand> {
	let state = State::default();
	let output = frame_output(ctx, view, &state, width, events);
	let commands = output.viewport_output[&egui::ViewportId::ROOT]
		.commands
		.clone();
	output.drop_without_applying_deltas();
	commands
}

fn title_bar_layout_rects(output: &egui::FullOutput) -> Vec<egui::Rect> {
	let mut rects: Vec<_> = output
		.shapes
		.iter()
		.filter_map(|clipped| {
			let rect = clipped.clip_rect;
			(rect.max.y <= 36.5).then_some(rect)
		})
		.collect();
	rects.sort_by(|left, right| {
		left.top()
			.total_cmp(&right.top())
			.then_with(|| left.left().total_cmp(&right.left()))
			.then_with(|| left.width().total_cmp(&right.width()))
	});
	rects
}

fn gateway_dot_center(output: &egui::FullOutput) -> Option<egui::Pos2> {
	output.shapes.iter().find_map(|clipped| {
		if let egui::Shape::Circle(circle) = &clipped.shape {
			((circle.radius - 3.0).abs() < 0.01).then_some(circle.center)
		} else {
			None
		}
	})
}

fn connected_gateway_state() -> State {
	State {
		gateway_connected: true,
		gateway_ping_ms: Some(42),
		gateway_host: "gateway.discord.gg".into(),
		gateway_connected_since: Some(Instant::now() - Duration::from_secs(90)),
		..Default::default()
	}
}

fn pointer(pos: egui::Pos2, button: PointerButton, pressed: bool) -> Vec<InputEvent> {
	vec![
		InputEvent::PointerMoved(pos),
		InputEvent::PointerButton {
			pos,
			button,
			pressed,
			modifiers: egui::Modifiers::NONE,
		},
	]
}

#[cfg(target_os = "windows")]
#[test]
fn hidden_title_strip_does_not_start_window_drag() {
	let ctx = egui::Context::default();
	let mut view = MessagingUi::default();
	let mut state = test_support::demo_state();
	let run = |view: &mut MessagingUi, state: &mut State, events| {
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1200.0, 760.0),
				)),
				focused: true,
				events,
				..Default::default()
			},
			|ui| {
				view.show(ui, state);
			},
		);
		let commands = output.viewport_output[&egui::ViewportId::ROOT]
			.commands
			.clone();
		output.drop_without_applying_deltas();
		commands
	};
	let pos = egui::pos2(600.0, 18.0);
	run(&mut view, &mut state, vec![]);
	assert_eq!(
		run(
			&mut view,
			&mut state,
			pointer(pos, PointerButton::Primary, true)
		),
		[ViewportCommand::StartDrag]
	);
	run(
		&mut view,
		&mut state,
		pointer(pos, PointerButton::Primary, false),
	);
	view.hide_title_bar = true;
	run(&mut view, &mut state, vec![]);
	assert!(
		!run(
			&mut view,
			&mut state,
			pointer(pos, PointerButton::Primary, true)
		)
		.contains(&ViewportCommand::StartDrag)
	);
}

fn palette(ctx: &egui::Context) -> design::Palette {
	let mut colors = None;
	ctx.run_ui(
		egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(320.0, 240.0),
			)),
			..Default::default()
		},
		|ui| colors = Some(design::palette(ui)),
	)
	.drop_without_applying_deltas();
	colors.unwrap()
}

#[test]
#[allow(clippy::field_reassign_with_default)] // Test stages gateway states.
fn gateway_status_dot_color_respects_latency_bands() {
	let ctx = egui::Context::default();
	let colors = palette(&ctx);
	let mut reconnecting = State::default();
	reconnecting.gateway_reconnect = Some(GatewayReconnect {
		attempt: 2,
		retry_in_ms: 4_000,
		since: Instant::now(),
	});
	assert_eq!(
		MessagingUi::gateway_status_dot_color(&colors, &reconnecting),
		colors.warning
	);
	let disconnected = State::default();
	assert_eq!(
		MessagingUi::gateway_status_dot_color(&colors, &disconnected),
		colors.danger
	);
	let mut good = State::default();
	good.gateway_connected = true;
	good.gateway_ping_ms = Some(80);
	assert_eq!(
		MessagingUi::gateway_status_dot_color(&colors, &good),
		colors.positive
	);
	let mut slow = State::default();
	slow.gateway_connected = true;
	slow.gateway_ping_ms = Some(250);
	assert_eq!(
		MessagingUi::gateway_status_dot_color(&colors, &slow),
		colors.warning
	);
}

#[test]
fn title_bar_layout_is_identical_with_and_without_gateway_hover() {
	let ctx = egui::Context::default();
	let state = connected_gateway_state();
	let width = 960.0;
	let mut view = MessagingUi::default();
	let baseline = frame_output(&ctx, &mut view, &state, width, vec![]);
	let dot = gateway_dot_center(&baseline).expect("title bar renders the gateway status dot");
	let hovered = frame_output(
		&ctx,
		&mut view,
		&state,
		width,
		vec![InputEvent::PointerMoved(dot)],
	);
	assert_eq!(
		title_bar_layout_rects(&baseline),
		title_bar_layout_rects(&hovered)
	);
	baseline.drop_without_applying_deltas();
	hovered.drop_without_applying_deltas();
}

#[test]
fn title_strip_primary_press_starts_drag_immediately_and_only_once() {
	for width in [760.0, 1200.0] {
		for dark in [true, false] {
			for x in [24.0, width * 0.22, width * 0.5] {
				let ctx = egui::Context::default();
				ctx.set_visuals(if dark {
					egui::Visuals::dark()
				} else {
					egui::Visuals::light()
				});
				let mut view = MessagingUi::default();
				frame(&ctx, &mut view, width, vec![]);
				let pos = egui::pos2(x, 18.0);
				assert_eq!(
					frame(
						&ctx,
						&mut view,
						width,
						pointer(pos, PointerButton::Primary, true)
					),
					[ViewportCommand::StartDrag],
					"width={width}, dark={dark}, x={x}"
				);
				let moved = pos + egui::vec2(12.0, 2.0);
				assert!(
					frame(
						&ctx,
						&mut view,
						width,
						vec![InputEvent::PointerMoved(moved)]
					)
					.is_empty()
				);
				assert!(
					frame(
						&ctx,
						&mut view,
						width,
						pointer(moved, PointerButton::Primary, false)
					)
					.is_empty()
				);
			}
		}
	}
}

#[test]
fn secondary_press_and_content_press_do_not_drag() {
	for (pos, button) in [
		(egui::pos2(24.0, 18.0), PointerButton::Secondary),
		(egui::pos2(24.0, 90.0), PointerButton::Primary),
	] {
		let ctx = egui::Context::default();
		let mut view = MessagingUi::default();
		frame(&ctx, &mut view, 760.0, vec![]);
		for pressed in [true, false] {
			assert!(frame(&ctx, &mut view, 760.0, pointer(pos, button, pressed)).is_empty());
		}
	}
}

#[cfg(target_os = "windows")]
#[test]
fn caption_buttons_act_without_dragging_and_title_double_click_maximizes() {
	for width in [760.0, 1200.0] {
		for (offset, expected) in [
			(35.0, ViewportCommand::Close),
			(81.0, ViewportCommand::Maximized(true)),
			(127.0, ViewportCommand::Minimized(true)),
		] {
			let ctx = egui::Context::default();
			let mut view = MessagingUi::default();
			frame(&ctx, &mut view, width, vec![]);
			let pos = egui::pos2(width - offset, 18.0);
			assert!(
				frame(
					&ctx,
					&mut view,
					width,
					pointer(pos, PointerButton::Primary, true)
				)
				.is_empty()
			);
			assert_eq!(
				frame(
					&ctx,
					&mut view,
					width,
					pointer(pos, PointerButton::Primary, false)
				),
				[expected]
			);
		}
	}
	let ctx = egui::Context::default();
	let mut view = MessagingUi::default();
	frame(&ctx, &mut view, 760.0, vec![]);
	let pos = egui::pos2(380.0, 18.0);
	for click in 0..2 {
		assert_eq!(
			frame(
				&ctx,
				&mut view,
				760.0,
				pointer(pos, PointerButton::Primary, true)
			),
			[ViewportCommand::StartDrag]
		);
		let released = frame(
			&ctx,
			&mut view,
			760.0,
			pointer(pos, PointerButton::Primary, false),
		);
		if click == 0 {
			assert!(released.is_empty());
		} else {
			assert_eq!(released, [ViewportCommand::Maximized(true)]);
		}
	}
}
