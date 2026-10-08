//! Documented guild soundboard sounds: list for playback. Creating, editing
//! and deleting sounds stay in the official client.
use crate::{DiscordApi, Failure};
use discord_protocol::soundboard as wire;
use model::{Id, Sound};
use reqwest::Method;

impl DiscordApi {
	pub(super) async fn soundboard_list(&self, guild: Id) -> Result<Vec<Sound>, Failure> {
		if guild.0 == 0 {
			return Err(Failure::Protocol);
		}
		let bytes = self
			.request_limited(
				Method::GET,
				&format!("/guilds/{guild}/soundboard-sounds"),
				None,
				wire::MAX_WIRE,
			)
			.await?;
		wire::sounds(&bytes, guild).map_err(|_| {
			Failure::ProtocolAt(
				"Soundboard sounds exceeded safe bounds or used an unsupported response",
			)
		})
	}

	pub(super) async fn soundboard_play(
		&self,
		channel: Id,
		sound: Id,
		source_guild: Option<Id>,
	) -> Result<(), Failure> {
		if channel.0 == 0 || sound.0 == 0 {
			return Err(Failure::Protocol);
		}
		let mut body = serde_json::json!({"sound_id": sound.to_string()});
		if let Some(guild) = source_guild {
			body["source_guild_id"] = serde_json::json!(guild.to_string());
		}
		self.request_limited(
			Method::POST,
			&format!("/channels/{channel}/send-soundboard-sound"),
			Some(body),
			64 * 1024,
		)
		.await
		.map(|_| ())
		.map_err(write_failure)
	}
}

fn write_failure(failure: Failure) -> Failure {
	match failure {
		Failure::Protocol | Failure::ProtocolAt(_) | Failure::Capacity => failure,
		_ => Failure::Ambiguous,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[tokio::test]
	async fn soundboard_list_decodes_and_play_posts_ids() {
		crate::ensure_tls_provider();
		tokio::time::timeout(std::time::Duration::from_secs(5), async {
			let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(std::sync::Arc::new(
				super::super::SessionSecret::from_owner_input("SYNTHETIC_SOUNDBOARD_TOKEN".into())
					.unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let server = tokio::spawn(async move {
				for (route, request_body, reply) in [
					(
						"GET /guilds/9/soundboard-sounds HTTP/1.1",
						"",
						r#"[{"name":"Airhorn","sound_id":"7","volume":1.0,"emoji_name":"📯","guild_id":"9","available":true}]"#,
					),
					(
						"POST /channels/2/send-soundboard-sound HTTP/1.1",
						r#"{"sound_id":"7","source_guild_id":"9"}"#,
						r#"{}"#,
					),
				] {
					let (mut socket, _) = listener.accept().await.unwrap();
					let mut request = Vec::new();
					loop {
						let mut bytes = [0; 1024];
						let n = tokio::io::AsyncReadExt::read(&mut socket, &mut bytes)
							.await
							.unwrap();
						assert!(n > 0);
						request.extend_from_slice(&bytes[..n]);
						if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
							let head = String::from_utf8_lossy(&request[..end]);
							let length: usize = head
								.lines()
								.find_map(|line| {
									line.to_ascii_lowercase()
										.strip_prefix("content-length: ")
										.map(str::to_owned)
								})
								.unwrap_or_default()
								.parse()
								.unwrap_or(0);
							if request.len() >= end + 4 + length {
								assert!(head.starts_with(route));
								if !request_body.is_empty() {
									let sent: serde_json::Value =
										serde_json::from_slice(&request[end + 4..end + 4 + length])
											.unwrap();
									let expected: serde_json::Value =
										serde_json::from_str(request_body).unwrap();
									assert_eq!(sent, expected);
								}
								break;
							}
						}
					}
					let response = format!(
						"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
						reply.len(),
						reply
					);
					tokio::io::AsyncWriteExt::write_all(&mut socket, response.as_bytes())
						.await
						.unwrap();
				}
			});
			let sounds = api.soundboard_list(model::Id(9)).await.unwrap();
			assert_eq!(sounds.len(), 1);
			assert_eq!(sounds[0].name, "Airhorn");
			api.soundboard_play(model::Id(2), model::Id(7), Some(model::Id(9)))
				.await
				.unwrap();
			server.await.unwrap();
		})
		.await
		.unwrap();
	}
}
