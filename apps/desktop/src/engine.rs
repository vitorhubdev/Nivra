//! Downloads, verifies and installs the sing-box engine that PROTECT's WireGuard route runs.
//! Only SagerNet's official Windows builds are offered. Nothing reaches disk before its SHA-256
//! matches, and the archive is opened only for its `sing-box.exe` entry.
use model::protect::EngineRelease;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
	io::{Cursor, Read},
	path::{Path, PathBuf},
	time::Duration,
};

const RELEASES: &str = "https://api.github.com/repos/SagerNet/sing-box/releases";
const MAX_METADATA: u64 = 2 * 1024 * 1024;
const MAX_ARCHIVE: u64 = 64 * 1024 * 1024;
const MAX_ENGINE: u64 = 128 * 1024 * 1024;
const VERSION_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The build this Nivra release was tested with. Its digest is fixed in the code, so the first
/// install trusts nothing but these exact bytes.
const PINNED_VERSION: &str = "1.14.3";

#[derive(Deserialize)]
struct Release {
	tag_name: String,
	draft: bool,
	prerelease: bool,
	assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
	name: String,
	size: u64,
	browser_download_url: String,
	digest: Option<String>,
}

/// The Windows architecture this build runs on, as SagerNet names it.
fn windows_arch() -> Option<&'static str> {
	if cfg!(all(windows, target_arch = "x86_64")) {
		Some("amd64")
	} else if cfg!(all(windows, target_arch = "aarch64")) {
		Some("arm64")
	} else {
		None
	}
}

fn archive_name(version: &str, arch: &str) -> String {
	format!("sing-box-{version}-windows-{arch}.zip")
}

fn release_url(version: &str, asset: &str) -> String {
	format!("https://github.com/SagerNet/sing-box/releases/download/v{version}/{asset}")
}

/// The release this build pins, when this platform has an official Windows build.
pub fn pinned() -> Option<EngineRelease> {
	let (size, sha256) = match windows_arch()? {
		"amd64" => (
			32_853_953,
			"d8b86ef58936dba9230ec6a57ffa4243933f3c14ef4809828b6c553440ed1518",
		),
		_ => (
			29_431_701,
			"195b35424c6e3f5913954c7f08c5d8ba2d2199f6d18eb27a3972a5a29cdaa78c",
		),
	};
	let asset = archive_name(PINNED_VERSION, windows_arch()?);
	Some(EngineRelease {
		version: PINNED_VERSION.into(),
		url: release_url(PINNED_VERSION, &asset),
		size,
		sha256: sha256.into(),
	})
}

/// The latest stable sing-box release for this platform, with the digest GitHub publishes for it.
pub async fn latest() -> Result<Option<EngineRelease>, &'static str> {
	let Some(arch) = windows_arch() else {
		return Ok(None);
	};
	let client = client()?;
	let body = read_bounded(&client, &format!("{RELEASES}/latest"), MAX_METADATA).await?;
	let release: Release = serde_json::from_slice(&body)
		.map_err(|_| "GitHub returned invalid sing-box release data.")?;
	if release.draft || release.prerelease {
		return Err("The latest sing-box release is not a stable build.");
	}
	let version = release.tag_name.trim_start_matches('v').to_owned();
	let wanted = archive_name(&version, arch);
	let asset = release
		.assets
		.into_iter()
		.find(|asset| asset.name == wanted)
		.ok_or("The latest sing-box release has no build for this Windows.")?;
	if asset.browser_download_url != release_url(&version, &wanted) || asset.size == 0 {
		return Err("The sing-box release asset is not from the official repository.");
	}
	if asset.size > MAX_ARCHIVE {
		return Err("The sing-box archive exceeds its size limit.");
	}
	let sha256 = asset
		.digest
		.as_deref()
		.and_then(|digest| digest.strip_prefix("sha256:"))
		.filter(|hex| is_sha256_hex(hex))
		.ok_or("GitHub did not publish a SHA-256 digest for this sing-box build.")?;
	Ok(Some(EngineRelease {
		version,
		url: asset.browser_download_url,
		size: asset.size,
		sha256: sha256.to_ascii_lowercase(),
	}))
}

/// Downloads, checks and installs `release` under `root`, then points the caller to it.
/// The previous engine versions in `root` are removed only after the new one answers `version`.
pub async fn install(release: EngineRelease, root: PathBuf) -> Result<PathBuf, &'static str> {
	if !trusted_download(&release.url) || release.size == 0 || release.size > MAX_ARCHIVE {
		return Err("The sing-box download is not from the official repository.");
	}
	if !is_sha256_hex(&release.sha256) || !is_safe_version(&release.version) {
		return Err("The sing-box release description is invalid.");
	}
	let client = client()?;
	let archive = read_bounded(&client, &release.url, MAX_ARCHIVE).await?;
	if archive.len() as u64 != release.size || !digest_matches(&archive, &release.sha256) {
		return Err(
			"The sing-box download did not match its SHA-256 digest. Nothing was installed.",
		);
	}
	let engine = extract_engine(&archive)?;
	drop(archive);
	let scan_root = root.clone();
	let dir = root.join(format!("sing-box-{}", release.version));
	let exe = dir.join("sing-box.exe");
	let version = release.version.clone();
	tokio::task::spawn_blocking(move || -> Result<(), &'static str> {
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).map_err(|_| "Could not create the sing-box folder.")?;
		std::fs::write(&exe, &engine).map_err(|_| "Could not save sing-box.exe.")?;
		if !answers_version(&exe, &version) {
			let _ = std::fs::remove_dir_all(&dir);
			return Err("The downloaded sing-box did not report the expected version.");
		}
		if let Ok(entries) = std::fs::read_dir(&scan_root) {
			for entry in entries.flatten() {
				let name = entry.file_name();
				let name = name.to_string_lossy();
				if name.starts_with("sing-box-") && entry.path() != dir {
					let _ = std::fs::remove_dir_all(entry.path());
				}
			}
		}
		Ok(())
	})
	.await
	.map_err(|_| "The sing-box install stopped unexpectedly.")??;
	Ok(root
		.join(format!("sing-box-{}", release.version))
		.join("sing-box.exe"))
}

/// The newest engine Nivra installed under `root`, with its version.
pub fn installed(root: &Path) -> Option<(PathBuf, String)> {
	std::fs::read_dir(root)
		.ok()?
		.flatten()
		.filter_map(|entry| {
			let name = entry.file_name().to_string_lossy().into_owned();
			let version = name.strip_prefix("sing-box-")?.to_owned();
			let exe = entry.path().join("sing-box.exe");
			let parsed = semver::Version::parse(&version).ok()?;
			exe.is_file().then_some((parsed, exe, version))
		})
		.max_by(|a, b| a.0.cmp(&b.0))
		.map(|(_, exe, version)| (exe, version))
}

/// Where Nivra keeps installed engines: `%LOCALAPPDATA%\nivra\engines`.
pub fn root() -> Option<PathBuf> {
	dirs::data_local_dir().map(|dir| dir.join("nivra").join("engines"))
}

/// Returns the `sing-box.exe` entry from the archive; any other entry is ignored.
fn extract_engine(archive: &[u8]) -> Result<Vec<u8>, &'static str> {
	let mut zip = zip::ZipArchive::new(Cursor::new(archive))
		.map_err(|_| "The sing-box archive is not a valid zip file.")?;
	for index in 0..zip.len() {
		let mut file = zip
			.by_index(index)
			.map_err(|_| "The sing-box archive is damaged.")?;
		if file.is_dir() {
			continue;
		}
		// `enclosed_name` refuses absolute and `..` paths, so no entry can land outside the folder.
		let Some(path) = file.enclosed_name() else {
			continue;
		};
		if path.file_name().and_then(|name| name.to_str()) != Some("sing-box.exe") {
			continue;
		}
		if file.size() > MAX_ENGINE {
			return Err("The sing-box executable exceeds its size limit.");
		}
		let mut bytes = Vec::with_capacity(file.size() as usize);
		(&mut file)
			.take(MAX_ENGINE + 1)
			.read_to_end(&mut bytes)
			.map_err(|_| "The sing-box archive is damaged.")?;
		if bytes.len() as u64 > MAX_ENGINE {
			return Err("The sing-box executable exceeds its size limit.");
		}
		return Ok(bytes);
	}
	Err("The sing-box archive has no sing-box.exe.")
}

/// Runs `sing-box version` and checks it names the release we just installed.
fn answers_version(exe: &Path, version: &str) -> bool {
	let mut command = std::process::Command::new(exe);
	command
		.arg("version")
		.stdin(std::process::Stdio::null())
		.stderr(std::process::Stdio::null());
	#[cfg(windows)]
	{
		use std::os::windows::process::CommandExt;
		command.creation_flags(CREATE_NO_WINDOW);
	}
	let Ok(mut child) = command.stdout(std::process::Stdio::piped()).spawn() else {
		return false;
	};
	let started = std::time::Instant::now();
	loop {
		match child.try_wait() {
			Ok(Some(_)) => break,
			Ok(None) if started.elapsed() < VERSION_CHECK_TIMEOUT => {
				std::thread::sleep(Duration::from_millis(50));
			}
			_ => {
				let _ = child.kill();
				let _ = child.wait();
				return false;
			}
		}
	}
	let mut output = String::new();
	if let Some(mut stdout) = child.stdout.take() {
		let _ = stdout.read_to_string(&mut output);
	}
	output.contains(&format!("version {version}"))
}

fn digest_matches(bytes: &[u8], expected_hex: &str) -> bool {
	let digest = Sha256::digest(bytes);
	let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
	hex.eq_ignore_ascii_case(expected_hex)
}

fn is_sha256_hex(value: &str) -> bool {
	value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_safe_version(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= 32
		&& value
			.bytes()
			.all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

fn trusted_download(url: &str) -> bool {
	url::Url::parse(url).is_ok_and(|url| trusted_url(&url))
}

/// Same host rules as the Nivra updater: HTTPS to GitHub only, nothing else.
fn trusted_url(url: &url::Url) -> bool {
	url.scheme() == "https"
		&& url.port_or_known_default() == Some(443)
		&& url.username().is_empty()
		&& url.password().is_none()
		&& url.fragment().is_none()
		&& matches!(
			url.host_str(),
			Some(
				"api.github.com"
					| "github.com" | "release-assets.githubusercontent.com"
					| "objects.githubusercontent.com"
			)
		)
}

fn client() -> Result<reqwest::Client, &'static str> {
	reqwest::Client::builder()
		.https_only(true)
		.no_proxy()
		.user_agent(concat!(
			"Nivra/",
			env!("CARGO_PKG_VERSION"),
			" (+https://github.com/vitorhubdev/Nivra)"
		))
		.connect_timeout(Duration::from_secs(10))
		.read_timeout(Duration::from_secs(30))
		.timeout(Duration::from_secs(600))
		.redirect(reqwest::redirect::Policy::custom(|attempt| {
			if attempt.previous().len() < 5 && trusted_url(attempt.url()) {
				attempt.follow()
			} else {
				attempt.error("Untrusted sing-box redirect")
			}
		}))
		.build()
		.map_err(|_| "Could not initialize the secure download.")
}

/// Reads a response no larger than `limit`, stopping as soon as it grows past it.
async fn read_bounded(
	client: &reqwest::Client,
	url: &str,
	limit: u64,
) -> Result<Vec<u8>, &'static str> {
	let mut response = client
		.get(url)
		.send()
		.await
		.and_then(reqwest::Response::error_for_status)
		.map_err(|_| "Could not reach GitHub for sing-box.")?;
	if response.content_length().is_some_and(|size| size > limit) {
		return Err("The sing-box download exceeds its size limit.");
	}
	let mut body = Vec::new();
	while let Some(chunk) = response
		.chunk()
		.await
		.map_err(|_| "The sing-box download was interrupted.")?
	{
		if body.len() as u64 + chunk.len() as u64 > limit {
			return Err("The sing-box download exceeds its size limit.");
		}
		body.extend_from_slice(&chunk);
	}
	Ok(body)
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Write;

	fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
		let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
		for (name, bytes) in entries {
			writer
				.start_file(*name, zip::write::SimpleFileOptions::default())
				.unwrap();
			writer.write_all(bytes).unwrap();
		}
		writer.finish().unwrap().into_inner()
	}

	#[test]
	fn extraction_takes_only_the_engine_and_refuses_escaping_paths() {
		let archive = zip_with(&[
			("../escape/sing-box.exe", b"evil"),
			("sing-box-1.14.3-windows-amd64/LICENSE", b"license"),
			("sing-box-1.14.3-windows-amd64/sing-box.exe", b"engine"),
		]);
		assert_eq!(extract_engine(&archive).unwrap(), b"engine");
	}

	#[test]
	fn an_archive_without_the_engine_is_refused() {
		let archive = zip_with(&[("README.md", b"text")]);
		assert!(extract_engine(&archive).is_err());
	}

	#[test]
	fn digests_must_match_exactly() {
		let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
		assert!(digest_matches(b"abc", expected));
		assert!(!digest_matches(b"abd", expected));
		assert!(is_sha256_hex(expected));
		assert!(!is_sha256_hex("not-a-digest"));
	}

	#[test]
	fn downloads_come_only_from_github_over_https() {
		assert!(trusted_download(
			"https://github.com/SagerNet/sing-box/releases/download/v1.14.3/sing-box-1.14.3-windows-amd64.zip"
		));
		assert!(!trusted_download(
			"http://github.com/SagerNet/sing-box/releases/x.zip"
		));
		assert!(!trusted_download("https://example.com/sing-box.zip"));
	}

	#[test]
	fn the_pinned_release_is_well_formed() {
		if let Some(pinned) = pinned() {
			assert!(is_sha256_hex(&pinned.sha256));
			assert!(trusted_download(&pinned.url));
			assert_eq!(pinned.version, PINNED_VERSION);
		}
	}

	/// Live check: downloads the pinned sing-box from GitHub, verifies it, and runs it.
	/// Writes only into a temporary folder. Run with `cargo test -p nivra live_pinned -- --ignored`.
	#[test]
	#[ignore = "needs network access to GitHub"]
	fn live_pinned_engine_downloads_verifies_and_answers() {
		let release = pinned().expect("this platform has a pinned build");
		let root = std::env::temp_dir().join(format!("nivra-live-engine-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		let runtime = tokio::runtime::Runtime::new().unwrap();
		let exe = runtime
			.block_on(install(release, root.clone()))
			.expect("the pinned build downloads, verifies and answers its version");
		assert!(exe.is_file());
		assert_eq!(
			installed(&root).map(|(_, version)| version),
			Some(PINNED_VERSION.to_owned())
		);
		let _ = std::fs::remove_dir_all(&root);
	}

	/// Live check: GitHub publishes a digest for the latest stable Windows build.
	#[test]
	#[ignore = "needs network access to GitHub"]
	fn live_latest_engine_has_a_published_digest() {
		let runtime = tokio::runtime::Runtime::new().unwrap();
		let latest = runtime
			.block_on(latest())
			.expect("GitHub answers the sing-box release query")
			.expect("this platform has a Windows build");
		assert!(is_sha256_hex(&latest.sha256));
		assert!(trusted_download(&latest.url));
		assert!(is_safe_version(&latest.version));
	}

	#[test]
	fn the_newest_installed_engine_is_chosen() {
		let root = std::env::temp_dir().join(format!("nivra-engine-scan-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		for version in ["1.14.2", "1.14.10", "1.14.3"] {
			let dir = root.join(format!("sing-box-{version}"));
			std::fs::create_dir_all(&dir).unwrap();
			std::fs::write(dir.join("sing-box.exe"), b"x").unwrap();
		}
		let (path, version) = installed(&root).unwrap();
		assert_eq!(version, "1.14.10");
		assert!(path.ends_with("sing-box.exe"));
		let _ = std::fs::remove_dir_all(&root);
	}
}
