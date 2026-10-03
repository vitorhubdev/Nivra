//! Applies notification device choices to filtered messages and explicit call ringing state.
use client_core::{State, auth::AuthState};
use model::{
	Id, PresenceStatus,
	notification_preferences::{Device, Sound},
};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub enum Alert {
	Message {
		channel: Id,
		title: String,
		body: String,
		avatar_key: String,
		image_path: Option<String>,
	},
}
#[derive(Default)]
pub struct Runtime {
	sounds: crate::notification_sounds::Sounds,
	options: Device,
	was_audible: bool,
	ring: Option<((Id, Sound), Instant)>,
	badge: Option<u32>,
	badge_check: Option<Instant>,
	badge_status: &'static str,
	/// Ordered join/leave cues with the earliest instant the next one may play.
	/// Every arrival and every departure plays, in order, paced so rapid
	/// sequences stay intelligible instead of collapsing into one slot.
	membership: VecDeque<(Sound, u8)>,
	membership_due: Option<Instant>,
	membership_tries: u8,
	/// Local toggle cues (mute/unmute/deafen, previews): the same cue twice in a
	/// row collapses to one, opposite toggles both play in order, and the queue
	/// retries while the worker is busy instead of dropping the press.
	local: VecDeque<(Sound, u8)>,
	local_tries: u8,
}
/// Minimum gap between two membership sounds so a rapid sequence stays
/// intelligible instead of chopping one cue with the next.
const MEMBERSHIP_GAP: Duration = Duration::from_millis(120);
/// Membership queue bound: a storm of rejoins keeps the latest screenful.
const MAX_MEMBERSHIP_CUES: usize = 8;
/// Local toggle queue bound: presses stay responsive without growing.
const MAX_LOCAL_CUES: usize = 4;

fn membership_volume(options: Device, membership: bool) -> u8 {
	if membership && options.volume == 0 {
		100
	} else {
		options.volume
	}
}

fn membership_due(due: Option<Instant>, now: Instant) -> bool {
	due.is_none_or(|at| now.saturating_duration_since(at) >= MEMBERSHIP_GAP)
}

fn push_membership_cue(queue: &mut VecDeque<(Sound, u8)>, cue: Sound, volume: u8) {
	if queue.back().is_some_and(|(last, _)| *last == cue) {
		return;
	}
	if queue.len() >= MAX_MEMBERSHIP_CUES {
		queue.pop_front();
	}
	queue.push_back((cue, volume));
}

fn push_local_cue(queue: &mut VecDeque<(Sound, u8)>, cue: Sound, volume: u8) {
	// The same toggle twice in a row collapses; opposite toggles both play in order.
	if queue.back().is_some_and(|(last, _)| *last == cue) {
		return;
	}
	if queue.len() >= MAX_LOCAL_CUES {
		queue.pop_front();
	}
	queue.push_back((cue, volume));
}

impl Runtime {
	fn ring_cue(&mut self, ringing: Option<(Id, Sound)>, now: Instant) -> Option<Sound> {
		if self.ring.map(|(key, _)| key) != ringing {
			// A finished ring must not play from the queue after the call was answered.
			self.local
				.retain(|(cue, _)| !matches!(cue, Sound::IncomingRing | Sound::OutgoingRing));
			if self.ring.is_some() {
				self.sounds.stop();
			}
			self.ring = ringing.map(|key| (key, now));
			return ringing.map(|(_, cue)| cue);
		}
		if let Some(((_, cue), played)) = &mut self.ring {
			let interval = if *cue == Sound::OutgoingRing {
				crate::notification_sounds::OUTGOING_RING_INTERVAL
			} else {
				crate::notification_sounds::RING_INTERVAL
			};
			if now.duration_since(*played) >= interval {
				*played = now;
				return Some(*cue);
			}
		}
		None
	}
	/// Direct pings last counted for the taskbar badge; zero while unread badges are off.
	/// Windows-only: read by the Windows tray sync; other platforms never query it.
	#[cfg(target_os = "windows")]
	pub fn pings(&self) -> u32 {
		self.badge.unwrap_or(0)
	}
	pub fn clear(&mut self, window: &winit::window::Window) {
		self.sounds.stop();
		self.ring = None;
		if self.badge.is_some_and(|count| count > 0) {
			let _ = platform::badge::set(window, 0);
		}
		self.badge = None;
	}
	/// Returns a coalesced desktop alert request. Sound is independent of desktop alerts.
	pub fn poll(
		&mut self,
		state: &mut State,
		ui: &mut ui::MessagingUi,
		window: &winit::window::Window,
		ctx: &eframe::egui::Context,
		fixture: bool,
	) -> Option<Alert> {
		let live = !fixture && !state.demo && state.auth == AuthState::Authenticated;
		let audible = live && ui.own_presence.status != PresenceStatus::DoNotDisturb;
		let options = ui.notification_options;
		let badges = platform::badge::supported() && options.unread_badge;
		if options != self.options || (self.was_audible && !audible) {
			self.sounds.stop();
			self.options = options;
		}
		self.was_audible = audible;
		let focused = ctx.input(|i| {
			i.focused
				&& i.viewport().visible() != Some(false)
				&& i.viewport().minimized != Some(true)
		});
		let mut alert = None;
		let mut sound = None;
		while let Some(notification) = state.take_notification() {
			let current = focused && ui.viewing_latest(notification.channel);
			let cue = if current {
				Sound::CurrentChannel
			} else {
				Sound::Message
			};
			if audible {
				if ui.notifications_enabled && !current {
					let image_path = state.user.as_ref().and_then(|user| {
						crate::avatars::notification_image_path(user.id, &notification.avatar_key)
					});
					alert = Some(Alert::Message {
						channel: notification.channel,
						title: notification.sender,
						body: notification.preview,
						avatar_key: notification.avatar_key,
						image_path,
					});
				}
				if options.allows(cue) {
					sound = Some(cue);
				}
			}
		}
		let incoming = state.voice.incoming.filter(|id| {
			audible && state.notification_allowed(*id) && options.allows(Sound::IncomingRing)
		});
		// Dialing is explicit local feedback, like mute/camera cues; DND only silences alerts.
		let outgoing = state
			.outgoing_ring()
			.filter(|_| live && options.allows(Sound::OutgoingRing));
		let ringing = incoming
			.map(|id| (id, Sound::IncomingRing))
			.or_else(|| outgoing.map(|id| (id, Sound::OutgoingRing)));
		if let Some(cue) = self.ring_cue(ringing, Instant::now()) {
			sound = Some(cue);
		}
		if self.ring.is_some() {
			ctx.request_repaint_after(Duration::from_millis(250));
		}
		if let Some(cue) = sound {
			push_local_cue(&mut self.local, cue, membership_volume(options, false));
		}
		// Call membership sounds are part of the call, including while Do Not Disturb is on.
		// Someone connecting or leaving the call you are in always makes a sound:
		// membership cues keep their own paced queue so a burst never collapses.
		while let Some(cue) = ui.notification_cues.first().copied() {
			ui.notification_cues.remove(0);
			let membership = matches!(cue, Sound::UserJoin | Sound::UserLeave);
			if membership {
				push_membership_cue(&mut self.membership, cue, membership_volume(options, true));
			} else if audible && options.allows(cue) {
				push_local_cue(&mut self.local, cue, membership_volume(options, false));
			}
		}
		if !ui.notification_cues.is_empty() {
			ctx.request_repaint();
		}
		// Explicit previews are allowed in the offline demo and intentionally ignore automatic mute choices.
		if let Some(preview) = ui.notification_preview.take() {
			push_local_cue(&mut self.local, preview, membership_volume(options, false));
		}
		let output = ui.voice_output.as_deref();
		if membership_due(self.membership_due, Instant::now())
			&& let Some((cue, volume)) = self.membership.pop_front()
		{
			if self.sounds.play(cue, volume, ctx, output) {
				self.membership_due = Some(Instant::now());
				self.membership_tries = 0;
			} else {
				self.membership.push_front((cue, volume));
				self.membership_tries = self.membership_tries.saturating_add(1);
				if self.membership_tries > 30 {
					self.membership.pop_front();
					self.membership_tries = 0;
				} else {
					ctx.request_repaint_after(Duration::from_millis(100));
				}
			}
		}
		if let Some((cue, volume)) = self.local.pop_front() {
			if self.sounds.play(cue, volume, ctx, output) {
				self.local_tries = 0;
			} else {
				self.local.push_front((cue, volume));
				self.local_tries = self.local_tries.saturating_add(1);
				if self.local_tries > 30 {
					eprintln!("Nivra: local cue unplayed after 3 s of worker pressure: {cue:?}");
					self.local.pop_front();
					self.local_tries = 0;
				} else {
					ctx.request_repaint_after(Duration::from_millis(100));
				}
			}
		}
		// Counts only change while frames run, so an idle window needs no badge timer: a
		// throttled frame schedules one follow-up recount, and then the window can sleep.
		let since = self.badge_check.map(|time| time.elapsed());
		if since.is_none_or(|since| since >= Duration::from_secs(1)) || !badges || !live {
			self.badge_check = Some(Instant::now());
			let pings = if live && badges {
				state
					.channels
					.iter()
					.try_fold(0u32, |total, channel| {
						if total >= 100 {
							return None;
						}
						Some(
							total
								.saturating_add(state.mention_count(channel.id))
								.min(100),
						)
					})
					.unwrap_or(100)
			} else {
				0
			};
			if platform::badge::supported() && self.badge != Some(pings) {
				self.badge_status = platform::badge::set(window, pings).err().unwrap_or("");
				self.badge = Some(pings);
			}
		} else if let Some(since) = since {
			ctx.request_repaint_after(Duration::from_secs(1).saturating_sub(since));
		}
		ui.notification_sound_status = if self.sounds.status().is_empty() {
			self.badge_status
		} else {
			self.sounds.status()
		};
		alert
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn membership_pacing_keeps_rapid_sequences_intelligible() {
		let now = Instant::now();
		assert!(membership_due(None, now));
		assert!(!membership_due(Some(now), now));
		assert!(membership_due(Some(now), now + MEMBERSHIP_GAP));
		assert!(!membership_due(
			Some(now),
			now + MEMBERSHIP_GAP - Duration::from_millis(1)
		));
	}

	#[test]
	fn cue_queues_keep_order_and_collapse_repeats() {
		let mut membership = VecDeque::new();
		push_membership_cue(&mut membership, Sound::UserLeave, 100);
		push_membership_cue(&mut membership, Sound::UserLeave, 100);
		push_membership_cue(&mut membership, Sound::UserJoin, 100);
		assert_eq!(
			membership.iter().map(|(cue, _)| *cue).collect::<Vec<_>>(),
			vec![Sound::UserLeave, Sound::UserJoin]
		);
		let mut local = VecDeque::new();
		push_local_cue(&mut local, Sound::Mute, 75);
		push_local_cue(&mut local, Sound::Mute, 75);
		push_local_cue(&mut local, Sound::Unmute, 75);
		assert_eq!(
			local.iter().map(|(cue, _)| *cue).collect::<Vec<_>>(),
			vec![Sound::Mute, Sound::Unmute]
		);
		for _ in 0..8 {
			push_local_cue(&mut local, Sound::Deafen, 75);
			push_local_cue(&mut local, Sound::Undeafen, 75);
		}
		assert_eq!(local.len(), MAX_LOCAL_CUES);
		assert_eq!(local.back().map(|(cue, _)| *cue), Some(Sound::Undeafen));
	}

	#[test]
	fn ringtone_timer_repeats_each_cue_and_stops_on_clear() {
		let mut runtime = Runtime::default();
		let now = Instant::now();
		let outgoing = Some((Id(2), Sound::OutgoingRing));
		assert_eq!(runtime.ring_cue(outgoing, now), Some(Sound::OutgoingRing));
		assert_eq!(
			runtime.ring_cue(outgoing, now + Duration::from_secs(2)),
			None
		);
		assert_eq!(
			runtime.ring_cue(outgoing, now + Duration::from_secs(3)),
			Some(Sound::OutgoingRing)
		);
		let incoming = Some((Id(2), Sound::IncomingRing));
		assert_eq!(
			runtime.ring_cue(incoming, now + Duration::from_secs(3)),
			Some(Sound::IncomingRing)
		);
		assert_eq!(
			runtime.ring_cue(incoming, now + Duration::from_secs(6)),
			None
		);
		assert_eq!(
			runtime.ring_cue(incoming, now + Duration::from_secs(9)),
			Some(Sound::IncomingRing)
		);
		assert_eq!(runtime.ring_cue(None, now + Duration::from_secs(9)), None);
		assert!(runtime.ring.is_none());
		assert_eq!(runtime.ring_cue(None, now + Duration::from_secs(15)), None);
	}
}
