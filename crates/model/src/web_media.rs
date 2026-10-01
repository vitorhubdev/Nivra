//! Shared HTTPS allowlist and URL rewriting for inline YouTube, X, and Vimeo playback.
//! Pure string parsing so `model` stays free of URL-crate and filesystem dependencies.

const PROVIDER_HOSTS: &[&str] = &[
	"youtube.com",
	"m.youtube.com",
	"youtu.be",
	"x.com",
	"twitter.com",
	"vimeo.com",
	"player.vimeo.com",
];

const NAVIGATION_HOSTS: &[&str] = &[
	"youtube.com",
	"www.youtube.com",
	"m.youtube.com",
	"youtube-nocookie.com",
	"www.youtube-nocookie.com",
	"x.com",
	"www.x.com",
	"twitter.com",
	"www.twitter.com",
	"vimeo.com",
	"www.vimeo.com",
	"player.vimeo.com",
];

struct HttpsUrl<'a> {
	host: String,
	path: &'a str,
	query: &'a str,
}

fn parse_https(value: &str) -> Option<HttpsUrl<'_>> {
	if value.len() > 2048
		|| !value
			.bytes()
			.all(|byte| byte.is_ascii_graphic() && byte != b'\\')
	{
		return None;
	}
	let rest = value
		.split_once("://")
		.and_then(|(scheme, rest)| scheme.eq_ignore_ascii_case("https").then_some(rest))?;
	let (authority, remainder) = match rest.find(['/', '?', '#']) {
		Some(index) => (&rest[..index], &rest[index..]),
		None => (rest, ""),
	};
	if authority.is_empty() || authority.contains('@') || authority.starts_with('[') {
		return None;
	}
	let host = match authority.rsplit_once(':') {
		Some((host, port)) => {
			if host.is_empty() || host.contains(':') || port != "443" {
				return None;
			}
			host
		}
		None => authority,
	};
	if host.is_empty() || host.starts_with('.') || host.ends_with('.') || host.contains("..") {
		return None;
	}
	let without_fragment = remainder
		.split_once('#')
		.map_or(remainder, |(path, _)| path);
	let (path, query) = without_fragment
		.split_once('?')
		.map_or((without_fragment, ""), |(path, query)| (path, query));
	Some(HttpsUrl {
		host: host.to_ascii_lowercase(),
		path: if path.is_empty() { "/" } else { path },
		query,
	})
}

fn path_segments(path: &str) -> impl Iterator<Item = &str> {
	path.strip_prefix('/').unwrap_or(path).split('/')
}

fn query_value<'a>(query: &'a str, name: &str) -> Option<&'a str> {
	query.split('&').find_map(|pair| {
		let (key, value) = pair.split_once('=')?;
		(key == name).then_some(value)
	})
}

fn valid_x_username(value: &str) -> bool {
	(1..=15).contains(&value.len())
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn video_id(value: &str) -> Option<&str> {
	(!value.is_empty()
		&& value.len() <= 32
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
	.then_some(value)
}

fn provider_host(host: &str) -> &str {
	host.strip_prefix("www.").unwrap_or(host)
}

/// Strict HTTPS host check used by the chat play button.
pub fn is_supported_host(value: &str) -> bool {
	parse_https(value).is_some_and(|url| PROVIDER_HOSTS.contains(&provider_host(&url.host)))
}

/// Webview navigation after an embed opens, including `youtube-nocookie` and `www` hosts.
pub fn navigation_allowed(value: &str) -> bool {
	parse_https(value).is_some_and(|url| NAVIGATION_HOSTS.contains(&url.host.as_str()))
}

fn youtube_id<'a>(url: &HttpsUrl<'a>) -> Option<&'a str> {
	let host = provider_host(&url.host);
	let id = match host {
		"youtu.be" => path_segments(url.path).next()?,
		"youtube.com" | "m.youtube.com" if url.path == "/watch" => query_value(url.query, "v")?,
		"youtube.com" | "m.youtube.com" => {
			let mut parts = path_segments(url.path);
			let kind = parts.next()?;
			if !matches!(kind, "shorts" | "embed") {
				return None;
			}
			parts.next()?
		}
		_ => return None,
	};
	video_id(id)
}

/// `https://i.ytimg.com/vi/{id}/hqdefault.jpg` for YouTube watch/shorts/embed/youtu.be URLs.
pub fn youtube_thumbnail_url(value: &str) -> Option<String> {
	let id = youtube_id(&parse_https(value)?)?;
	Some(format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"))
}

/// Rewrite a supported provider URL to the isolated autoplay embed (or canonical X status).
pub fn normalize(value: &str) -> Option<String> {
	let url = parse_https(value)?;
	match provider_host(&url.host) {
		"youtu.be" | "youtube.com" | "m.youtube.com" => {
			let id = youtube_id(&url)?;
			Some(format!(
				"https://www.youtube-nocookie.com/embed/{id}?autoplay=1"
			))
		}
		"x.com" | "twitter.com" => {
			let parts: Vec<_> = path_segments(url.path)
				.filter(|part| !part.is_empty())
				.take(4)
				.collect();
			if parts.len() < 3
				|| !valid_x_username(parts[0])
				|| parts[1] != "status"
				|| !parts[2].bytes().all(|byte| byte.is_ascii_digit())
			{
				return None;
			}
			Some(format!("https://x.com/{}/status/{}", parts[0], parts[2]))
		}
		"vimeo.com" => {
			let id = path_segments(url.path)
				.find(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))?;
			if id.is_empty() {
				return None;
			}
			Some(format!("https://player.vimeo.com/video/{id}?autoplay=1"))
		}
		"player.vimeo.com" => {
			let mut parts = path_segments(url.path);
			if parts.next()? != "video" {
				return None;
			}
			let id = parts.next()?;
			if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
				return None;
			}
			Some(format!("https://player.vimeo.com/video/{id}?autoplay=1"))
		}
		_ => None,
	}
}

fn video_file(path: &str) -> bool {
	path.rsplit('/')
		.next()
		.unwrap_or(path)
		.rsplit_once('.')
		.is_some_and(|(_, extension)| {
			matches!(
				extension.to_ascii_lowercase().as_str(),
				"mp4" | "webm" | "mov" | "m4v"
			)
		})
}

fn is_direct_video_url(value: &str) -> bool {
	if value.len() > 2048 || value.contains('#') || value.contains('\\') {
		return false;
	}
	let Some(url) = parse_https(value) else {
		return false;
	};
	let path = url.path.to_ascii_lowercase();
	if path.contains("%2f") || path.contains("%5c") || !video_file(&path) {
		return false;
	}
	match provider_host(&url.host) {
		"media.discordapp.net" => {
			path.starts_with("/external/") || path.starts_with("/attachments/")
		}
		"cdn.discordapp.com" => path.starts_with("/attachments/"),
		"video.twimg.com" => true,
		_ => false,
	}
}

/// Direct video file on the embed allowlist. Proxy URL wins over the provider page.
pub fn direct_embed_video<'a>(proxy: Option<&'a str>, url: Option<&'a str>) -> Option<&'a str> {
	[proxy, url]
		.into_iter()
		.flatten()
		.find(|value| is_direct_video_url(value))
}

/// What an embed card should do. A direct file plays in-app; YouTube and Vimeo open outside.
pub enum EmbedVideo<'a> {
	File(&'a str),
	YouTube(&'a str),
	Vimeo(&'a str),
}

pub fn classify_embed_video<'a>(
	page: Option<&'a str>,
	video_url: Option<&'a str>,
	proxy: Option<&'a str>,
) -> Option<EmbedVideo<'a>> {
	if let Some(file) = direct_embed_video(proxy, video_url) {
		return Some(EmbedVideo::File(file));
	}
	let page = page.or(video_url)?;
	let url = parse_https(page)?;
	match provider_host(&url.host) {
		"youtu.be" | "youtube.com" | "m.youtube.com" => Some(EmbedVideo::YouTube(page)),
		"vimeo.com" | "player.vimeo.com" => Some(EmbedVideo::Vimeo(page)),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn well_formed(host: &str) -> String {
		match host {
			"youtu.be" => format!("https://{host}/dQw4w9WgXcQ"),
			"youtube.com" | "m.youtube.com" => format!("https://{host}/watch?v=dQw4w9WgXcQ"),
			"x.com" | "twitter.com" => format!("https://{host}/user/status/123"),
			"vimeo.com" => format!("https://{host}/123456"),
			"player.vimeo.com" => format!("https://{host}/video/123456"),
			other => panic!("provider host {other} is missing a well-formed fixture"),
		}
	}

	#[test]
	fn provider_hosts_cannot_drift_between_play_button_and_normalize() {
		assert_eq!(
			PROVIDER_HOSTS,
			[
				"youtube.com",
				"m.youtube.com",
				"youtu.be",
				"x.com",
				"twitter.com",
				"vimeo.com",
				"player.vimeo.com"
			]
		);
		for host in PROVIDER_HOSTS {
			let url = well_formed(host);
			assert!(is_supported_host(&url), "{url}");
			assert!(normalize(&url).is_some(), "{url}");
		}
	}

	#[test]
	fn normalizes_supported_media_without_open_redirects() {
		assert_eq!(
			normalize("https://youtu.be/dQw4w9WgXcQ?si=tracking").as_deref(),
			Some("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1")
		);
		assert_eq!(
			normalize("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=ignored").as_deref(),
			Some("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1")
		);
		assert_eq!(
			normalize("https://m.youtube.com/shorts/dQw4w9WgXcQ").as_deref(),
			Some("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1")
		);
		assert_eq!(
			normalize("https://youtube.com/embed/dQw4w9WgXcQ").as_deref(),
			Some("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1")
		);
		assert_eq!(
			normalize("https://x.com/user/status/123?utm_source=ignored").as_deref(),
			Some("https://x.com/user/status/123")
		);
		assert_eq!(
			normalize("https://twitter.com/user/status/123").as_deref(),
			Some("https://x.com/user/status/123")
		);
		assert_eq!(
			normalize("https://vimeo.com/123456").as_deref(),
			Some("https://player.vimeo.com/video/123456?autoplay=1")
		);
		assert_eq!(
			normalize("https://player.vimeo.com/video/123456").as_deref(),
			Some("https://player.vimeo.com/video/123456?autoplay=1")
		);
		assert!(normalize("https://youtube.com.evil.test/watch?v=dQw4w9WgXcQ").is_none());
		assert!(normalize("http://x.com/user/status/123").is_none());
		assert!(normalize("https://youtube.com:444/watch?v=dQw4w9WgXcQ").is_none());
		assert!(normalize("https://user@x.com/user/status/123").is_none());
	}

	#[test]
	fn supported_hosts_use_stricter_https_port_and_exact_host_rules() {
		for url in [
			"https://youtube.com/watch?v=dQw4w9WgXcQ",
			"https://www.youtube.com/watch?v=dQw4w9WgXcQ",
			"https://youtube.com:443/watch?v=dQw4w9WgXcQ",
			"https://youtu.be/dQw4w9WgXcQ",
			"https://m.youtube.com/shorts/dQw4w9WgXcQ",
			"https://x.com/example/status/123",
			"https://twitter.com/example/status/123",
			"https://vimeo.com/123456",
			"https://player.vimeo.com/video/123456",
		] {
			assert!(is_supported_host(url), "{url}");
		}
		for url in [
			"http://youtube.com/watch?v=dQw4w9WgXcQ",
			"https://youtube.com.evil.test/watch?v=dQw4w9WgXcQ",
			"https://user@x.com/example/status/123",
			"https://user:pass@youtube.com/watch?v=dQw4w9WgXcQ",
			"https://youtube.com:444/watch?v=dQw4w9WgXcQ",
			"https://example.com/video",
		] {
			assert!(!is_supported_host(url), "{url}");
			assert!(normalize(url).is_none(), "{url}");
		}
	}

	#[test]
	fn doubled_www_prefix_is_not_a_supported_host() {
		assert!(!is_supported_host(
			"https://www.www.youtube.com/watch?v=dQw4w9WgXcQ"
		));
		assert!(normalize("https://www.www.youtube.com/watch?v=dQw4w9WgXcQ").is_none());
		assert!(is_supported_host(
			"https://www.youtube.com/watch?v=dQw4w9WgXcQ"
		));
	}

	#[test]
	fn x_usernames_use_the_platform_charset() {
		assert!(normalize("https://x.com/a\"b<>/status/123").is_none());
		assert_eq!(
			normalize("https://x.com/jack/status/20").as_deref(),
			Some("https://x.com/jack/status/20")
		);
	}

	#[test]
	fn vimeo_ids_skip_empty_path_segments() {
		assert_eq!(
			normalize("https://vimeo.com/channels//123456").as_deref(),
			Some("https://player.vimeo.com/video/123456?autoplay=1")
		);
	}

	#[test]
	fn youtube_thumbnail_is_built_only_for_youtube_ids() {
		assert_eq!(
			youtube_thumbnail_url("https://youtu.be/dQw4w9WgXcQ").as_deref(),
			Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg")
		);
		assert_eq!(
			youtube_thumbnail_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ").as_deref(),
			Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg")
		);
		assert!(youtube_thumbnail_url("https://x.com/user/status/123").is_none());
	}

	#[test]
	fn navigation_allows_embed_hosts_without_widening_the_play_button() {
		assert!(navigation_allowed(
			"https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1"
		));
		assert!(navigation_allowed("https://www.x.com/user/status/123"));
		assert!(!is_supported_host(
			"https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1"
		));
		assert!(!navigation_allowed(
			"https://youtube.com.evil.test/watch?v=dQw4w9WgXcQ"
		));
		assert!(!navigation_allowed(
			"https://youtube-nocookie.com:444/embed/x"
		));
	}

	#[test]
	fn direct_mp4_plays_in_app_and_youtube_stays_outside() {
		let file = "https://media.discordapp.net/external/video.twimg.com/ext/oobe-intro.mp4?backend=b2";
		assert_eq!(direct_embed_video(Some(file), None), Some(file));
		assert!(matches!(
			classify_embed_video(Some("https://x.com/user/status/1"), None, Some(file)),
			Some(EmbedVideo::File(url)) if url == file
		));
		assert!(matches!(
			classify_embed_video(Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ"), None, None),
			Some(EmbedVideo::YouTube(_))
		));
		assert!(matches!(
			classify_embed_video(Some("https://vimeo.com/123456"), None, None),
			Some(EmbedVideo::Vimeo(_))
		));
		assert!(direct_embed_video(Some("https://evil.test/clip.mp4"), None).is_none());
		assert!(direct_embed_video(Some("https://user@video.twimg.com/clip.mp4"), None).is_none());
		assert!(
			direct_embed_video(
				Some("https://media.discordapp.net/external/video.twimg.com/ext/%2fclip.mp4"),
				None
			)
			.is_none()
		);
	}
}
