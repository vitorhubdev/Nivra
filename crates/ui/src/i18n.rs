//! Small built-in locale catalog, one file per language.
//!
//! `locales/en.ftl` lists every key the app knows; `pt-BR.ftl` and `es.ftl` carry the
//! translations. Adding a language is adding a file plus one line in `bundled()` — no
//! screen changes. A key missing from a file falls back to English, and the
//! completeness test names it instead of letting it slip through.

use model::Language;

/// Raw locale tables, embedded in the binary at build time.
const EN_FTL: &str = include_str!("../locales/en.ftl");
const PT_BR_FTL: &str = include_str!("../locales/pt-BR.ftl");
const ES_FTL: &str = include_str!("../locales/es.ftl");

/// One parsed table: English text to this language's text.
type Table = std::collections::HashMap<&'static str, &'static str>;

/// Fluent subset used by these files: `\"`, `\\`, `\n`, `\t` and `\u{…}`.
fn unescape(value: &str) -> &'static str {
	let mut out = String::with_capacity(value.len());
	let mut characters = value.chars();
	while let Some(character) = characters.next() {
		if character != '\\' {
			out.push(character);
			continue;
		}
		match characters.next() {
			Some('n') => out.push('\n'),
			Some('t') => out.push('\t'),
			Some('"') => out.push('"'),
			Some('\\') => out.push('\\'),
			Some('u') => {
				let mut digits = String::new();
				for next in characters.by_ref() {
					if next == '}' {
						break;
					}
					digits.push(next);
				}
				if let Some(hex) = digits.strip_prefix('{')
					&& let Some(code) = u32::from_str_radix(hex, 16).ok()
					&& let Some(character) = char::from_u32(code)
				{
					out.push(character);
				}
			}
			Some(other) => out.push(other),
			None => out.push('\\'),
		}
	}
	// The tables are parsed once and live for the process, as the strings did.
	Box::leak(out.into_boxed_str())
}

/// `key = "value"` per line; comments start with `#`.
fn parse(source: &'static str) -> Table {
	let mut table = Table::with_capacity(2048);
	for line in source.lines() {
		let line = line.trim();
		if line.is_empty() || line.starts_with('#') {
			continue;
		}
		let Some((key, value)) = line.split_once(" = ") else {
			continue;
		};
		let key = unescape(key.trim());
		let Some(value) = value
			.trim()
			.strip_prefix('"')
			.and_then(|v| v.strip_suffix('"'))
		else {
			continue;
		};
		table.entry(key).or_insert_with(|| unescape(value));
	}
	table
}

/// Every bundled table, parsed once on first use.
fn bundled() -> &'static [(Language, Table)] {
	static TABLES: std::sync::OnceLock<Vec<(Language, Table)>> = std::sync::OnceLock::new();
	TABLES.get_or_init(|| {
		vec![
			(Language::PortugueseBrazil, parse(PT_BR_FTL)),
			(Language::Spanish, parse(ES_FTL)),
			(Language::English, parse(EN_FTL)),
		]
	})
}

fn table(language: Language) -> Option<&'static Table> {
	bundled()
		.iter()
		.find(|(bundled_language, _)| *bundled_language == language)
		.map(|(_, table)| table)
}

#[cfg(test)]
fn table_keys(language: Language) -> impl Iterator<Item = &'static str> {
	table(language)
		.into_iter()
		.flat_map(|table| table.keys().copied())
}

fn lookup(language: Language, key: &str) -> Option<&'static str> {
	table(language)?.get(key).copied()
}

/// Every key the app knows, from the English manifest.
#[cfg(any(test, feature = "demo"))]
fn known_keys() -> impl Iterator<Item = &'static str> {
	table(Language::English)
		.into_iter()
		.flat_map(|table| table.keys().copied())
}

pub fn text(language: Language, english: &'static str) -> &'static str {
	let translated = match language {
		Language::English => return english,
		_ => lookup(language, english),
	};
	match translated {
		Some(text) => text,
		None => {
			#[cfg(test)]
			if known_keys().any(|key| key == english) {
				UNTRANSLATED_KEYS.with(|keys| keys.borrow_mut().push(english.to_owned()));
			}
			english
		}
	}
}

/// Translate a runtime string (an error or a status that only exists at run time).
/// Known keys come from the catalog; anything else is returned unchanged.
pub fn text_str(language: Language, english: &str) -> std::borrow::Cow<'_, str> {
	let translated = match language {
		Language::English => return std::borrow::Cow::Borrowed(english),
		_ => lookup(language, english),
	};
	match translated {
		Some(text) => std::borrow::Cow::Borrowed(text),
		None => {
			#[cfg(test)]
			if known_keys().any(|key| key == english) {
				// Only known keys are recorded, so runtime statuses stay quiet.
				UNTRANSLATED_KEYS.with(|keys| keys.borrow_mut().push(english.to_owned()));
			}
			std::borrow::Cow::Borrowed(english)
		}
	}
}

const INTERFACE_LANGUAGE_KEY: &str = "nivra-interface-language";

pub fn store_interface_language(ctx: &egui::Context, language: Language) {
	ctx.data_mut(|data| data.insert_temp(egui::Id::unique(INTERFACE_LANGUAGE_KEY), language));
}

/// Language for shared chrome; English when no frame stored one (tests, previews).
pub fn interface_language(ctx: &egui::Context) -> Language {
	ctx.data(|data| data.get_temp::<Language>(egui::Id::unique(INTERFACE_LANGUAGE_KEY)))
		.unwrap_or(Language::English)
}

/// Translate using the language stored on the current egui context.
#[macro_export]
macro_rules! tr_ui {
	($ui:expr, $english:expr) => {
		$crate::i18n::text($crate::i18n::interface_language($ui.ctx()), $english)
	};
}

/// Translate a runtime string using the language stored on the current egui context.
#[macro_export]
macro_rules! tr_str {
	($ui:expr, $english:expr) => {
		$crate::i18n::text_str($crate::i18n::interface_language($ui.ctx()), $english)
	};
}

/// Translate using the language stored on an egui context (dialogs without `Ui`).
#[macro_export]
macro_rules! tr_ctx {
	($ctx:expr, $english:expr) => {
		$crate::i18n::text($crate::i18n::interface_language($ctx), $english)
	};
}

/// Keys that fell back to English on this thread since the last drain. Tests
/// render surfaces in another language and require this list to stay empty.
#[cfg(test)]
pub fn drain_untranslated_keys() -> Vec<String> {
	UNTRANSLATED_KEYS.with(|keys| std::mem::take(&mut *keys.borrow_mut()))
}

#[cfg(test)]
thread_local! {
	static UNTRANSLATED_KEYS: std::cell::RefCell<Vec<String>> =
		const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
mod tests {
	use super::{known_keys, lookup, table_keys, text};
	use model::Language;

	#[test]
	fn every_bundled_locale_covers_every_known_key() {
		for language in [Language::PortugueseBrazil, Language::Spanish] {
			let missing: Vec<&str> = known_keys()
				.filter(|key| lookup(language, key).is_none())
				.collect();
			assert!(missing.is_empty(), "{language:?} missing {missing:?}");
		}
	}

	#[test]
	fn a_locale_file_only_holds_keys_the_app_uses() {
		// English is the manifest: a translation file may not invent keys.
		let manifest: Vec<&str> = known_keys().collect();
		for language in [Language::PortugueseBrazil, Language::Spanish] {
			let extra: Vec<&str> = table_keys(language)
				.filter(|key| !manifest.contains(key))
				.collect();
			assert!(extra.is_empty(), "{language:?} has unknown keys: {extra:?}");
		}
	}

	#[test]
	fn locales_fall_back_to_english_without_empty_controls() {
		assert_eq!(
			text(Language::PortugueseBrazil, "Voice & Video"),
			"Voz e vídeo"
		);
		assert_eq!(text(Language::Spanish, "Voice & Video"), "Voz y video");
		assert_eq!(
			text(Language::Spanish, "Untranslated sentinel"),
			"Untranslated sentinel"
		);
	}
}
