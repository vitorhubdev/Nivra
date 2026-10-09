//! Documented guild scheduled-event list. RSVP stays out until the write
//! path is live verified.
use crate::{DiscordApi, Failure};
use discord_protocol::scheduled_events as wire;
use model::{Id, ScheduledEvent};
use reqwest::Method;

impl DiscordApi {
	pub(super) async fn scheduled_events(&self, guild: Id) -> Result<Vec<ScheduledEvent>, Failure> {
		if guild.0 == 0 {
			return Err(Failure::Protocol);
		}
		let bytes = self
			.request_limited(
				Method::GET,
				&format!("/guilds/{guild}/scheduled-events?with_user_count=true"),
				None,
				wire::MAX_WIRE,
			)
			.await?;
		wire::events(&bytes, guild).map_err(|_| {
			Failure::ProtocolAt(
				"Scheduled events exceeded safe bounds or used an unsupported response",
			)
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
		let mut request = Vec::new();
		loop {
			let mut bytes = [0; 1024];
			let n = tokio::io::AsyncReadExt::read(socket, &mut bytes)
				.await
				.unwrap();
			assert!(n > 0);
			request.extend_from_slice(&bytes[..n]);
			if request.windows(4).position(|w| w == b"\r\n\r\n").is_some() {
				let head = String::from_utf8_lossy(&request).into_owned();
				return head;
			}
		}
	}

	#[tokio::test]
	async fn scheduled_event_list_decodes() {
		crate::ensure_tls_provider();
		tokio::time::timeout(std::time::Duration::from_secs(5), async {
			let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(std::sync::Arc::new(
				super::super::SessionSecret::from_owner_input("SYNTHETIC_EVENTS_TOKEN".into())
					.unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let server = tokio::spawn(async move {
				let (mut socket, _) = listener.accept().await.unwrap();
				let head = read_request(&mut socket).await;
				assert!(head.starts_with(
					"GET /guilds/9/scheduled-events?with_user_count=true HTTP/1.1"
				));
				let page = r#"[{"id":"5","guild_id":"9","channel_id":"2","name":"Game night","description":"Bring dice","scheduled_start_time":"2026-10-20T19:00:00+00:00","status":1,"entity_type":2,"user_count":7}]"#;
				let response = format!(
					"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
					page.len(),
					page
				);
				tokio::io::AsyncWriteExt::write_all(&mut socket, response.as_bytes())
					.await
					.unwrap();
			});
			let events = api.scheduled_events(model::Id(9)).await.unwrap();
			assert_eq!(events.len(), 1);
			assert_eq!(events[0].name, "Game night");
			assert_eq!(events[0].user_count, 7);
			server.await.unwrap();
		})
		.await
		.unwrap();
	}
}
