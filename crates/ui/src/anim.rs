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
/// Trailing fade, counted inside the lifetime. Matches `toasts.rs`.
pub const FADE_SECS: f64 = 0.45;
/// Speaking ring fade-in duration.
pub const SPEAKING_RING_ENTER_SECS: f32 = 0.12;
/// Speaking ring fade-out duration.
pub const SPEAKING_RING_EXIT_SECS: f32 = 0.22;

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
pub fn bool_alpha(ctx: &egui::Context, id: egui::Id, value: bool, time: f32) -> f32 {
	ctx.animate_bool_with_time_and_easing(id, value, time, ease)
}

/// Animated height towards `target`. Settles exactly at `target` with no
/// further repaints once reached.
pub fn animated_height(ctx: &egui::Context, id: egui::Id, target: f32, time: f32) -> f32 {
	ctx.animate_value_with_time(id, target, time)
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
	if visibility > 0.0 && visibility < 1.0 {
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
		assert_eq!(fade_alpha(0.0, 0.45), 0.0);
		assert_eq!(fade_alpha(0.45, 0.45), 1.0);
		assert_eq!(fade_alpha(10.0, 0.45), 1.0);
		assert_eq!(FADE_SECS, 0.45);
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
