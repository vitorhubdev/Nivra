use crate::{DiscordApi, Failure, RequestTrace};
use model::{
	Id,
	forum::{FallbackReason, FallbackReport, Page},
};
use reqwest::Method;
use std::time::Instant;

/// Why the per-forum search route could not serve this page.
enum SearchError {
	/// The service answered with this non-success status.
	Status(u16, usize),
	/// The page exceeded the search wire budget.
	Oversized(usize),
	/// The page did not decode into a valid page.
	Decode(usize),
	/// A failure the caller must see unchanged (session, rate limit, forbidden, network).
	Failure(Failure),
}

impl SearchError {
	/// The fallback reason and the bytes involved, or `None` for a visible failure.
	fn reason(self) -> Option<(FallbackReason, usize)> {
		Some(match self {
			Self::Status(status, bytes) => (FallbackReason::Status(status), bytes),
			Self::Oversized(bytes) => (FallbackReason::Oversized, bytes),
			Self::Decode(bytes) => (FallbackReason::Decode, bytes),
			Self::Failure(_) => return None,
		})
	}
}

/// Map one traced request outcome to the fallback decision. Only a rejected search route
/// (a non-success status or an oversized page) and an undecodable body fall back; a denial
/// or a session failure is reported as itself.
fn search_error(failure: Failure, trace: RequestTrace) -> SearchError {
	match failure {
		Failure::Capacity => SearchError::Oversized(trace.bytes),
		Failure::Protocol => match trace.status {
			Some(status) => SearchError::Status(status, trace.bytes),
			None => SearchError::Failure(Failure::Protocol),
		},
		other => SearchError::Failure(other),
	}
}

impl DiscordApi {
	/// Active posts of one forum. The gateway only syncs joined threads, so the list is fetched.
	///
	/// The per-forum search route is unofficial client behavior; a service that rejects it falls
	/// back to the documented guild-wide active list for the first page. The fallback is timed
	/// and reported so the app can log it and explain a slow list.
	pub(super) async fn forum_posts(
		&self,
		parent: Id,
		guild: Id,
		offset: usize,
	) -> Result<Page, Failure> {
		if parent.0 == 0 || guild.0 == 0 || offset > model::forum::MAX_POSTS {
			return Err(Failure::Protocol);
		}
		let started = Instant::now();
		match self.searched_posts(parent, guild, offset).await {
			Ok(page) => Ok(page),
			Err(SearchError::Failure(failure)) => Err(failure),
			Err(reason) if offset == 0 => {
				let Some((reason, bytes)) = reason.reason() else {
					return Err(Failure::Protocol);
				};
				let mut page = self.active_guild_posts(parent, guild).await?;
				page.fallback = Some(FallbackReport {
					reason,
					bytes,
					elapsed: started.elapsed(),
				});
				Ok(page)
			}
			Err(_) => Err(Failure::Protocol),
		}
	}

	async fn searched_posts(
		&self,
		parent: Id,
		guild: Id,
		offset: usize,
	) -> Result<Page, SearchError> {
		let path = format!(
			"/channels/{parent}/threads/search?archived=false&sort_by=last_message_time&sort_order=desc&limit={}&offset={offset}",
			model::forum::PAGE_SIZE
		);
		let (result, trace) = self
			.request_limited_traced(Method::GET, &path, None, discord_protocol::forum::MAX_WIRE)
			.await;
		let bytes = result.map_err(|failure| search_error(failure, trace))?;
		discord_protocol::decode::<discord_protocol::forum::Reply>(&bytes)
			.map_err(|_| SearchError::Decode(bytes.len()))?
			.into_page(parent, guild)
			.map_err(|_| SearchError::Decode(bytes.len()))
	}

	async fn active_guild_posts(&self, parent: Id, guild: Id) -> Result<Page, Failure> {
		let bytes = self
			.request_limited(
				Method::GET,
				&format!("/guilds/{guild}/threads/active"),
				None,
				discord_protocol::forum::GUILD_MAX_WIRE,
			)
			.await?;
		discord_protocol::decode::<discord_protocol::forum::GuildActive>(&bytes)
			.map_err(|_| Failure::Protocol)?
			.into_page(parent, guild)
			.map_err(|_| Failure::Protocol)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::{Command, Event, auth::SessionSecret};
	use std::{sync::Arc, time::Duration};
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};

	#[tokio::test]
	async fn forum_posts_are_scoped_paged_and_reject_foreign_rows() {
		crate::ensure_tls_provider();
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_FORUM_TOKEN".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let row = |guild: &str| format!(r#"{{"id":"9","guild_id":"{guild}","parent_id":"2","type":11,"name":"Synthetic","thread_metadata":{{"archived":false}},"applied_tags":["31"]}}"#);
			// The starter message travels with the search page; no per-post request is made.
			let starter = r#"{"id":"9","channel_id":"9","author":{"id":"5","username":"Synthetic","discriminator":"0"},"content":"Synthetic starter","attachments":[{"id":"77","filename":"a.png","content_type":"image/png","size":10,"url":"https://cdn.test/a.png","proxy_url":"https://media.test/a.png","width":8,"height":8}],"reactions":[{"count":2,"me":false,"emoji":{"id":null,"name":"🔥"}}]}"#;
			let server = tokio::spawn(async move {
				for (route, status, body) in [
					("/channels/2/threads/search?archived=false&sort_by=last_message_time&sort_order=desc&limit=25&offset=0","200 OK",format!(r#"{{"threads":[{}],"members":[],"has_more":true,"total_results":2,"first_messages":[{starter}]}}"#, row("1"))),
					("/channels/2/threads/search?archived=false&sort_by=last_message_time&sort_order=desc&limit=25&offset=25","200 OK",r#"{"threads":[],"members":[],"has_more":false}"#.to_owned()),
					("/channels/2/threads/search?archived=false&sort_by=last_message_time&sort_order=desc&limit=25&offset=0","404 Not Found",r#"{"code":0}"#.to_owned()),
					("/guilds/1/threads/active","200 OK",format!(r#"{{"threads":[{}],"members":[]}}"#, row("1"))),
					("/channels/2/threads/search?archived=false&sort_by=last_message_time&sort_order=desc&limit=25&offset=0","200 OK",format!(r#"{{"threads":[{}],"members":[],"has_more":false}}"#, row("7"))),
					("/guilds/1/threads/active","403 Forbidden",r#"{"code":50013}"#.to_owned()),
					("/channels/2/threads/search?archived=false&sort_by=last_message_time&sort_order=desc&limit=25&offset=0","403 Forbidden",r#"{"code":50013}"#.to_owned()),
				] {
					let (mut socket, _) = listener.accept().await.unwrap();
					let mut request = Vec::new();
					loop {
						let mut buffer = [0; 1024];
						let n = socket.read(&mut buffer).await.unwrap();
						assert!(n > 0);
						request.extend_from_slice(&buffer[..n]);
						assert!(request.len() < 4096);
						if request.windows(4).any(|w| w == b"\r\n\r\n") {
							break;
						}
					}
					let text = std::str::from_utf8(&request).unwrap();
					assert!(text.starts_with(&format!("GET {route} HTTP/1.1\r\n")));
					assert!(text.contains("SYNTHETIC_FORUM_TOKEN"));
					socket
						.write_all(
							format!(
								"HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
								body.len()
							)
							.as_bytes(),
						)
						.await
						.unwrap();
				}
			});
			let Event::ForumPosts {
				parent: Id(2),
				request: 4,
				result: Ok(page),
			} = api
				.execute(Command::ForumPosts {
					parent: Id(2),
					guild: Id(1),
					offset: 0,
					request: 4,
				})
				.await
			else {
				panic!()
			};
			assert_eq!(page.threads[0].id, Id(9));
			assert!(page.more);
			assert_eq!(page.previews.len(), 1);
			assert_eq!(page.previews[0].1.images.len(), 1);
			assert_eq!(page.threads[0].tags.as_deref().unwrap().applied, vec![Id(31)]);
			let exhausted = api.forum_posts(Id(2), Id(1), 25).await.unwrap();
			assert!(exhausted.threads.is_empty() && !exhausted.more);
			// A rejected search route falls back to the documented guild-wide active list.
			let fallback = api.forum_posts(Id(2), Id(1), 0).await.unwrap();
			assert_eq!(fallback.threads[0].id, Id(9));
			assert!(!fallback.more);
			let report = fallback.fallback.expect("the fallback is reported");
			assert_eq!(report.reason, model::forum::FallbackReason::Status(404));
			assert_eq!(report.bytes, r#"{"code":0}"#.len());
			let line = report.log_line(Id(2));
			assert!(line.contains("reason=status:404"), "{line}");
			assert!(line.contains(&format!("bytes={}", r#"{"code":0}"#.len())), "{line}");
			assert!(line.contains("elapsed_ms="), "{line}");
			assert!(!line.contains("threads/search"), "no URL in the log line: {line}");
			// A foreign-scoped row is rejected, and so is its fallback.
			assert!(matches!(
				api.forum_posts(Id(2), Id(1), 0).await,
				Err(Failure::Forbidden)
			));
			// Only a rejected response falls back; a denial is reported as one.
			assert!(matches!(
				api.forum_posts(Id(2), Id(1), 0).await,
				Err(Failure::Forbidden)
			));
			assert!(matches!(
				api.forum_posts(Id(0), Id(1), 0).await,
				Err(Failure::Protocol)
			));
			assert!(matches!(
				api.forum_posts(Id(2), Id(1), model::forum::MAX_POSTS + 1).await,
				Err(Failure::Protocol)
			));
			server.await.unwrap();
		})
		.await
		.unwrap();
	}

	/// One local response per accepted connection, in order. `declared` is sent as the
	/// Content-Length even when the body is shorter (the early size rejection path).
	fn serve(
		listener: TcpListener,
		responses: Vec<(String, String, u64, Vec<u8>)>,
	) -> tokio::task::JoinHandle<()> {
		tokio::spawn(async move {
			for (route, status, declared, body) in responses {
				let (mut socket, _) = listener.accept().await.unwrap();
				let mut request = Vec::new();
				loop {
					let mut buffer = [0; 1024];
					let n = socket.read(&mut buffer).await.unwrap();
					assert!(n > 0);
					request.extend_from_slice(&buffer[..n]);
					assert!(request.len() < 4096);
					if request.windows(4).any(|w| w == b"\r\n\r\n") {
						break;
					}
				}
				let text = std::str::from_utf8(&request).unwrap();
				assert!(text.starts_with(&format!("GET {route} HTTP/1.1\r\n")));
				assert!(text.contains("SYNTHETIC_FORUM_TOKEN"));
				socket
					.write_all(
						format!(
							"HTTP/1.1 {status}\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n"
						)
						.as_bytes(),
					)
					.await
					.unwrap();
				socket.write_all(&body).await.unwrap();
			}
		})
	}

	#[tokio::test]
	async fn fallback_reasons_are_reported_for_oversized_and_undecodable_pages() {
		crate::ensure_tls_provider();
		tokio::time::timeout(Duration::from_secs(15), async {
			let row = r#"{"id":"9","guild_id":"1","parent_id":"2","type":11,"name":"Synthetic","thread_metadata":{"archived":false}}"#;
			let active = format!(r#"{{"threads":[{row}],"members":[]}}"#).into_bytes();
			let search = "/channels/2/threads/search?archived=false&sort_by=last_message_time&sort_order=desc&limit=25&offset=0";
			let new_api = |base: String| {
				let mut api = DiscordApi::new(Arc::new(
					SessionSecret::from_owner_input("SYNTHETIC_FORUM_TOKEN".into()).unwrap(),
				))
				.unwrap();
				api.base = base;
				api
			};

			// Decode failure: a 200 that is not a forum page.
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let base = format!("http://{}", listener.local_addr().unwrap());
			let garbage = br#"{"unexpected":true}"#.to_vec();
			let server = serve(
				listener,
				vec![
					(search.to_owned(), "200 OK".into(), garbage.len() as u64, garbage.clone()),
					(
						"/guilds/1/threads/active".into(),
						"200 OK".into(),
						active.len() as u64,
						active.clone(),
					),
				],
			);
			let api = new_api(base);
			let page = api.forum_posts(Id(2), Id(1), 0).await.unwrap();
			let report = page.fallback.expect("decode fallback is reported");
			assert_eq!(report.reason, model::forum::FallbackReason::Decode);
			assert_eq!(report.bytes, garbage.len());
			let line = report.log_line(Id(2));
			assert!(line.contains("reason=decode"), "{line}");
			assert!(line.contains(&format!("bytes={}", garbage.len())), "{line}");
			server.await.unwrap();

			// Oversized: the declared length is over the search wire budget, so the client
			// rejects the page before reading it and falls back.
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let base = format!("http://{}", listener.local_addr().unwrap());
			let declared = discord_protocol::forum::MAX_WIRE as u64 + 4096;
			let server = serve(
				listener,
				vec![
					(search.to_owned(), "200 OK".into(), declared, Vec::new()),
					(
						"/guilds/1/threads/active".into(),
						"200 OK".into(),
						active.len() as u64,
						active.clone(),
					),
				],
			);
			let api = new_api(base);
			let page = api.forum_posts(Id(2), Id(1), 0).await.unwrap();
			let report = page.fallback.expect("oversized fallback is reported");
			assert_eq!(report.reason, model::forum::FallbackReason::Oversized);
			assert_eq!(report.bytes, declared as usize);
			let line = report.log_line(Id(2));
			assert!(line.contains("reason=oversized"), "{line}");
			assert!(line.contains(&format!("bytes={declared}")), "{line}");
			server.await.unwrap();
		})
		.await
		.unwrap();
	}
}
