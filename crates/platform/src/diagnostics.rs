//! Bounded rotating diagnostic log with mandatory redaction before anything is written.
//!
//! Files live in `<data dir>/logs`: `nivra.log` plus `nivra.1.log`..`nivra.4.log`, each at
//! most [`MAX_FILE_BYTES`]. Secrets, signed URLs, e-mail addresses and snowflake ids are
//! masked by [`redact`] on the way in and again when a summary is read back, so a copy of
//! the log is safe to paste into a bug report.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// One rotated file plus the live one; oldest is `nivra.4.log`.
const MAX_FILES: usize = 5;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// One written line never grows past this; longer diagnostics are truncated.
const MAX_LINE_BYTES: usize = 2 * 1024;
const CRASH_FILE: &str = "last-crash.txt";

static WRITE_LOCK: Mutex<()> = Mutex::new(());
/// Set exactly once by [`init_app_logging`]; anything else (tests, helper binaries)
/// keeps the user's log untouched and writes to a scratch directory instead.
static APP_LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
	Error,
	Warn,
	Info,
}

impl Level {
	fn label(self) -> &'static str {
		match self {
			Self::Error => "ERROR",
			Self::Warn => "WARN ",
			Self::Info => "INFO ",
		}
	}
}

/// Marks the real log directory for the installed app. Call once from `main` before
/// anything can log or panic. Tests never call it, so a test run can no longer append
/// `host=127.0.0.1` probe lines to the user's own `nivra.log` (v1.0.12 bug).
pub fn init_app_logging() {
	let Ok(dir) = crate::migration::ensure_data_dir() else {
		return;
	};
	let dir = dir.join("logs");
	let _ = std::fs::create_dir_all(&dir);
	if APP_LOG_DIR.set(dir.clone()).is_ok() {
		purge_test_lines_in(&dir);
	}
}

/// The log directory: an explicit override, else the app directory marked by
/// [`init_app_logging`], else a per-process scratch directory so tests and helper
/// binaries never write into the user's data.
pub fn log_dir() -> PathBuf {
	if let Some(dir) = std::env::var_os("NIVRA_LOG_DIR") {
		return PathBuf::from(dir);
	}
	if let Some(dir) = APP_LOG_DIR.get() {
		return dir.clone();
	}
	std::env::temp_dir().join("nivra-scratch-logs")
}

/// Removes lines written by old test runs that probed `127.0.0.1` from the live log and
/// its rotations, keeping every real line. A genuine off-host redirect from Discord logs
/// its own host and must survive. Rewrites only files that match.
fn purge_test_lines_in(dir: &Path) {
	for name in std::iter::once("nivra.log".to_string())
		.chain((1..MAX_FILES).map(|i| format!("nivra.{i}.log")))
	{
		let path = dir.join(name);
		let Ok(text) = std::fs::read_to_string(&path) else {
			continue;
		};
		if !text.contains("host=127.0.0.1") {
			continue;
		}
		let kept: String = text
			.lines()
			.filter(|line| !line.contains("host=127.0.0.1"))
			.collect::<Vec<_>>()
			.join("\n");
		let kept = if kept.is_empty() {
			String::new()
		} else {
			format!("{kept}\n")
		};
		let temporary = path.with_extension("purge");
		if std::fs::write(&temporary, kept).is_ok() {
			let _ = std::fs::rename(&temporary, &path);
		}
	}
}

/// Convenience wrappers so call sites read as diagnostics instead of `eprintln!`.
pub fn error(message: &str) {
	append(Level::Error, message);
}

pub fn warn(message: &str) {
	append(Level::Warn, message);
}

pub fn info(message: &str) {
	append(Level::Info, message);
}

/// Caps a log line at MAX_LINE_BYTES without splitting a multibyte character.
/// Byte truncation panics on a split character, so logging must never do it.
fn truncate_line(mut line: String) -> String {
	if line.len() > MAX_LINE_BYTES {
		let mut end = MAX_LINE_BYTES;
		while !line.is_char_boundary(end) {
			end -= 1;
		}
		line.truncate(end);
		line.push('\n');
	}
	line
}

/// Writes one redacted line, rotating the file first when it reached its bound.
pub fn append(level: Level, message: &str) {	let _guard = WRITE_LOCK
		.lock()
		.unwrap_or_else(|poison| poison.into_inner());
	let dir = log_dir();
	let _ = std::fs::create_dir_all(&dir);
	let path = dir.join("nivra.log");
	if std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0) >= MAX_FILE_BYTES {
		rotate(&dir);
	}
	let mut line = format!("{} {} {}\n", timestamp(), level.label(), redact(message));
	line = truncate_line(line);
	if let Ok(mut file) = std::fs::OpenOptions::new()
		.create(true)
		.append(true)
		.open(&path)
	{
		let _ = file.write_all(line.as_bytes());
	}
}

/// Local wall-clock time for every line: `YYYY-MM-DD HH:MM:SS`.
pub fn timestamp() -> String {
	let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
	format!(
		"{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
		now.year(),
		u8::from(now.month()),
		now.day(),
		now.hour(),
		now.minute(),
		now.second()
	)
}

fn rotate(dir: &Path) {
	// Drop the oldest first, then shift up so the live file becomes `nivra.1.log`.
	let _ = std::fs::remove_file(dir.join(format!("nivra.{}.log", MAX_FILES - 1)));
	for index in (1..MAX_FILES - 1).rev() {
		let from = dir.join(format!("nivra.{index}.log"));
		if from.exists() {
			let _ = std::fs::rename(&from, dir.join(format!("nivra.{}.log", index + 1)));
		}
	}
	let _ = std::fs::rename(dir.join("nivra.log"), dir.join("nivra.1.log"));
}

/// The newest `lines` lines of the log, redacted again, oldest first.
pub fn tail(lines: usize) -> String {
	tail_in(&log_dir(), lines)
}

fn tail_in(dir: &Path, lines: usize) -> String {
	let mut files = vec![dir.join("nivra.log")];
	for index in 1..MAX_FILES {
		files.push(dir.join(format!("nivra.{index}.log")));
	}
	let mut collected: Vec<String> = Vec::new();
	for file in files {
		let Ok(text) = std::fs::read_to_string(&file) else {
			continue;
		};
		for line in text.lines().rev() {
			collected.push(redact(line));
			if collected.len() >= lines {
				break;
			}
		}
		if collected.len() >= lines {
			break;
		}
	}
	collected.reverse();
	collected.join("\n")
}

/// Copyable diagnostics: app version, OS, architecture and the last 300 redacted lines.
pub fn summary() -> String {
	format!(
		"Nivra {}\nOS: {} {}\n\n{}",
		env!("CARGO_PKG_VERSION"),
		std::env::consts::OS,
		std::env::consts::ARCH,
		tail(300)
	)
}

/// The user's Desktop directory; where an exported diagnostics archive lands.
pub fn desktop_dir() -> Option<PathBuf> {
	dirs::desktop_dir()
}

/// The live log plus rotations, limited to lines stamped in the last 24 local hours.
/// Lines from the older epoch-only format are kept whole; rotation already bounds them.
pub fn log_last_24h() -> String {
	log_last_24h_in(&log_dir())
}

fn log_last_24h_in(dir: &Path) -> String {
	let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
	let cutoff = now - time::Duration::hours(24);
	let cutoff = format!(
		"{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
		cutoff.year(),
		u8::from(cutoff.month()),
		cutoff.day(),
		cutoff.hour(),
		cutoff.minute(),
		cutoff.second()
	);
	let mut files = vec![dir.join("nivra.log")];
	for index in 1..MAX_FILES {
		files.push(dir.join(format!("nivra.{index}.log")));
	}
	let mut out = String::new();
	for file in files {
		let Ok(text) = std::fs::read_to_string(&file) else {
			continue;
		};
		for line in text.lines() {
			let stamped = line.len() >= 19 && line.as_bytes().get(4) == Some(&b'-');
			if !stamped || line >= cutoff.as_str() {
				out.push_str(line);
				out.push('\n');
			}
		}
	}
	out
}

/// Opens the log folder in the OS file manager.
pub fn open_log_folder() {
	let dir = log_dir();
	let _ = std::fs::create_dir_all(&dir);
	#[cfg(target_os = "windows")]
	{
		let _ = crate::processes::hidden_command("explorer")
			.arg(&dir)
			.spawn();
	}
	#[cfg(target_os = "macos")]
	{
		let _ = std::process::Command::new("open").arg(&dir).spawn();
	}
	#[cfg(target_os = "linux")]
	{
		let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
	}
	#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
	{
		let _ = &dir;
	}
}

/// Installs a panic hook that logs the failure and leaves a report for the next launch.
pub fn install_panic_hook() {
	std::panic::set_hook(Box::new(|info| {
		let thread = std::thread::current()
			.name()
			.map(str::to_owned)
			.unwrap_or_else(|| "unnamed".to_string());
		let report = format!(
			"panic on thread {thread}: {info}\n{}",
			std::backtrace::Backtrace::force_capture()
		);
		append(Level::Error, &report);
		let dir = log_dir();
		let _ = std::fs::create_dir_all(&dir);
		let _ = std::fs::write(dir.join(CRASH_FILE), redact(&report));
	}));
}

/// Removes and returns the crash report left by the previous run, if any.
pub fn take_crash_report() -> Option<String> {
	take_crash_report_in(&log_dir())
}

fn take_crash_report_in(dir: &Path) -> Option<String> {
	let path = dir.join(CRASH_FILE);
	let report = std::fs::read_to_string(&path).ok()?;
	let _ = std::fs::remove_file(&path);
	(!report.trim().is_empty()).then_some(report)
}

/// Keys whose value is masked up to the next structural delimiter.
const SECRET_KEYS: &[&str] = &[
	"access_token",
	"refresh_token",
	"client_secret",
	"code_verifier",
	"token",
	"password",
	"secret",
	"session",
];
/// Header-like keys: the rest of the line is the value and is masked entirely.
const HEADER_KEYS: &[&str] = &[
	"authorization",
	"cookie",
	"set-cookie",
	"proxy-authorization",
];
/// Signed-URL query parameters used by the Discord CDN.
const SIGNED_PARAMS: &[&str] = &["ex", "is", "hm", "signature", "sig"];

/// Masks secrets, headers, signed-URL parameters, e-mails and snowflake ids.
pub fn redact(line: &str) -> String {
	let mut out = line.to_string();
	for key in HEADER_KEYS {
		out = mask_header(&out, key);
	}
	for key in SECRET_KEYS {
		out = mask_value(&out, key);
	}
	for key in SIGNED_PARAMS {
		out = mask_value(&out, key);
	}
	out = mask_emails(&out);
	mask_snowflakes(&out)
}

fn find_ci(haystack: &str, needle: &str, from: usize) -> Option<usize> {
	haystack
		.get(from..)?
		.to_ascii_lowercase()
		.find(needle)
		.map(|index| index + from)
}

fn is_boundary(byte: u8) -> bool {
	!byte.is_ascii_alphanumeric() && byte != b'_'
}

/// `Authorization: Bearer abc` -> `Authorization: <redacted>` (rest of the line).
fn mask_header(line: &str, key: &str) -> String {
	let mut result = line.to_string();
	let mut from = 0;
	while let Some(position) = find_ci(&result, key, from) {
		let key_end = position + key.len();
		let boundary = position == 0 || is_boundary(result.as_bytes()[position - 1]);
		if !boundary {
			from = key_end;
			continue;
		}
		let Some(separator) = result[key_end..].find([':', '=']) else {
			break;
		};
		let value_start = key_end + separator + 1;
		result.replace_range(value_start.., " <redacted>");
		break;
	}
	result
}

/// `token=abc&x=1` -> `token=<redacted>&x=1`; quoted JSON values keep their quotes.
fn mask_value(line: &str, key: &str) -> String {
	let mut result = line.to_string();
	let mut from = 0;
	while let Some(position) = find_ci(&result, key, from) {
		let key_end = position + key.len();
		let boundary = position == 0 || is_boundary(result.as_bytes()[position - 1]);
		if !boundary {
			from = key_end;
			continue;
		}
		let rest = &result[key_end..];
		let separator = rest.find([':', '=']);
		let Some(separator) = separator else {
			break;
		};
		if separator > 3 {
			// Not `key=value`: the key appears inside prose.
			from = key_end;
			continue;
		}
		let value_start = key_end + separator + 1;
		let mut start = value_start;
		while let Some(character) = result[start..].chars().next() {
			if matches!(character, ' ' | '=' | ':') {
				start += character.len_utf8();
			} else {
				break;
			}
		}
		// A quoted JSON value keeps its quotes; the closing quote is not masked.
		let quote = result[start..].chars().next();
		let closes = matches!(quote, Some('"' | '\''));
		if closes {
			start += 1;
		}
		let mut end = start;
		while let Some(character) = result[end..].chars().next() {
			if matches!(character, ' ' | ',' | '&' | '}' | ']' | ')' | '\n' | '\r') {
				break;
			}
			if closes && character == quote.expect("checked") {
				break;
			}
			end += character.len_utf8();
		}
		if end > start {
			result.replace_range(start..end, "<redacted>");
			from = start + "<redacted>".len();
		} else {
			from = key_end;
		}
	}
	result
}

fn mask_emails(line: &str) -> String {
	let bytes = line.as_bytes();
	let mut result = String::with_capacity(line.len());
	let mut cursor = 0;
	let mut index = 0;
	while index < bytes.len() {
		if bytes[index] == b'@' {
			// Walk back over the local part and forward over the domain.
			let mut start = index;
			while start > 0
				&& !bytes[start - 1].is_ascii_whitespace()
				&& bytes[start - 1] != b'<'
			{
				start -= 1;
			}
			let mut end = index + 1;
			while end < bytes.len()
				&& !bytes[end].is_ascii_whitespace()
				&& !matches!(bytes[end], b'>' | b',' | b';')
			{
				end += 1;
			}
			// The byte walks can stop inside a multibyte character; snap the
			// span edges to whole characters so every slice below is safe.
			while !line.is_char_boundary(start) {
				start += 1;
			}
			while !line.is_char_boundary(end) {
				end -= 1;
			}
			if end > index + 1 && line[index + 1..end].contains('.') {
				// Flush in line coordinates: the result buffer shrinks with
				// every replacement, so its length must never be reused as a
				// line offset.
				result.push_str(&line[cursor..start]);
				result.push_str("<email>");
				cursor = end;
				index = end;
				continue;
			}
		}
		let character = line[index..].chars().next().expect("index in range");
		index += character.len_utf8();
	}
	result.push_str(&line[cursor..]);
	result
}

fn mask_snowflakes(line: &str) -> String {
	let bytes = line.as_bytes();
	let mut result = String::with_capacity(line.len());
	let mut index = 0;
	while index < bytes.len() {
		if bytes[index].is_ascii_digit() {
			let start = index;
			while index < bytes.len() && bytes[index].is_ascii_digit() {
				index += 1;
			}
			if index - start >= 15 {
				result.push_str("<id>");
				continue;
			}
			result.push_str(&line[start..index]);
			continue;
		}
		let character = line[index..].chars().next().expect("index in range");
		result.push(character);
		index += character.len_utf8();
	}
	result
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn secrets_headers_signed_urls_emails_and_ids_are_masked() {
		assert_eq!(
			redact("Authorization: Bearer abc123"),
			"Authorization: <redacted>"
		);
		assert_eq!(redact("cookie: a=b; c=d"), "cookie: <redacted>");
		assert_eq!(redact("token=abc&x=1"), "token=<redacted>&x=1");
		assert_eq!(
			redact(r#"{"access_token":"secret-value","x":1}"#),
			r#"{"access_token":"<redacted>","x":1}"#
		);
		assert_eq!(
			redact("https://cdn.discordapp.com/a.png?ex=123&is=456&hm=789"),
			"https://cdn.discordapp.com/a.png?ex=<redacted>&is=<redacted>&hm=<redacted>"
		);
		assert_eq!(redact("mail me at user@example.com"), "mail me at <email>");
		assert_eq!(
			redact("cc a@example.com and b@example.org done"),
			"cc <email> and <email> done"
		);
		assert_eq!(
			redact("mail usuário@example.com hoje"),
			"mail <email> hoje"
		);
		assert_eq!(redact("id 123456789012345678"), "id <id>");
		// Ordinary prose and short numbers survive.
		assert_eq!(
			redact("downloaded 3 files in 120 ms"),
			"downloaded 3 files in 120 ms"
		);
	}

	#[test]
	fn truncation_never_splits_a_multibyte_character() {
		// The emoji straddles the byte budget; a naive cut would panic.
		let message = "x".repeat(MAX_LINE_BYTES - "INFO ".len() - 2) + "📯 tail";
		let line = truncate_line(format!("INFO {message}\n"));
		assert!(line.len() <= MAX_LINE_BYTES + 1);
		assert!(line.ends_with('\n'));
		assert!(!line.contains('📯'), "split emoji must be dropped, not cut");
		// Short lines pass through untouched.
		assert_eq!(truncate_line("INFO ok\n".into()), "INFO ok\n");
	}

	/// Child process for the panic-hook test: panics after installing the hook.
	#[test]
	fn panic_hook_child() {
		if std::env::var_os("NIVRA_PANIC_CHILD").is_none() {
			return;
		}
		install_panic_hook();
		panic!("synthetic child panic");
	}

	#[test]
	fn panic_hook_leaves_a_report_for_the_next_launch() {
		let dir = std::env::temp_dir().join(format!("nivra-crash-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		let status = std::process::Command::new(std::env::current_exe().unwrap())
			.args([
				"--exact",
				"diagnostics::tests::panic_hook_child",
				"--nocapture",
			])
			.env("NIVRA_LOG_DIR", &dir)
			.env("NIVRA_PANIC_CHILD", "1")
			.status()
			.expect("spawn the child test");
		assert!(!status.success(), "the child must have panicked");
		let report = take_crash_report_in(&dir).expect("crash report");
		assert!(report.contains("synthetic child panic"), "{report}");
		assert!(dir.join("nivra.log").exists());
		let _ = std::fs::remove_dir_all(&dir);
	}

	/// Child process for the forum fallback line: writes one line into the scratch dir.
	#[test]
	fn forum_fallback_child() {
		if std::env::var_os("NIVRA_FALLBACK_CHILD").is_none() {
			return;
		}
		let report = model::forum::FallbackReport {
			reason: model::forum::FallbackReason::Status(404),
			bytes: 12,
			elapsed: std::time::Duration::from_millis(4_200),
		};
		warn(&report.log_line(model::Id(42)));
	}

	#[test]
	fn forum_fallback_line_lands_in_the_log_with_a_timestamp() {
		let dir = std::env::temp_dir().join(format!("nivra-fallback-log-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		let status = std::process::Command::new(std::env::current_exe().unwrap())
			.args([
				"--exact",
				"diagnostics::tests::forum_fallback_child",
				"--nocapture",
			])
			.env("NIVRA_LOG_DIR", &dir)
			.env("NIVRA_FALLBACK_CHILD", "1")
			.status()
			.expect("spawn the child test");
		assert!(status.success(), "the child writes one log line");
		let log = std::fs::read_to_string(dir.join("nivra.log")).expect("nivra.log");
		let line = log
			.lines()
			.find(|line| line.contains("forum fallback:"))
			.unwrap_or_else(|| panic!("no fallback line in: {log}"));
		assert!(line.contains("reason=status:404"), "{line}");
		assert!(line.contains("bytes=12"), "{line}");
		assert!(line.contains("elapsed_ms=4200"), "{line}");
		// The line starts with the local timestamp the log requires.
		assert!(line.len() > 19, "{line}");
		assert_eq!(line.as_bytes()[4], b'-', "{line}");
		assert_eq!(line.as_bytes()[10], b' ', "{line}");
		assert_eq!(line.as_bytes()[13], b':', "{line}");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn rotation_keeps_five_bounded_files() {
		let dir = std::env::temp_dir().join(format!(
			"nivra-log-{}-{:?}",
			std::process::id(),
			std::thread::current().id()
		));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		// Simulate a full live file without writing 2 MiB in the test.
		std::fs::write(dir.join("nivra.log"), vec![b'x'; MAX_FILE_BYTES as usize]).unwrap();
		rotate(&dir);
		assert!(dir.join("nivra.1.log").exists());
		assert!(!dir.join("nivra.log").exists());
		for _ in 0..8 {
			std::fs::write(dir.join("nivra.log"), vec![b'y'; 16]).unwrap();
			rotate(&dir);
		}
		let files: Vec<_> = std::fs::read_dir(&dir)
			.unwrap()
			.flatten()
			.map(|entry| entry.file_name().to_string_lossy().into_owned())
			.collect();
		// The live file was renamed away by the last rotation: four rotated files remain.
		assert_eq!(files.len(), MAX_FILES - 1);
		assert!(!dir.join(format!("nivra.{MAX_FILES}.log")).exists());
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn scratch_log_dir_never_touches_user_data_without_init() {
		// Tests and helper binaries run without `init_app_logging`; their scratch dir must
		// live under the OS temp directory, never under the user's data dir.
		if std::env::var_os("NIVRA_LOG_DIR").is_some() {
			return;
		}
		let dir = log_dir();
		assert!(
			dir.starts_with(std::env::temp_dir()),
			"uninitialized processes must log under the temp dir: {dir:?}"
		);
		assert!(
			dir.file_name()
				.is_some_and(|name| name == "nivra-scratch-logs"),
			"scratch dir must be unmistakable: {dir:?}"
		);
	}

	#[test]
	fn every_line_starts_with_a_local_timestamp() {
		let stamp = timestamp();
		assert_eq!(stamp.len(), 19, "{stamp}");
		assert_eq!(stamp.as_bytes()[4], b'-', "{stamp}");
		assert_eq!(stamp.as_bytes()[7], b'-', "{stamp}");
		assert_eq!(stamp.as_bytes()[10], b' ', "{stamp}");
		assert_eq!(stamp.as_bytes()[13], b':', "{stamp}");
		assert_eq!(stamp.as_bytes()[16], b':', "{stamp}");
	}

	#[test]
	fn export_keeps_only_the_last_24_hours() {
		let dir = std::env::temp_dir().join(format!(
			"nivra-export-{}-{:?}",
			std::process::id(),
			std::thread::current().id()
		));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		let current = format!("{} INFO probe-new-now", timestamp());
		std::fs::write(
			dir.join("nivra.log"),
			format!(
				"2000-01-01 00:00:00 WARN probe-old-2000
{current}
legacy-epoch-line
"
			),
		)
		.unwrap();
		let kept = log_last_24h_in(&dir);
		assert!(kept.contains("probe-new-now"), "{kept}");
		assert!(!kept.contains("probe-old-2000"), "{kept}");
		// Legacy lines without a stamped prefix are kept; rotation already bounds them.
		assert!(kept.contains("legacy-epoch-line"), "{kept}");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn purge_keeps_real_lines_and_drops_old_probe_lines() {
		let dir = std::env::temp_dir().join(format!(
			"nivra-purge-{}-{:?}",
			std::process::id(),
			std::thread::current().id()
		));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(
			dir.join("nivra.log"),
			"1 INFO real startup line\n2 INFO text preview refused: host=127.0.0.1 status=404\n3 WARN text preview redirect: host=127.0.0.1 status=302 location-host=evil.test\n4 ERROR real failure\n5 WARN text preview redirect: host=cdn.discordapp.com status=302 location-host=shady.example\n",
		)
		.unwrap();
		std::fs::write(
			dir.join("nivra.1.log"),
			"5 INFO text preview refused: host=127.0.0.1 status=403\n",
		)
		.unwrap();
		purge_test_lines_in(&dir);
		let live = std::fs::read_to_string(dir.join("nivra.log")).unwrap();
		assert!(live.contains("real startup line"), "{live}");
		assert!(live.contains("real failure"), "{live}");
		// A genuine off-host redirect logged by the real app survives the purge.
		assert!(live.contains("location-host=shady.example"), "{live}");
		assert!(!live.contains("127.0.0.1"), "{live}");
		assert!(!live.contains("location-host=evil.test"), "{live}");
		let rotated = std::fs::read_to_string(dir.join("nivra.1.log")).unwrap();
		assert!(rotated.is_empty(), "{rotated:?}");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn tail_reads_the_newest_lines_and_redacts_again() {
		let dir = std::env::temp_dir().join(format!(
			"nivra-tail-{}-{:?}",
			std::process::id(),
			std::thread::current().id()
		));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(
			dir.join("nivra.log"),
			"1 INFO ok
token=abc
2 WARN careful
3 ERROR failed
",
		)
		.unwrap();
		assert_eq!(
			tail_in(&dir, 2),
			"2 WARN careful
3 ERROR failed"
				.replace("token=abc", "<redacted>")
		);
		assert!(tail_in(&dir, 3).contains("token=<redacted>"));
		let _ = std::fs::remove_dir_all(&dir);
	}
}
