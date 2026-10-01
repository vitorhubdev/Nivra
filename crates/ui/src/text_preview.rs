//! Bounded, read-only preview of text, markdown and code attachments.
//!
//! Nothing here performs I/O: the desktop fetches a bounded body off the render
//! thread and hands it in. The module only decides whether the bytes are worth
//! showing and trims them so a huge file can never stall layout.
use crate::attachments::FileKind;

/// Attachments larger than this are never fetched for an inline preview. The
/// desktop also refuses a declared size above this before opening a request.
pub const MAX_PREVIEW_BYTES: u64 = 256 * 1024;
/// Longest decoded body kept, so one pathological line cannot blow up layout.
pub const MAX_PREVIEW_CHARS: usize = 200_000;
/// Bytes inspected for NUL when deciding whether a file is binary.
const BINARY_SNIFF_BYTES: usize = 8192;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PreviewFormat {
	Markdown,
	Code,
	Plain,
}

/// The preview format for a filename/type pair, or `None` for anything that is
/// not plain text, markdown or source code.
pub fn preview_format(filename: &str, content_type: Option<&str>) -> Option<PreviewFormat> {
	if crate::attachments::file_kind(filename, content_type) != FileKind::Text
		&& crate::attachments::file_kind(filename, content_type) != FileKind::Code
	{
		return None;
	}
	let extension = filename
		.rsplit_once('.')
		.map(|(_, extension)| extension.to_ascii_lowercase())
		.unwrap_or_default();
	let mime = content_type
		.map(|kind| {
			kind.split(';')
				.next()
				.unwrap_or(kind)
				.trim()
				.to_ascii_lowercase()
		})
		.unwrap_or_default();
	match extension.as_str() {
		"md" | "markdown" => return Some(PreviewFormat::Markdown),
		"rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "json" | "toml" | "yaml" | "yml" | "html"
		| "css" | "c" | "h" | "cpp" | "hpp" | "java" | "kt" | "swift" | "go" | "rb" | "sh"
		| "xml" | "sql" => return Some(PreviewFormat::Code),
		_ => {}
	}
	if mime == "text/markdown" {
		return Some(PreviewFormat::Markdown);
	}
	Some(PreviewFormat::Plain)
}

/// Characters drawn in one preview step. The rest stays in memory and appears
/// when the reader asks, so a long file does not layout all at once.
pub const PREVIEW_WINDOW_CHARS: usize = 8_000;

/// Decode a preview body from raw bytes. Returns `None` when the body is empty,
/// effectively blank, or looks binary (a NUL in the first block of a file that
/// is not UTF-16). A UTF-8, UTF-16 LE or UTF-16 BE BOM selects that encoding.
/// Valid UTF-8 is kept. Other bytes are Windows-1252, so a legacy `.txt` does
/// not turn into replacement characters. The result is capped by input bytes
/// and character count.
pub fn decode_preview(bytes: &[u8]) -> Option<(String, bool)> {
	if bytes.is_empty() {
		return None;
	}
	let (text, consumed_all) = if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
		decode_utf16(rest, true)
	} else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
		decode_utf16(rest, false)
	} else {
		let raw = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
		if raw.is_empty() || raw.iter().take(BINARY_SNIFF_BYTES).any(|byte| *byte == 0) {
			return None;
		}
		let over_cap = raw.len() > MAX_PREVIEW_BYTES as usize;
		let slice = &raw[..raw.len().min(MAX_PREVIEW_BYTES as usize)];
		// A cut at the byte cap can split a UTF-8 character. Lossy keeps the
		// valid prefix. A short file that is not UTF-8 is Windows-1252.
		let text = if over_cap || std::str::from_utf8(slice).is_ok() {
			String::from_utf8_lossy(slice).into_owned()
		} else {
			slice.iter().copied().map(windows_1252).collect()
		};
		(text, !over_cap)
	};
	let mut body = String::new();
	let mut char_truncated = false;
	for (chars, character) in text.chars().enumerate() {
		if chars == MAX_PREVIEW_CHARS {
			char_truncated = true;
			break;
		}
		body.push(character);
	}
	if body.trim().is_empty() {
		return None;
	}
	Some((body, !consumed_all || char_truncated))
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> (String, bool) {
	let mut units = Vec::new();
	let mut index = 0;
	let mut hit_cap = false;
	while index + 1 < bytes.len() {
		if units.len() >= MAX_PREVIEW_CHARS {
			hit_cap = true;
			break;
		}
		let unit = if little_endian {
			u16::from_le_bytes([bytes[index], bytes[index + 1]])
		} else {
			u16::from_be_bytes([bytes[index], bytes[index + 1]])
		};
		units.push(unit);
		index += 2;
	}
	(
		String::from_utf16_lossy(&units),
		index >= bytes.len() && !hit_cap,
	)
}

fn windows_1252(byte: u8) -> char {
	match byte {
		0x80 => '\u{20AC}',
		0x82 => '\u{201A}',
		0x83 => '\u{0192}',
		0x84 => '\u{201E}',
		0x85 => '\u{2026}',
		0x86 => '\u{2020}',
		0x87 => '\u{2021}',
		0x88 => '\u{02C6}',
		0x89 => '\u{2030}',
		0x8A => '\u{0160}',
		0x8B => '\u{2039}',
		0x8C => '\u{0152}',
		0x8E => '\u{017D}',
		0x91 => '\u{2018}',
		0x92 => '\u{2019}',
		0x93 => '\u{201C}',
		0x94 => '\u{201D}',
		0x95 => '\u{2022}',
		0x96 => '\u{2013}',
		0x97 => '\u{2014}',
		0x98 => '\u{02DC}',
		0x99 => '\u{2122}',
		0x9A => '\u{0161}',
		0x9B => '\u{203A}',
		0x9C => '\u{0153}',
		0x9E => '\u{017E}',
		0x9F => '\u{0178}',
		other => char::from(other),
	}
}

/// State for the single open preview dialog. The desktop fills it after fetching
/// a bounded body; the UI only renders it.
#[derive(Clone, Debug)]
pub struct TextPreview {
	pub filename: String,
	pub format: PreviewFormat,
	pub text: String,
	pub truncated: bool,
	/// Characters currently laid out. Starts at one window.
	pub shown: usize,
}

impl TextPreview {
	/// The slice laid out this frame.
	pub fn visible_text(&self) -> &str {
		match self.text.char_indices().nth(self.shown) {
			Some((index, _)) => &self.text[..index],
			None => self.text.as_str(),
		}
	}

	pub fn has_more(&self) -> bool {
		self.text.chars().nth(self.shown).is_some()
	}

	pub fn show_more(&mut self) {
		self.shown = self.shown.saturating_add(PREVIEW_WINDOW_CHARS);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_text_and_code_are_previewable() {
		assert_eq!(
			preview_format("notes.txt", None),
			Some(PreviewFormat::Plain)
		);
		assert_eq!(
			preview_format("README.md", None),
			Some(PreviewFormat::Markdown)
		);
		assert_eq!(preview_format("main.rs", None), Some(PreviewFormat::Code));
		assert_eq!(preview_format("data.json", None), Some(PreviewFormat::Code));
		assert_eq!(preview_format("clip.mp4", Some("video/mp4")), None);
		assert_eq!(preview_format("photo.png", None), None);
		assert_eq!(preview_format("report.pdf", Some("application/pdf")), None);
	}

	#[test]
	fn reported_markdown_type_wins_over_extension() {
		assert_eq!(
			preview_format("download", Some("text/markdown; charset=utf-8")),
			Some(PreviewFormat::Markdown)
		);
		assert_eq!(
			preview_format("download", Some("text/plain")),
			Some(PreviewFormat::Plain)
		);
	}

	#[test]
	fn binary_and_blank_bodies_are_rejected() {
		assert_eq!(decode_preview(b""), None);
		assert_eq!(decode_preview(b"   \n\t "), None);
		assert_eq!(decode_preview(&[0x00, 0x01, 0x02]), None);
	}

	#[test]
	fn lossy_decode_keeps_invalid_bytes() {
		let (text, truncated) = decode_preview("caf\u{e9}".as_bytes()).unwrap();
		assert_eq!(text, "caf\u{e9}");
		assert!(!truncated);
	}

	#[test]
	fn oversized_bodies_are_capped_and_flagged() {
		let big = vec![b'a'; MAX_PREVIEW_BYTES as usize + 10];
		let (text, truncated) = decode_preview(&big).unwrap();
		assert!(truncated);
		// The byte cap bites first, then the character cap bounds the body.
		assert_eq!(text.chars().count(), MAX_PREVIEW_CHARS);
	}

	#[test]
	fn utf16_bom_is_text_not_binary() {
		let mut bytes = vec![0xFF, 0xFE];
		for unit in "Olá".encode_utf16() {
			bytes.extend(unit.to_le_bytes());
		}
		let (text, truncated) = decode_preview(&bytes).unwrap();
		assert_eq!(text, "Olá");
		assert!(!truncated);
		let mut big_endian = vec![0xFE, 0xFF];
		for unit in "Hi".encode_utf16() {
			big_endian.extend(unit.to_be_bytes());
		}
		assert_eq!(decode_preview(&big_endian).unwrap().0, "Hi");
	}

	#[test]
	fn windows_1252_keeps_legacy_punctuation() {
		let (text, truncated) = decode_preview(&[b'c', b'a', b'f', 0xE9, 0x80]).unwrap();
		assert_eq!(text, "caf\u{e9}\u{20AC}");
		assert!(!truncated);
	}

	#[test]
	fn a_long_preview_opens_one_window_at_a_time() {
		use std::time::Instant;
		let body = "a".repeat(20_000);
		let started = Instant::now();
		let (text, _) = decode_preview(body.as_bytes()).unwrap();
		let decode = started.elapsed();
		let mut preview = TextPreview {
			filename: "notes.txt".into(),
			format: PreviewFormat::Plain,
			text,
			truncated: false,
			shown: PREVIEW_WINDOW_CHARS,
		};
		assert_eq!(preview.visible_text().chars().count(), PREVIEW_WINDOW_CHARS);
		assert!(preview.has_more());
		preview.show_more();
		assert_eq!(preview.visible_text().chars().count(), 16_000);
		preview.show_more();
		assert!(!preview.has_more());
		println!("decode 20000 ascii chars: {decode:?}");
	}
}
