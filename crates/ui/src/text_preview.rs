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

/// Decode a preview body from raw bytes. Returns `None` when the body is empty,
/// effectively blank, or looks binary (a NUL in the first block). Decoding is
/// lossy so stray invalid bytes render instead of failing the whole preview,
/// and the result is capped by both input bytes and character count.
pub fn decode_preview(bytes: &[u8]) -> Option<(String, bool)> {
	if bytes.is_empty() || bytes.iter().take(BINARY_SNIFF_BYTES).any(|byte| *byte == 0) {
		return None;
	}
	let truncated = bytes.len() as u64 > MAX_PREVIEW_BYTES;
	let slice = &bytes[..bytes.len().min(MAX_PREVIEW_BYTES as usize)];
	let text = String::from_utf8_lossy(slice);
	let mut body = String::new();
	let mut chars = 0usize;
	let mut char_truncated = false;
	for character in text.chars() {
		if chars == MAX_PREVIEW_CHARS {
			char_truncated = true;
			break;
		}
		body.push(character);
		chars += 1;
	}
	if body.trim().is_empty() {
		return None;
	}
	Some((body, truncated || char_truncated))
}

/// State for the single open preview dialog. The desktop fills it after fetching
/// a bounded body; the UI only renders it.
#[derive(Clone, Debug)]
pub struct TextPreview {
	pub filename: String,
	pub format: PreviewFormat,
	pub text: String,
	pub truncated: bool,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_text_and_code_are_previewable() {
		assert_eq!(preview_format("notes.txt", None), Some(PreviewFormat::Plain));
		assert_eq!(preview_format("README.md", None), Some(PreviewFormat::Markdown));
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
}
