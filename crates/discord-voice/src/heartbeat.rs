//! Heartbeat bookkeeping shared by the call and stream sockets.
//!
//! Discord echoes the nonce of every heartbeat. Under load an echo can arrive after the next
//! heartbeat has already been sent, so the ledger keeps a small window of unanswered nonces
//! instead of only the newest one. Echoes are accepted only for nonces this socket really sent;
//! anything else is a protocol violation and stays an error.

use std::collections::VecDeque;
use std::time::Duration;
use tokio::time::Instant;

/// Unanswered nonces kept at once. Older ones are treated as lost.
const WINDOW: usize = 8;
/// Consecutive heartbeat intervals without any echo before the socket counts as half-open.
pub const MISSED_LIMIT: u8 = 3;

/// What an echo meant for the ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Echo {
	/// A nonce this socket sent and had not yet settled. `rtt` is `None` when the nonce had
	/// already left the window, so its round trip is unknown.
	Settled { rtt: Option<Duration> },
	/// A nonce that was already settled. Harmless; it proves nothing new.
	Duplicate,
	/// Missing, malformed, zero, or a nonce that was never sent.
	Unknown,
}

#[derive(Debug, Default)]
pub struct Ledger {
	last: u64,
	acked: u64,
	pending: VecDeque<(u64, Instant)>,
	missed: u8,
}

impl Ledger {
	/// Registers a heartbeat about to be sent and returns its nonce.
	pub fn send(&mut self, now: Instant) -> u64 {
		self.last = self.last.wrapping_add(1);
		if self.pending.len() == WINDOW {
			self.pending.pop_front();
		}
		self.pending.push_back((self.last, now));
		self.last
	}

	/// Called once per heartbeat interval, before the next send. Returns `true` once the socket
	/// has stayed silent for `MISSED_LIMIT` intervals in a row.
	pub fn interval_elapsed(&mut self) -> bool {
		if !self.pending.is_empty() {
			self.missed = self.missed.saturating_add(1);
		}
		self.missed >= MISSED_LIMIT
	}

	/// Applies an echo. `nonce` is the raw `d.t` value from the acknowledgement.
	pub fn acknowledge(&mut self, nonce: Option<u64>, now: Instant) -> Echo {
		let Some(nonce) = nonce.filter(|nonce| *nonce >= 1 && *nonce <= self.last) else {
			return Echo::Unknown;
		};
		if nonce <= self.acked && !self.pending.iter().any(|(sent, _)| *sent == nonce) {
			return Echo::Duplicate;
		}
		let rtt = self
			.pending
			.iter()
			.find(|(sent, _)| *sent == nonce)
			.map(|(_, at)| now.saturating_duration_since(*at));
		self.pending.retain(|(sent, _)| *sent > nonce);
		self.acked = self.acked.max(nonce);
		// Any echo for a nonce we sent proves the socket is alive, even a late one.
		self.missed = 0;
		Echo::Settled { rtt }
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn current_echo_settles_with_round_trip() {
		let start = Instant::now();
		let mut ledger = Ledger::default();
		let nonce = ledger.send(start);
		let echo = ledger.acknowledge(Some(nonce), start + Duration::from_millis(70));
		assert_eq!(
			echo,
			Echo::Settled {
				rtt: Some(Duration::from_millis(70))
			}
		);
	}

	#[test]
	fn late_echo_of_an_earlier_nonce_is_tolerated() {
		let start = Instant::now();
		let mut ledger = Ledger::default();
		let first = ledger.send(start);
		let second = ledger.send(start + Duration::from_secs(5));
		let late = ledger.acknowledge(Some(first), start + Duration::from_secs(6));
		assert!(matches!(late, Echo::Settled { rtt: Some(_) }));
		// The newer heartbeat is still waiting for its own echo.
		let current = ledger.acknowledge(Some(second), start + Duration::from_secs(6));
		assert!(matches!(current, Echo::Settled { rtt: Some(_) }));
	}

	#[test]
	fn repeated_echo_is_a_duplicate_not_an_error() {
		let start = Instant::now();
		let mut ledger = Ledger::default();
		let nonce = ledger.send(start);
		ledger.acknowledge(Some(nonce), start);
		assert_eq!(ledger.acknowledge(Some(nonce), start), Echo::Duplicate);
	}

	#[test]
	fn never_sent_or_malformed_echoes_are_unknown() {
		let start = Instant::now();
		let mut ledger = Ledger::default();
		ledger.send(start);
		assert_eq!(ledger.acknowledge(Some(2), start), Echo::Unknown);
		assert_eq!(ledger.acknowledge(Some(0), start), Echo::Unknown);
		assert_eq!(ledger.acknowledge(None, start), Echo::Unknown);
	}

	#[test]
	fn echo_for_a_nonce_that_left_the_window_still_counts() {
		let start = Instant::now();
		let mut ledger = Ledger::default();
		let first = ledger.send(start);
		for _ in 0..WINDOW {
			ledger.send(start);
		}
		assert_eq!(
			ledger.acknowledge(Some(first), start),
			Echo::Settled { rtt: None }
		);
	}

	#[test]
	fn pending_window_is_bounded() {
		let start = Instant::now();
		let mut ledger = Ledger::default();
		for _ in 0..(WINDOW * 4) {
			ledger.send(start);
		}
		assert_eq!(ledger.pending.len(), WINDOW);
	}

	#[test]
	fn silence_for_the_limit_counts_as_half_open_and_any_echo_resets() {
		let start = Instant::now();
		let mut ledger = Ledger::default();
		ledger.send(start);
		assert!(!ledger.interval_elapsed());
		assert!(!ledger.interval_elapsed());
		assert!(ledger.interval_elapsed());
		ledger.acknowledge(Some(1), start);
		ledger.send(start);
		assert!(!ledger.interval_elapsed());
	}

	#[test]
	fn no_outstanding_heartbeat_is_not_silence() {
		let mut ledger = Ledger::default();
		for _ in 0..(MISSED_LIMIT * 2) {
			assert!(!ledger.interval_elapsed());
		}
	}
}
