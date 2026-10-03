//! Fixed 40ms startup with a bounded reorder window for 2.5?120ms Opus packets.
const SLOTS: usize = 8;
#[derive(Default)]
pub(crate) struct Jitter {
	packets: Vec<(u16, Vec<u8>)>,
	next: Option<u16>,
	wait: u8,
	missing: u8,
}
impl Jitter {
	pub fn clear(&mut self) {
		*self = Self::default();
	}
	pub fn push(&mut self, sequence: u16, opus: Vec<u8>) {
		if opus.len() > 1275 {
			return;
		}
		if self.next.is_none() {
			self.next = Some(sequence);
			self.wait = 2;
		}
		let distance = sequence.wrapping_sub(self.next.unwrap());
		if distance >= 32768 {
			return;
		}
		if distance >= SLOTS as u16 {
			self.packets.clear();
			self.next = Some(sequence);
			self.wait = 2;
		}
		if self.packets.len() < SLOTS && !self.packets.iter().any(|(id, _)| *id == sequence) {
			self.packets.push((sequence, opus));
		}
	}
	/// An empty packet requests Opus packet-loss concealment, at most three consecutive packets.
	pub fn pop(&mut self) -> Option<Vec<u8>> {
		if self.wait > 0 {
			// Short packets can fill the fixed window before 40ms. Start before
			// the next arrival would repeatedly reset a full window.
			if self.packets.len() < SLOTS {
				self.wait -= 1;
				return None;
			}
			self.wait = 0;
		}
		let next = self.next?;
		self.next = Some(next.wrapping_add(1));
		if let Some(index) = self.packets.iter().position(|(id, _)| *id == next) {
			self.missing = 0;
			return Some(self.packets.swap_remove(index).1);
		}
		self.missing += 1;
		if self.missing > 3 {
			if let Some((min_idx, _)) = self
				.packets
				.iter()
				.enumerate()
				.filter(|(_, (id, _))| id.wrapping_sub(next) < 32768)
				.min_by_key(|(_, (id, _))| id.wrapping_sub(next))
			{
				let (seq, data) = self.packets.swap_remove(min_idx);
				self.next = Some(seq.wrapping_add(1));
				self.missing = 0;
				return Some(data);
			}
			self.clear();
			return None;
		}
		Some(Vec::new())
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn bounded_reordering_loss_duplicates_and_wrap() {
		let mut jitter = Jitter::default();
		jitter.push(u16::MAX, vec![1]);
		jitter.push(1, vec![3]);
		jitter.push(0, vec![2]);
		jitter.push(0, vec![9]);
		assert!(jitter.pop().is_none());
		assert!(jitter.pop().is_none());
		assert_eq!(jitter.pop(), Some(vec![1]));
		assert_eq!(jitter.pop(), Some(vec![2]));
		assert_eq!(jitter.pop(), Some(vec![3]));
		for _ in 0..3 {
			assert_eq!(jitter.pop(), Some(vec![]));
		}
		assert!(jitter.pop().is_none());
		assert!(jitter.pop().is_none());
		for sequence in 0..1000 {
			jitter.push(sequence, vec![1; 1275]);
			assert!(jitter.packets.len() <= SLOTS);
		}
		jitter.push(1000, vec![0; 1276]);
		assert!(jitter.packets.len() <= SLOTS);
		// 15s at 20 ms is 750 packets. A stall that far ahead drops the old audio
		// instead of playing it in a burst; the next pop is the new packet.
		jitter.push(1750, vec![7]);
		assert_eq!(jitter.packets, vec![(1750, vec![7])]);
		assert!(jitter.pop().is_none());
		assert!(jitter.pop().is_none());
		assert_eq!(jitter.pop(), Some(vec![7]));
	}

	#[test]
	fn burst_loss_preserves_buffered_recovery_and_resets_on_far_jump() {
		let mut jitter = Jitter::default();
		jitter.push(0, vec![10]);
		jitter.push(5, vec![15]);
		jitter.push(6, vec![16]);
		assert!(jitter.pop().is_none());
		assert!(jitter.pop().is_none());
		assert_eq!(jitter.pop(), Some(vec![10]));
		// 3 packet loss concealment packets for 1, 2, 3
		assert_eq!(jitter.pop(), Some(vec![]));
		assert_eq!(jitter.pop(), Some(vec![]));
		assert_eq!(jitter.pop(), Some(vec![]));
		// Packet 4 is missing (missing > 3), but packets 5 and 6 were buffered:
		// instead of dropping them with clear(), it jumps to packet 5 and delivers it.
		assert_eq!(jitter.pop(), Some(vec![15]));
		assert_eq!(jitter.pop(), Some(vec![16]));
		// Once drained, 3 concealments then clear/None.
		assert_eq!(jitter.pop(), Some(vec![]));
		assert_eq!(jitter.pop(), Some(vec![]));
		assert_eq!(jitter.pop(), Some(vec![]));
		assert!(jitter.pop().is_none());
	}
}
