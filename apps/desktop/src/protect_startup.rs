//! Bounded startup ordering. Unknown or complete scope never silently opens a direct login.
use model::protect::Scope;
use std::time::{Duration, Instant};

const LOAD_LIMIT: Duration = Duration::from_secs(25);
// Includes the existing bounded preparation and transport-shutdown operations.
const RESTORE_LIMIT: Duration = Duration::from_secs(90);

#[derive(Clone, Copy, Debug)]
enum Phase {
	Off,
	Loading { deadline: Instant },
	WaitingForDiscord { scope: Scope },
	Restoring { scope: Scope, deadline: Instant },
	Blocked,
}

pub(crate) struct Restore {
	phase: Phase,
}
impl Restore {
	pub fn new(enabled: bool, now: Instant) -> Self {
		Self {
			phase: if enabled {
				Phase::Loading {
					deadline: now + LOAD_LIMIT,
				}
			} else {
				Phase::Off
			},
		}
	}
	pub fn loading(&self) -> bool {
		matches!(self.phase, Phase::Loading { .. })
	}
	pub fn restoring(&self) -> bool {
		matches!(self.phase, Phase::Restoring { .. })
	}
	pub fn blocks_login(&self) -> bool {
		match self.phase {
			Phase::Loading { .. } | Phase::Blocked => true,
			Phase::Restoring { scope, .. } => scope.protects_login(),
			Phase::Off | Phase::WaitingForDiscord { .. } => false,
		}
	}
	pub fn loaded(&mut self, scope: Option<Scope>, now: Instant) {
		self.phase = match scope {
			Some(scope) if scope.protects_login() => Phase::Restoring {
				scope,
				deadline: now + RESTORE_LIMIT,
			},
			Some(scope) => Phase::WaitingForDiscord { scope },
			None => Phase::Blocked,
		};
	}
	pub fn discord_ready(&mut self, ready: bool, now: Instant) {
		if ready && let Phase::WaitingForDiscord { scope } = self.phase {
			self.begin(scope, now);
		}
	}
	pub fn begin(&mut self, scope: Scope, now: Instant) {
		self.phase = Phase::Restoring {
			scope,
			deadline: now + RESTORE_LIMIT,
		};
	}
	pub fn finish(&mut self) {
		self.phase = Phase::Off;
	}
	pub fn fail(&mut self) {
		self.phase = if self.blocks_login() {
			Phase::Blocked
		} else {
			Phase::Off
		};
	}
	pub fn deadline(&self) -> Option<Instant> {
		match self.phase {
			Phase::Loading { deadline } | Phase::Restoring { deadline, .. } => Some(deadline),
			_ => None,
		}
	}
	pub fn expired(&self, now: Instant) -> bool {
		self.deadline().is_some_and(|deadline| now >= deadline)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn partial_scopes_wait_for_discord_then_restore_without_blocking_login() {
		let now = Instant::now();
		for scope in [Scope::Videos, Scope::Voice, Scope::Streams] {
			let mut restore = Restore::new(true, now);
			assert!(restore.blocks_login(), "the saved scope is not known yet");
			restore.loaded(Some(scope), now);
			assert!(!restore.blocks_login());
			restore.discord_ready(false, now + Duration::from_secs(600));
			assert!(!restore.restoring());
			assert!(
				!restore.expired(now + Duration::from_secs(600)),
				"offline Discord must not consume the VPN retry budget"
			);
			restore.discord_ready(true, now + Duration::from_secs(600));
			assert!(restore.restoring());
			assert!(!restore.blocks_login());
			restore.fail();
			assert!(
				!restore.blocks_login(),
				"partial failure does not log out Discord"
			);
		}
	}
	#[test]
	fn complete_and_unknown_fail_closed_until_an_explicit_success_or_disable() {
		let now = Instant::now();
		for scope in [None, Some(Scope::Complete)] {
			let mut restore = Restore::new(true, now);
			restore.loaded(scope, now);
			assert!(restore.blocks_login());
			restore.fail();
			assert!(restore.blocks_login());
			restore.discord_ready(true, now);
			assert!(
				!restore.restoring(),
				"a failed complete restore cannot start itself later"
			);
			restore.begin(Scope::Complete, now);
			assert!(restore.blocks_login());
			restore.finish();
			assert!(!restore.blocks_login());
		}
	}
	#[test]
	fn load_and_restore_deadlines_are_bounded_and_do_not_extend_on_each_poll() {
		let now = Instant::now();
		let mut restore = Restore::new(true, now);
		assert!(restore.expired(now + LOAD_LIMIT));
		restore.loaded(Some(Scope::Complete), now);
		for _ in 0..100 {
			restore.discord_ready(true, now + Duration::from_secs(80));
		}
		assert!(restore.expired(now + RESTORE_LIMIT));
		restore.fail();
		assert!(restore.blocks_login());
		assert!(restore.deadline().is_none(), "no busy loop after failure");
		assert!(!Restore::new(false, now).blocks_login());
	}
}
