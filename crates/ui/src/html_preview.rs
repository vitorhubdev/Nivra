//! Static, bounded conversion of an HTML attachment into markdown for the preview.
//!
//! Nothing is executed or fetched. Scripts, styles, frames, embedded objects and
//! comments are dropped, an image becomes a text placeholder, only `http(s)` links
//! survive, and every character of text is escaped so the markdown renderer shows
//! what the page says and never interprets it. The module performs no I/O and its
//! output is bounded by the preview limits, so a hostile file cannot stall layout.
use crate::text_preview::MAX_PREVIEW_CHARS;

/// Longest HTML body converted. Callers already stop at `MAX_PREVIEW_BYTES`.
const MAX_INPUT_BYTES: usize = 512 * 1024;
/// Output size at which conversion stops reading further markup.
const MAX_OUTPUT_BYTES: usize = MAX_PREVIEW_CHARS * 4;
/// Open emphasis, code and link spans tracked at once. Deeper ones stay plain text.
const MAX_SPANS: usize = 64;
/// Nested list levels that get their own indentation.
const MAX_LIST_INDENT: usize = 8;
/// Nested quote levels that get their own `>` prefix.
const MAX_QUOTE_DEPTH: usize = 8;
/// Longest attribute value kept (only `href` and `alt` are read).
const MAX_ATTRIBUTE_CHARS: usize = 2048;
const MAX_TITLE_CHARS: usize = 200;

/// Elements whose whole content is dropped.
const SKIPPED: &[&str] = &[
	"template", "iframe", "object", "svg", "math", "canvas", "video", "audio", "noembed", "select",
	"datalist",
];
/// Elements that start a new paragraph.
const BLOCKS: &[&str] = &[
	"p",
	"div",
	"section",
	"article",
	"header",
	"footer",
	"main",
	"nav",
	"aside",
	"form",
	"fieldset",
	"figure",
	"figcaption",
	"details",
	"summary",
	"address",
	"dl",
	"dt",
	"dd",
	"table",
	"thead",
	"tbody",
	"tfoot",
	"caption",
	"center",
	"menu",
	"dir",
];

/// Convert an HTML document to markdown that is safe to hand to the preview's
/// markdown renderer. Malformed or hostile input never fails: it is read as far as
/// it makes sense and the rest is ignored.
pub fn to_markdown(html: &str) -> String {
	let mut end = html.len().min(MAX_INPUT_BYTES);
	while !html.is_char_boundary(end) {
		end -= 1;
	}
	Converter::new().convert(&html[..end])
}

#[derive(Clone, PartialEq, Eq)]
enum Mark {
	Strong,
	Emphasis,
	Strike,
	Code,
	Link(String),
}

struct Span {
	mark: Mark,
	/// Byte offset in the output where the span's content begins.
	start: usize,
}

struct List {
	ordered: bool,
	next: u32,
}

#[derive(Default)]
struct Attributes {
	href: Option<String>,
	alt: Option<String>,
}

struct Converter {
	out: String,
	/// The next character starts a line, so the quote prefix is still owed.
	at_line_start: bool,
	/// No content character has been written on this line yet.
	first_on_line: bool,
	/// Only digits have been written on this line (`1.` would become a list).
	digits_only: bool,
	/// The line holds nothing but a list or heading marker.
	after_marker: bool,
	marker_len: usize,
	quote: usize,
	lists: Vec<List>,
	spans: Vec<Span>,
	/// Spans closed by a line break, reopened when the next line has content.
	reopen: Vec<Mark>,
	pre: Option<String>,
	skip: Option<(String, usize)>,
	title: Option<String>,
}

impl Converter {
	fn new() -> Self {
		Self {
			out: String::new(),
			at_line_start: true,
			first_on_line: true,
			digits_only: true,
			after_marker: false,
			marker_len: 0,
			quote: 0,
			lists: Vec::new(),
			spans: Vec::new(),
			reopen: Vec::new(),
			pre: None,
			skip: None,
			title: None,
		}
	}

	fn convert(mut self, html: &str) -> String {
		let mut index = 0;
		while index < html.len() && self.out.len() < MAX_OUTPUT_BYTES {
			let Some(offset) = html[index..].find('<') else {
				self.text(&html[index..]);
				break;
			};
			self.text(&html[index..index + offset]);
			index = self.markup(html, index + offset);
		}
		self.finish()
	}

	/// Handle the markup that starts at `at` (a `<`) and return where reading resumes.
	fn markup(&mut self, html: &str, at: usize) -> usize {
		let rest = &html[at + 1..];
		if rest.starts_with("!--") {
			return rest.find("-->").map_or(html.len(), |end| at + 1 + end + 3);
		}
		let Some(first) = rest.chars().next() else {
			self.text("<");
			return html.len();
		};
		match first {
			'!' | '?' => html[at..].find('>').map_or(html.len(), |end| at + end + 1),
			'/' if rest[1..].starts_with(|c: char| c.is_ascii_alphabetic()) => {
				let name = tag_name(&rest[1..]);
				self.end_tag(&name);
				html[at..].find('>').map_or(html.len(), |end| at + end + 1)
			}
			c if c.is_ascii_alphabetic() => self.start_tag(html, at + 1),
			_ => {
				self.text("<");
				at + 1
			}
		}
	}

	fn start_tag(&mut self, html: &str, from: usize) -> usize {
		let name = tag_name(&html[from..]);
		let (attributes, after) = parse_attributes(html, from + name.len());
		self.open(&name, &attributes);
		if matches!(
			name.as_str(),
			"script" | "style" | "title" | "textarea" | "xmp"
		) {
			// Raw text elements: the content is never markup, so skip to the close.
			let close = find_closing(html, after, &name);
			if name == "title" && self.skip.is_none() && self.title.is_none() {
				self.title = collapse(&decode_entities(&html[after..close]), MAX_TITLE_CHARS);
			}
			return html[close..]
				.find('>')
				.map_or(html.len(), |end| close + end + 1);
		}
		after
	}

	fn open(&mut self, name: &str, attributes: &Attributes) {
		if let Some((skipped, depth)) = self.skip.as_mut() {
			if skipped == name {
				*depth += 1;
			}
			return;
		}
		if let Some(pre) = self.pre.as_mut() {
			if name == "br" {
				pre.push('\n');
			}
			return;
		}
		if SKIPPED.contains(&name) {
			self.skip = Some((name.to_owned(), 1));
			return;
		}
		match name {
			"br" => self.hard_break(),
			"hr" => {
				self.block_break();
				self.write("---");
				self.block_break();
			}
			"h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
				self.block_break();
				let level = usize::from(name.as_bytes()[1] - b'0');
				self.marker(&format!("{} ", "#".repeat(level)));
			}
			"ul" | "ol" => {
				if self.lists.is_empty() {
					self.block_break();
				} else {
					self.ensure_newline();
				}
				self.lists.push(List {
					ordered: name == "ol",
					next: 1,
				});
			}
			"li" => {
				self.ensure_newline();
				let indent = "    ".repeat(self.lists.len().saturating_sub(1).min(MAX_LIST_INDENT));
				let marker = match self.lists.last_mut() {
					Some(list) if list.ordered => {
						let number = list.next;
						list.next = list.next.saturating_add(1);
						format!("{number}. ")
					}
					_ => "- ".to_owned(),
				};
				self.marker(&format!("{indent}{marker}"));
			}
			"blockquote" => {
				self.block_break();
				self.quote = self.quote.saturating_add(1);
			}
			"pre" => {
				self.block_break();
				self.pre = Some(String::new());
			}
			"tr" => self.block_break(),
			"a" => {
				if let Some(url) = attributes
					.href
					.as_deref()
					.and_then(crate::markdown::external_url)
				{
					self.open_span(Mark::Link(url));
				}
			}
			"strong" | "b" => self.open_span(Mark::Strong),
			"em" | "i" => self.open_span(Mark::Emphasis),
			"del" | "s" | "strike" => self.open_span(Mark::Strike),
			"code" | "kbd" | "samp" | "tt" => self.open_span(Mark::Code),
			"img" => {
				let alt = attributes
					.alt
					.as_deref()
					.and_then(|alt| collapse(alt, MAX_TITLE_CHARS));
				let placeholder = match alt {
					Some(alt) => format!("[image: {alt}]"),
					None => "[image]".to_owned(),
				};
				self.text(" ");
				self.text(&placeholder);
			}
			_ if BLOCKS.contains(&name) => self.block_break_open(),
			_ => {}
		}
	}

	fn end_tag(&mut self, name: &str) {
		if let Some((skipped, depth)) = self.skip.as_mut() {
			if skipped == name {
				*depth = depth.saturating_sub(1);
				if *depth == 0 {
					self.skip = None;
				}
			}
			return;
		}
		if self.pre.is_some() {
			if name == "pre" {
				self.flush_pre();
			}
			return;
		}
		match name {
			"h1" | "h2" | "h3" | "h4" | "h5" | "h6" => self.block_break(),
			"ul" | "ol" => {
				self.lists.pop();
				if self.lists.is_empty() {
					self.block_break();
				} else {
					self.ensure_newline();
				}
			}
			"li" => self.ensure_newline(),
			"blockquote" => {
				self.block_break();
				self.quote = self.quote.saturating_sub(1);
			}
			"tr" => {
				self.trim_trailing_spaces();
				if self.out.ends_with(" |") {
					self.out.truncate(self.out.len() - 2);
				}
				self.block_break();
			}
			"td" | "th" => {
				self.trim_trailing_spaces();
				self.write(" | ");
			}
			"a" => self.close_span(&Mark::Link(String::new())),
			"strong" | "b" => self.close_span(&Mark::Strong),
			"em" | "i" => self.close_span(&Mark::Emphasis),
			"del" | "s" | "strike" => self.close_span(&Mark::Strike),
			"code" | "kbd" | "samp" | "tt" => self.close_span(&Mark::Code),
			"p" => self.block_break(),
			_ if BLOCKS.contains(&name) => self.block_break(),
			_ => {}
		}
	}

	fn text(&mut self, raw: &str) {
		if raw.is_empty() || self.skip.is_some() {
			return;
		}
		let decoded = decode_entities(raw);
		if let Some(pre) = self.pre.as_mut() {
			if pre.len() < MAX_OUTPUT_BYTES {
				pre.push_str(&decoded);
			}
			return;
		}
		let escape = !self.spans.iter().any(|span| span.mark == Mark::Code);
		for character in decoded.chars() {
			if character.is_whitespace() {
				if !self.out.is_empty() && !self.out.ends_with([' ', '\n']) {
					self.out.push(' ');
				}
			} else if !character.is_control() && !is_bidi_control(character) {
				self.put(character, escape);
			}
		}
	}

	fn put(&mut self, character: char, escape: bool) {
		self.after_marker = false;
		if self.at_line_start {
			self.flush_prefix();
		}
		self.reopen_pending();
		if escape {
			let needs_escape = matches!(
				character,
				'\\' | '`' | '*' | '_' | '~' | '|' | '[' | ']' | '<' | '>' | '#' | '&'
			) || (self.first_on_line && matches!(character, '-' | '+' | '='))
				|| (self.digits_only && matches!(character, '.' | ')'));
			if needs_escape {
				self.out.push('\\');
			}
		}
		self.out.push(character);
		if !character.is_ascii_digit() {
			self.digits_only = false;
		}
		self.first_on_line = false;
	}

	/// Structural text: never escaped, and every line gets the quote prefix.
	fn write(&mut self, text: &str) {
		for character in text.chars() {
			if character == '\n' {
				self.out.push('\n');
				self.reset_line();
			} else {
				if self.at_line_start {
					self.flush_prefix();
				}
				self.out.push(character);
			}
		}
	}

	fn flush_prefix(&mut self) {
		self.at_line_start = false;
		for _ in 0..self.quote.min(MAX_QUOTE_DEPTH) {
			self.out.push_str("> ");
		}
	}

	fn reset_line(&mut self) {
		self.at_line_start = true;
		self.first_on_line = true;
		self.digits_only = true;
		self.after_marker = false;
	}

	/// Write a list or heading marker that belongs to the content that follows.
	fn marker(&mut self, marker: &str) {
		if self.at_line_start {
			self.flush_prefix();
		}
		let start = self.out.len();
		self.write(marker);
		self.marker_len = self.out.len() - start;
		self.after_marker = true;
	}

	/// A marker that never received content (`<li></li>`) is removed.
	fn drop_empty_marker(&mut self) {
		if self.after_marker {
			self.out.truncate(self.out.len() - self.marker_len);
			self.after_marker = false;
			self.at_line_start = self.out.is_empty() || self.out.ends_with('\n');
			self.clamp_spans();
		}
	}

	/// Spans that began inside text that was just removed now begin at the end.
	fn clamp_spans(&mut self) {
		let end = self.out.len();
		for span in &mut self.spans {
			span.start = span.start.min(end);
		}
	}

	fn trim_trailing_spaces(&mut self) {
		while self.out.ends_with(' ') {
			self.out.pop();
		}
		self.clamp_spans();
	}

	/// End the paragraph and leave one blank line.
	fn block_break(&mut self) {
		self.drop_empty_marker();
		self.split_spans();
		self.trim_trailing_spaces();
		if self.out.is_empty() || self.out.ends_with("\n\n") {
			return;
		}
		self.out.push_str(if self.out.ends_with('\n') {
			"\n"
		} else {
			"\n\n"
		});
		self.reset_line();
	}

	/// A block opened inside a list item shares the item's line.
	fn block_break_open(&mut self) {
		if !self.after_marker {
			self.block_break();
		}
	}

	fn ensure_newline(&mut self) {
		self.drop_empty_marker();
		self.split_spans();
		self.trim_trailing_spaces();
		if !self.out.is_empty() && !self.out.ends_with('\n') {
			self.out.push('\n');
		}
		self.reset_line();
	}

	fn hard_break(&mut self) {
		if self.after_marker || self.out.is_empty() || self.out.ends_with('\n') {
			return;
		}
		self.split_spans();
		self.trim_trailing_spaces();
		self.out.push_str("  \n");
		self.reset_line();
	}

	/// Close every open span at a line break and reopen it on the next line, so
	/// emphasis and links that wrap several blocks mark each of their lines.
	fn split_spans(&mut self) {
		if self.spans.is_empty() {
			return;
		}
		self.clamp_spans();
		let mut marks = Vec::with_capacity(self.spans.len());
		while let Some(span) = self.spans.pop() {
			marks.push(span.mark.clone());
			self.wrap(span);
		}
		marks.reverse();
		self.reopen = marks;
	}

	/// Reopen the spans a line break closed, once content arrives on the new line.
	fn reopen_pending(&mut self) {
		if self.reopen.is_empty() {
			return;
		}
		if self.at_line_start {
			self.flush_prefix();
		}
		for mark in std::mem::take(&mut self.reopen) {
			if self.spans.len() < MAX_SPANS {
				self.spans.push(Span {
					mark,
					start: self.out.len(),
				});
			}
		}
	}

	fn open_span(&mut self, mark: Mark) {
		self.reopen_pending();
		if self.spans.len() >= MAX_SPANS || self.spans.iter().any(|span| span.mark == Mark::Code) {
			return;
		}
		// The quote prefix goes before the span so wrapping never swallows it.
		if self.at_line_start {
			self.flush_prefix();
		}
		self.spans.push(Span {
			mark,
			start: self.out.len(),
		});
	}

	fn close_span(&mut self, mark: &Mark) {
		let same = |other: &Mark| std::mem::discriminant(other) == std::mem::discriminant(mark);
		let Some(position) = self.spans.iter().rposition(|span| same(&span.mark)) else {
			// Split at a line break and still without content on the new line.
			if let Some(position) = self.reopen.iter().rposition(same) {
				self.reopen.truncate(position);
			}
			return;
		};
		// Spans opened after this one were never closed: their text stays plain.
		self.spans.truncate(position + 1);
		if let Some(span) = self.spans.pop() {
			self.wrap(span);
		}
	}

	fn wrap(&mut self, span: Span) {
		let content = self.out.split_off(span.start);
		let trimmed = content.trim_matches([' ', '\n']);
		// Emphasis cannot cross a line, and an empty span has nothing to mark.
		if trimmed.is_empty() || trimmed.contains('\n') {
			self.out.push_str(&content);
			return;
		}
		let leading = &content[..content.len() - content.trim_start_matches([' ', '\n']).len()];
		let trailing = &content[content.trim_end_matches([' ', '\n']).len()..];
		let wrapped = match &span.mark {
			Mark::Strong => format!("**{trimmed}**"),
			Mark::Emphasis => format!("*{trimmed}*"),
			Mark::Strike => format!("~~{trimmed}~~"),
			Mark::Code => inline_code(trimmed),
			Mark::Link(url) => {
				format!(
					"[{trimmed}]({})",
					url.replace('(', "%28").replace(')', "%29")
				)
			}
		};
		self.out.push_str(leading);
		self.out.push_str(&wrapped);
		self.out.push_str(trailing);
	}

	fn flush_pre(&mut self) {
		let Some(body) = self.pre.take() else {
			return;
		};
		let body = body.strip_prefix('\n').unwrap_or(&body);
		let body = body.trim_end_matches('\n');
		if body.trim().is_empty() {
			return;
		}
		let fence = "`".repeat(longest_run(body, '`').max(2) + 1);
		self.write(&fence);
		self.write("\n");
		self.write(body);
		self.write("\n");
		self.write(&fence);
		self.block_break();
	}

	fn finish(mut self) -> String {
		self.flush_pre();
		// An unclosed span keeps its text, unmarked.
		self.spans.clear();
		let body = self.out.trim_matches(['\n', ' ']);
		let mut result = match self.title.as_deref() {
			Some(title) => format!("**{}**\n\n{body}", escape(title)),
			None => body.to_owned(),
		};
		if let Some((end, _)) = result.char_indices().nth(MAX_PREVIEW_CHARS) {
			result.truncate(end);
		}
		result
	}
}

/// Lower-case ASCII tag name at the start of `text`.
fn tag_name(text: &str) -> String {
	text.chars()
		.take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | ':'))
		.map(|c| c.to_ascii_lowercase())
		.collect()
}

/// Read attributes up to the closing `>` and return the ones that are used along
/// with the index after the tag. An unterminated quote consumes the rest of the file.
fn parse_attributes(html: &str, mut index: usize) -> (Attributes, usize) {
	let bytes = html.as_bytes();
	let mut attributes = Attributes::default();
	loop {
		while index < bytes.len() && (bytes[index].is_ascii_whitespace() || bytes[index] == b'/') {
			index += 1;
		}
		if index >= bytes.len() {
			return (attributes, bytes.len());
		}
		if bytes[index] == b'>' {
			return (attributes, index + 1);
		}
		let name_start = index;
		while index < bytes.len()
			&& !bytes[index].is_ascii_whitespace()
			&& !matches!(bytes[index], b'=' | b'>' | b'/')
		{
			index += 1;
		}
		let name = html[name_start..index].to_ascii_lowercase();
		while index < bytes.len() && bytes[index].is_ascii_whitespace() {
			index += 1;
		}
		let mut value = None;
		if index < bytes.len() && bytes[index] == b'=' {
			index += 1;
			while index < bytes.len() && bytes[index].is_ascii_whitespace() {
				index += 1;
			}
			if index < bytes.len() && matches!(bytes[index], b'"' | b'\'') {
				let quote = bytes[index];
				index += 1;
				let start = index;
				while index < bytes.len() && bytes[index] != quote {
					index += 1;
				}
				value = Some(&html[start..index]);
				if index >= bytes.len() {
					return (attributes, bytes.len());
				}
				index += 1;
			} else {
				let start = index;
				while index < bytes.len()
					&& !bytes[index].is_ascii_whitespace()
					&& bytes[index] != b'>'
				{
					index += 1;
				}
				value = Some(&html[start..index]);
			}
		}
		let Some(value) = value else {
			continue;
		};
		let slot = match name.as_str() {
			"href" => &mut attributes.href,
			"alt" => &mut attributes.alt,
			_ => continue,
		};
		let decoded = decode_entities(value);
		*slot = Some(decoded.chars().take(MAX_ATTRIBUTE_CHARS).collect());
	}
}

/// Index of the `</name` that ends a raw text element, or the end of the file.
fn find_closing(html: &str, from: usize, name: &str) -> usize {
	let bytes = html.as_bytes();
	let needle = name.as_bytes();
	let mut index = from;
	while let Some(offset) = html[index..].find("</") {
		let start = index + offset;
		let name_at = start + 2;
		if bytes.len() >= name_at + needle.len()
			&& bytes[name_at..name_at + needle.len()].eq_ignore_ascii_case(needle)
			&& bytes
				.get(name_at + needle.len())
				.is_none_or(|byte| !byte.is_ascii_alphanumeric())
		{
			return start;
		}
		index = name_at;
	}
	html.len()
}

/// Decode the entities that matter for reading. Anything unknown stays as typed.
fn decode_entities(text: &str) -> String {
	if !text.contains('&') {
		return text.to_owned();
	}
	let mut out = String::with_capacity(text.len());
	let mut rest = text;
	while let Some(at) = rest.find('&') {
		out.push_str(&rest[..at]);
		rest = &rest[at..];
		if let Some((decoded, used)) = entity(rest) {
			out.push(decoded);
			rest = &rest[used..];
		} else {
			out.push('&');
			rest = &rest[1..];
		}
	}
	out.push_str(rest);
	out
}

fn entity(text: &str) -> Option<(char, usize)> {
	let end = text.bytes().take(12).position(|byte| byte == b';')?;
	let body = &text[1..end];
	let decoded = if let Some(number) = body.strip_prefix('#') {
		let code = match number.strip_prefix(['x', 'X']) {
			Some(hex) => u32::from_str_radix(hex, 16).ok()?,
			None => number.parse().ok()?,
		};
		char::from_u32(code)
			.filter(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
			.unwrap_or('\u{FFFD}')
	} else {
		named_entity(body)?
	};
	Some((decoded, end + 1))
}

fn named_entity(name: &str) -> Option<char> {
	Some(match name {
		"amp" => '&',
		"lt" => '<',
		"gt" => '>',
		"quot" => '"',
		"apos" => '\'',
		"nbsp" => '\u{a0}',
		"copy" => '\u{a9}',
		"reg" => '\u{ae}',
		"trade" => '\u{2122}',
		"hellip" => '\u{2026}',
		"mdash" => '\u{2014}',
		"ndash" => '\u{2013}',
		"lsquo" => '\u{2018}',
		"rsquo" => '\u{2019}',
		"ldquo" => '\u{201c}',
		"rdquo" => '\u{201d}',
		"bull" => '\u{2022}',
		"middot" => '\u{b7}',
		"laquo" => '\u{ab}',
		"raquo" => '\u{bb}',
		"euro" => '\u{20ac}',
		"pound" => '\u{a3}',
		"yen" => '\u{a5}',
		"cent" => '\u{a2}',
		"sect" => '\u{a7}',
		"deg" => '\u{b0}',
		"plusmn" => '\u{b1}',
		"times" => '\u{d7}',
		"divide" => '\u{f7}',
		"larr" => '\u{2190}',
		"rarr" => '\u{2192}',
		"uarr" => '\u{2191}',
		"darr" => '\u{2193}',
		"hearts" => '\u{2665}',
		_ => return None,
	})
}

/// Whitespace-collapsed, control-free text of at most `limit` characters, or `None`
/// when nothing readable remains.
fn collapse(text: &str, limit: usize) -> Option<String> {
	let mut out = String::new();
	for character in text.chars() {
		if character.is_whitespace() {
			if !out.is_empty() && !out.ends_with(' ') {
				out.push(' ');
			}
		} else if !character.is_control() && !is_bidi_control(character) {
			out.push(character);
		}
		if out.chars().count() >= limit {
			break;
		}
	}
	let out = out.trim().to_owned();
	(!out.is_empty()).then_some(out)
}

/// Escape every character markdown would otherwise interpret.
fn escape(text: &str) -> String {
	let mut out = String::with_capacity(text.len());
	for character in text.chars() {
		if matches!(
			character,
			'\\' | '`' | '*' | '_' | '~' | '|' | '[' | ']' | '<' | '>' | '#' | '&'
		) {
			out.push('\\');
		}
		out.push(character);
	}
	out
}

/// Bidirectional controls can reorder text to disguise what it says.
fn is_bidi_control(character: char) -> bool {
	matches!(character, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn longest_run(text: &str, target: char) -> usize {
	let mut longest = 0;
	let mut current = 0;
	for character in text.chars() {
		if character == target {
			current += 1;
			longest = longest.max(current);
		} else {
			current = 0;
		}
	}
	longest
}

fn inline_code(content: &str) -> String {
	let fence = "`".repeat(longest_run(content, '`') + 1);
	if content.starts_with('`') || content.ends_with('`') {
		format!("{fence} {content} {fence}")
	} else {
		format!("{fence}{content}{fence}")
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_page_reads_as_headings_paragraphs_and_marks() {
		let html = "<html><head><title>Notes</title></head><body>\
			<h1>Release</h1><p>Plain <b>bold</b> and <i>slanted</i> and <code>code</code>.</p>\
			<p>Second paragraph</p></body></html>";
		let markdown = to_markdown(html);
		assert_eq!(
			markdown,
			"**Notes**\n\n# Release\n\nPlain **bold** and *slanted* and `code`.\n\nSecond paragraph"
		);
	}

	#[test]
	fn scripts_styles_comments_and_frames_never_reach_the_output() {
		let html = "<style>body{color:red}</style><script>alert('x')</script>\
			<!-- secret -->visible<iframe src=\"https://evil.example\">inside</iframe>\
			<svg><text>vector</text></svg><template>hidden</template> end";
		let markdown = to_markdown(html);
		assert_eq!(markdown, "visible end");
	}

	#[test]
	fn an_unterminated_script_swallows_the_rest_without_panicking() {
		assert_eq!(to_markdown("before<script>never closed <b>x"), "before");
		assert_eq!(to_markdown("a<!-- never closed"), "a");
		assert_eq!(to_markdown("a<p class=\"open"), "a");
		assert_eq!(to_markdown("<"), "\\<");
		assert_eq!(to_markdown("a < b"), "a \\< b");
	}

	#[test]
	fn text_is_escaped_so_markdown_cannot_be_injected() {
		let markdown = to_markdown(
			"<p>*not bold* _x_ [a](https://evil.example) &lt;@123&gt; # not a heading ||spoiler||</p>",
		);
		assert_eq!(
			markdown,
			"\\*not bold\\* \\_x\\_ \\[a\\](https://evil.example) \\<@123\\> \\# not a heading \\|\\|spoiler\\|\\|"
		);
		assert_eq!(to_markdown("<p>- dash</p>"), "\\- dash");
		assert_eq!(to_markdown("<p>1. first</p>"), "1\\. first");
		assert_eq!(to_markdown("<p>1984 was</p>"), "1984 was");
		assert_eq!(to_markdown("<p>&amp;amp;</p>"), "\\&amp;");
	}

	#[test]
	fn only_web_links_survive_and_they_keep_their_text() {
		let markdown = to_markdown(
			"<a href=\"https://example.com/a?b=1&amp;c=2\">site</a> \
			<a href=\"javascript:alert(1)\">script</a> \
			<a href=\"file:///etc/passwd\">file</a> \
			<a href=\"data:text/html,x\">data</a>",
		);
		assert_eq!(
			markdown,
			"[site](https://example.com/a?b=1&c=2) script file data"
		);
	}

	#[test]
	fn an_image_is_text_and_never_a_request() {
		let markdown = to_markdown(
			"<img src=\"https://tracker.example/p.gif\" alt=\"Logo *x*\"><img src=\"x\">",
		);
		assert_eq!(markdown, "\\[image: Logo \\*x\\*\\] \\[image\\]");
		assert!(!markdown.contains("tracker.example"));
		assert!(!markdown.contains("!["));
	}

	#[test]
	fn lists_nest_and_number() {
		let markdown = to_markdown(
			"<ul><li>one<ul><li>inner</li></ul></li><li>two</li></ul>\
			<ol><li>first</li><li>second</li></ol>",
		);
		assert_eq!(markdown, "- one\n    - inner\n- two\n\n1. first\n2. second");
	}

	#[test]
	fn empty_list_items_and_paragraphs_in_items_stay_tidy() {
		assert_eq!(to_markdown("<ul><li></li><li>x</li></ul>"), "- x");
		assert_eq!(to_markdown("<ul><li><p>a</p></li></ul>"), "- a");
		assert_eq!(to_markdown("<h2></h2><p>x</p>"), "x");
	}

	#[test]
	fn preformatted_text_keeps_its_lines_and_cannot_close_its_own_fence() {
		let markdown = to_markdown("<pre>\nline 1\n  line 2 ``` tail\n</pre>after");
		assert_eq!(markdown, "````\nline 1\n  line 2 ``` tail\n````\n\nafter");
	}

	#[test]
	fn quotes_prefix_every_line_and_nest() {
		let markdown =
			to_markdown("<blockquote>one<br>two<blockquote>deep</blockquote></blockquote>");
		assert_eq!(markdown, "> one  \n> two\n\n> > deep");
	}

	#[test]
	fn table_rows_become_lines_of_cells() {
		let markdown = to_markdown(
			"<table><tr><th>a</th><th>b</th></tr><tr><td>1</td><td>2</td></tr></table>",
		);
		assert_eq!(markdown, "a | b\n\n1 | 2");
	}

	#[test]
	fn emphasis_hugs_its_text_and_never_crosses_a_paragraph() {
		assert_eq!(to_markdown("a<b> bold </b>c"), "a **bold** c");
		assert_eq!(to_markdown("<b></b>x"), "x");
		assert_eq!(to_markdown("<b>one</p><p>two</b>"), "**one**\n\n**two**");
		assert_eq!(to_markdown("<b>a<br>b</b>"), "**a**  \n**b**");
		assert_eq!(
			to_markdown("<a href=\"https://x.example/\"><div>Card</div><div>Two</div></a>"),
			"[Card](https://x.example/)\n\n[Two](https://x.example/)"
		);
		assert_eq!(to_markdown("<b><p></p></b>after"), "after");
		assert_eq!(
			to_markdown("<ul><li><b>x</b></li><li><b></b></li></ul>"),
			"- **x**"
		);
		assert_eq!(to_markdown("<b>left open"), "left open");
		assert_eq!(to_markdown("<b><i>x</b></i>"), "**x**");
	}

	#[test]
	fn code_spans_are_literal() {
		assert_eq!(to_markdown("<code>a*b_c</code>"), "`a*b_c`");
		assert_eq!(to_markdown("<code>`tick`</code>"), "`` `tick` ``");
		assert_eq!(to_markdown("<code><b>x</b></code>"), "`x`");
	}

	#[test]
	fn entities_decode_and_unknown_ones_stay_as_typed() {
		assert_eq!(
			to_markdown("<p>caf&eacute; &copy; &#65; &#x42; &nosuch;</p>"),
			"caf\\&eacute; \u{a9} A B \\&nosuch;"
		);
		assert_eq!(
			to_markdown("<p>&#0; &#1114112; &#xD800;</p>"),
			"\u{FFFD} \u{FFFD} \u{FFFD}"
		);
	}

	#[test]
	fn controls_and_bidirectional_overrides_are_removed() {
		assert_eq!(to_markdown("<p>a\u{202e}b\u{7}c\u{200f}d</p>"), "abcd");
		assert_eq!(to_markdown("<p>x</p><p>\t \n</p><p>y</p>"), "x\n\ny");
	}

	#[test]
	fn hostile_input_stays_bounded_and_does_not_stall() {
		use std::time::Instant;
		let started = Instant::now();
		for html in [
			"<a ".repeat(100_000),
			"<b>".repeat(100_000),
			"<ul><li>".repeat(50_000),
			"<blockquote>".repeat(50_000) + "x",
			"&".repeat(300_000),
			"<p>".to_owned() + &"x".repeat(2_000_000),
			"<!--".repeat(100_000),
			"<script>".repeat(50_000),
			"<table><tr><td>".repeat(30_000),
		] {
			let markdown = to_markdown(&html);
			assert!(markdown.chars().count() <= MAX_PREVIEW_CHARS);
		}
		assert!(
			started.elapsed().as_secs() < 20,
			"conversion took {:?}",
			started.elapsed()
		);
	}

	#[test]
	fn a_page_at_the_fetch_cap_converts_in_a_blink() {
		use std::time::Instant;
		let row = "<div class=\"row\"><h2>Section</h2><p>Some <b>bold</b>, <i>slanted</i> and a \
			<a href=\"https://example.com/page?a=1&amp;b=2\">link</a> &amp; entities.</p>\
			<ul><li>one</li><li>two <code>x*y</code></li></ul>\
			<table><tr><td>a</td><td>b</td></tr></table><script>var x = 1;</script></div>\n";
		let html = row.repeat(crate::text_preview::MAX_PREVIEW_BYTES as usize / row.len());
		assert!(html.len() <= crate::text_preview::MAX_PREVIEW_BYTES as usize);
		let started = Instant::now();
		let markdown = to_markdown(&html);
		let convert = started.elapsed();
		let started = Instant::now();
		let formatted = crate::markdown::Formatted::parse_document(
			&markdown
				.chars()
				.take(crate::text_preview::PREVIEW_WINDOW_CHARS)
				.collect::<String>(),
		);
		println!(
			"html {} bytes -> markdown {} chars in {convert:?}; first window parsed in {:?}",
			html.len(),
			markdown.chars().count(),
			started.elapsed()
		);
		assert!(formatted.bytes() > 0 && !markdown.is_empty());
		assert!(convert.as_millis() < 2_000, "converted in {convert:?}");
	}

	#[test]
	fn the_output_never_exceeds_the_preview_window() {
		let html = format!("<p>{}</p>", "word ".repeat(300_000));
		assert!(to_markdown(&html).chars().count() <= MAX_PREVIEW_CHARS);
	}

	#[test]
	fn the_real_renderer_shows_escaped_text_exactly_as_the_page_says_it() {
		let page = "<p>*x* _y_ ||z|| &lt;@123&gt; [a](https://evil.example) # h \
			&lt;:emoji:1&gt; `c` ~~s~~</p>";
		let shown = crate::markdown::plain_text(&to_markdown(page));
		assert_eq!(
			shown,
			"*x* _y_ ||z|| <@123> [a](https://evil.example) # h <:emoji:1> `c` ~~s~~"
		);
	}

	#[test]
	fn the_converted_page_survives_the_real_markdown_renderer() {
		let html = "<h1>T</h1><ul><li>a <a href=\"https://example.com\">b</a></li></ul>\
			<blockquote>q</blockquote><pre>code</pre><table><tr><td>c</td></tr></table>\
			<p>*&lt;@1&gt;* ||x||</p>";
		let markdown = to_markdown(html);
		let formatted = crate::markdown::Formatted::parse_document(&markdown);
		assert!(formatted.bytes() > 0);
	}
}
