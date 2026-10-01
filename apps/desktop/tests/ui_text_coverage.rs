//! Coverage guard: user-visible text must go through the i18n catalog.
//!
//! A literal handed straight to a widget (`ui.label("…")`, `RichText::new("…")`,
//! `on_hover_text("…")`, …) can never be translated, so this test fails when one
//! appears outside the catalog module. A few strings are deliberately literal:
//! format-only fragments, synthetic demo data and proper nouns, listed below.

use std::{fs, path::Path};

/// (needle, reason) pairs that are allowed to stay literal.
const ALLOWED: &[(&str, &str)] = &[
	("Nivra", "product name"),
	("Discord", "product name"),
	("#RRGGBB", "colour placeholder"),
	// Synthetic demo content: fixed text, not interface copy.
	("discord.gg/", "example invite"),
	("hTKzmak", "synthetic invite sample"),
	("general", "synthetic channel name"),
	("Synthetic Latin text", "demo content"),
	("Synthetic navigation", "demo content"),
	("\u{65e5}\u{672c}\u{8a9e}", "demo content"),
	(
		"Welcome back. All of this is synthetic, offline data.",
		"synthetic demo banner",
	),
	(
		"Resize, maximize/restore, and move between monitors.",
		"rendering harness banner",
	),
	// Pending: still literal because the surface has no pt-BR/es string yet.
	// Every new literal fails this test; this list is the known debt.
	("Forget account", "pending translation"),
	("Keep", "pending translation"),
	("Discard and Continue", "pending translation"),
	("Keep Working", "pending translation"),
	(
		"Resize, maximize/restore, and move between monitors.\\\\nAt rest, surface and client dimensions must match.",
		"pending translation",
	),
	("Loading older threads…", "pending translation"),
	("Started by", "pending translation"),
	("•", "pending translation"),
	("Preview", "pending translation"),
	("Copy download link", "pending translation"),
	("Open original…", "pending translation"),
	("Copy link", "pending translation"),
	("Cancel download", "pending translation"),
	("Dismiss", "pending translation"),
	("Loading audio…", "pending translation"),
	("No conversations available here.", "pending translation"),
	("Notification Settings", "pending translation"),
	("Loading channel settings…", "pending translation"),
	("Discard", "pending translation"),
	("+ Add role or member", "pending translation"),
	("Remove Role / Member", "pending translation"),
	("Reveal spoiler component", "pending translation"),
	("Reveal spoiler media", "pending translation"),
	("Submitting…", "pending translation"),
	("Clear selection", "pending translation"),
	("No matching options loaded", "pending translation"),
	(
		"Refine your search to see more results",
		"pending translation",
	),
	(
		"Type to search members; available roles and channels are listed",
		"pending translation",
	),
	("Choose files…", "pending translation"),
	("Reveal spoiler attachment", "pending translation"),
	("Open media", "pending translation"),
	("Loading note…", "pending translation"),
	("Retry", "pending translation"),
	("Action", "pending translation"),
	("This cannot be undone.", "pending translation"),
	("Continue", "pending translation"),
	("Cancel", "pending translation"),
	("Enable", "pending translation"),
	("Line", "pending translation"),
	("Text display limited", "pending translation"),
	("Embed display limited", "pending translation"),
	(
		"Additional embed content is not supported",
		"pending translation",
	),
	("A custom emoji.", "pending translation"),
	("Copy emoji", "pending translation"),
	("Remove emoji", "pending translation"),
	("Retry sticker packs", "pending translation"),
	(
		"Showing the first 1,000 custom emoji. Refine your search for more.",
		"pending translation",
	),
	("Hover a sticker to preview it", "pending translation"),
	("Click a GIF to send it right away", "pending translation"),
	("Hover an emoji to preview it", "pending translation"),
	("Loading trending categories…", "pending translation"),
	("View source", "pending translation"),
	("Enter a message...", "pending translation"),
	("Posting…", "pending translation"),
	("Latest message unavailable", "pending translation"),
	("·", "pending translation"),
	("Archived", "pending translation"),
	("Loading posts…", "pending translation"),
	("Load more posts", "pending translation"),
	("Load archived posts", "pending translation"),
	(
		"Archived posts need a connected session with history access.",
		"pending translation",
	),
	("Loading archived posts…", "pending translation"),
	("Older archived posts", "pending translation"),
	("No older archived posts reported.", "pending translation"),
	("No matching destinations", "pending translation"),
	(
		"Source message is no longer available",
		"pending translation",
	),
	(
		"Group actions unavailable while disconnected or busy.",
		"pending translation",
	),
	("Choosing image…", "pending translation"),
	("Move up", "pending translation"),
	("Move down", "pending translation"),
	("Folder name and color…", "pending translation"),
	("Ungroup servers", "pending translation"),
	("Move outside folders", "pending translation"),
	("Group with server", "pending translation"),
	("Sync…", "pending translation"),
	("Not sure?", "pending translation"),
	("for now.", "pending translation"),
	("Invites look like", "pending translation"),
	(
		"Choose a conversation to see its people.",
		"pending translation",
	),
	("No people returned for this view.", "pending translation"),
	("Retry shortcuts", "pending translation"),
	(
		"· Save requested, check the connection before retrying",
		"pending translation",
	),
	("Copy edit text", "pending translation"),
	("Replying to ", "pending translation"),
	("View original", "pending translation"),
	(
		"Draft budget full. Clear an existing draft to continue.",
		"pending translation",
	),
	("Clear this draft", "pending translation"),
	(
		"Pick a channel or direct message from the list.",
		"pending translation",
	),
	("Delete", "pending translation"),
	("Keep Message", "pending translation"),
	("Delete selected", "pending translation"),
	("Keep Messages", "pending translation"),
	("Reveal spoiler", "pending translation"),
	("↑↓ choose · Tab/Enter insert · Esc", "pending translation"),
	(
		"Check the conversation before sending again.",
		"pending translation",
	),
	("Restore to composer", "pending translation"),
	("Loading your profile…", "pending translation"),
	("Saving profile…", "pending translation"),
	("You have unsaved changes.", "pending translation"),
	("Leave blank to use your username.", "pending translation"),
	("Copy activity", "pending translation"),
	("Copy webhook ID", "pending translation"),
	("Remove Friend", "pending translation"),
	(
		"This account was deleted. The conversation stays so you can read it.",
		"pending translation",
	),
	("Loading profile…", "pending translation"),
	("Retry profile", "pending translation"),
	("Edit profile", "pending translation"),
	("Offline preview · synthetic", "pending translation"),
	("Loading reactions…", "pending translation"),
	(
		"No matching users in this conversation.",
		"pending translation",
	),
	(
		"Pinned messages are unavailable while disconnected or without channel access.",
		"pending translation",
	),
	(
		"More pins may exist, but this page has no usable continuation.",
		"pending translation",
	),
	("Order on this page", "pending translation"),
	(
		"Indexing is incomplete; results may be missing.",
		"pending translation",
	),
	(
		"Spoiler media - open the message to reveal it.",
		"pending translation",
	),
	("No matching users", "pending translation"),
	("Remove dates", "pending translation"),
	("Clear Filters", "pending translation"),
	("Copy", "pending translation"),
	("Save .txt", "pending translation"),
	("Rename", "pending translation"),
	("Profile", "pending translation"),
	("Message", "pending translation"),
	("Change Nickname", "pending translation"),
	("Copy User ID", "pending translation"),
	("Recipients will land in", "pending translation"),
	("Edit link.", "pending translation"),
	("Load Invites", "pending translation"),
	(
		"Members use the color of the highest role they have on this list. Drag roles to reorder them.",
		"pending translation",
	),
	("Edit Role", "pending translation"),
	("Move Up", "pending translation"),
	("Move Down", "pending translation"),
	("← Back to Roles", "pending translation"),
	("Remove Icon", "pending translation"),
	("Edit", "pending translation"),
	("▾", "pending translation"),
	("This server has no stickers yet.", "pending translation"),
	(
		"Showing the first 500 stickers. Search to narrow the results.",
		"pending translation",
	),
	("No stickers found.", "pending translation"),
	("Loading sticker packs…", "pending translation"),
	("Loading sticker details…", "pending translation"),
	("Retry sticker details", "pending translation"),
	(
		"Finish composing text before opening or closing.",
		"pending translation",
	),
	(
		"Try a channel, server or person name.",
		"pending translation",
	),
	(
		"This is the beginning of the conversation.",
		"pending translation",
	),
	("Thread started from this message", "pending translation"),
	("Toggle Deleted Highlight", "pending translation"),
	("Remove Message", "pending translation"),
	("Extensions", "pending translation"),
	(". See all ", "pending translation"),
	("used", "pending translation"),
	("Message deleted", "pending translation"),
	("\\\\u{21aa} Forwarded", "pending translation"),
	("[Deleted message had no text]", "pending translation"),
	(
		"Display limited · Copy message for the full text",
		"pending translation",
	),
	("Hide spoilers", "pending translation"),
	("(edited)", "pending translation"),
	("Application interaction pending…", "pending translation"),
	("Only you can see this  •", "pending translation"),
	("Dismiss message", "pending translation"),
	(
		"Participant list unavailable with the current access.",
		"pending translation",
	),
	(
		"Last known participants · reconnect to refresh",
		"pending translation",
	),
	(
		"Camera capture is unavailable on this platform.",
		"pending translation",
	),
	("Choose camera", "pending translation"),
	("Open voice", "pending translation"),
];

fn call_sites(source: &str) -> Vec<(usize, &str, String)> {
	// (pattern, label) pairs for the widget calls that paint text.
	const PATTERNS: &[(&str, &str)] = &[
		("label(", "label"),
		("ui.small(", "small"),
		("ui.heading(", "heading"),
		("button(", "button"),
		("small_button(", "small button"),
		("secondary_button(", "secondary button"),
		("on_hover_text(", "hover"),
		("RichText::new(", "rich text"),
		("semibold(", "semibold"),
		("selectable_label(", "selectable label"),
		("wrap_label(", "wrap label"),
		("hint_text(", "hint"),
	];
	let mut found: Vec<(usize, &str, String)> = Vec::new();
	for (pattern, label) in PATTERNS {
		let mut from = 0;
		while let Some(at) = source[from..].find(pattern) {
			let start = from + at + pattern.len();
			from = start;
			let rest = &source[start..];
			let trimmed = rest.trim_start();
			if !trimmed.starts_with('"') {
				continue;
			}
			let quote = start + (rest.len() - trimmed.len());
			let Some(end) = source[quote + 1..].find('"') else {
				continue;
			};
			let literal = source[quote + 1..quote + 1 + end].to_owned();
			if literal.is_empty() || literal.starts_with('#') || literal.starts_with('{') {
				continue;
			}
			// A call that already asks for a translation is fine.
			let before = &source[..quote];
			if before.trim_end().ends_with("tr_ui!")
				|| before.trim_end().ends_with("tr_str!")
				|| before.trim_end().ends_with("text(")
				|| before.trim_end().ends_with("text_str(")
			{
				continue;
			}
			let line = source[..quote].matches('\n').count() + 1;
			found.push((line, label, literal));
		}
	}
	found.sort();
	found
}

fn allowed(literal: &str) -> bool {
	ALLOWED
		.iter()
		.any(|(needle, _)| literal.contains(needle) || needle.contains(literal))
}

fn sources() -> Vec<std::path::PathBuf> {
	// Integration tests run with the package directory as the working directory.
	let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
	let mut files = Vec::new();
	for directory in ["crates/ui/src", "apps/desktop/src"] {
		collect(&root.join(directory), &mut files);
	}
	files.sort();
	files
}

fn collect(directory: &Path, files: &mut Vec<std::path::PathBuf>) {
	let Ok(entries) = fs::read_dir(directory) else {
		return;
	};
	for entry in entries.flatten() {
		let path = entry.path();
		if path.is_dir() {
			collect(&path, files);
		} else if path.extension().is_some_and(|extension| extension == "rs")
			&& path.file_name().is_some_and(|name| name != "i18n.rs")
		{
			files.push(path);
		}
	}
}

#[test]
fn user_visible_text_goes_through_the_catalog() {
	let offenders: Vec<String> = sources()
		.into_iter()
		.flat_map(|path| {
			let source = fs::read_to_string(&path).expect("source file");
			call_sites(&source)
				.into_iter()
				.filter(|(_, _, literal)| !allowed(literal))
				.map(|(line, label, literal)| {
					format!("{}:{line} {label} {literal:?}", path.display())
				})
				.collect::<Vec<_>>()
		})
		.collect();
	assert!(
		offenders.is_empty(),
		"{} literal(s) bypass the catalog; wrap them in tr_ui!/tr_str! or add the\nstring to ALLOWED:\n{}",
		offenders.len(),
		offenders.join("\n")
	);
}
