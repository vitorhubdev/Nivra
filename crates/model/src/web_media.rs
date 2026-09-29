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
}
