use client_core::auth::AuthState;
use egui::CornerRadius;
use model::Freshness;
use ui::MessagingUi;

#[test]
fn account_card_and_composer_pill_alignment() {
	for width in [900.0, 1280.0, 1920.0] {
		let ctx = egui::Context::default();
		ui::design::apply(&ctx);
		let mut state = test_support::demo_state();
		state.auth = AuthState::Authenticated;
		state.gateway_connected = true;
		state.freshness = Freshness::Fresh;
		assert!(state.selected.is_some());

		let mut view = Box::new(MessagingUi::default());
		let height = 760.0;

		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(width, height),
				)),
				..Default::default()
			},
			|ui| {
				view.show(ui, &mut state);
			},
		);

		let pills: Vec<egui::Rect> = output
			.shapes
			.iter()
			.filter_map(|shape| match &shape.shape {
				egui::Shape::Rect(rect)
					if rect.corner_radius == CornerRadius::same(8)
						&& rect.rect.bottom() >= height - 20.0 =>
				{
					Some(rect.rect)
				}
				_ => None,
			})
			.collect();

		// Find the account pill (on the left sidebar area) and composer pill (on the main chat area)
		let account_pill = pills
			.iter()
			.copied()
			.find(|r| r.left() < 350.0 && r.width() < 300.0)
			.expect("account pill with corner radius 8 at bottom");

		let composer_pill = pills
			.iter()
			.copied()
			.find(|r| r.left() >= 240.0 && r.width() > 300.0)
			.expect("composer pill with corner radius 8 at bottom");

		// Both pills must dock to the bottom with the exact same bottom baseline (margin bottom: 8)
		assert_eq!(
			account_pill.bottom(),
			height - 8.0,
			"account pill bottom must be window bottom - 8.0 at width {width}"
		);
		assert_eq!(
			composer_pill.bottom(),
			height - 8.0,
			"composer pill bottom must be window bottom - 8.0 at width {width}"
		);

		// Both pills must have identical height (56.0px)
		assert_eq!(
			account_pill.height(),
			56.0,
			"account pill height must be 56.0px at width {width}"
		);
		assert_eq!(
			composer_pill.height(),
			56.0,
			"composer pill height must be 56.0px at width {width}"
		);

		// Both pills must share the exact same vertical center line
		assert_eq!(
			account_pill.center().y,
			composer_pill.center().y,
			"account pill and composer pill must share the same vertical center line at width {width}"
		);

		output.drop_without_applying_deltas();
	}
}
