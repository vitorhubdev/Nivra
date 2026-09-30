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
/// Max messages in one selection. Six through this cap is a download batch.
pub const MAX_SELECT: usize = 15;
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

/// Toggle one id, bounded by `MAX_SELECT`. Delete stays capped at `MAX_DELETE`.
pub fn toggle(selected: &mut BTreeSet<Id>, id: Id) -> bool {
	if selected.remove(&id) {
		return true;
	}
	if selected.len() >= MAX_SELECT {
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
		if selected.len() >= MAX_SELECT {
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
	} else if selected_count > MAX_DELETE {
		Some("You can delete up to 5 at a time")
	} else if deletable_count != selected_count {
		Some("Only your messages can be deleted here")
	} else {
		None
	}
}

/// Reason when download is disabled; `None` means enabled.
pub fn download_disabled_reason(attachment_count: usize) -> Option<&'static str> {
	if attachment_count == 0 {
		Some("No attachments in the selection")
	} else if attachment_count > MAX_DOWNLOAD {
		Some("You can download up to 15 attachments at a time")
	} else {
		None
	}
}

/// One message for `.txt` / `.md` export, in timeline order.
pub struct TxtMessage {
	pub author: String,
	pub when: String,
	pub text: String,
	pub attachments: Vec<String>,
	/// Same length as `attachments` when a URL is known; missing entries stay names.
	pub links: Vec<Option<String>>,
}

/// One line per message, in the order given: `Author — hh:mm: text`.
pub fn format_txt(messages: &[TxtMessage]) -> String {
	let mut out = String::new();
	for m in messages {
		out.push_str(&m.author);
		out.push_str(" — ");
		out.push_str(&m.when);
		out.push_str(": ");
		out.push_str(&m.text);
		write_txt_files(&mut out, m);
		out.push('\n');
	}
	out
}

fn write_txt_files(out: &mut String, message: &TxtMessage) {
	if message.attachments.is_empty() {
		return;
	}
	out.push_str(" · ");
	for (index, name) in message.attachments.iter().enumerate() {
		if index > 0 {
			out.push_str(", ");
		}
		out.push_str(name);
		if let Some(url) = message
			.links
			.get(index)
			.and_then(|url| url.as_deref())
			.filter(|url| !url.is_empty())
		{
			out.push(' ');
			out.push_str(url);
		}
	}
}

/// Markdown for the same messages: author, time, text, and attachment links.
pub fn format_md(messages: &[TxtMessage]) -> String {
	let mut out = String::new();
	for message in messages {
		out.push_str("**");
		out.push_str(&message.author.replace('*', ""));
		out.push_str("** — ");
		out.push_str(&message.when);
		out.push_str("\n\n");
		out.push_str(&message.text);
		out.push('\n');
		for (index, name) in message.attachments.iter().enumerate() {
			out.push_str("\n- ");
			let url = message
				.links
				.get(index)
				.and_then(|url| url.as_deref())
				.filter(|url| !url.is_empty());
			if let Some(url) = url {
				out.push('[');
				out.push_str(&name.replace(['[', ']'], ""));
				out.push_str("](");
				out.push_str(&url.replace(')', "%29"));
				out.push(')');
			} else {
				out.push_str(name);
			}
		}
		out.push_str("\n\n");
	}
	out
}

/// Formats a loaded history a chunk at a time so a frame does not walk every message.
pub struct ExportJob {
	rest: Vec<TxtMessage>,
	done: usize,
	total: usize,
	markdown: bool,
	out: String,
	/// Next index in the open timeline. None once every loaded message is copied.
	capture_at: Option<usize>,
}

impl ExportJob {
	pub const CHUNK: usize = 40;

	pub fn start(messages: Vec<TxtMessage>, markdown: bool) -> Self {
		let total = messages.len();
		Self {
			rest: messages,
			done: 0,
			total,
			markdown,
			out: String::new(),
			capture_at: None,
		}
	}

	/// Copies the open timeline one chunk at a time. Nothing is cloned here.
	pub fn open(markdown: bool) -> Self {
		Self {
			rest: Vec::new(),
			done: 0,
			total: 0,
			markdown,
			out: String::new(),
			capture_at: Some(0),
		}
	}

	pub fn capturing(&self) -> bool {
		self.capture_at.is_some()
	}

	pub fn capture_index(&self) -> usize {
		self.capture_at.unwrap_or(0)
	}

	/// Stores one copied chunk. `more` means the timeline still has later messages.
	pub fn store_captured(&mut self, rows: Vec<TxtMessage>, more: bool) {
		let added = rows.len();
		self.total += added;
		self.rest.extend(rows);
		self.capture_at = more.then_some(self.capture_index() + added);
	}

	pub fn markdown(&self) -> bool {
		self.markdown
	}

	pub fn progress(&self) -> (usize, usize) {
		(self.done, self.total)
	}

	/// Appends one chunk. `true` when nothing remains.
	pub fn step(&mut self) -> bool {
		let count = self.rest.len().min(Self::CHUNK);
		let chunk: Vec<TxtMessage> = self.rest.drain(..count).collect();
		if self.markdown {
			self.out.push_str(&format_md(&chunk));
		} else {
			self.out.push_str(&format_txt(&chunk));
		}
		self.done += count;
		self.rest.is_empty()
	}

	pub fn take(self) -> String {
		self.out
	}
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

/// `photo.png` at 0, then `photo (1).png` … `photo (99).png`.
pub fn filename_with_index(filename: &str, index: usize) -> String {
	if index == 0 {
		return filename.to_owned();
	}
	let (stem, extension) = match filename.rsplit_once('.') {
		Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => {
			(stem, Some(extension))
		}
		_ => (filename, None),
	};
	match extension {
		Some(extension) => format!("{stem} ({index}).{extension}"),
		None => format!("{stem} ({index})"),
	}
}

/// First free name from the original through ` (99)`, using `taken`.
pub fn first_free_filename(filename: &str, mut taken: impl FnMut(&str) -> bool) -> Option<String> {
	for index in 0..=99 {
		let named = filename_with_index(filename, index);
		if !taken(&named) {
			return Some(named);
		}
	}
	None
}

/// 0..1 removal progress over `REMOVAL_SECS`; height and opacity scale with `1-t`.
pub fn removal_progress(elapsed: f32) -> f32 {
	crate::anim::progress(elapsed, REMOVAL_SECS)
}

/// Scroll compensation while removing: keep the viewport stable as height shrinks.
pub fn removal_height(base: f32, elapsed: f32) -> f32 {
	(base * (1.0 - removal_progress(elapsed))).max(0.0)
}

/// Adds visible ids in order until the selection cap. Returns how many were added.
pub fn select_all_visible(ordered: &[Id], selected: &mut BTreeSet<Id>) -> usize {
	let mut added = 0;
	for id in ordered {
		if selected.len() >= MAX_SELECT {
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

/// First row that survives `removed` and still meets the viewport.
/// The inset is how far into that row the current scroll sits, so one
/// layout pass can keep it still instead of stepping once per delete.
pub fn stable_scroll_anchor(
	rows: &[(Id, f32)],
	scroll: f32,
	removed: &BTreeSet<Id>,
) -> Option<(Id, f32)> {
	let mut y = 0.0;
	for (id, height) in rows {
		let bottom = y + *height;
		if !removed.contains(id) && bottom > scroll + 0.5 {
			let inset = (scroll - y).max(0.0).min((*height).max(0.0));
			return Some((*id, inset));
		}
		y = bottom;
	}
	None
}

/// Scroll offset that places `anchor` at the same inset after `removed` rows are gone.
pub fn offset_keeping_anchor(rows: &[(Id, f32)], anchor: Id, inset: f32) -> f32 {
	let mut y = 0.0;
	for (id, height) in rows {
		if *id == anchor {
			return y + inset.min((*height).max(0.0));
		}
		y += *height;
	}
	y
}

/// Per-file manager state. Transfers run on the desktop; this is the rendered snapshot.
#[derive(Clone)]
pub struct BatchFileView {
	pub name: String,
	pub received: u64,
	pub total: u64,
	pub status: BatchFileStatus,
	pub error: Option<&'static str>,
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
	fn group_removal_anchors_once_instead_of_stepping_per_echo() {
		let rows = vec![
			(Id(1), 100.0),
			(Id(2), 100.0),
			(Id(3), 100.0),
			(Id(4), 100.0),
			(Id(5), 100.0),
		];
		let scroll = 150.0;
		let removed = BTreeSet::from([Id(2), Id(3)]);
		let (anchor, inset) = stable_scroll_anchor(&rows, scroll, &removed).unwrap();
		assert_eq!(anchor, Id(4));
		assert!(!removed.contains(&anchor));
		let kept: Vec<_> = rows
			.iter()
			.copied()
			.filter(|(id, _)| !removed.contains(id))
			.collect();
		let after = offset_keeping_anchor(&kept, anchor, inset);
		// One collapse: the viewport moves by the deleted height above the
		// survivor, not by each echoed row (100 + 100) added onto the scroll.
		assert_eq!(after, 100.0);
		let per_echo = scroll + 100.0 + 100.0;
		assert!(per_echo - scroll > (scroll - after).abs());

		let tail = BTreeSet::from([Id(5)]);
		let (anchor, inset) = stable_scroll_anchor(&rows, scroll, &tail).unwrap();
		assert_eq!(anchor, Id(2));
		let kept: Vec<_> = rows
			.iter()
			.copied()
			.filter(|(id, _)| !tail.contains(id))
			.collect();
		assert_eq!(offset_keeping_anchor(&kept, anchor, inset), scroll);
	}

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
		let ordered: Vec<Id> = (1..=20).map(Id).collect();
		let mut sel = BTreeSet::new();
		let added = range_select(&ordered, Some(Id(2)), Id(5), &mut sel);
		assert_eq!(added, 4);
		assert!(sel.contains(&Id(2)) && sel.contains(&Id(5)));
		let added2 = range_select(&ordered, Some(Id(1)), Id(20), &mut sel);
		assert_eq!(added2, MAX_SELECT - 4);
		assert_eq!(sel.len(), MAX_SELECT);
	}

	#[test]
	fn delete_and_download_limits_with_reasons() {
		let mut sel = BTreeSet::new();
		for i in 1..=15u64 {
			assert!(toggle(&mut sel, Id(i)));
		}
		assert!(!toggle(&mut sel, Id(99)));
		assert_eq!(sel.len(), MAX_SELECT);
		assert_eq!(
			delete_disabled_reason(6, 6),
			Some("You can delete up to 5 at a time")
		);
		assert_eq!(
			delete_disabled_reason(0, 5),
			Some("Only your messages can be deleted here")
		);
		assert_eq!(
			delete_disabled_reason(5, 6),
			Some("You can delete up to 5 at a time")
		);
		assert_eq!(
			delete_disabled_reason(2, 3),
			Some("Only your messages can be deleted here")
		);
		assert_eq!(delete_disabled_reason(3, 3), None);
		assert_eq!(
			download_disabled_reason(0),
			Some("No attachments in the selection")
		);
		assert_eq!(
			download_disabled_reason(16),
			Some("You can download up to 15 attachments at a time")
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
				links: vec![Some("https://cdn.example/b.png".into())],
			},
			TxtMessage {
				author: "A".into(),
				when: "10:00".into(),
				text: "first".into(),
				attachments: vec![],
				links: vec![],
			},
		];
		let out = format_txt(&msgs);
		assert!(out.find("B — 10:01: second").unwrap() < out.find("A — 10:00: first").unwrap());
		assert!(out.contains("b.png https://cdn.example/b.png"));
		assert!(!out.contains('['));
		let md = format_md(&msgs);
		assert!(md.contains("**B** — 10:01"));
		assert!(md.contains("[b.png](https://cdn.example/b.png)"));
		assert!(md.find("second").unwrap() < md.find("first").unwrap());
	}

	#[test]
	fn export_job_formats_in_chunks() {
		let messages: Vec<TxtMessage> = (0..41)
			.map(|index| TxtMessage {
				author: "A".into(),
				when: "12:00".into(),
				text: format!("m{index}"),
				attachments: vec![],
				links: vec![],
			})
			.collect();
		let expected = format_txt(&messages);
		let mut job = ExportJob::start(messages, false);
		assert!(!job.step());
		assert_eq!(job.progress(), (40, 41));
		assert!(job.step());
		assert_eq!(job.take(), expected);
		let mut open = ExportJob::open(false);
		assert!(open.capturing());
		assert_eq!(open.capture_index(), 0);
		open.store_captured(
			(0..ExportJob::CHUNK)
				.map(|index| TxtMessage {
					author: "A".into(),
					when: "12:00".into(),
					text: format!("c{index}"),
					attachments: vec![],
					links: vec![],
				})
				.collect(),
			true,
		);
		assert!(open.capturing());
		assert_eq!(open.capture_index(), ExportJob::CHUNK);
		open.store_captured(Vec::new(), false);
		assert!(!open.capturing());
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
	fn repeated_download_names_use_parenthetical_index() {
		assert_eq!(filename_with_index("photo.png", 0), "photo.png");
		assert_eq!(filename_with_index("photo.png", 1), "photo (1).png");
		assert_eq!(filename_with_index("photo.png", 99), "photo (99).png");
		assert_eq!(filename_with_index("notes", 2), "notes (2)");
		let mut taken = vec!["photo.png".to_owned(), "photo (1).png".to_owned()];
		let next = first_free_filename("photo.png", |name| taken.iter().any(|item| item == name));
		assert_eq!(next.as_deref(), Some("photo (2).png"));
		taken.push(next.unwrap());
		assert_eq!(
			first_free_filename("photo.png", |name| taken.iter().any(|item| item == name))
				.as_deref(),
			Some("photo (3).png")
		);
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
	fn select_all_visible_fills_in_order_up_to_select_cap() {
		let ordered: Vec<Id> = (1..=20).map(Id).collect();
		let mut sel = BTreeSet::new();
		sel.insert(Id(3));
		let added = select_all_visible(&ordered, &mut sel);
		assert_eq!(added, MAX_SELECT - 1);
		assert_eq!(sel.len(), MAX_SELECT);
		assert!(sel.contains(&Id(9)));
		let added2 = select_all_visible(&ordered, &mut sel);
		assert_eq!(added2, 0);
	}
}
