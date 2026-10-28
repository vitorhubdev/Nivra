//! Pace normal microphone callback batches; discard excessive stale PCM after clock drift.
use crate::Frame;
use std::{collections::VecDeque, sync::mpsc::Receiver};

pub(crate) struct CapturePacer {
	// Eight bounded channel frames plus eight preallocated local slots. Only a
	// queue above four frames triggers catch-up; ordinary two-frame batches stay intact.
	pending: VecDeque<Frame>,
	dropped: u64,
}
impl Default for CapturePacer {
	fn default() -> Self {
		Self {
			pending: VecDeque::with_capacity(8),
			dropped: 0,
		}
	}
}
impl CapturePacer {
	pub fn take_dropped(&mut self) -> u64 {
		std::mem::take(&mut self.dropped)
	}
	/// Local detection while alone: consume audio without retaining it for transmission.
	pub fn preview(&mut self, input: &Receiver<Frame>) -> Option<Frame> {
		self.pending.clear();
		let mut latest = None;
		for _ in 0..8 {
			let Ok(frame) = input.try_recv() else { break };
			latest = Some(frame);
		}
		latest
	}
	pub fn next(&mut self, input: &Receiver<Frame>, enabled: bool, stalled: bool) -> Option<Frame> {
		if !enabled || stalled {
			self.pending.clear();
			for _ in 0..8 {
				if input.try_recv().is_err() {
					break;
				}
			}
			return None;
		}
		let frame = self.pending.pop_front();
		for _ in 0..8 {
			let Ok(frame) = input.try_recv() else { break };
			if self.pending.len() == 8 {
				self.pending.pop_front();
				self.dropped += 1;
			}
			self.pending.push_back(frame);
		}
		if self.pending.len() > 4 {
			self.dropped += u64::from(frame.is_some());
			while self.pending.len() > 2 {
				self.pending.pop_front();
				self.dropped += 1;
			}
			return self.pending.pop_front();
		}
		frame
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn normal_batched_capture_keeps_every_frame_in_order() {
		let (send, receive) = std::sync::mpsc::sync_channel(8);
		let mut pacer = CapturePacer::default();
		for tick in 0..100 {
			if tick % 2 == 0 {
				for id in tick..tick + 2 {
					send.try_send([id as f32; 960]).unwrap();
				}
			}
			let frame = pacer.next(&receive, true, false);
			if tick == 0 {
				assert!(frame.is_none());
			} else {
				assert_eq!(frame.unwrap(), [(tick - 1) as f32; 960]);
			}
		}
		assert_eq!(pacer.next(&receive, true, false).unwrap(), [99.0; 960]);
		assert_eq!(pacer.take_dropped(), 0);
	}
	#[test]
	fn delayed_capture_returns_near_live_without_growing_or_replaying_muted_audio() {
		let (send, receive) = std::sync::mpsc::sync_channel(8);
		let mut pacer = CapturePacer::default();
		for id in 0..8 {
			send.try_send([id as f32; 960]).unwrap();
		}
		assert_eq!(pacer.next(&receive, true, false).unwrap(), [6.0; 960]);
		assert_eq!(pacer.take_dropped(), 6);
		assert!(pacer.next(&receive, false, false).is_none());
		assert!(pacer.next(&receive, true, false).is_none());
		assert_eq!(pacer.pending.capacity(), 8);
	}
}
