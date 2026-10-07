//! Embeds every license text the packaged app ships under `licenses/`, deflate-compressed, so
//! the in-app licenses screen works without reading the install directory. The source list
//! mirrors what `cargo xtask package` stages; `licenses::tests` fails when the two drift.
use std::io::Write;
use std::path::{Path, PathBuf};

/// `(source relative to the workspace root, package path, component)`.
const FILES: &[(&str, &str, &str)] = &[
	("LICENSE-MIT", "LICENSE-MIT", "nivraext"),
	("LICENSE-APACHE", "LICENSE-APACHE", "nivraext"),
	(
		"THIRD_PARTY_NOTICES.md",
		"THIRD_PARTY_NOTICES.md",
		"nivraext",
	),
	(
		"assets/sounds/README.md",
		"licenses/notification-sounds.md",
		"sounds",
	),
	(
		"assets/fonts/NotoSansArabic-OFL.txt",
		"licenses/NotoSansArabic-OFL.txt",
		"fonts",
	),
	(
		"assets/fonts/NotoSansMath-OFL.txt",
		"licenses/NotoSansMath-OFL.txt",
		"fonts",
	),
	(
		"assets/fonts/Inter-OFL.txt",
		"licenses/Inter-OFL.txt",
		"fonts",
	),
	(
		"assets/twemoji/LICENSE-GRAPHICS",
		"licenses/Twemoji-CC-BY-4.0.txt",
		"emoji",
	),
	(
		"assets/twemoji/LICENSE-UNICODE",
		"licenses/Unicode-LICENSE.txt",
		"emoji",
	),
	(
		"assets/icons/LICENSE",
		"licenses/Phosphor-Icons-MIT.txt",
		"icons",
	),
	(
		"assets/icons/LICENSE-SIMPLE-ICONS",
		"licenses/Simple-Icons-CC0.txt",
		"icons",
	),
];

/// Directories copied whole from `assets/licenses/<dir>` to `licenses/<dir>`; the directory
/// name is the component.
const DIRECTORIES: &[&str] = &[
	"files",
	"notifications",
	"login",
	"voice",
	"audio",
	"dependencies",
];

fn main() {
	let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
	let root = manifest.join("../..");
	let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("out dir"));
	let blobs = out.join("license_bundle");
	std::fs::create_dir_all(&blobs).expect("create license blob directory");

	let mut sources = Vec::new();
	for (source, id, component) in FILES {
		let path = root.join(source);
		println!("cargo:rerun-if-changed={}", path.display());
		sources.push((path, (*id).to_owned(), *component));
	}
	for directory in DIRECTORIES {
		let base = root.join("assets/licenses").join(directory);
		println!("cargo:rerun-if-changed={}", base.display());
		let mut files = Vec::new();
		walk(&base, &mut files);
		for path in files {
			let relative = path
				.strip_prefix(&base)
				.expect("walked path under its base");
			let relative = relative
				.components()
				.map(|part| part.as_os_str().to_str().expect("UTF-8 license path"))
				.collect::<Vec<_>>()
				.join("/");
			sources.push((path, format!("licenses/{directory}/{relative}"), *directory));
		}
	}
	sources.sort_by(|a, b| a.1.cmp(&b.1));

	let mut generated = String::from("static BUNDLE: &[Bundled] = &[\n");
	let mut compressed_total = 0usize;
	let mut raw_total = 0usize;
	for (index, (path, id, component)) in sources.iter().enumerate() {
		let raw = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
		assert!(
			std::str::from_utf8(&raw).is_ok(),
			"{} is not UTF-8 text",
			path.display()
		);
		let mut encoder =
			flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
		encoder.write_all(&raw).expect("deflate license text");
		let compressed = encoder.finish().expect("finish deflate");
		let blob = blobs.join(format!("{index}.deflate"));
		std::fs::write(&blob, &compressed).expect("write license blob");
		raw_total += raw.len();
		compressed_total += compressed.len();
		let title = id.rsplit('/').next().unwrap_or(id);
		generated.push_str(&format!(
			"\tBundled {{ entry: Entry {{ id: {id:?}, title: {title:?}, component: {component:?} }}, data: include_bytes!({:?}) }},\n",
			blob.display().to_string()
		));
	}
	generated.push_str("];\n");
	generated.push_str(&format!(
		"/// Uncompressed size of every embedded text, in bytes.\n#[allow(dead_code)]\nconst RAW_BYTES: usize = {raw_total};\n/// Deflated size of every embedded text, in bytes.\n#[allow(dead_code)]\nconst COMPRESSED_BYTES: usize = {compressed_total};\n"
	));
	std::fs::write(out.join("license_bundle.rs"), generated).expect("write license bundle");
}

/// Every file below `directory`, skipping source archives (`*.crate`, shipped only in the
/// release's source zip) and the `pr-evidence` folders packaging leaves out.
fn walk(directory: &Path, files: &mut Vec<PathBuf>) {
	let entries = std::fs::read_dir(directory)
		.unwrap_or_else(|e| panic!("read {}: {e}", directory.display()));
	for entry in entries {
		let entry = entry.expect("directory entry");
		let path = entry.path();
		if entry.file_name() == "pr-evidence" {
			continue;
		}
		if entry.file_type().expect("file type").is_dir() {
			walk(&path, files);
		} else if path
			.extension()
			.is_none_or(|extension| extension != "crate")
		{
			files.push(path);
		}
	}
}
