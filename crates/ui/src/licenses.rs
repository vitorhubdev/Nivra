//! Every license text the packaged app ships, embedded deflate-compressed by `build.rs` and
//! decoded only when one is opened, plus the screen that lists them.
use crate::{design, dialog, i18n};
use egui::RichText;
use model::Language;
use std::io::Read;

/// One embedded license text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
	/// Path of the same file inside the release package, such as `licenses/voice/x.txt`.
	pub id: &'static str,
	/// File name shown in the list.
	pub title: &'static str,
	/// Key of the component group the text belongs to.
	pub component: &'static str,
}

struct Bundled {
	entry: Entry,
	data: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/license_bundle.rs"));

const REPOSITORY: &str = "https://github.com/vitorhubdev/Nivra";

/// Static lead-in of the line naming where MPL-2.0 source is published.
pub const MPL_SOURCE_PREFIX: &str =
	"Source code for the MPL-2.0 components in this version is published on its release page as";

/// Component groups in display order, with the English label used as the i18n key.
const COMPONENTS: &[(&str, &str)] = &[
	("nivraext", "Nivra"),
	("sounds", "Notification sounds"),
	("fonts", "Fonts"),
	("emoji", "Emoji"),
	("icons", "Icons"),
	("files", "Core libraries"),
	("notifications", "Notifications"),
	("login", "Sign-in"),
	("voice", "Voice & Video"),
	("audio", "Audio playback"),
	("dependencies", "Other dependencies"),
];

fn rank(component: &str) -> usize {
	COMPONENTS
		.iter()
		.position(|(key, _)| *key == component)
		.unwrap_or(COMPONENTS.len())
}

fn english_label(component: &'static str) -> &'static str {
	COMPONENTS
		.iter()
		.find(|(key, _)| *key == component)
		.map_or(component, |(_, label)| label)
}

/// Translated heading of a component group.
pub fn component_label(language: Language, component: &'static str) -> &'static str {
	i18n::text(language, english_label(component))
}

/// Every embedded entry, grouped by component in display order, then by file name.
pub fn all() -> Vec<&'static Entry> {
	let mut entries: Vec<_> = BUNDLE.iter().map(|bundled| &bundled.entry).collect();
	entries.sort_by(|a, b| {
		(rank(a.component), a.title, a.id).cmp(&(rank(b.component), b.title, b.id))
	});
	entries
}

pub fn find(id: &str) -> Option<&'static Entry> {
	BUNDLE
		.iter()
		.find(|bundled| bundled.entry.id == id)
		.map(|bundled| &bundled.entry)
}

/// Decoded text of `id`; empty when nothing with that id is embedded.
pub fn text(id: &str) -> String {
	let Some(bundled) = BUNDLE.iter().find(|bundled| bundled.entry.id == id) else {
		return String::new();
	};
	let mut text = String::new();
	flate2::read::DeflateDecoder::new(bundled.data)
		.read_to_string(&mut text)
		.expect("build.rs embeds valid deflated UTF-8");
	text
}

/// Name of the release asset carrying the MPL-2.0 sources for this version.
pub fn source_archive_name() -> String {
	format!(
		"Nivra-{}-third-party-sources.zip",
		env!("CARGO_PKG_VERSION")
	)
}

/// Release page of this version, where the source archive is attached.
pub fn release_page() -> String {
	format!("{REPOSITORY}/releases/tag/v{}", env!("CARGO_PKG_VERSION"))
}

/// Licenses screen state. Only the selected text is ever held decoded.
#[derive(Default)]
pub struct LicensesUi {
	pub open: bool,
	pub selected: Option<&'static str>,
	pub filter: String,
	detail: Option<String>,
	drawn_pass: Option<u64>,
}

impl LicensesUi {
	/// Opens the screen on the full list.
	pub fn open(&mut self) {
		self.open = true;
		self.filter.clear();
		self.back();
	}
	pub fn close(&mut self) {
		self.open = false;
		self.back();
	}
	/// Shows the full text of `id`. Returns false, changing nothing, for unknown ids.
	pub fn select(&mut self, id: &str) -> bool {
		let Some(entry) = find(id) else {
			return false;
		};
		self.selected = Some(entry.id);
		self.detail = Some(text(entry.id));
		true
	}
	/// Returns from a text to the list.
	pub fn back(&mut self) {
		self.selected = None;
		self.detail = None;
	}
	pub fn set_filter(&mut self, filter: impl Into<String>) {
		self.filter = filter.into();
	}
	/// Entries whose path, file name or component contains every word of the filter,
	/// ignoring case, in [`all`] order.
	pub fn visible_entries(&self) -> Vec<&'static Entry> {
		let words: Vec<String> = self
			.filter
			.split_whitespace()
			.map(str::to_lowercase)
			.collect();
		all()
			.into_iter()
			.filter(|entry| {
				let haystack = format!(
					"{} {} {}",
					entry.id,
					entry.component,
					english_label(entry.component)
				)
				.to_lowercase();
				words.iter().all(|word| haystack.contains(word.as_str()))
			})
			.collect()
	}
	/// Full text of the selected entry.
	pub fn detail_text(&self) -> Option<&str> {
		self.detail.as_deref()
	}
}

/// Draws the licenses screen while it is open. Hosts that may both reach it in one pass
/// (settings and the first-run notices) can call this unconditionally; it draws once.
pub fn show(ctx: &egui::Context, licenses: &mut LicensesUi, language: Language) {
	if !licenses.open {
		return;
	}
	let pass = ctx.cumulative_pass_nr();
	if licenses.drawn_pass == Some(pass) {
		return;
	}
	licenses.drawn_pass = Some(pass);
	let response = dialog::Dialog::new("licenses", i18n::text(language, "Licenses"))
		.width(720.0)
		.show(ctx, |d| {
			d.content(|ui| {
				let colors = design::palette(ui);
				ui.horizontal_wrapped(|ui| {
					ui.spacing_mut().item_spacing.x = 4.0;
					ui.label(
						RichText::new(i18n::text(language, MPL_SOURCE_PREFIX))
							.size(13.0)
							.color(colors.muted),
					);
					ui.label(
						RichText::new(source_archive_name())
							.size(13.0)
							.color(colors.text),
					);
					let page = release_page();
					ui.hyperlink_to(RichText::new(&page).size(13.0), page);
				});
				ui.add_space(4.0);
				if licenses.selected.is_none() {
					dialog::input(
						ui,
						egui::TextEdit::singleline(&mut licenses.filter)
							.hint_text(i18n::text(language, "Filter licenses")),
					);
				}
			});
			if let Some(entry) = licenses.selected.and_then(find) {
				d.content(|ui| {
					let colors = design::palette(ui);
					if design::text_action(ui, i18n::text(language, "All licenses")).clicked() {
						licenses.back();
						return;
					}
					ui.label(design::semibold(ui, entry.title, 16.0).color(colors.text_strong));
					ui.label(RichText::new(entry.id).size(12.0).color(colors.muted));
				});
				if let Some(text) = licenses.detail_text() {
					let text = text.to_owned();
					d.scroll(260.0, |ui| {
						let colors = design::palette(ui);
						ui.add(
							egui::Label::new(
								RichText::new(text)
									.size(12.0)
									.monospace()
									.color(colors.text),
							)
							.wrap()
							.selectable(true),
						);
					});
				}
			} else {
				let entries = licenses.visible_entries();
				let mut picked = None;
				d.scroll(220.0, |ui| {
					let colors = design::palette(ui);
					if entries.is_empty() {
						design::hint(ui, i18n::text(language, "No licenses match this filter."));
					}
					ui.spacing_mut().item_spacing.y = 2.0;
					for group in entries.chunk_by(|a, b| a.component == b.component) {
						ui.add_space(8.0);
						ui.label(design::eyebrow(
							ui,
							component_label(language, group[0].component),
							colors.muted,
						));
						for entry in group {
							if crate::settings::nav_item(ui, entry.title, false)
								.on_hover_text(entry.id)
								.clicked()
							{
								picked = Some(entry.id);
							}
						}
					}
				});
				if let Some(id) = picked {
					licenses.select(id);
				}
			}
		});
	if response.close {
		licenses.close();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::collections::BTreeSet;
	use std::path::{Path, PathBuf};

	/// Directories `cargo xtask package` copies whole from `assets/licenses`.
	const DIRECTORIES: &[&str] = &[
		"files",
		"notifications",
		"login",
		"voice",
		"audio",
		"dependencies",
	];
	/// `(source relative to the workspace root, package path)` for the single files.
	const SINGLES: &[(&str, &str)] = &[
		("assets/sounds/README.md", "licenses/notification-sounds.md"),
		(
			"assets/fonts/NotoSansCJK-LICENSE.txt",
			"licenses/NotoSansCJK-LICENSE.txt",
		),
		(
			"assets/fonts/NotoSansArabic-OFL.txt",
			"licenses/NotoSansArabic-OFL.txt",
		),
		(
			"assets/fonts/NotoSansMath-OFL.txt",
			"licenses/NotoSansMath-OFL.txt",
		),
		("assets/fonts/Inter-OFL.txt", "licenses/Inter-OFL.txt"),
		(
			"assets/twemoji/LICENSE-GRAPHICS",
			"licenses/Twemoji-CC-BY-4.0.txt",
		),
		(
			"assets/twemoji/LICENSE-UNICODE",
			"licenses/Unicode-LICENSE.txt",
		),
		("assets/icons/LICENSE", "licenses/Phosphor-Icons-MIT.txt"),
		(
			"assets/icons/LICENSE-SIMPLE-ICONS",
			"licenses/Simple-Icons-CC0.txt",
		),
		("LICENSE-MIT", "LICENSE-MIT"),
		("LICENSE-APACHE", "LICENSE-APACHE"),
		("THIRD_PARTY_NOTICES.md", "THIRD_PARTY_NOTICES.md"),
	];
	const XTASK: &str = include_str!("../../../tools/xtask/src/main.rs");

	fn root() -> PathBuf {
		Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
	}

	fn walk(directory: &Path, files: &mut Vec<PathBuf>) {
		for entry in std::fs::read_dir(directory).unwrap() {
			let entry = entry.unwrap();
			if entry.file_name() == "pr-evidence" {
				continue;
			}
			if entry.file_type().unwrap().is_dir() {
				walk(&entry.path(), files);
			} else if !entry.file_name().to_string_lossy().ends_with(".crate") {
				files.push(entry.path());
			}
		}
	}

	/// Package path → source on disk for everything packaging stages as license text.
	fn expected() -> Vec<(String, PathBuf)> {
		let root = root();
		let mut expected: Vec<_> = SINGLES
			.iter()
			.map(|(source, id)| ((*id).to_owned(), root.join(source)))
			.collect();
		for directory in DIRECTORIES {
			let base = root.join("assets/licenses").join(directory);
			let mut files = Vec::new();
			walk(&base, &mut files);
			assert!(!files.is_empty(), "{directory} has no license files");
			for path in files {
				let relative = path
					.strip_prefix(&base)
					.unwrap()
					.to_string_lossy()
					.replace('\\', "/");
				expected.push((format!("licenses/{directory}/{relative}"), path));
			}
		}
		expected
	}

	#[test]
	fn every_packaged_license_is_embedded_with_its_text() {
		let expected = expected();
		for (id, source) in &expected {
			let embedded = text(id);
			assert!(!embedded.is_empty(), "{id} is not embedded");
			assert_eq!(
				embedded.as_bytes(),
				std::fs::read(source).unwrap(),
				"{id} differs from {}",
				source.display()
			);
		}
		let expected: BTreeSet<_> = expected.iter().map(|(id, _)| id.as_str()).collect();
		for entry in all() {
			assert!(
				expected.contains(entry.id),
				"{} is embedded but has no packaged source",
				entry.id
			);
		}
		assert_eq!(all().len(), expected.len());
		assert!(
			all().iter().all(|entry| !entry.id.ends_with(".crate")),
			"source archives belong in the release zip, not the binary"
		);
		assert!(find("README.md").is_none());
	}

	#[test]
	fn packaging_copies_no_license_directory_the_bundle_misses() {
		let copied: BTreeSet<_> = XTASK
			.match_indices("\"assets/licenses/")
			.map(|(start, needle)| {
				let rest = &XTASK[start + needle.len()..];
				&rest[..rest.find('"').unwrap()]
			})
			.collect();
		for directory in &copied {
			assert!(
				DIRECTORIES.contains(directory),
				"xtask copies assets/licenses/{directory}, which the licenses screen does not embed"
			);
		}
		assert_eq!(copied.len(), DIRECTORIES.len(), "{copied:?}");
		for (_, id) in SINGLES {
			let name = id.rsplit('/').next().unwrap();
			assert!(XTASK.contains(name), "xtask no longer stages {id}");
		}
	}

	#[test]
	fn registry_is_grouped_and_every_component_is_labelled() {
		let entries = all();
		assert!(entries.windows(2).all(|pair| {
			(rank(pair[0].component), pair[0].title) <= (rank(pair[1].component), pair[1].title)
		}));
		for entry in &entries {
			assert!(
				rank(entry.component) < COMPONENTS.len(),
				"{}",
				entry.component
			);
			assert_eq!(entry.id.rsplit('/').next(), Some(entry.title));
		}
		const { assert!(COMPRESSED_BYTES < RAW_BYTES) };
		for (key, label) in COMPONENTS.iter().filter(|(key, _)| *key != "nivraext") {
			for language in [Language::PortugueseBrazil, Language::Spanish] {
				assert_ne!(component_label(language, key), *label, "{language:?}");
			}
		}
	}

	#[test]
	fn screen_opens_selects_and_filters() {
		let mut screen = LicensesUi::default();
		assert!(!screen.open);
		screen.set_filter("stale");
		screen.open();
		assert!(screen.open);
		assert!(screen.filter.is_empty());
		assert_eq!(screen.visible_entries(), all());
		assert_eq!(screen.detail_text(), None);

		assert!(screen.select("THIRD_PARTY_NOTICES.md"));
		assert_eq!(screen.selected, Some("THIRD_PARTY_NOTICES.md"));
		assert!(
			screen
				.detail_text()
				.is_some_and(|text| text.starts_with("# Third-party notices")),
			"{:?}",
			screen.detail_text().map(|text| &text[..80.min(text.len())])
		);
		assert!(!screen.select("licenses/missing.txt"));
		assert_eq!(screen.selected, Some("THIRD_PARTY_NOTICES.md"));
		screen.back();
		assert_eq!((screen.selected, screen.detail_text()), (None, None));

		screen.set_filter("SYMPHONIA mpl");
		let visible = screen.visible_entries();
		assert!(!visible.is_empty());
		assert!(visible.iter().all(|entry| {
			let id = entry.id.to_lowercase();
			id.contains("symphonia") && id.contains("mpl")
		}));
		screen.set_filter("voice");
		assert!(
			screen
				.visible_entries()
				.iter()
				.all(|e| e.component == "voice")
		);
		assert!(screen.visible_entries().len() > 100);
		screen.set_filter("no-such-license-anywhere");
		assert!(screen.visible_entries().is_empty());

		screen.select("LICENSE-MIT");
		screen.close();
		assert!(!screen.open);
		assert_eq!(screen.detail_text(), None);
	}

	#[test]
	fn source_line_names_this_versions_release() {
		let version = env!("CARGO_PKG_VERSION");
		assert_eq!(
			source_archive_name(),
			format!("Nivra-{version}-third-party-sources.zip")
		);
		assert_eq!(
			release_page(),
			format!("https://github.com/vitorhubdev/Nivra/releases/tag/v{version}")
		);
	}
}
