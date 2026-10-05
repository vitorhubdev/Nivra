//! Shared animation helpers: easing, clamped dt, fades and animated heights.
//!
//! Unifies `ease_out_cubic` and clamped `stable_dt` from `timeline.rs` with the
//! toast fade in `toasts.rs`. Pure helpers never repaint; egui wrappers repaint
//! only while animating, so idle frames schedule zero repaints.

/// Minimum frame step: clamps `stable_dt` against timer jitter.
/// Matches `timeline.rs` scroll glides.
pub const MIN_DT: f32 = 1.0 / 240.0;
/// Maximum frame step: tab-switch or hibernation never jumps an animation.
/// Matches `timeline.rs` scroll glides.
pub const MAX_DT: f32 = 0.05;

/// Motion tokens: every UI motion uses [`ease`] and one of these three durations.
/// `SHORT` is direct feedback (hover, press, speaking ring in), `MEDIUM` covers menus,
/// toasts, banners, new-message entry and the ring out, `LONG` covers dialogs, side
/// panels and channel switches. New call sites must use a token, never a raw value.
pub const SHORT_SECS: f32 = 0.12;
pub const MEDIUM_SECS: f32 = 0.22;
pub const LONG_SECS: f32 = 0.34;
/// The same three tokens as [`std::time::Duration`] for non-egui call sites.
pub const SHORT: std::time::Duration = std::time::Duration::from_millis(120);
pub const MEDIUM: std::time::Duration = std::time::Duration::from_millis(220);
pub const LONG: std::time::Duration = std::time::Duration::from_millis(340);
const _: () = assert!(SHORT_SECS < MEDIUM_SECS && MEDIUM_SECS < LONG_SECS);
/// Trailing toast fade, counted inside the lifetime.
pub const FADE_SECS: f64 = MEDIUM_SECS as f64;
/// Speaking ring fade-in duration.
pub const SPEAKING_RING_ENTER_SECS: f32 = SHORT_SECS;
/// Speaking ring fade-out duration.
pub const SPEAKING_RING_EXIT_SECS: f32 = MEDIUM_SECS;

/// egui-data key holding the owner's "reduce motion" choice for this context.
const REDUCE_MOTION: &str = "nivra-reduce-motion";

/// Records the owner's reduce-motion preference for [`bool_alpha`], [`animated_height`],
/// [`hover`], [`popup_alpha`] and [`speaking_ring`]. The app sets this every frame from
/// its stored preference; tests may set it directly on their own context.
pub fn set_reduce_motion(ctx: &egui::Context, reduce: bool) {
	ctx.data_mut(|data| data.insert_temp(egui::Id::unique(REDUCE_MOTION), reduce));
}

/// Whether the owner asked for reduced motion (default off).
pub fn reduce_motion(ctx: &egui::Context) -> bool {
	ctx.data(|data| {
		data.get_temp(egui::Id::unique(REDUCE_MOTION))
			.unwrap_or(false)
	})
}

/// Cubic ease-out, clamped to `[0, 1]`. Shared by scroll glides, fades and rings.
/// `ease(0.0) == 0.0`, `ease(1.0) == 1.0`, `ease(0.5) == 0.875`.
pub fn ease(t: f32) -> f32 {
	let t = t.clamp(0.0, 1.0);
	let rest = 1.0 - t;
	1.0 - rest * rest * rest
}

/// Compatibility alias for the previous `timeline.rs` helper.
pub fn ease_out_cubic(t: f32) -> f32 {
	ease(t)
}

/// Clamps a frame delta to `[MIN_DT, MAX_DT]` so animations never stall nor jump.
pub fn clamp_dt(dt: f32) -> f32 {
	dt.clamp(MIN_DT, MAX_DT)
}

/// 0..1 progress for `elapsed / duration`. Zero or negative durations finish at once.
pub fn progress(elapsed: f32, duration: f32) -> f32 {
	if duration <= 0.0 {
		return 1.0;
	}
	(elapsed / duration).clamp(0.0, 1.0)
}

/// 0..1 alpha for a trailing fade: `remaining` seconds left of a `fade`-long tail.
pub fn fade_alpha(remaining: f32, fade: f32) -> f32 {
	if fade <= 0.0 {
		return if remaining > 0.0 { 1.0 } else { 0.0 };
	}
	(remaining / fade).clamp(0.0, 1.0)
}

/// Eased 0..1 visibility for a boolean. Repaints only while animating;
/// settled `0.0`/`1.0` schedules zero extra frames (egui stops by itself).
/// With reduce motion the value settles in the same frame.
pub fn bool_alpha(ctx: &egui::Context, id: egui::Id, value: bool, time: f32) -> f32 {
	let time = if reduce_motion(ctx) { 0.0 } else { time };
	ctx.animate_bool_with_time_and_easing(id, value, time, ease)
}

/// Animated value towards `target`. Settles exactly at `target` with no further
/// repaints once reached. With reduce motion the target is returned immediately.
pub fn animated_height(ctx: &egui::Context, id: egui::Id, target: f32, time: f32) -> f32 {
	if reduce_motion(ctx) {
		// Keep egui's stored value in sync so turning reduce motion off later does
		// not pop from a stale position (Codex #83 P2).
		let _ = ctx.animate_value_with_time(id, target, 0.0);
		return target;
	}
	ctx.animate_value_with_time(id, target, time)
}

/// Eased 0..1 hover amount for `hot`, using [`SHORT_SECS`]. Animates in both
/// directions and stops requesting frames once settled.
pub fn hover(ctx: &egui::Context, id: egui::Id, hot: bool) -> f32 {
	bool_alpha(ctx, id, hot, SHORT_SECS)
}

/// Applies [`popup_alpha`] to the popup content `ui`. Call once at the top of a menu,
/// popup or dialog body so every surface opens with the same motion.
pub fn popup_motion(ui: &mut egui::Ui) {
	let alpha = popup_alpha(ui.ctx(), ui.scope_id().with("menu-motion"));
	ui.set_opacity(alpha);
}

/// Eased 0..1 opening alpha for a popup, menu or dialog. The first pass after the
/// surface was absent starts from 0; while it stays open the value rests at 1.
pub fn popup_alpha(ctx: &egui::Context, id: egui::Id) -> f32 {
	let seen = id.with("motion-seen");
	let pass = ctx.cumulative_pass_nr();
	let fresh = ctx.data_mut(|data| {
		let last = data.get_temp::<u64>(seen);
		data.insert_temp(seen, pass);
		last.is_none_or(|last| pass.saturating_sub(last) > 1)
	});
	if fresh {
		// Settle the stored value at 0 so this pass animates from the closed state.
		bool_alpha(ctx, id.with("motion-open"), false, 0.0);
	}
	bool_alpha(ctx, id.with("motion-open"), true, MEDIUM_SECS)
}

/// Schedules the next frame only when needed: immediate repaint while `remaining_secs <= 0`,
/// delayed repaint for a finite future deadline, and zero repaints for idle/infinite.
pub fn request_until(ctx: &egui::Context, remaining_secs: f64) {
	if remaining_secs <= 0.0 {
		ctx.request_repaint();
	} else if remaining_secs.is_finite() {
		ctx.request_repaint_after(std::time::Duration::from_secs_f64(remaining_secs));
	}
}

/// Ring radius offset (px) and stroke opacity from eased visibility and energy.
pub fn speaking_ring_geometry(visibility: f32, energy: u8) -> (f32, f32) {
	let energy = f32::from(energy) / 255.0;
	let radius = (2.0 + 5.0 * energy) * visibility;
	let opacity = visibility * (0.35 + 0.65 * energy);
	(radius, opacity)
}

/// Eased speaking-ring visibility plus geometry derived from smoothed energy.
pub fn speaking_ring(
	ctx: &egui::Context,
	id: egui::Id,
	active: bool,
	energy: u8,
) -> (f32, f32, f32) {
	let time = if active {
		SPEAKING_RING_ENTER_SECS
	} else {
		SPEAKING_RING_EXIT_SECS
	};
	let visibility = bool_alpha(ctx, id, active, time);
	let (radius, opacity) = speaking_ring_geometry(visibility, energy);
	if !reduce_motion(ctx) && visibility > 0.0 && visibility < 1.0 {
		request_until(ctx, f64::from(time));
	}
	(visibility, radius, opacity)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn ease_curve_starts_ends_and_eases_out() {
		assert_eq!(ease(0.0), 0.0);
		assert_eq!(ease(1.0), 1.0);
		assert!((ease(0.5) - 0.875).abs() < 1e-6);
		assert!(ease(0.25) > 0.25 && ease(0.25) < ease(0.5) && ease(0.5) < ease(0.75));
		assert_eq!(ease(-1.0), 0.0);
		assert_eq!(ease(2.0), 1.0);
		assert_eq!(ease_out_cubic(0.5), ease(0.5));
	}

	#[test]
	fn dt_is_clamped_against_jitter_and_hibernation() {
		assert_eq!(clamp_dt(0.0), MIN_DT);
		assert_eq!(clamp_dt(10.0), MAX_DT);
		assert_eq!(clamp_dt(0.016), 0.016);
		assert_eq!(clamp_dt(MIN_DT), MIN_DT);
		assert_eq!(clamp_dt(MAX_DT), MAX_DT);
	}

	#[test]
	fn progress_and_fade_finish_at_one() {
		assert_eq!(progress(0.0, 1.0), 0.0);
		assert_eq!(progress(1.0, 1.0), 1.0);
		assert_eq!(progress(10.0, 1.0), 1.0);
		assert_eq!(progress(-1.0, 1.0), 0.0);
		assert_eq!(progress(0.5, 0.0), 1.0);
		assert_eq!(fade_alpha(0.0, 0.22), 0.0);
		assert_eq!(fade_alpha(0.22, 0.22), 1.0);
		assert_eq!(fade_alpha(10.0, 0.22), 1.0);
		assert_eq!(FADE_SECS, f64::from(MEDIUM_SECS));
	}

	#[test]
	fn motion_tokens_cover_short_medium_and_long() {
		assert_eq!(SPEAKING_RING_ENTER_SECS, SHORT_SECS);
		assert_eq!(SPEAKING_RING_EXIT_SECS, MEDIUM_SECS);
		assert_eq!(SHORT.as_secs_f32(), SHORT_SECS);
		assert_eq!(MEDIUM.as_secs_f32(), MEDIUM_SECS);
		assert_eq!(LONG.as_secs_f32(), LONG_SECS);
	}

	#[test]
	fn reduce_motion_keeps_the_stored_height_in_sync() {
		let ctx = egui::Context::default();
		let id = egui::Id::unique("reduce-motion-sync");
		set_reduce_motion(&ctx, true);
		assert_eq!(animated_height(&ctx, id, 240.0, LONG_SECS), 240.0);
		assert_eq!(animated_height(&ctx, id, 0.0, LONG_SECS), 0.0);
		set_reduce_motion(&ctx, false);
		// Without the stored-value sync this would interpolate from 240 and pop.
		assert_eq!(animated_height(&ctx, id, 0.0, LONG_SECS), 0.0);
	}

	#[test]
	fn reduce_motion_settles_every_helper_in_one_frame() {
		let ctx = egui::Context::default();
		set_reduce_motion(&ctx, true);
		assert!(reduce_motion(&ctx));
		let id = egui::Id::unique("reduce-motion-bool");
		assert_eq!(bool_alpha(&ctx, id, true, LONG_SECS), 1.0);
		let hover = egui::Id::unique("reduce-motion-hover");
		assert_eq!(crate::anim::hover(&ctx, hover, true), 1.0);
		let height = egui::Id::unique("reduce-motion-height");
		assert_eq!(animated_height(&ctx, height, 24.0, LONG_SECS), 24.0);
		let ring = egui::Id::unique("reduce-motion-ring");
		assert_eq!(speaking_ring(&ctx, ring, true, 128).0, 1.0);
		set_reduce_motion(&ctx, false);
		assert!(!reduce_motion(&ctx));
	}

	#[test]
	fn settled_animation_requests_no_further_frames() {
		let ctx = egui::Context::default();
		let id = egui::Id::unique("settled-bool");
		let mut time = 0.0f64;
		let mut value = 0.0;
		for _ in 0..240 {
			let input = egui::RawInput {
				time: Some(time),
				..Default::default()
			};
			ctx.begin_pass(input);
			value = bool_alpha(&ctx, id, true, SHORT_SECS);
			let mut output = ctx.end_pass();
			output.textures_delta.clear();
			if value >= 1.0 {
				break;
			}
			time += 1.0 / 60.0;
		}
		assert_eq!(value, 1.0);
		let input = egui::RawInput {
			time: Some(time),
			..Default::default()
		};
		ctx.begin_pass(input);
		let first = bool_alpha(&ctx, id, true, SHORT_SECS);
		let mut output = ctx.end_pass();
		output.textures_delta.clear();
		assert_eq!(first, 1.0);
		// A settled wrapper keeps returning the target; the kittest test proves that it
		// also stops requesting frames.
		assert_eq!(bool_alpha(&ctx, id, true, SHORT_SECS), 1.0);
	}

	#[test]
	fn speaking_ring_geometry_tracks_energy_and_visibility() {
		let (radius, opacity) = speaking_ring_geometry(0.0, 255);
		assert_eq!(radius, 0.0);
		assert_eq!(opacity, 0.0);
		let (radius, opacity) = speaking_ring_geometry(1.0, 255);
		assert!((radius - 7.0).abs() < 1e-3);
		assert!((opacity - 1.0).abs() < 1e-3);
		let (low, _) = speaking_ring_geometry(1.0, 0);
		let (high, _) = speaking_ring_geometry(1.0, 255);
		assert!(high > low);
		assert_eq!(SPEAKING_RING_ENTER_SECS, 0.12);
		assert_eq!(SPEAKING_RING_EXIT_SECS, 0.22);
	}

	#[test]
	fn finished_animations_schedule_no_extra_frames() {
		let ctx = egui::Context::default();
		// Pure helpers take no ctx, so they cannot repaint by construction.
		assert_eq!(progress(10.0, 1.0), 1.0);
		assert_eq!(fade_alpha(10.0, 0.45), 1.0);
		// Infinite/idle deadlines are a no-op: must not panic nor schedule.
		request_until(&ctx, f64::INFINITY);
		// Eased wrappers settle without extra work once at rest.
		let id = egui::Id::unique("anim-test-bool");
		assert_eq!(bool_alpha(&ctx, id, true, 0.0), 1.0);
		let hid = egui::Id::unique("anim-test-height");
		assert_eq!(animated_height(&ctx, hid, 24.0, 0.0), 24.0);
	}
}

#[cfg(test)]
mod kittest_tests {
	use super::*;
	use egui_kittest::kittest::Queryable as _;

	#[test]
	fn hover_animation_settles_without_extra_frames() {
		let mut harness = egui_kittest::HarnessBuilder::default()
			.allow_missing_glyphs()
			.with_step_dt(1.0 / 60.0)
			.with_max_steps(240)
			.build_ui(|ui| {
				crate::design::button(ui, "Hover", crate::design::ButtonKind::Primary);
			});
		harness
			.get_by_role_and_label(egui::Role::Button, "Hover")
			.hover();
		let frames = harness.run();
		assert!(
			frames <= 60,
			"hover animation must settle, ran {frames} frames"
		);
		// A settled animation settles again in one frame: no continuous repainting.
		assert_eq!(
			harness.run(),
			1,
			"settled hover must not keep requesting frames"
		);
	}

	struct HoverFixture {
		hover: f32,
	}

	fn hover_harness(reduce: bool) -> egui_kittest::Harness<'static, HoverFixture> {
		egui_kittest::HarnessBuilder::default()
			.allow_missing_glyphs()
			.with_step_dt(1.0 / 60.0)
			.with_max_steps(240)
			.build_ui_state(
				move |ui, fixture: &mut HoverFixture| {
					if reduce {
						set_reduce_motion(ui.ctx(), true);
					}
					let (rect, response) =
						ui.allocate_exact_size(egui::vec2(80.0, 32.0), egui::Sense::click());
					response.widget_info(|| {
						egui::WidgetInfo::labeled(egui::Role::Button, true, "Hover")
					});
					let t = hover(ui.ctx(), response.id.with("hover"), response.hovered());
					fixture.hover = t;
					ui.painter()
						.rect_filled(rect, 4, egui::Color32::from_gray((t * 255.0) as u8));
				},
				HoverFixture { hover: 0.0 },
			)
	}

	#[test]
	fn hover_animation_settles_at_full() {
		let mut harness = hover_harness(false);
		// Seed the animation state at rest so the hovered frame is a transition.
		harness.run_steps(1);
		harness
			.get_by_role_and_label(egui::Role::Button, "Hover")
			.hover();
		harness.run_steps(1);
		assert!(
			harness.state().hover < 1.0,
			"the first hovered frame must still be animating"
		);
		let frames = harness.run();
		assert!(
			frames <= 60,
			"hover animation must settle, ran {frames} frames"
		);
		assert_eq!(harness.state().hover, 1.0);
	}

	#[test]
	fn reduce_motion_hover_settles_in_one_frame() {
		let mut harness = hover_harness(true);
		harness
			.get_by_role_and_label(egui::Role::Button, "Hover")
			.hover();
		harness.run_steps(1);
		assert_eq!(
			harness.state().hover,
			1.0,
			"reduce motion must settle every animation in one frame"
		);
	}
}
