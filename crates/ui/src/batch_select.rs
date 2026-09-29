//! Smart selection for batch actions: reserved column, ranges and limits.
//!
//! Pure logic behind Item 4. UI in `timeline.rs` keeps the 36px column always
//! reserved and fades boxes via `anim`; this module owns limits, Shift ranges,
//! `.txt` ordering, sequential download queue (mock, no network) and the
//! 180ms removal progress.
use model::Id;
use std::collections::BTreeSet;

/// Max messages per delete action. Matches `timeline.rs`.
pub const MAX_DELETE: usize = 5;
/// Max attachments per download action.
pub const MAX_DOWNLOAD: usize = 15;
/// Removal animation: height and opacity to 0.
pub const REMOVAL_SECS: f32 = 0.18;
/// Selection column is always reserved so rows never jump.
pub const SELECT_COL_WIDTH: f32 = 36.0;

/// Width reserved for the selection column, independent of mode.
pub fn reserved_width(_mode_on: bool) -> f32 {
	SELECT_COL_WIDTH
}

/// Toggle one id, bounded by `MAX_DELETE`.
pub fn toggle(selected: &mut BTreeSet<Id>, id: Id) -> bool {
	if selected.remove(&id) {
		return true;
	}
	if selected.len() >= MAX_DELETE {
		return false;
	}
	selected.insert(id);
	true
}

/// Shift+click range over `ordered` from `anchor` to `target`, bounded.
pub fn range_select(
	ordered: &[Id],
	anchor: Option<Id>,
	target: Id,
	selected: &mut BTreeSet<Id>,
) -> usize {
	let Some(anchor) = anchor else {
		return if toggle(selected, target) { 1 } else { 0 };
	};
	let ai = ordered.iter().position(|id| *id == anchor);
	let ti = ordered.iter().position(|id| *id == target);
	let (Some(ai), Some(ti)) = (ai, ti) else {
		return if toggle(selected, target) { 1 } else { 0 };
	};
	let (lo, hi) = if ai <= ti { (ai, ti) } else { (ti, ai) };
	let mut added = 0;
	for id in &ordered[lo..=hi] {
		if selected.contains(id) {
			continue;
		}
		if selected.len() >= MAX_DELETE {
			break;
		}
		selected.insert(*id);
		added += 1;
	}
	added
}

/// Only what the user may delete, in order, up to `MAX_DELETE`.
pub fn deletable(selected: &BTreeSet<Id>, can_delete: &dyn Fn(Id) -> bool) -> Vec<Id> {
	selected
		.iter()
		.copied()
		.filter(|id| can_delete(*id))
		.take(MAX_DELETE)
		.collect()
}

/// Reason when delete is disabled; `None` means enabled. Never hides the button.
pub fn delete_disabled_reason(
	deletable_count: usize,
	selected_count: usize,
) -> Option<&'static str> {
	if selected_count == 0 {
		Some("Select messages to enable actions")
	} else if deletable_count == 0 {
		Some("None of the selected messages can be deleted")
	} else if selected_count > MAX_DELETE {
		Some("Maximum 5 messages per delete")
	} else {
		None
	}
}

/// Reason when download is disabled; `None` means enabled.
pub fn download_disabled_reason(attachment_count: usize) -> Option<&'static str> {
	if attachment_count == 0 {
		Some("No attachments in the selection")
	} else if attachment_count > MAX_DOWNLOAD {
		Some("Maximum 15 attachments per download")
	} else {
		None
	}
}

/// One message for `.txt` export, in timeline order.
pub struct TxtMessage {
	pub author: String,
	pub when: String,
	pub text: String,
	pub attachments: Vec<String>,
}

/// Formats messages in order as `Author [when]: text`, attachments listed by name.
pub fn format_txt(messages: &[TxtMessage]) -> String {
	let mut out = String::new();
	for m in messages {
		out.push_str(&format!("{} [{}]: {}\n", m.author, m.when, m.text));
		if !m.attachments.is_empty() {
			out.push_str(&format!("Attachments: {}\n", m.attachments.join(", ")));
		}
	}
	out
}

/// Sequential download queue (mock, no network): one at a time, retry failed.
#[derive(Default)]
pub struct DownloadQueue {
	items: Vec<u64>,
	index: usize,
	done: Vec<u64>,
	failed: Vec<u64>,
	cancelled: bool,
}
impl DownloadQueue {
	pub fn start(items: Vec<u64>) -> Self {
		Self {
			items,
			index: 0,
			done: Vec::new(),
			failed: Vec::new(),
			cancelled: false,
		}
	}
	#[allow(clippy::should_implement_trait)] // Queue cursor, not an iterator.
	pub fn next(&mut self) -> Option<u64> {
		if self.cancelled {
			return None;
		}
		if self.index < self.items.len() {
			let id = self.items[self.index];
			self.index += 1;
			Some(id)
		} else {
			None
		}
	}
	pub fn complete(&mut self, id: u64, success: bool) {
		if success {
			self.done.push(id);
		} else {
			self.failed.push(id);
		}
	}
	pub fn retry_failed(&mut self) {
		let retry = std::mem::take(&mut self.failed);
		for id in retry {
			self.items.push(id);
		}
	}
	pub fn cancel(&mut self) {
		self.cancelled = true;
	}
	pub fn done(&self) -> &[u64] {
		&self.done
	}
	pub fn failed(&self) -> &[u64] {
		&self.failed
	}
	pub fn is_done(&self) -> bool {
		!self.cancelled && self.index >= self.items.len() && self.failed.is_empty()
	}
	pub fn progress(&self) -> (usize, usize) {
		(self.done.len(), self.items.len())
	}
}

/// 0..1 removal progress over `REMOVAL_SECS`; height and opacity scale with `1-t`.
pub fn removal_progress(elapsed: f32) -> f32 {
	crate::anim::progress(elapsed, REMOVAL_SECS)
}

/// Scroll compensation while removing: keep the viewport stable as height shrinks.
pub fn removal_height(base: f32, elapsed: f32) -> f32 {
	(base * (1.0 - removal_progress(elapsed))).max(0.0)
}

/// Adds visible ids in order until the delete cap. Returns how many were added.
pub fn select_all_visible(ordered: &[Id], selected: &mut BTreeSet<Id>) -> usize {
	let mut added = 0;
	for id in ordered {
		if selected.len() >= MAX_DELETE {
			break;
		}
		if selected.insert(*id) {
			added += 1;
		}
	}
	added
}

/// Hold a fully-faded row invisible until the server echoes the delete.
/// Past this the row reappears instead of vanishing silently on a failed delete.
pub const REMOVAL_HOLD_SECS: f64 = 5.0;

/// Per-file manager state. Transfers run on the desktop; this is the rendered snapshot.
#[derive(Clone)]
pub struct BatchFileView {
	pub name: String,
	pub received: u64,
	pub total: u64,
	pub status: BatchFileStatus,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BatchFileStatus {
	Queued,
	Active,
	Done,
	Failed,
	Cancelled,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn entering_mode_keeps_row_size() {
		assert_eq!(
			reserved_width(false),
			SELECT_COL_WIDTH,
			"column always reserved"
		);
		assert_eq!(reserved_width(true), SELECT_COL_WIDTH);
	}

	#[test]
	fn shift_range_selects_in_order_with_limit() {
		let ordered = vec![Id(1), Id(2), Id(3), Id(4), Id(5), Id(6), Id(7)];
		let mut sel = BTreeSet::new();
		let added = range_select(&ordered, Some(Id(2)), Id(5), &mut sel);
		assert_eq!(added, 4);
		assert!(sel.contains(&Id(2)) && sel.contains(&Id(5)));
		let added2 = range_select(&ordered, Some(Id(1)), Id(7), &mut sel);
		assert!(added2 <= MAX_DELETE);
		assert!(sel.len() <= MAX_DELETE);
	}

	#[test]
	fn delete_and_download_limits_with_reasons() {
		let mut sel = BTreeSet::new();
		for i in 1..=5u64 {
			assert!(toggle(&mut sel, Id(i)));
		}
		assert!(!toggle(&mut sel, Id(99)));
		assert_eq!(
			delete_disabled_reason(0, 5),
			Some("None of the selected messages can be deleted")
		);
		assert_eq!(
			delete_disabled_reason(5, 6),
			Some("Maximum 5 messages per delete")
		);
		assert_eq!(delete_disabled_reason(3, 3), None);
		assert_eq!(
			download_disabled_reason(0),
			Some("No attachments in the selection")
		);
		assert_eq!(
			download_disabled_reason(16),
			Some("Maximum 15 attachments per download")
		);
		assert_eq!(download_disabled_reason(15), None);
	}

	#[test]
	fn txt_keeps_order_and_lists_attachments() {
		let msgs = vec![
			TxtMessage {
				author: "B".into(),
				when: "10:01".into(),
				text: "second".into(),
				attachments: vec!["b.png".into()],
			},
			TxtMessage {
				author: "A".into(),
				when: "10:00".into(),
				text: "first".into(),
				attachments: vec![],
			},
		];
		let out = format_txt(&msgs);
		assert!(out.find("B [10:01]: second").unwrap() < out.find("A [10:00]: first").unwrap());
		assert!(out.contains("b.png"));
	}

	#[test]
	fn download_queue_is_sequential_with_retry() {
		let mut q = DownloadQueue::start(vec![1, 2, 3]);
		assert_eq!(q.next(), Some(1));
		q.complete(1, true);
		assert_eq!(q.next(), Some(2));
		q.complete(2, false);
		assert_eq!(q.next(), Some(3));
		q.complete(3, true);
		assert_eq!(q.next(), None);
		assert_eq!(q.failed(), &[2]);
		q.retry_failed();
		assert_eq!(q.next(), Some(2));
		q.complete(2, true);
		assert!(q.is_done());
		assert_eq!(q.progress(), (3, 4));
	}

	#[test]
	fn removal_animation_ends() {
		assert_eq!(removal_progress(0.0), 0.0);
		assert_eq!(removal_progress(10.0), 1.0);
		assert_eq!(removal_height(20.0, 10.0), 0.0);
		assert!(removal_height(20.0, 0.0) > 19.0);
		assert!(REMOVAL_HOLD_SECS > REMOVAL_SECS as f64);
	}

	#[test]
	fn select_all_visible_fills_in_order_up_to_delete_cap() {
		let ordered = vec![Id(9), Id(3), Id(7), Id(1), Id(5), Id(2), Id(8)];
		let mut sel = BTreeSet::new();
		sel.insert(Id(3));
		let added = select_all_visible(&ordered, &mut sel);
		assert_eq!(added, MAX_DELETE - 1);
		assert_eq!(sel.len(), MAX_DELETE);
		assert!(sel.contains(&Id(9)));
		let added2 = select_all_visible(&ordered, &mut sel);
		assert_eq!(added2, 0);
	}
}
