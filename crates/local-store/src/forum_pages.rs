//! Bounded per-account cache of forum post lists, so a visited forum opens instantly.
//!
//! The snapshot holds only what a card shows: title, author, tags, counters, the excerpt
//! and image metadata. Message bodies are never stored here.
use super::{LocalStore, Result, StoreError};
use model::{Id, forum::CachedPage};
use rusqlite::{OptionalExtension, params};
use std::time::{SystemTime, UNIX_EPOCH};

/// At most this many forums per account; the oldest snapshot is evicted.
pub const MAX_FORUMS: usize = 20;
/// One snapshot's JSON never exceeds this.
pub const MAX_JSON_BYTES: usize = 256 * 1024;
/// A snapshot older than seven days is stale and removed on read.
pub const MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60;

fn now_secs() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_or(0, |elapsed| elapsed.as_secs())
}

impl LocalStore {
	/// The fresh cached page for one forum, or `None` when missing, stale or invalid.
	pub fn forum_page(&self, account: Id, forum: Id) -> Result<Option<CachedPage>> {
		let row: Option<(String, i64)> = self
			.0
			.query_row(
				"SELECT value, fetched_at FROM forum_pages WHERE account=?1 AND forum=?2",
				params![account.to_string(), forum.to_string()],
				|row| Ok((row.get(0)?, row.get(1)?)),
			)
			.optional()?;
		let Some((value, fetched_at)) = row else {
			return Ok(None);
		};
		let account = account.to_string();
		let forum = forum.to_string();
		if fetched_at < now_secs().saturating_sub(MAX_AGE_SECS) as i64 {
			self.0.execute(
				"DELETE FROM forum_pages WHERE account=?1 AND forum=?2",
				params![account, forum],
			)?;
			return Ok(None);
		}
		if value.len() > MAX_JSON_BYTES {
			return Err(StoreError::Incompatible);
		}
		let page: CachedPage =
			serde_json::from_str(&value).map_err(|_| StoreError::Incompatible)?;
		if !page.valid() {
			return Err(StoreError::Incompatible);
		}
		Ok(Some(page))
	}

	/// Store one forum's card snapshot, evicting the oldest forums past the cap.
	pub fn save_forum_page(&self, account: Id, forum: Id, page: &CachedPage) -> Result<()> {
		if forum.0 == 0 || !page.valid() {
			return Err(StoreError::Capacity);
		}
		let value = serde_json::to_string(page).map_err(|_| StoreError::Incompatible)?;
		if value.len() > MAX_JSON_BYTES {
			return Err(StoreError::Capacity);
		}
		let account = account.to_string();
		let forum = forum.to_string();
		let now = now_secs() as i64;
		let transaction = self.0.unchecked_transaction()?;
		transaction.execute(
			"DELETE FROM forum_pages WHERE account=?1 AND fetched_at < ?2",
			params![account, now - MAX_AGE_SECS as i64],
		)?;
		transaction.execute(
			"INSERT INTO forum_pages(account,forum,value,fetched_at) VALUES(?1,?2,?3,?4)
			 ON CONFLICT(account,forum) DO UPDATE SET value=excluded.value, fetched_at=excluded.fetched_at",
			params![account, forum, value, now],
		)?;
		transaction.execute(
			"DELETE FROM forum_pages WHERE account=?1 AND forum NOT IN (
				SELECT forum FROM forum_pages WHERE account=?1 ORDER BY fetched_at DESC LIMIT ?2
			 )",
			params![account, MAX_FORUMS as i64],
		)?;
		transaction.commit()?;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::forum::{CachedImage, CachedPost, CachedStarter};
	use model::{EmbedMedia, Reaction, ReactionEmoji};

	fn post(id: u64) -> CachedPost {
		CachedPost {
			id: Id(id),
			name: format!("Synthetic post {id}"),
			last_message: Some(Id(id + 1_000)),
			message_count: Some(3),
			applied: vec![Id(31)],
			pinned: id.is_multiple_of(2),
			starter: Some(CachedStarter {
				author_id: Id(7),
				author: "Synthetic".into(),
				roles: vec![],
				webhook: false,
				excerpt: "Synthetic excerpt".into(),
				images: vec![CachedImage {
					media: EmbedMedia {
						url: Some(format!("https://cdn.example/{id}.png")),
						proxy_url: None,
						width: 800,
						height: 600,
						placeholder: Vec::new(),
					},
					id: Some(Id(id + 2_000)),
					size: 4_096,
					spoiler: false,
					animated: false,
					video: false,
				}],
				image_count: 1,
				reactions: vec![Reaction {
					emoji: ReactionEmoji {
						id: None,
						name: Some("🔥".into()),
					},
					count: 3,
					me: false,
					me_burst: false,
				}],
			}),
		}
	}

	fn page(posts: usize) -> CachedPage {
		CachedPage {
			posts: (0..posts).map(|index| post(1_000 + index as u64)).collect(),
		}
	}

	fn store() -> LocalStore {
		LocalStore::initialize(rusqlite::Connection::open_in_memory().unwrap()).unwrap()
	}

	#[test]
	fn a_forum_page_round_trips_and_stays_inside_its_account() {
		let store = store();
		let saved = page(3);
		store.save_forum_page(Id(1), Id(20), &saved).unwrap();
		assert_eq!(store.forum_page(Id(1), Id(20)).unwrap(), Some(saved));
		// Another account never sees it, and neither does another forum.
		assert_eq!(store.forum_page(Id(2), Id(20)).unwrap(), None);
		assert_eq!(store.forum_page(Id(1), Id(21)).unwrap(), None);
	}

	#[test]
	fn a_snapshot_expires_after_seven_days_and_is_removed_on_read() {
		let store = store();
		store.save_forum_page(Id(1), Id(20), &page(2)).unwrap();
		let stale = now_secs().saturating_sub(MAX_AGE_SECS + 60) as i64;
		store
			.0
			.execute(
				"UPDATE forum_pages SET fetched_at=?1 WHERE account=?2 AND forum=?3",
				params![stale, Id(1).to_string(), Id(20).to_string()],
			)
			.unwrap();
		assert_eq!(store.forum_page(Id(1), Id(20)).unwrap(), None);
		let remaining: i64 = store
			.0
			.query_row("SELECT count(*) FROM forum_pages", [], |row| row.get(0))
			.unwrap();
		assert_eq!(remaining, 0, "an expired snapshot is deleted on read");
	}

	#[test]
	fn the_forum_cap_evicts_the_oldest_snapshot() {
		let store = store();
		for index in 0..(MAX_FORUMS + 5) {
			let forum = Id(100 + index as u64);
			store.save_forum_page(Id(1), forum, &page(1)).unwrap();
			// Distinct recent save times so eviction has an order without expiring.
			store
				.0
				.execute(
					"UPDATE forum_pages SET fetched_at=?1 WHERE account=?2 AND forum=?3",
					params![
						now_secs() as i64 - (MAX_FORUMS as i64 + 5 - index as i64),
						Id(1).to_string(),
						forum.to_string()
					],
				)
				.unwrap();
		}
		let count: i64 = store
			.0
			.query_row("SELECT count(*) FROM forum_pages", [], |row| row.get(0))
			.unwrap();
		assert_eq!(count, MAX_FORUMS as i64);
		assert_eq!(store.forum_page(Id(1), Id(100)).unwrap(), None);
		assert!(
			store
				.forum_page(Id(1), Id(100 + MAX_FORUMS as u64 + 4))
				.unwrap()
				.is_some(),
			"the newest snapshots survive"
		);
	}

	#[test]
	fn oversized_snapshots_are_refused_instead_of_written() {
		let store = store();
		// More posts than the card cache allows.
		let mut crowded = page(model::forum::MAX_POSTS + 1);
		assert!(!crowded.valid());
		assert_eq!(
			store.save_forum_page(Id(1), Id(20), &crowded),
			Err(StoreError::Capacity)
		);
		// A valid post count whose JSON still exceeds the byte cap.
		crowded.posts.truncate(model::forum::MAX_POSTS);
		for post in &mut crowded.posts {
			post.starter.as_mut().unwrap().excerpt = "x".repeat(model::forum::MAX_PREVIEW_EXCERPT);
		}
		let json = serde_json::to_string(&crowded).unwrap();
		if json.len() > MAX_JSON_BYTES {
			assert_eq!(
				store.save_forum_page(Id(1), Id(20), &crowded),
				Err(StoreError::Capacity)
			);
		}
		let remaining: i64 = store
			.0
			.query_row("SELECT count(*) FROM forum_pages", [], |row| row.get(0))
			.unwrap();
		assert_eq!(remaining, 0);
	}

	#[test]
	fn a_two_hundred_post_snapshot_loads_well_under_a_hundred_milliseconds() {
		let store = store();
		let saved = page(model::forum::MAX_POSTS);
		let started = std::time::Instant::now();
		store.save_forum_page(Id(1), Id(20), &saved).unwrap();
		let write = started.elapsed();
		let started = std::time::Instant::now();
		let loaded = store.forum_page(Id(1), Id(20)).unwrap().unwrap();
		let read = started.elapsed();
		assert_eq!(loaded.posts.len(), model::forum::MAX_POSTS);
		println!("200-post forum cache: write {write:?}, read {read:?} (debug, in-memory SQLite)");
		assert!(
			read < std::time::Duration::from_millis(100),
			"cache read took {read:?}"
		);
	}
}
