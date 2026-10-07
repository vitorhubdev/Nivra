//! Loads the CJK fonts the operating system provides.
//!
//! Nivra no longer embeds a CJK face. When CJK text first appears, a worker
//! thread searches the platform's font directories in the documented order,
//! reads each candidate once and keeps the faces that add coverage. The
//! resulting fonts are installed into egui's fallback families, so kana, Han
//! and Hangul render from system files and the executable carries none.
//!
//! Eframe's `system_fonts` provider still serves emoji and other scripts, but it
//! cannot be the only CJK path: with a non-Chinese locale it picks a Japanese
//! face for Han text and fails on simplified-only codepoints such as 语. Every
//! candidate here is validated per face against one sample scalar per script, so
//! a Chinese face is added when the platform's fallback choice would not cover
//! the script.
//!
//! [`load_from`] takes its candidates as an argument, so the search is testable
//! against a temporary root instead of the machine's real font directory.

use egui::FontData;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A script a CJK fallback has to cover.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Script {
	Japanese,
	SimplifiedChinese,
	Korean,
}

impl Script {
	/// One scalar this script needs; Chinese covers the traditional forms too.
	pub const fn sample(self) -> char {
		match self {
			Self::Japanese => 'あ',
			Self::SimplifiedChinese => '语',
			Self::Korean => '한',
		}
	}
}

/// Every script this module checks.
pub const SCRIPTS: [Script; 3] = [Script::Japanese, Script::SimplifiedChinese, Script::Korean];

/// A system face ready for egui's font definitions.
pub struct LoadedFont {
	/// Registry name in egui's font map, such as `system:msyh:0`.
	pub name: String,
	pub data: FontData,
}

/// Font files looked up in the Windows font directories, in preference order.
/// Microsoft YaHei covers kana and Han; Malgun Gothic adds Hangul.
#[cfg(any(target_os = "windows", test))]
pub(crate) const WINDOWS: &[&str] = &[
	"msyh.ttc",
	"YuGothR.ttc",
	"YuGothic.ttf",
	"malgun.ttf",
	"msgothic.ttc",
	"simsun.ttc",
];

/// Font files looked up in the macOS font directories, in preference order.
#[cfg(any(target_os = "macos", test))]
pub(crate) const MACOS: &[&str] = &[
	"PingFang.ttc",
	"Hiragino Sans GB.ttc",
	"Hiragino Sans.ttc",
	"Hiragino Kaku Gothic ProN W3.otf",
	"AppleSDGothicNeo.ttc",
	"Songti.ttc",
	"STHeiti Light.ttc",
];

/// macOS font directories, in preference order.
#[cfg(any(target_os = "macos", test))]
const MACOS_ROOTS: &[&str] = &[
	"/System/Library/Fonts",
	"/System/Library/Fonts/Supplemental",
	"/Library/Fonts",
];

/// Known Noto CJK locations on Linux, after the fontconfig answer.
#[cfg(any(not(any(target_os = "windows", target_os = "macos")), test))]
pub(crate) const LINUX: &[&str] = &[
	"/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
	"/usr/share/fonts/opentype/noto/NotoSansCJK-VF.otf.ttc",
	"/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
	"/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
	"/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
];

/// Fontconfig languages asked for on Linux, in order.
#[cfg(any(not(any(target_os = "windows", target_os = "macos")), test))]
const FC_LANG: [&str; 3] = ["ja", "zh", "ko"];

/// Largest candidate file read into memory. Real CJK faces are 10–30 MB; the
/// ceiling stops a stray or planted file from allocating without bound.
const MAX_FONT_BYTES: u64 = 128 * 1024 * 1024;

/// Reads the installed candidates and returns the faces worth installing.
pub fn load_fallbacks() -> Vec<LoadedFont> {
	load_from(&candidate_paths())
}

/// Reads each candidate once, keeping only faces that add coverage.
pub(crate) fn load_from(candidates: &[PathBuf]) -> Vec<LoadedFont> {
	let mut loaded: Vec<LoadedFont> = Vec::new();
	let mut missing = SCRIPTS.to_vec();
	for path in candidates {
		if missing.is_empty() {
			break;
		}
		let Some(bytes) = read_candidate(path, MAX_FONT_BYTES) else {
			continue;
		};
		let mut faces: Vec<(u32, Vec<Script>)> = Vec::new();
		for script in &missing {
			let Some(index) = covering_face(&bytes, script.sample()) else {
				continue;
			};
			match faces.iter_mut().find(|(face, _)| *face == index) {
				Some((_, scripts)) => scripts.push(*script),
				None => faces.push((index, vec![*script])),
			}
		}
		for (index, scripts) in faces {
			let name = font_name(path, index);
			if loaded.iter().any(|font| font.name == name) {
				continue;
			}
			for script in &scripts {
				missing.retain(|candidate| candidate != script);
			}
			loaded.push(LoadedFont {
				name,
				data: FontData::from_blob(bytes.clone(), index),
			});
		}
	}
	loaded
}

/// Reads a candidate within `ceiling` bytes; a file that grows past it is refused.
fn read_candidate(path: &Path, ceiling: u64) -> Option<Arc<Vec<u8>>> {
	let file = std::fs::File::open(path).ok()?;
	if !file.metadata().ok()?.is_file() || file.metadata().ok()?.len() > ceiling {
		return None;
	}
	let mut bytes = Vec::new();
	std::io::Read::take(file, ceiling + 1)
		.read_to_end(&mut bytes)
		.ok()?;
	if bytes.len() as u64 > ceiling || bytes.len() < 12 {
		return None;
	}
	Some(Arc::new(bytes))
}

impl LoadedFont {
	/// Does this exact face map `c` to a real glyph?
	pub fn covers(&self, c: char) -> bool {
		face_covers(self.data.bytes(), self.data.index, c)
	}
}

/// The first face in `bytes` that maps `sample` to a real glyph.
fn covering_face(bytes: &[u8], sample: char) -> Option<u32> {
	(0..face_count(bytes)).find(|index| face_covers(bytes, *index, sample))
}

/// Does one face of `bytes` map `sample` to a real glyph?
fn face_covers(bytes: &[u8], index: u32, sample: char) -> bool {
	use skrifa::MetadataProvider as _;
	skrifa::FontRef::from_index(bytes, index)
		.ok()
		.and_then(|font| font.charmap().map(sample))
		.is_some_and(|glyph| glyph != skrifa::GlyphId::NOTDEF)
}

/// Faces in a file: `.ttc`/`.otc` collections start with a `ttcf` header.
fn face_count(bytes: &[u8]) -> u32 {
	let count = (bytes.len() >= 12 && &bytes[..4] == b"ttcf")
		.then(|| u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]));
	count.map_or(1, |count| count.clamp(1, 32))
}

/// Registry name for one face; unique per file and index.
fn font_name(path: &Path, index: u32) -> String {
	let stem = path
		.file_stem()
		.and_then(|stem| stem.to_str())
		.unwrap_or("font");
	format!("system:{stem}:{index}")
}

/// Candidate files on this platform, in search order.
fn candidate_paths() -> Vec<PathBuf> {
	let mut paths = Vec::new();
	#[cfg(target_os = "windows")]
	for root in windows_roots() {
		for name in WINDOWS {
			push_unique(&mut paths, root.join(name));
		}
	}
	#[cfg(target_os = "macos")]
	for root in MACOS_ROOTS {
		for name in MACOS {
			push_unique(&mut paths, Path::new(root).join(name));
		}
	}
	#[cfg(not(any(target_os = "windows", target_os = "macos")))]
	{
		for language in FC_LANG {
			if let Some(path) = fontconfig_match(language) {
				push_unique(&mut paths, path);
			}
		}
		for path in LINUX {
			push_unique(&mut paths, PathBuf::from(path));
		}
	}
	paths
}

/// Windows font directories: machine fonts first, then per-user installs.
#[cfg(target_os = "windows")]
fn windows_roots() -> Vec<PathBuf> {
	let mut roots = Vec::new();
	if let Some(root) = std::env::var_os("SystemRoot") {
		push_unique(&mut roots, PathBuf::from(root).join("Fonts"));
	}
	push_unique(&mut roots, PathBuf::from(r"C:\Windows\Fonts"));
	if let Some(local) = std::env::var_os("LOCALAPPDATA") {
		roots.push(
			PathBuf::from(local)
				.join("Microsoft")
				.join("Windows")
				.join("Fonts"),
		);
	}
	roots
}

/// Windows candidates under one root, in preference order.
#[cfg(test)]
pub(crate) fn windows_candidates_in(root: &Path) -> Vec<PathBuf> {
	WINDOWS.iter().map(|name| root.join(name)).collect()
}

/// macOS candidates under one root, in preference order.
#[cfg(test)]
pub(crate) fn macos_candidates_in(root: &Path) -> Vec<PathBuf> {
	MACOS.iter().map(|name| root.join(name)).collect()
}

/// The known Linux candidates under a fake root, in preference order.
#[cfg(test)]
pub(crate) fn linux_candidates_in(root: &Path) -> Vec<PathBuf> {
	LINUX
		.iter()
		.map(|path| root.join(path.trim_start_matches('/')))
		.collect()
}

/// The font fontconfig picks for a language, when fontconfig is installed.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn fontconfig_match(language: &str) -> Option<PathBuf> {
	let output = std::process::Command::new("fc-match")
		.arg("--format=%{file}")
		.arg(format!(":lang={language}"))
		.output()
		.ok()?;
	if !output.status.success() {
		return None;
	}
	let path = String::from_utf8(output.stdout).ok()?;
	let path = path.trim();
	(!path.is_empty()).then(|| PathBuf::from(path))
}

fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
	if !paths.contains(&path) {
		paths.push(path);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn scratch(name: &str) -> PathBuf {
		let root = std::env::temp_dir().join(format!("nivra-cjk-{}-{name}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).expect("scratch font root");
		root
	}

	fn inter_bytes() -> Vec<u8> {
		std::fs::read(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/../../assets/fonts/Inter-Regular.ttf"
		))
		.expect("bundled Inter")
	}

	fn inter() -> Vec<u8> {
		inter_bytes()
	}

	#[test]
	fn windows_candidates_prefer_yahei_then_yu_gothic_then_malgun() {
		let root = scratch("windows-order");
		for name in ["malgun.ttf", "simsun.ttc", "msyh.ttc"] {
			std::fs::write(root.join(name), b"").expect("candidate");
		}
		let candidates = windows_candidates_in(&root);
		assert_eq!(candidates[0], root.join("msyh.ttc"));
		let malgun = candidates
			.iter()
			.position(|path| path.ends_with("malgun.ttf"))
			.expect("Malgun listed");
		let yugothic = candidates
			.iter()
			.position(|path| path.ends_with("YuGothR.ttc"))
			.expect("Yu Gothic listed");
		assert!(yugothic < malgun, "Yu Gothic is tried before Malgun");
	}

	#[test]
	fn macos_candidates_prefer_pingfang_then_hiragino_then_korean() {
		let candidates: Vec<_> = MACOS_ROOTS
			.iter()
			.flat_map(|base| macos_candidates_in(Path::new(base)))
			.collect();
		assert_eq!(
			candidates[0],
			Path::new(MACOS_ROOTS[0]).join("PingFang.ttc")
		);
		let pingfang = candidates
			.iter()
			.position(|path| path.ends_with("PingFang.ttc"))
			.expect("PingFang listed");
		let hiragino = candidates
			.iter()
			.position(|path| path.ends_with("Hiragino Sans GB.ttc"))
			.expect("Hiragino listed");
		let korean = candidates
			.iter()
			.position(|path| path.ends_with("AppleSDGothicNeo.ttc"))
			.expect("Korean face listed");
		assert!(pingfang < hiragino && hiragino < korean);
	}

	#[test]
	fn linux_known_locations_are_searched_in_distribution_order() {
		let root = scratch("linux-order");
		let candidates = linux_candidates_in(&root);
		assert_eq!(candidates.len(), LINUX.len());
		assert_eq!(FC_LANG, ["ja", "zh", "ko"]);
		assert!(candidates[0].ends_with("opentype/noto/NotoSansCJK-Regular.ttc"));
		assert!(candidates.iter().all(|path| path.starts_with(&root)));
	}

	#[test]
	fn a_temp_root_without_fonts_loads_nothing() {
		let root = scratch("empty");
		assert!(load_from(&windows_candidates_in(&root)).is_empty());
	}

	#[test]
	fn a_file_without_the_sample_is_not_loaded() {
		let root = scratch("coverage");
		let inter = inter();
		assert_eq!(covering_face(&inter, 'あ'), None, "Inter has no kana");
		assert_eq!(covering_face(&inter, '语'), None, "Inter has no Han");
		assert_eq!(covering_face(&inter, '한'), None, "Inter has no Hangul");
		assert_eq!(covering_face(&inter, 'H'), Some(0), "Inter covers Latin");
		let path = root.join("msyh.ttc");
		std::fs::write(&path, &inter).expect("fake candidate");
		assert!(
			load_from(&[path]).is_empty(),
			"a Latin face must not be offered for CJK"
		);
	}

	#[test]
	fn an_empty_or_corrupt_file_is_skipped_without_panicking() {
		let root = scratch("corrupt");
		let path = root.join("msyh.ttc");
		std::fs::write(&path, b"not a font").expect("candidate");
		assert!(load_from(&[path, root.join("missing.ttc")]).is_empty());
	}

	#[test]
	fn a_candidate_over_the_byte_ceiling_is_refused_without_reading_it() {
		let root = scratch("ceiling");
		let path = root.join("huge.ttc");
		let file = std::fs::File::create(&path).expect("candidate");
		file.set_len(9).expect("sparse size");
		assert!(
			read_candidate(&path, 8).is_none(),
			"9 bytes over an 8-byte cap"
		);
		assert!(read_candidate(&path, 9).is_none(), "too short for a header");
		assert!(read_candidate(&root.join("missing.ttc"), 8).is_none());
		let inter = root.join("inter.ttf");
		std::fs::write(&inter, inter_bytes()).expect("real face");
		let loaded = read_candidate(&inter, MAX_FONT_BYTES).expect("real face within the cap");
		assert_eq!(loaded.len(), inter_bytes().len());
	}

	#[test]
	fn a_collection_header_cannot_point_out_of_bounds() {
		let mut bytes = b"ttcf".to_vec();
		bytes.extend_from_slice(&[0, 1, 0, 0]);
		bytes.extend_from_slice(&u32::MAX.to_be_bytes());
		assert_eq!(covering_face(&bytes, 'H'), None);
		let mut bytes = b"ttcf".to_vec();
		bytes.extend_from_slice(&[0, 1, 0, 0]);
		bytes.extend_from_slice(&0u32.to_be_bytes());
		assert_eq!(covering_face(&bytes, 'H'), None);
	}

	#[test]
	fn the_live_candidates_never_panic_and_name_every_face_once() {
		// The machine may have none of these; the app still has to start and warn.
		let fonts = load_fallbacks();
		for (index, font) in fonts.iter().enumerate() {
			assert!(
				fonts[..index]
					.iter()
					.all(|earlier| earlier.name != font.name),
				"duplicate face name {}",
				font.name
			);
			assert!(font.name.starts_with("system:"));
		}
	}

	#[test]
	fn a_loaded_face_reports_the_scalars_it_maps() {
		let font = LoadedFont {
			name: "system:inter:0".into(),
			data: FontData::from_owned(inter_bytes()),
		};
		assert!(font.covers('H'));
		assert!(!font.covers('あ'));
		assert!(!font.covers('语'));
		assert!(!font.covers('한'));
	}
}
