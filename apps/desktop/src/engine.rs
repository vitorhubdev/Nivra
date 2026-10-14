//! Downloads, verifies and installs the sing-box engine that PROTECT's WireGuard route runs.
//! Only SagerNet's official builds are offered. Nothing reaches disk before its SHA-256
//! matches, and the archive is read only for its `sing-box` executable entry.
use model::protect::EngineRelease;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
	io::{Cursor, Read},
	path::{Path, PathBuf},
	sync::atomic::{AtomicBool, AtomicU64, Ordering},
	time::{Duration, SystemTime, UNIX_EPOCH},
};

const RELEASES: &str = "https://api.github.com/repos/SagerNet/sing-box/releases";
const MAX_METADATA: u64 = 2 * 1024 * 1024;
const MAX_ARCHIVE: u64 = 64 * 1024 * 1024;
const MAX_ENGINE: u64 = 128 * 1024 * 1024;
const VERSION_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
/// The latest-release check runs at most once a day, the first time the WireGuard panel opens.
const CHECK_EVERY: u64 = 24 * 60 * 60;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The build this Nivra release was tested with. Its digest is fixed in the code, so the first
/// install trusts nothing but these exact bytes.
const PINNED_VERSION: &str = "1.14.3";

/// Progress of the download in flight, for the PROTECT panel. Only one install runs at a time.
static ACTIVE: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);
static RECEIVED: AtomicU64 = AtomicU64::new(0);
static EXPECTED: AtomicU64 = AtomicU64::new(0);

/// Asks the running install to stop; it ends with an error and installs nothing.
pub fn cancel() {
	CANCEL.store(true, Ordering::Release);
}

/// Fraction of the archive received while an install runs; `None` when nothing is downloading.
pub fn progress() -> Option<f32> {
	if !ACTIVE.load(Ordering::Acquire) {
		return None;
	}
	let expected = EXPECTED.load(Ordering::Acquire);
	let received = RECEIVED.load(Ordering::Acquire);
	(expected > 0).then(|| (received as f64 / expected as f64).min(1.0) as f32)
}

/// Clears the progress flags when an install ends, whatever its outcome.
struct Activity;
impl Drop for Activity {
	fn drop(&mut self) {
		ACTIVE.store(false, Ordering::Release);
	}
}

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

/// The build this machine runs, named the way SagerNet names its files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Target {
	os: &'static str,
	arch: &'static str,
	zip: bool,
	entry: &'static str,
}

impl Target {
	fn asset(self, version: &str) -> String {
		let extension = if self.zip { "zip" } else { "tar.gz" };
		format!("sing-box-{version}-{}-{}.{extension}", self.os, self.arch)
	}
}

fn target() -> Option<Target> {
	let arch = if cfg!(target_arch = "x86_64") {
		"amd64"
	} else if cfg!(target_arch = "aarch64") {
		"arm64"
	} else {
		return None;
	};
	if cfg!(windows) {
		Some(Target {
			os: "windows",
			arch,
			zip: true,
			entry: "sing-box.exe",
		})
	} else if cfg!(target_os = "linux") {
		Some(Target {
			os: "linux",
			arch,
			zip: false,
			entry: "sing-box",
		})
	} else if cfg!(target_os = "macos") {
		Some(Target {
			os: "darwin",
			arch,
			zip: false,
			entry: "sing-box",
		})
	} else {
		None
	}
}

/// Size and SHA-256 of the pinned archive for each platform SagerNet builds.
fn pinned_digest(target: Target) -> Option<(u64, &'static str)> {
	Some(match (target.os, target.arch) {
		("windows", "amd64") => (
			32_853_953,
			"d8b86ef58936dba9230ec6a57ffa4243933f3c14ef4809828b6c553440ed1518",
		),
		("windows", "arm64") => (
			29_431_701,
			"195b35424c6e3f5913954c7f08c5d8ba2d2199f6d18eb27a3972a5a29cdaa78c",
		),
		("linux", "amd64") => (
			31_678_266,
			"e2bdf179c15a3652955dc44867e33ca0965c9c9b91b34267dcc5a5d639a5feee",
		),
		("linux", "arm64") => (
			29_110_944,
			"29dae24d76bea3bc62d07dd3e60cfc12003de7debbd6b768df9be9c68360174e",
		),
		("darwin", "amd64") => (
			31_473_185,
			"6d002b74d03b478547349cc10af89e007868596e5aa931816d45da9f544715a9",
		),
		("darwin", "arm64") => (
			29_163_190,
			"0360ce634a04c26a6b4fd957517987bb4f6c8950f63435ce3c5af2fcae5860c5",
		),
		_ => return None,
	})
}

fn release_url(version: &str, asset: &str) -> String {
	format!("https://github.com/SagerNet/sing-box/releases/download/v{version}/{asset}")
}

/// The release this build pins, for this machine's platform.
pub fn pinned() -> Option<EngineRelease> {
	let target = target()?;
	let (size, sha256) = pinned_digest(target)?;
	let asset = target.asset(PINNED_VERSION);
	Some(EngineRelease {
		version: PINNED_VERSION.into(),
		url: release_url(PINNED_VERSION, &asset),
		size,
		sha256: sha256.into(),
	})
}

/// The latest stable sing-box release for this platform, with the digest GitHub publishes for it.
pub async fn latest() -> Result<Option<EngineRelease>, &'static str> {
	let Some(target) = target() else {
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
	let wanted = target.asset(&version);
	let asset = release
		.assets
		.into_iter()
		.find(|asset| asset.name == wanted)
		.ok_or("The latest sing-box release has no build for this computer.")?;
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
	let target = target().ok_or("sing-box is not available for this computer.")?;
	if !trusted_download(&release.url) || release.size == 0 || release.size > MAX_ARCHIVE {
		return Err("The sing-box download is not from the official repository.");
	}
	if !is_sha256_hex(&release.sha256) || !is_safe_version(&release.version) {
		return Err("The sing-box release description is invalid.");
	}
	let client = client()?;
	CANCEL.store(false, Ordering::Release);
	RECEIVED.store(0, Ordering::Release);
	EXPECTED.store(release.size, Ordering::Release);
	ACTIVE.store(true, Ordering::Release);
	let _activity = Activity;
	let archive = download_tracked(&client, &release.url, MAX_ARCHIVE).await?;
	if archive.len() as u64 != release.size || !digest_matches(&archive, &release.sha256) {
		return Err(
			"The sing-box download did not match its SHA-256 digest. Nothing was installed.",
		);
	}
	let engine = extract_engine(&archive, target)?;
	drop(archive);
	let dir = root.join(format!("sing-box-{}", release.version));
	let exe = dir.join(target.entry);
	let version = release.version.clone();
	let scan_root = root.clone();
	tokio::task::spawn_blocking(move || -> Result<(), &'static str> {
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).map_err(|_| "Could not create the sing-box folder.")?;
		std::fs::write(&exe, &engine).map_err(|_| "Could not save the sing-box executable.")?;
		make_executable(&exe)?;
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
		.join(target.entry))
}

/// The newest engine Nivra installed under `root`, with its version.
pub fn installed(root: &Path) -> Option<(PathBuf, String)> {
	let entry_name = target()?.entry;
	std::fs::read_dir(root)
		.ok()?
		.flatten()
		.filter_map(|entry| {
			let name = entry.file_name().to_string_lossy().into_owned();
			let version = name.strip_prefix("sing-box-")?.to_owned();
			let exe = entry.path().join(entry_name);
			let parsed = semver::Version::parse(&version).ok()?;
			exe.is_file().then_some((parsed, exe, version))
		})
		.max_by(|a, b| a.0.cmp(&b.0))
		.map(|(_, exe, version)| (exe, version))
}

/// Where Nivra keeps installed engines: `%LOCALAPPDATA%\nivra\engines` on Windows.
pub fn root() -> Option<PathBuf> {
	dirs::data_local_dir().map(|dir| dir.join("nivra").join("engines"))
}

/// True when no latest-release check has succeeded in the last day.
pub fn check_due(root: &Path) -> bool {
	let last = std::fs::read_to_string(root.join("last-check"))
		.ok()
		.and_then(|text| text.trim().parse::<u64>().ok());
	last.is_none_or(|last| now_secs().saturating_sub(last) >= CHECK_EVERY)
}

/// Records a successful latest-release check, so the next automatic one waits a day.
pub fn mark_checked(root: &Path) {
	if std::fs::create_dir_all(root).is_ok() {
		let _ = std::fs::write(root.join("last-check"), now_secs().to_string());
	}
}

fn now_secs() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map(|elapsed| elapsed.as_secs())
		.unwrap_or(0)
}

/// Returns the executable entry from the archive; every other entry is ignored.
fn extract_engine(archive: &[u8], target: Target) -> Result<Vec<u8>, &'static str> {
	if target.zip {
		extract_from_zip(archive, target.entry)
	} else {
		extract_from_tar_gz(archive, target.entry)
	}
}

fn extract_from_zip(archive: &[u8], entry: &str) -> Result<Vec<u8>, &'static str> {
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
		if path.file_name().and_then(|name| name.to_str()) != Some(entry) {
			continue;
		}
		let size = file.size();
		return read_engine(&mut file, size);
	}
	Err("The sing-box archive has no sing-box executable.")
}

fn extract_from_tar_gz(archive: &[u8], entry: &str) -> Result<Vec<u8>, &'static str> {
	let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
	let entries = tar
		.entries()
		.map_err(|_| "The sing-box archive is not a valid tar.gz file.")?;
	for item in entries {
		let mut item = item.map_err(|_| "The sing-box archive is damaged.")?;
		if !item.header().entry_type().is_file() {
			continue;
		}
		let matches = item
			.path()
			.ok()
			.and_then(|path| path.file_name().map(|name| name == entry))
			.unwrap_or(false);
		if !matches {
			continue;
		}
		let size = item.size();
		return read_engine(&mut item, size);
	}
	Err("The sing-box archive has no sing-box executable.")
}

fn read_engine(file: &mut impl Read, size: u64) -> Result<Vec<u8>, &'static str> {
	if size > MAX_ENGINE {
		return Err("The sing-box executable exceeds its size limit.");
	}
	let mut bytes = Vec::with_capacity(size as usize);
	file.take(MAX_ENGINE + 1)
		.read_to_end(&mut bytes)
		.map_err(|_| "The sing-box archive is damaged.")?;
	if bytes.len() as u64 > MAX_ENGINE {
		return Err("The sing-box executable exceeds its size limit.");
	}
	Ok(bytes)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), &'static str> {
	use std::os::unix::fs::PermissionsExt;
	std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
		.map_err(|_| "Could not mark sing-box as executable.")
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> Result<(), &'static str> {
	Ok(())
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

/// Like `read_bounded`, but counts bytes for the progress bar and honours a cancel request.
async fn download_tracked(
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
		if CANCEL.load(Ordering::Acquire) {
			return Err("Download cancelled. Nothing was installed.");
		}
		if body.len() as u64 + chunk.len() as u64 > limit {
			return Err("The sing-box download exceeds its size limit.");
		}
		body.extend_from_slice(&chunk);
		RECEIVED.store(body.len() as u64, Ordering::Release);
	}
	Ok(body)
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

	fn tar_gz_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
		let mut builder = tar::Builder::new(Vec::new());
		for (name, bytes) in entries {
			let mut header = tar::Header::new_gnu();
			header.set_size(bytes.len() as u64);
			header.set_mode(0o755);
			header.set_cksum();
			builder.append_data(&mut header, name, *bytes).unwrap();
		}
		let tar_bytes = builder.into_inner().unwrap();
		let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
		encoder.write_all(&tar_bytes).unwrap();
		encoder.finish().unwrap()
	}

	const WINDOWS: Target = Target {
		os: "windows",
		arch: "amd64",
		zip: true,
		entry: "sing-box.exe",
	};
	const LINUX: Target = Target {
		os: "linux",
		arch: "amd64",
		zip: false,
		entry: "sing-box",
	};

	#[test]
	fn zip_extraction_takes_only_the_engine_and_refuses_escaping_paths() {
		let archive = zip_with(&[
			("../escape/sing-box.exe", b"evil"),
			("sing-box-1.14.3-windows-amd64/LICENSE", b"license"),
			("sing-box-1.14.3-windows-amd64/sing-box.exe", b"engine"),
		]);
		assert_eq!(extract_engine(&archive, WINDOWS).unwrap(), b"engine");
	}

	#[test]
	fn tar_gz_extraction_takes_only_the_engine() {
		let archive = tar_gz_with(&[
			("sing-box-1.14.3-linux-amd64/LICENSE", b"license"),
			("sing-box-1.14.3-linux-amd64/sing-box", b"engine"),
		]);
		assert_eq!(extract_engine(&archive, LINUX).unwrap(), b"engine");
	}

	#[test]
	fn an_archive_without_the_engine_is_refused() {
		assert!(extract_engine(&zip_with(&[("README.md", b"text")]), WINDOWS).is_err());
		assert!(extract_engine(&tar_gz_with(&[("README.md", b"text")]), LINUX).is_err());
	}

	#[test]
	fn every_platform_pins_a_well_formed_trusted_release() {
		for target in [
			WINDOWS,
			Target {
				arch: "arm64",
				..WINDOWS
			},
			LINUX,
			Target {
				arch: "arm64",
				..LINUX
			},
			Target {
				os: "darwin",
				arch: "amd64",
				zip: false,
				entry: "sing-box",
			},
			Target {
				os: "darwin",
				arch: "arm64",
				zip: false,
				entry: "sing-box",
			},
		] {
			let (size, sha256) = pinned_digest(target).expect("every platform is pinned");
			assert!(size > 0);
			assert!(is_sha256_hex(sha256));
			let url = release_url(PINNED_VERSION, &target.asset(PINNED_VERSION));
			assert!(trusted_download(&url), "{url}");
		}
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
	fn the_daily_check_waits_a_day_after_a_success() {
		let root = std::env::temp_dir().join(format!("nivra-engine-due-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		assert!(check_due(&root), "never checked");
		mark_checked(&root);
		assert!(!check_due(&root), "checked just now");
		let _ = std::fs::remove_dir_all(&root);
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

	/// Live check: GitHub publishes a digest for the latest stable build of this platform.
	#[test]
	#[ignore = "needs network access to GitHub"]
	fn live_latest_engine_has_a_published_digest() {
		let runtime = tokio::runtime::Runtime::new().unwrap();
		let latest = runtime
			.block_on(latest())
			.expect("GitHub answers the sing-box release query")
			.expect("this platform has a build");
		assert!(is_sha256_hex(&latest.sha256));
		assert!(trusted_download(&latest.url));
		assert!(is_safe_version(&latest.version));
	}

	#[test]
	fn the_newest_installed_engine_is_chosen() {
		let root = std::env::temp_dir().join(format!("nivra-engine-scan-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		let entry = target().expect("tests run on a supported platform").entry;
		for version in ["1.14.2", "1.14.10", "1.14.3"] {
			let dir = root.join(format!("sing-box-{version}"));
			std::fs::create_dir_all(&dir).unwrap();
			std::fs::write(dir.join(entry), b"x").unwrap();
		}
		let (path, version) = installed(&root).unwrap();
		assert_eq!(version, "1.14.10");
		assert!(path.ends_with(entry));
		let _ = std::fs::remove_dir_all(&root);
	}
}
