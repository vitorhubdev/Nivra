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
pub const MAX_SELECT: usize = 25;
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
	if !messages.is_empty() {
		let digest = sha256_hex(out.as_bytes());
		out.push_str("---\nExported with Nivra client · SHA-256: `");
		out.push_str(&digest);
		out.push_str("`\n");
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

/// Classification of a message's content for smart batch selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MessageContentKind {
	TextOnly,
	MediaOnly,
	Mixed,
	FileOnly,
	Empty,
}

/// Dynamic summary of currently selected messages.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectionSummary {
	pub total_selected: usize,
	pub text_count: usize,
	pub media_count: usize,
	pub file_count: usize,
	pub deletable_count: usize,
}

impl SelectionSummary {
	pub fn is_only_media(&self) -> bool {
		self.media_count > 0 && self.text_count == 0 && self.file_count == 0
	}

	pub fn is_only_text(&self) -> bool {
		self.text_count > 0 && self.media_count == 0 && self.file_count == 0
	}

	pub fn is_mixed(&self) -> bool {
		(self.text_count > 0 && (self.media_count > 0 || self.file_count > 0))
			|| (self.media_count > 0 && self.file_count > 0)
	}
}

/// Categorize a single message into its content kind.
pub fn classify_message(
	content: &str,
	attachments: &[model::Attachment],
	embeds: &[model::Embed],
) -> MessageContentKind {
	let has_text = !content.trim().is_empty();
	let mut has_media = false;
	let mut has_other_file = false;

	for attachment in attachments {
		if attachment.is_image() || attachment.is_video() {
			has_media = true;
		} else {
			has_other_file = true;
		}
	}

	for embed in embeds {
		if embed.image.is_some() || embed.video.is_some() || embed.thumbnail.is_some() {
			has_media = true;
		}
	}

	match (has_text, has_media, has_other_file) {
		(true, false, false) => MessageContentKind::TextOnly,
		(false, true, false) => MessageContentKind::MediaOnly,
		(false, false, true) => MessageContentKind::FileOnly,
		(false, false, false) => MessageContentKind::Empty,
		_ => MessageContentKind::Mixed,
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QuickFilter {
	Last(usize),
	AllVisible,
	OnlyText,
	OnlyImages,
	OnlyVideos,
	Media,
	Files,
	OnlyMine,
}

pub type MessageLookupFn<'a> =
	dyn Fn(Id) -> Option<(String, Vec<model::Attachment>, Vec<model::Embed>, Id)> + 'a;

pub fn apply_quick_filter(
	ordered: &[Id],
	filter: QuickFilter,
	selected: &mut BTreeSet<Id>,
	own_user: Option<Id>,
	lookup: &MessageLookupFn<'_>,
) -> usize {
	selected.clear();
	let mut added = 0;

	match filter {
		QuickFilter::Last(count) => {
			for id in ordered.iter().rev().take(count) {
				if selected.len() >= MAX_SELECT {
					break;
				}
				if selected.insert(*id) {
					added += 1;
				}
			}
		}
		QuickFilter::AllVisible => {
			for id in ordered {
				if selected.len() >= MAX_SELECT {
					break;
				}
				if selected.insert(*id) {
					added += 1;
				}
			}
		}
		QuickFilter::OnlyText => {
			for id in ordered.iter().rev() {
				if selected.len() >= MAX_SELECT {
					break;
				}
				if let Some((content, attachments, embeds, _author)) = lookup(*id)
					&& classify_message(&content, &attachments, &embeds)
						== MessageContentKind::TextOnly
					&& selected.insert(*id)
				{
					added += 1;
				}
			}
		}
		QuickFilter::OnlyImages => {
			for id in ordered.iter().rev() {
				if selected.len() >= MAX_SELECT {
					break;
				}
				if let Some((_, attachments, embeds, _author)) = lookup(*id) {
					let has_img = attachments.iter().any(|a| a.is_image())
						|| embeds
							.iter()
							.any(|e| e.image.is_some() || e.thumbnail.is_some());
					if has_img && selected.insert(*id) {
						added += 1;
					}
				}
			}
		}
		QuickFilter::OnlyVideos => {
			for id in ordered.iter().rev() {
				if selected.len() >= MAX_SELECT {
					break;
				}
				if let Some((_, attachments, embeds, _author)) = lookup(*id) {
					let has_vid = attachments.iter().any(|a| a.is_video())
						|| embeds.iter().any(|e| e.video.is_some());
					if has_vid && selected.insert(*id) {
						added += 1;
					}
				}
			}
		}
		QuickFilter::Media => {
			for id in ordered.iter().rev() {
				if selected.len() >= MAX_SELECT {
					break;
				}
				if let Some((_, attachments, embeds, _author)) = lookup(*id) {
					let has_media = attachments.iter().any(|a| a.is_image() || a.is_video())
						|| embeds.iter().any(|e| {
							e.image.is_some() || e.video.is_some() || e.thumbnail.is_some()
						});
					if has_media && selected.insert(*id) {
						added += 1;
					}
				}
			}
		}
		QuickFilter::Files => {
			for id in ordered.iter().rev() {
				if selected.len() >= MAX_SELECT {
					break;
				}
				if let Some((_, attachments, _, _author)) = lookup(*id) {
					let has_file = attachments.iter().any(|a| !a.is_image() && !a.is_video());
					if has_file && selected.insert(*id) {
						added += 1;
					}
				}
			}
		}
		QuickFilter::OnlyMine => {
			if let Some(own) = own_user {
				for id in ordered.iter().rev() {
					if selected.len() >= MAX_SELECT {
						break;
					}
					if let Some((_, _, _, author)) = lookup(*id)
						&& author == own && selected.insert(*id)
					{
						added += 1;
					}
				}
			}
		}
	}
	added
}

/// Formats messages as a self-contained, responsive HTML file in Telegram/Discord export style.
pub fn format_html(channel_name: &str, messages: &[TxtMessage]) -> String {
	let mut out = String::with_capacity(messages.len() * 512 + 1024);
	out.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
	out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n");
	out.push_str("<title>Nivra Export - ");
	out.push_str(&html_escape(channel_name));
	out.push_str("</title>\n<style>\n");
	out.push_str("body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif; margin: 0; padding: 24px; background: #1e1f22; color: #dbdee1; }\n");
	out.push_str(".export-container { max-width: 760px; margin: 0 auto; }\n");
	out.push_str(
		".header { border-bottom: 1px solid #35363c; padding-bottom: 16px; margin-bottom: 24px; }\n",
	);
	out.push_str(".header h1 { margin: 0 0 8px 0; font-size: 20px; color: #f2f3f5; }\n");
	out.push_str(".header .meta { font-size: 13px; color: #949ba4; }\n");
	out.push_str(
		".message { display: flex; margin-bottom: 16px; padding: 4px 8px; border-radius: 6px; }\n",
	);
	out.push_str(".message:hover { background: #2b2d31; }\n");
	out.push_str(".avatar { width: 40px; height: 40px; border-radius: 50%; background: #5865f2; color: #fff; display: flex; align-items: center; justify-content: center; font-weight: bold; font-size: 16px; margin-right: 12px; flex-shrink: 0; }\n");
	out.push_str(".msg-body { flex: 1; min-width: 0; }\n");
	out.push_str(".msg-header { margin-bottom: 4px; }\n");
	out.push_str(
		".author { font-weight: 600; color: #f2f3f5; margin-right: 8px; font-size: 14px; }\n",
	);
	out.push_str(".time { font-size: 11px; color: #949ba4; }\n");
	out.push_str(".text { font-size: 14px; line-height: 1.4; word-break: break-word; white-space: pre-wrap; margin-bottom: 6px; }\n");
	out.push_str(".attachments { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 6px; }\n");
	out.push_str(".attachment-link { display: inline-flex; align-items: center; padding: 6px 10px; background: #2b2d31; border: 1px solid #35363c; border-radius: 4px; color: #00a8fc; text-decoration: none; font-size: 13px; }\n");
	out.push_str(".attachment-link:hover { text-decoration: underline; background: #313338; }\n");
	out.push_str(".media-preview { max-width: 320px; max-height: 240px; border-radius: 6px; display: block; margin-top: 6px; }\n");
	out.push_str(".footer { border-top: 1px solid #35363c; padding-top: 16px; margin-top: 32px; font-size: 12px; color: #949ba4; text-align: center; }\n");
	out.push_str("@media (prefers-color-scheme: light) {\n");
	out.push_str("  body { background: #ffffff; color: #313338; }\n");
	out.push_str("  .header { border-bottom-color: #e3e5e8; }\n");
	out.push_str("  .header h1 { color: #060607; }\n");
	out.push_str("  .header .meta { color: #5c64f4; }\n");
	out.push_str("  .message:hover { background: #f2f3f5; }\n");
	out.push_str("  .author { color: #060607; }\n");
	out.push_str("  .time { color: #5c5f66; }\n");
	out.push_str(
		"  .attachment-link { background: #f2f3f5; border-color: #e3e5e8; color: #006ce7; }\n",
	);
	out.push_str("  .attachment-link:hover { background: #e3e5e8; }\n");
	out.push_str("  .footer { border-top-color: #e3e5e8; color: #5c5f66; }\n");
	out.push_str("}\n</style>\n</head>\n<body>\n<div class=\"export-container\">\n");
	out.push_str("<div class=\"header\">\n<h1>");
	out.push_str(&html_escape(channel_name));
	out.push_str("</h1>\n<div class=\"meta\">");
	out.push_str(&format!("{} messages exported", messages.len()));
	out.push_str("</div>\n</div>\n");

	for m in messages {
		out.push_str("<div class=\"message\">\n");
		let initial = m.author.chars().next().unwrap_or('?').to_uppercase();
		out.push_str(&format!("<div class=\"avatar\">{initial}</div>\n"));
		out.push_str("<div class=\"msg-body\">\n<div class=\"msg-header\">\n");
		out.push_str("<span class=\"author\">");
		out.push_str(&html_escape(&m.author));
		out.push_str("</span><span class=\"time\">");
		out.push_str(&html_escape(&m.when));
		out.push_str("</span></div>\n");
		if !m.text.trim().is_empty() {
			out.push_str("<div class=\"text\">");
			out.push_str(&html_escape(&m.text));
			out.push_str("</div>\n");
		}
		if !m.attachments.is_empty() {
			out.push_str("<div class=\"attachments\">\n");
			for (idx, name) in m.attachments.iter().enumerate() {
				let link = m.links.get(idx).and_then(|l| l.as_deref()).unwrap_or("");
				let is_img = name.ends_with(".png")
					|| name.ends_with(".jpg")
					|| name.ends_with(".jpeg")
					|| name.ends_with(".webp")
					|| name.ends_with(".gif");
				if is_img && !link.is_empty() {
					out.push_str(&format!(
						"<a href=\"{}\" target=\"_blank\"><img class=\"media-preview\" src=\"{}\" alt=\"{}\" loading=\"lazy\" /></a>\n",
						html_escape(link),
						html_escape(link),
						html_escape(name)
					));
				} else if !link.is_empty() {
					out.push_str(&format!(
						"<a class=\"attachment-link\" href=\"{}\" target=\"_blank\">📎 {}</a>\n",
						html_escape(link),
						html_escape(name)
					));
				} else {
					out.push_str(&format!(
						"<span class=\"attachment-link\">📎 {}</span>\n",
						html_escape(name)
					));
				}
			}
			out.push_str("</div>\n");
		}
		out.push_str("</div>\n</div>\n");
	}

	let digest = sha256_hex(out.as_bytes());
	out.push_str("<div class=\"footer\">\n");
	out.push_str(&format!(
		"Exported with Nivra client · SHA-256: <code>{digest}</code>\n"
	));
	out.push_str("</div>\n</div>\n</body>\n</html>\n");
	out
}

/// Computes lowercase hexadecimal SHA-256 digest of arbitrary bytes.
pub fn sha256_hex(data: &[u8]) -> String {
	let hash = sha256(data);
	let mut s = String::with_capacity(64);
	for b in hash {
		use std::fmt::Write;
		let _ = write!(s, "{b:02x}");
	}
	s
}

/// Standard SHA-256 implementation (FIPS 180-4) without external dependencies.
pub fn sha256(data: &[u8]) -> [u8; 32] {
	const K: [u32; 64] = [
		0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
		0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
		0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
		0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
		0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
		0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
		0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
		0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
		0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
		0xc67178f2,
	];

	let mut h: [u32; 8] = [
		0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
		0x5be0cd19,
	];

	let bit_len = (data.len() as u64).wrapping_mul(8);
	let mut padded = data.to_vec();
	padded.push(0x80);
	while (padded.len() % 64) != 56 {
		padded.push(0);
	}
	padded.extend_from_slice(&bit_len.to_be_bytes());

	for chunk in padded.as_chunks::<64>().0 {
		let mut w = [0u32; 64];
		for (i, slot) in w.iter_mut().take(16).enumerate() {
			*slot = u32::from_be_bytes([
				chunk[i * 4],
				chunk[i * 4 + 1],
				chunk[i * 4 + 2],
				chunk[i * 4 + 3],
			]);
		}
		for i in 16..64 {
			let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
			let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
			w[i] = w[i - 16]
				.wrapping_add(s0)
				.wrapping_add(w[i - 7])
				.wrapping_add(s1);
		}

		let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h_val] = h;

		for i in 0..64 {
			let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
			let ch = (e & f) ^ (!e & g);
			let temp1 = h_val
				.wrapping_add(s1)
				.wrapping_add(ch)
				.wrapping_add(K[i])
				.wrapping_add(w[i]);
			let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
			let maj = (a & b) ^ (a & c) ^ (b & c);
			let temp2 = s0.wrapping_add(maj);

			h_val = g;
			g = f;
			f = e;
			e = d.wrapping_add(temp1);
			d = c;
			c = b;
			b = a;
			a = temp1.wrapping_add(temp2);
		}

		h[0] = h[0].wrapping_add(a);
		h[1] = h[1].wrapping_add(b);
		h[2] = h[2].wrapping_add(c);
		h[3] = h[3].wrapping_add(d);
		h[4] = h[4].wrapping_add(e);
		h[5] = h[5].wrapping_add(f);
		h[6] = h[6].wrapping_add(g);
		h[7] = h[7].wrapping_add(h_val);
	}

	let mut result = [0u8; 32];
	for (i, word) in h.iter().enumerate() {
		result[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
	}
	result
}

fn html_escape(s: &str) -> String {
	let mut escaped = String::with_capacity(s.len());
	for c in s.chars() {
		match c {
			'&' => escaped.push_str("&amp;"),
			'<' => escaped.push_str("&lt;"),
			'>' => escaped.push_str("&gt;"),
			'"' => escaped.push_str("&quot;"),
			'\'' => escaped.push_str("&#39;"),
			other => escaped.push(other),
		}
	}
	escaped
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
		let ordered: Vec<Id> = (1..=30).map(Id).collect();
		let mut sel = BTreeSet::new();
		let added = range_select(&ordered, Some(Id(2)), Id(5), &mut sel);
		assert_eq!(added, 4);
		assert!(sel.contains(&Id(2)) && sel.contains(&Id(5)));
		let added2 = range_select(&ordered, Some(Id(1)), Id(30), &mut sel);
		assert_eq!(added2, MAX_SELECT - 4);
		assert_eq!(sel.len(), MAX_SELECT);
	}

	#[test]
	fn delete_and_download_limits_with_reasons() {
		let mut sel = BTreeSet::new();
		for i in 1..=MAX_SELECT as u64 {
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
		let ordered: Vec<Id> = (1..=30).map(Id).collect();
		let mut sel = BTreeSet::new();
		sel.insert(Id(3));
		let added = select_all_visible(&ordered, &mut sel);
		assert_eq!(added, MAX_SELECT - 1);
		assert_eq!(sel.len(), MAX_SELECT);
		assert!(sel.contains(&Id(9)));
		let added2 = select_all_visible(&ordered, &mut sel);
		assert_eq!(added2, 0);
	}

	fn test_attachment(id: Id, filename: &str, content_type: Option<&str>) -> model::Attachment {
		model::Attachment {
			id,
			filename: filename.into(),
			description: None,
			content_type: content_type.map(str::to_string),
			size: 100,
			media: model::EmbedMedia::default(),
			spoiler: false,
			duration_ms: None,
			waveform: Vec::new(),
		}
	}

	#[test]
	fn classify_message_distinguishes_content_types() {
		assert_eq!(classify_message("", &[], &[]), MessageContentKind::Empty);
		assert_eq!(
			classify_message("hello world", &[], &[]),
			MessageContentKind::TextOnly
		);

		let img_attachment = test_attachment(Id(1), "image.png", Some("image/png"));
		let doc_attachment = test_attachment(Id(2), "notes.pdf", Some("application/pdf"));

		assert_eq!(
			classify_message("", std::slice::from_ref(&img_attachment), &[]),
			MessageContentKind::MediaOnly
		);
		assert_eq!(
			classify_message("", std::slice::from_ref(&doc_attachment), &[]),
			MessageContentKind::FileOnly
		);
		assert_eq!(
			classify_message("look at this", &[img_attachment], &[]),
			MessageContentKind::Mixed
		);

		let embed_with_img = model::Embed {
			image: Some(model::EmbedMedia {
				url: Some("https://example.com/pic.jpg".into()),
				proxy_url: None,
				width: 100,
				height: 100,
				placeholder: Vec::new(),
			}),
			..Default::default()
		};
		assert_eq!(
			classify_message("", &[], &[embed_with_img]),
			MessageContentKind::MediaOnly
		);
	}

	#[test]
	fn selection_summary_helpers() {
		let media_only = SelectionSummary {
			total_selected: 3,
			text_count: 0,
			media_count: 3,
			file_count: 0,
			deletable_count: 0,
		};
		assert!(media_only.is_only_media());
		assert!(!media_only.is_only_text());
		assert!(!media_only.is_mixed());

		let text_only = SelectionSummary {
			total_selected: 2,
			text_count: 2,
			media_count: 0,
			file_count: 0,
			deletable_count: 2,
		};
		assert!(text_only.is_only_text());
		assert!(!text_only.is_only_media());
		assert!(!text_only.is_mixed());

		let mixed = SelectionSummary {
			total_selected: 2,
			text_count: 1,
			media_count: 1,
			file_count: 0,
			deletable_count: 1,
		};
		assert!(mixed.is_mixed());
		assert!(!mixed.is_only_text());
		assert!(!mixed.is_only_media());
	}

	#[test]
	fn apply_quick_filter_rules() {
		let ordered: Vec<Id> = (1..=10).map(Id).collect();
		let mut selected = BTreeSet::new();

		let mock_lookup =
			|id: Id| -> Option<(String, Vec<model::Attachment>, Vec<model::Embed>, Id)> {
				match id.0 {
					1..=3 => Some((
						"text msg".into(),
						vec![],
						vec![],
						Id(100), // author 100
					)),
					4..=6 => Some((
						"".into(),
						vec![test_attachment(id, "pic.png", Some("image/png"))],
						vec![],
						Id(200), // author 200
					)),
					7..=8 => Some((
						"".into(),
						vec![test_attachment(id, "video.mp4", Some("video/mp4"))],
						vec![],
						Id(100),
					)),
					_ => Some(("doc".into(), vec![], vec![], Id(300))),
				}
			};

		// Last 5
		let count = apply_quick_filter(
			&ordered,
			QuickFilter::Last(5),
			&mut selected,
			Some(Id(100)),
			&mock_lookup,
		);
		assert_eq!(count, 5);
		assert_eq!(selected.len(), 5);
		assert!(selected.contains(&Id(10)));
		assert!(selected.contains(&Id(6)));

		// OnlyText
		let count = apply_quick_filter(
			&ordered,
			QuickFilter::OnlyText,
			&mut selected,
			Some(Id(100)),
			&mock_lookup,
		);
		assert_eq!(count, 5); // 1, 2, 3, 9, 10
		assert!(selected.contains(&Id(1)));
		assert!(selected.contains(&Id(2)));
		assert!(selected.contains(&Id(3)));

		// OnlyImages
		let count = apply_quick_filter(
			&ordered,
			QuickFilter::OnlyImages,
			&mut selected,
			Some(Id(100)),
			&mock_lookup,
		);
		assert_eq!(count, 3); // 4, 5, 6
		assert!(selected.contains(&Id(4)));
		assert!(selected.contains(&Id(5)));
		assert!(selected.contains(&Id(6)));

		// OnlyMine
		let count = apply_quick_filter(
			&ordered,
			QuickFilter::OnlyMine,
			&mut selected,
			Some(Id(100)),
			&mock_lookup,
		);
		assert_eq!(count, 5); // 1, 2, 3, 7, 8
		assert!(selected.contains(&Id(7)));
		assert!(selected.contains(&Id(8)));

		// Last 20 on an array of 25 messages selects exactly 20 messages
		let ordered_25: Vec<Id> = (1..=25).map(Id).collect();
		let count_20 = apply_quick_filter(
			&ordered_25,
			QuickFilter::Last(20),
			&mut selected,
			Some(Id(100)),
			&mock_lookup,
		);
		assert_eq!(count_20, 20);
		assert_eq!(selected.len(), 20);
	}

	#[test]
	fn html_export_contains_structure_and_escapes() {
		let messages = vec![
			TxtMessage {
				author: "Alice <script>".into(),
				when: "14:30".into(),
				text: "Check & verify \"quotes\"".into(),
				attachments: vec!["pic.png".into()],
				links: vec![Some("https://cdn.example.com/pic.png".into())],
			},
			TxtMessage {
				author: "Bob".into(),
				when: "14:32".into(),
				text: "Regular file".into(),
				attachments: vec!["notes.txt".into()],
				links: vec![Some("https://cdn.example.com/notes.txt".into())],
			},
		];

		let html = format_html("general & alerts", &messages);
		assert!(html.contains("Nivra Export - general &amp; alerts"));
		assert!(html.contains("Alice &lt;script&gt;"));
		assert!(html.contains("Check &amp; verify &quot;quotes&quot;"));
		assert!(
			html.contains("<img class=\"media-preview\" src=\"https://cdn.example.com/pic.png\"")
		);
		assert!(html.contains("📎 notes.txt"));
		assert!(html.contains("<!DOCTYPE html>"));
		assert!(html.contains("Exported with Nivra client · SHA-256: <code>"));
	}

	#[test]
	fn sha256_standard_vectors_and_md_checksum() {
		// NIST / standard test vectors
		assert_eq!(
			sha256_hex(b""),
			"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
		);
		assert_eq!(
			sha256_hex(b"hello world"),
			"b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
		);

		let messages = vec![TxtMessage {
			author: "Alice".into(),
			when: "10:00".into(),
			text: "Note".into(),
			attachments: vec![],
			links: vec![],
		}];
		let md = format_md(&messages);
		assert!(md.contains("Exported with Nivra client · SHA-256: `"));
	}
}
