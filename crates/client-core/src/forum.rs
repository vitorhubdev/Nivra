//! Forum containers list their posts (threads) and create one post at a time.
use crate::{Command, MAX_CONTENT, MAX_NAV, State, auth::AuthState, auth::Failure};
use model::{Channel, Id, permissions as p};

pub const MAX_TITLE: usize = 100;

/// The on-demand active-post list of one forum; the gateway only delivers joined posts.
#[derive(Default)]
pub struct Posts {
	// At most 200 starter previews, bounded like the posts they sit beside.
	previews: std::collections::BTreeMap<Id, model::forum::Starter>,
	/// The last guild-wide fallback for the open forum, shown as a slow-list notice.
	pub fallback: Option<model::forum::FallbackReport>,
	/// The same report, held until the app logs it once.
	pending_fallback_log: Option<(Id, model::forum::FallbackReport)>,
	pub parent: Option<Id>,
	pub request: u64,
	pub loading: bool,
	/// Posts admitted so far, used as the search offset of the next page.
	pub loaded: usize,
	pub more: bool,
	pub error: Option<&'static str>,
}

impl Posts {
	pub(crate) fn clear_previews(&mut self) {
		self.previews.clear();
	}

	/// Remember what a post card shows of its starter; fixtures seed theirs the same way.
	pub fn remember_preview(&mut self, post: Id, starter: model::forum::Starter) {
		if post.0 > 0
			&& starter.valid()
			&& (self.previews.contains_key(&post) || self.previews.len() < model::forum::MAX_POSTS)
		{
			self.previews.insert(post, starter);
		}
	}
}

#[derive(Default)]
pub struct Posting {
	pub request: u64,
	pub pending: Option<(Id, u64)>,
	pub error: Option<&'static str>,
	/// A freshly created post the UI should open on its next frame.
	pub created: Option<Id>,
}

impl State {
	pub(crate) fn prune_post_previews(&mut self) {
		let mut previews = std::mem::take(&mut self.posts.previews);
		previews.retain(|post, _| self.can_view(*post));
		self.posts.previews = previews;
	}

	/// The starter message's card preview: images, author and reactions.
	/// The last fallback report, consumed once so the app logs it exactly once.
	pub fn take_fallback_log(&mut self) -> Option<(Id, model::forum::FallbackReport)> {
		self.posts.pending_fallback_log.take()
	}

	pub fn post_preview(&self, post: Id) -> Option<&model::forum::Starter> {
		self.can_view(post)
			.then(|| self.posts.previews.get(&post))
			.flatten()
	}

	/// The reaction a post card shows: the forum's default one when used, else the most used.
	pub fn post_reaction(&self, post: &Channel) -> Option<&model::Reaction> {
		let reactions = &self.posts.previews.get(&post.id)?.reactions;
		let default = post
			.parent_id
			.and_then(|forum| self.channel(forum))
			.and_then(|forum| forum.tags.as_deref())
			.and_then(|tags| tags.reaction.as_ref());
		self.can_view(post.id)
			.then(|| {
				default
					.and_then(|emoji| reactions.iter().find(|r| r.emoji.same(emoji)))
					.or_else(|| reactions.first())
			})
			.flatten()
	}

	/// The forum's post defaults, for the list's initial sort, layout and tag matching.
	pub fn forum_defaults(&self, forum: Id) -> Option<&model::forum::Tags> {
		self.channel(forum)
			.filter(|channel| matches!(channel.kind, 15 | 16))
			.and_then(|channel| channel.tags.as_deref())
	}

	/// Tags a forum offers, in the order its moderators arranged them.
	pub fn forum_tags(&self, forum: Id) -> &[model::forum::Tag] {
		self.channel(forum)
			.filter(|channel| matches!(channel.kind, 15 | 16))
			.and_then(|channel| channel.tags.as_deref())
			.map_or(&[], |tags| &tags.available)
	}

	pub fn forum_requires_tag(&self, forum: Id) -> bool {
		self.channel(forum)
			.and_then(|channel| channel.tags.as_deref())
			.is_some_and(|tags| tags.required)
	}

	/// Tags applied to a post that its forum still offers, in the forum's order.
	pub fn post_tags(&self, post: &Channel) -> Vec<&model::forum::Tag> {
		let Some(applied) = post.tags.as_deref().map(|tags| &tags.applied) else {
			return Vec::new();
		};
		post.parent_id
			.map(|forum| self.forum_tags(forum))
			.unwrap_or_default()
			.iter()
			.filter(|tag| applied.contains(&tag.id))
			.collect()
	}

	/// Moderated tags need thread management; everyone else may apply the rest.
	pub fn can_apply_tag(&self, forum: Id, tag: &model::forum::Tag) -> bool {
		!tag.moderated || self.permission(forum, p::MANAGE_THREADS) == Some(true)
	}

	pub fn is_forum(&self, channel: Id) -> bool {
		self.channels
			.iter()
			.any(|c| c.id == channel && c.guild.is_some() && matches!(c.kind, 15 | 16))
	}

	/// Loaded posts of a forum: active threads the gateway delivered, newest activity first.
	pub fn forum_posts(&self, parent: Id) -> Vec<&Channel> {
		let mut posts: Vec<_> = self
			.channels
			.iter()
			.filter(|c| c.parent_id == Some(parent) && matches!(c.kind, 11 | 12))
			.collect();
		posts.sort_by_key(|c| std::cmp::Reverse(c.last_message.unwrap_or(c.id)));
		posts
	}

	/// Is one post unread? Posts arrive through the on-demand list as well as the gateway, so
	/// a post the service never sent a read row for counts as unread once the snapshot is
	/// complete and the service reports a message in it.
	pub fn post_unread(&self, post: &Channel) -> bool {
		if let Some(unread) = self.channel_unread(post) {
			return unread || self.unread_count(post.id) > 0;
		}
		self.gateway_connected
			&& self.read_state.known()
			&& self.can_view(post.id)
			&& post.last_message.is_some()
	}

	/// Forum containers carry no messages themselves; Discord marks them unread when one of
	/// their posts is, so the sidebar row needs the same aggregate.
	pub fn forum_unread(&self, forum: Id) -> bool {
		self.is_forum(forum)
			&& self
				.channels
				.iter()
				.any(|post| self.is_post_of(post, forum) && self.post_unread(post))
	}

	/// Loaded posts whose starter has not been read; replies do not make a post new again.
	pub fn forum_new_count(&self, forum: Id) -> u32 {
		if !self.is_forum(forum) {
			return 0;
		}
		self.channels
			.iter()
			.filter(|post| {
				self.is_post_of(post, forum)
					&& self.post_unread(post)
					&& self
						.read_marker(post.id)
						.is_some_and(|read| read.is_none_or(|id| id < post.id))
			})
			.count() as u32
	}

	fn is_post_of(&self, post: &Channel, forum: Id) -> bool {
		post.parent_id == Some(forum) && matches!(post.kind, 11 | 12)
	}

	pub fn can_load_posts(&self, parent: Id) -> bool {
		// Fixtures ship their own posts; a demo session has nothing to fetch them with.
		!self.demo
			&& self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.is_forum(parent)
			&& self.can_read_history(parent)
	}

	/// Loads the first page of a forum, or the next one when `more` is set.
	pub fn request_forum_posts(&mut self, parent: Id, more: bool) -> Option<Command> {
		if !self.can_load_posts(parent) {
			return None;
		}
		let current = self.posts.parent == Some(parent);
		if self.posts.loading && current {
			return None;
		}
		if more {
			if !current || !self.posts.more || self.posts.loaded >= model::forum::MAX_POSTS {
				return None;
			}
		} else if current && (self.posts.loaded > 0 || self.posts.error.is_some()) {
			return None;
		}
		let guild = self.channel(parent)?.guild?;
		let offset = if more { self.posts.loaded } else { 0 };
		self.posts.request = self.posts.request.wrapping_add(1);
		self.posts.parent = Some(parent);
		self.posts.loading = true;
		self.posts.error = None;
		if !more {
			self.posts.loaded = 0;
			self.posts.more = false;
		}
		Some(Command::ForumPosts {
			parent,
			guild,
			offset,
			request: self.posts.request,
		})
	}

	/// Re-arms the loader so the next frame of this forum fetches its posts again.
	pub fn reload_forum_posts(&mut self, parent: Id) {
		if self.posts.parent == Some(parent) && !self.posts.loading {
			// Keep the request counter monotonic so a late reply cannot match a fresh load.
			self.posts = Posts {
				request: self.posts.request,
				..Posts::default()
			};
		}
	}

	pub fn apply_forum_posts(
		&mut self,
		parent: Id,
		request: u64,
		result: Result<model::forum::Page, Failure>,
	) {
		if let Err(failure) = &result
			&& failure.ends_session()
			&& *failure != Failure::Capacity
		{
			self.fail(*failure);
			return;
		}
		if self.posts.parent != Some(parent) || self.posts.request != request || !self.posts.loading
		{
			return;
		}
		self.posts.loading = false;
		let guild = self
			.channel(parent)
			.and_then(|c| c.guild)
			.filter(|_| self.can_load_posts(parent));
		let page = match result {
			Err(failure) => {
				self.posts.error = Some(failure.label());
				return;
			}
			Ok(page) => page,
		};
		let Some(guild) = guild.filter(|guild| page.valid(parent, *guild)) else {
			self.posts.error = Some("The service returned unexpected posts");
			return;
		};
		// A guild-wide fallback is shown as a slow-list notice and logged once by the app;
		// a later successful search page clears the notice.
		self.posts.fallback = page.fallback;
		if let Some(report) = page.fallback {
			self.posts.pending_fallback_log = Some((parent, report));
		}
		for (post, starter) in page.previews {
			self.posts.remember_preview(post, starter);
		}
		// Every returned row advances the offset, even one this state already knew.
		let returned = page.threads.len();
		for post in page.threads {
			if let Some(index) = self.channel_index(post.id) {
				let existing = &self.channels[index];
				if existing.parent_id == Some(parent) && existing.guild == Some(guild) {
					let tag_bytes =
						|channel: &Channel| channel.tags.as_ref().map_or(0, |tags| tags.bytes());
					let bytes =
						self.navigation_bytes() - existing.name.capacity() - tag_bytes(existing)
							+ post.name.capacity()
							+ tag_bytes(&post);
					if bytes + self.permissions.bytes() > model::account::MAX_BYTES {
						self.posts.error = Some("Posts exceed the navigation budget");
						self.posts.more = false;
						return;
					}
					let existing = &mut self.channels[index];
					existing.last_message = existing.last_message.max(post.last_message);
					existing.message_count = post.message_count.or(existing.message_count);
					existing.name = post.name;
					existing.tags = post.tags;
					self.set_navigation_bytes(bytes);
				}
				continue;
			}
			let bytes = self.navigation_bytes();
			if self.channels.len() + self.guilds.len() >= MAX_NAV
				|| bytes + post.bytes() + self.permissions.bytes() > model::account::MAX_BYTES
				|| self.channels.try_reserve_exact(1).is_err()
			{
				self.posts.error = Some("Posts exceed the navigation budget");
				self.posts.more = false;
				return;
			}
			self.channels.push(post);
			self.invalidate_navigation();
		}
		self.posts.loaded = self.posts.loaded.saturating_add(returned);
		self.posts.more = page.more && returned > 0;
	}

	pub fn can_create_post(&self, parent: Id) -> bool {
		self.posting.pending.is_none() && self.can_post(parent)
	}

	fn can_post(&self, parent: Id) -> bool {
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.is_forum(parent)
			&& self.can_view(parent)
			&& self.permission(parent, p::SEND_MESSAGES) == Some(true)
	}

	/// A forum container is not a text channel, so `can_attach` never covers it; the starter
	/// message still needs the container's own attachment permission, including while pending.
	pub fn can_attach_post(&self, parent: Id) -> bool {
		self.can_post(parent) && self.permission(parent, p::ATTACH_FILES) == Some(true)
	}

	pub fn create_post(&mut self, parent: Id, title: &str, content: &str) -> Option<Command> {
		self.create_post_with_attachments(parent, title, content, &[], &[])
	}

	/// Create one post, optionally with files staged for its starter message and tags applied
	/// to the new thread.
	pub fn create_post_with_attachments(
		&mut self,
		parent: Id,
		title: &str,
		content: &str,
		filenames: &[&str],
		tags: &[Id],
	) -> Option<Command> {
		let title = title.trim();
		let content = content.trim();
		if !self.can_create_post(parent)
			|| title.is_empty()
			|| title.chars().count() > MAX_TITLE
			|| (content.is_empty() && filenames.is_empty())
			|| content.chars().count() > MAX_CONTENT
		{
			return None;
		}
		if !filenames.is_empty() && !self.can_attach_post(parent) {
			return None;
		}
		if filenames.len() > crate::MAX_ATTACHMENTS
			|| filenames.iter().any(|name| {
				name.trim().is_empty()
					|| name.len() > 256
					|| matches!(*name, "." | "..")
					|| name
						.chars()
						.any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
			}) {
			self.posting.error = Some("Attachment filename is invalid or too long");
			return None;
		}
		let offered = self.forum_tags(parent);
		if tags.len() > model::forum::MAX_APPLIED_TAGS
			|| tags.iter().enumerate().any(|(i, id)| {
				tags[..i].contains(id)
					|| !offered
						.iter()
						.any(|tag| tag.id == *id && self.can_apply_tag(parent, tag))
			}) {
			self.posting.error = Some("Choose up to 5 tags this forum offers");
			return None;
		}
		if tags.is_empty() && self.forum_requires_tag(parent) {
			self.posting.error = Some("This forum requires at least one tag");
			return None;
		}
		let guild = self.channel(parent)?.guild?;
		self.posting.request = self.posting.request.wrapping_add(1);
		self.posting.pending = Some((parent, self.posting.request));
		self.posting.error = None;
		Some(Command::CreatePost {
			parent,
			guild,
			title: title.to_owned(),
			content: content.to_owned(),
			attachments: filenames.iter().map(|name| (*name).to_owned()).collect(),
			tags: tags.to_vec(),
			request: self.posting.request,
		})
	}

	pub fn apply_post(&mut self, parent: Id, request: u64, result: Result<Channel, Failure>) {
		if self.posting.pending != Some((parent, request)) {
			return;
		}
		self.posting.pending = None;
		let guild = self
			.channels
			.iter()
			.find(|c| c.id == parent)
			.and_then(|c| c.guild);
		let post = match result {
			Err(failure) if failure.ends_session() && failure != Failure::Capacity => {
				self.fail(failure);
				return;
			}
			Err(failure) => {
				self.posting.error = Some(failure.label());
				return;
			}
			Ok(post) => post,
		};
		if guild.is_none()
			|| post.id.0 == 0
			|| post.kind != 11
			|| post.parent_id != Some(parent)
			|| post.guild != guild
			|| post.name.len() > 512
		{
			self.posting.error = Some("The service returned an unexpected post");
			return;
		}
		if let Some(existing) = self.channels.iter_mut().find(|c| c.id == post.id) {
			if existing.parent_id != post.parent_id || existing.guild != post.guild {
				self.posting.error = Some("Post conflicts with current navigation");
				return;
			}
			existing.message_count = post.message_count.or(existing.message_count);
			existing.tags = post.tags.clone();
		} else {
			let bytes = self.navigation_bytes();
			if self.channels.len() + self.guilds.len() >= MAX_NAV
				|| bytes + post.bytes() + self.permissions.bytes() > model::account::MAX_BYTES
				|| self.channels.try_reserve_exact(1).is_err()
			{
				self.posting.error = Some("Post exceeds the navigation budget");
				return;
			}
			self.channels.push(post.clone());
			self.invalidate_navigation();
		}
		self.posting.created = Some(post.id);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event};

	fn channel(id: u64, parent: Option<Id>, kind: u8) -> Channel {
		Channel {
			id: Id(id),
			guild: Some(Id(1)),
			parent_id: parent,
			kind,
			name: "Synthetic".into(),
			position: 0,
			recipients: vec![],
			last_message: None,
			icon: None,
			member_list_id: None,
			message_count: Some(3),
			tags: None,
		}
	}
	fn state() -> State {
		let mut state = State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			user: Some(model::User {
				primary_guild: None,
				id: Id(2),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			}),
			guilds: vec![model::Guild {
				stickers: None,
				emojis: None,
				id: Id(1),
				name: "Synthetic".into(),
				icon: None,
				premium_tier: 0,
			}],
			channels: vec![
				channel(10, None, 0),
				channel(20, None, 15),
				channel(21, Some(Id(20)), 11),
				channel(22, Some(Id(20)), 11),
			],
			..State::default()
		};
		state.channels[3].last_message = Some(Id(500));
		crate::tests::grant_permissions(&mut state);
		state
	}

	#[test]
	fn forum_pages_carry_starter_previews_and_applied_tags() {
		let mut state = state();
		state.channels[1].tags = Some(Box::new(model::forum::Tags {
			available: vec![
				model::forum::Tag {
					id: Id(31),
					name: "Help".into(),
					moderated: false,
					emoji_id: None,
					emoji_name: Some("🔥".into()),
				},
				model::forum::Tag {
					id: Id(32),
					name: "Moderated".into(),
					moderated: true,
					emoji_id: None,
					emoji_name: None,
				},
			],
			required: true,
			reaction: Some(model::ReactionEmoji {
				id: None,
				name: Some("🔥".into()),
			}),
			layout: model::forum::Layout::Gallery,
			..Default::default()
		}));
		let image = |url: &str| model::forum::StarterImage {
			media: model::EmbedMedia {
				url: Some(url.into()),
				proxy_url: None,
				width: 8,
				height: 8,
				placeholder: Vec::new(),
			},
			id: model::Id(700_000),
			size: 4096,
			spoiler: false,
			animated: false,
			video: false,
		};
		let starter = |image_count: u16| model::forum::Starter {
			author_id: Id(7),
			author: "Synthetic".into(),
			roles: vec![],
			webhook: false,
			excerpt: "Starter excerpt".into(),
			images: (0..image_count.min(4))
				.map(|index| image(&format!("https://cdn.example/{index}.png")))
				.collect(),
			image_count,
			reactions: vec![model::Reaction {
				emoji: model::ReactionEmoji {
					id: None,
					name: Some("🔥".into()),
				},
				count: 4,
				me: false,
				me_burst: false,
			}],
		};
		state.request_forum_posts(Id(20), false).unwrap();
		let mut first = channel(21, Some(Id(20)), 11);
		first.tags = Some(Box::new(model::forum::Tags {
			applied: vec![Id(31)],
			..Default::default()
		}));
		let page = model::forum::Page {
			threads: vec![first, channel(22, Some(Id(20)), 11)],
			more: false,
			previews: vec![(Id(21), starter(5)), (Id(22), starter(0))],
			fallback: None,
		};
		state.apply_forum_posts(Id(20), state.posts.request, Ok(page));
		let post = state.channel(Id(21)).unwrap();
		assert_eq!(
			state.forum_defaults(Id(20)).unwrap().layout,
			model::forum::Layout::Gallery
		);
		assert_eq!(state.forum_tags(Id(20)).len(), 2);
		assert!(state.forum_requires_tag(Id(20)));
		assert_eq!(
			state
				.post_tags(post)
				.iter()
				.map(|tag| tag.id)
				.collect::<Vec<_>>(),
			vec![Id(31)]
		);
		assert!(state.can_apply_tag(Id(20), &state.forum_tags(Id(20))[0]));
		// A moderated tag needs thread management; the granted owner has it, a bare state not.
		assert!(state.can_apply_tag(Id(20), &state.forum_tags(Id(20))[1]));
		let moderated = state.forum_tags(Id(20))[1].clone();
		let outsider = State {
			channels: vec![channel(20, None, 15)],
			..State::default()
		};
		assert!(!outsider.can_apply_tag(Id(20), &moderated));
		// A card folds the starter into four images and a total count for its "+N" badge.
		let preview = state.post_preview(Id(21)).unwrap();
		assert_eq!(preview.images.len(), 4);
		assert_eq!(preview.image_count, 5);
		assert_eq!(state.post_preview(Id(22)).unwrap().images.len(), 0);
		// The forum's default reaction wins over the most used one when it is present.
		assert_eq!(
			state.post_reaction(post).unwrap().emoji.name.as_deref(),
			Some("🔥")
		);
		assert!(state.post_preview(Id(999)).is_none());
	}

	#[test]
	fn a_guild_wide_fallback_is_kept_for_the_notice_and_logged_once() {
		let mut state = state();
		state.request_forum_posts(Id(20), false).unwrap();
		let report = model::forum::FallbackReport {
			reason: model::forum::FallbackReason::Status(404),
			bytes: 12,
			elapsed: std::time::Duration::from_millis(4_200),
		};
		state.apply_forum_posts(
			Id(20),
			state.posts.request,
			Ok(model::forum::Page {
				threads: vec![channel(21, Some(Id(20)), 11)],
				more: false,
				previews: Vec::new(),
				fallback: Some(report),
			}),
		);
		// The notice copy survives for the open forum, and the log entry is consumed once.
		assert_eq!(state.posts.fallback, Some(report));
		assert!(state.posts.fallback.is_some_and(|report| report.slow()));
		let (forum, logged) = state.take_fallback_log().expect("one log entry");
		assert_eq!((forum, logged), (Id(20), report));
		assert!(state.take_fallback_log().is_none(), "logged only once");
		assert_eq!(state.posts.fallback, Some(report));
		// The line carries the hashed forum, reason, bytes and time, and no raw snowflake.
		let line = report.log_line(Id(20));
		assert!(line.contains("reason=status:404"), "{line}");
		assert!(line.contains("bytes=12"), "{line}");
		assert!(line.contains("elapsed_ms=4200"), "{line}");
		let hash = line
			.split("forum=")
			.nth(1)
			.and_then(|rest| rest.split_whitespace().next())
			.expect("hashed forum");
		assert_eq!(hash.len(), 8, "{line}");
		assert!(hash.chars().all(|c| c.is_ascii_hexdigit()), "{line}");
		assert_ne!(report.log_line(Id(20)), report.log_line(Id(21)));
		// A fast fallback keeps the report but does not deserve the notice.
		let fast = model::forum::FallbackReport {
			elapsed: std::time::Duration::from_millis(120),
			..report
		};
		assert!(!fast.slow());
	}

	#[test]
	fn post_tags_are_validated_against_the_forum() {
		let mut state = state();
		state.channels[1].tags = Some(Box::new(model::forum::Tags {
			available: vec![model::forum::Tag {
				id: Id(31),
				name: "Help".into(),
				moderated: false,
				emoji_id: None,
				emoji_name: Some("🔥".into()),
			}],
			required: true,
			..Default::default()
		}));
		assert!(
			state
				.create_post_with_attachments(Id(20), "Title", "Body", &[], &[Id(99)])
				.is_none(),
			"a tag the forum does not offer is refused"
		);
		assert!(state.posting.error.is_some());
		state.posting.error = None;
		assert!(
			state
				.create_post_with_attachments(Id(20), "Title", "Body", &[], &[])
				.is_none(),
			"a forum that requires tags refuses a bare post"
		);
		state.posting.error = None;
		let Some(Command::CreatePost { tags, .. }) =
			state.create_post_with_attachments(Id(20), "Title", "Body", &[], &[Id(31)])
		else {
			panic!("tagged post command expected")
		};
		assert_eq!(tags, vec![Id(31)]);
	}

	#[test]
	fn forum_title_replacement_updates_budget_and_preserves_original_when_full() {
		let mut state = state();
		let before = state.navigation_bytes();
		let previous_capacity = state.channel(Id(21)).unwrap().name.capacity();
		state.request_forum_posts(Id(20), false).unwrap();
		let mut post = channel(21, Some(Id(20)), 11);
		post.name = "Longer synthetic title".into();
		let expected = before - previous_capacity + post.name.capacity();
		state.apply_forum_posts(
			Id(20),
			state.posts.request,
			Ok(model::forum::Page {
				threads: vec![post],
				more: false,
				previews: Vec::new(),
				fallback: None,
			}),
		);
		assert_eq!(state.navigation_bytes(), expected);
		state.guilds[0].name = String::with_capacity(
			model::account::MAX_BYTES - expected - state.permissions.bytes()
				+ state.guilds[0].name.capacity()
				- 32,
		);
		state.invalidate_navigation();
		let original = state.channel(Id(21)).unwrap().clone();
		state.reload_forum_posts(Id(20));
		state.request_forum_posts(Id(20), false).unwrap();
		let mut post = original.clone();
		post.name = "x".repeat(128);
		post.last_message = Some(Id(900));
		state.apply_forum_posts(
			Id(20),
			state.posts.request,
			Ok(model::forum::Page {
				threads: vec![post],
				more: true,
				previews: Vec::new(),
				fallback: None,
			}),
		);
		assert!(state.channel(Id(21)) == Some(&original));
		assert_eq!(
			state.posts.error,
			Some("Posts exceed the navigation budget")
		);
		assert!(!state.posts.more && !state.posts.loading);
		assert!(state.navigation_bytes() + state.permissions.bytes() <= model::account::MAX_BYTES);
	}

	#[test]
	fn forum_posts_load_on_demand_page_forward_and_reload_after_a_sync() {
		let mut state = state();
		// Only joined posts arrive over the gateway, so the list fetches the rest.
		let Some(Command::ForumPosts {
			parent: Id(20),
			guild: Id(1),
			offset: 0,
			request,
		}) = state.request_forum_posts(Id(20), false)
		else {
			panic!("the first page should be requested");
		};
		assert!(state.request_forum_posts(Id(20), false).is_none());
		assert!(state.request_forum_posts(Id(20), true).is_none());
		let page = |ids: &[u64], more| model::forum::Page {
			threads: ids
				.iter()
				.map(|id| channel(*id, Some(Id(20)), 11))
				.collect(),
			more,
			previews: Vec::new(),
			fallback: None,
		};
		// A stale reply for another request is ignored.
		state.apply_forum_posts(Id(20), request.wrapping_sub(1), Ok(page(&[30], false)));
		assert!(state.channels.iter().all(|c| c.id != Id(30)));
		state.apply_forum_posts(Id(20), request, Ok(page(&[23, 21], true)));
		let posts: Vec<_> = state.forum_posts(Id(20)).iter().map(|c| c.id).collect();
		assert_eq!(posts, vec![Id(22), Id(23), Id(21)]);
		assert_eq!(state.posts.loaded, 2);
		let Some(Command::ForumPosts { offset: 2, .. }) = state.request_forum_posts(Id(20), true)
		else {
			panic!("the next page continues from the loaded count");
		};
		state.apply_forum_posts(Id(20), state.posts.request, Ok(page(&[], false)));
		assert!(!state.posts.more && state.posts.error.is_none());
		assert!(state.request_forum_posts(Id(20), true).is_none());
		// A page scoped to another parent or guild never reaches navigation.
		state.reload_forum_posts(Id(20));
		let request = match state.request_forum_posts(Id(20), false) {
			Some(Command::ForumPosts { request, .. }) => request,
			_ => panic!("a reloaded forum fetches again"),
		};
		let mut foreign = page(&[31], false);
		foreign.threads[0].guild = Some(Id(7));
		state.apply_forum_posts(Id(20), request, Ok(foreign));
		assert_eq!(
			state.posts.error,
			Some("The service returned unexpected posts")
		);
		assert!(state.channels.iter().all(|c| c.id != Id(31)));
		// A failure surfaces once and only a retry clears it.
		state.reload_forum_posts(Id(20));
		let request = match state.request_forum_posts(Id(20), false) {
			Some(Command::ForumPosts { request, .. }) => request,
			_ => panic!("a reloaded forum fetches again"),
		};
		state.apply_forum_posts(Id(20), request, Err(Failure::Capacity));
		assert!(state.posts.error.is_some() && !state.posts.loading);
		assert!(state.request_forum_posts(Id(20), false).is_none());
		// A thread snapshot replaces this scope, so the fetched page must be taken again.
		state
			.apply_threads_sync(Id(1), Some(vec![Id(20)]), vec![], vec![])
			.unwrap();
		assert!(state.posts.error.is_none() && state.posts.loaded == 0);
		assert!(matches!(
			state.request_forum_posts(Id(20), false),
			Some(Command::ForumPosts { offset: 0, .. })
		));
		// A disconnected session asks for nothing.
		state.gateway_connected = false;
		state.posts = Posts::default();
		assert!(!state.can_load_posts(Id(20)));
		assert!(state.request_forum_posts(Id(20), false).is_none());
	}

	#[test]
	fn forum_selection_lists_posts_without_history_and_counts_replies() {
		let mut state = state();
		assert!(state.select(Id(20)).is_none());
		assert_eq!(state.selected, Some(Id(20)));
		assert!(!state.history_pending);
		let posts: Vec<_> = state.forum_posts(Id(20)).iter().map(|c| c.id).collect();
		assert_eq!(posts, vec![Id(22), Id(21)]);
		let message = model::Message {
			poll: None,
			sticker_items: Vec::new(),
			reactions: Some(vec![]),
			id: Id(600),
			channel: Id(21),
			author: state.user.clone().unwrap(),
			content: "Synthetic reply".into(),
			edited: false,
			edited_at: None,
			revision: 0,
			nonce: None,
			reply_to: None,
			kind: 0,
			reply_deleted: false,
			interaction: None,
			forwarded: false,
			unsupported: false,
			components: vec![],
			application_id: None,
			flags: 0,
			ephemeral: false,
			extra_content: Default::default(),
			embeds: vec![],
			attachments: vec![],
			author_nick: None,
			author_roles: vec![],
			mention_roles: vec![],
			mention_everyone: false,
			suppress_notifications: false,
			mentions: Vec::new(),
			embeds_suppressed: false,
		};
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Message(message),
		});
		let post = state.channels.iter().find(|c| c.id == Id(21)).unwrap();
		assert_eq!(post.message_count, Some(4));
		assert_eq!(post.last_message, Some(Id(600)));
		assert_eq!(state.forum_posts(Id(20))[0].id, Id(21));
	}

	#[test]
	fn posts_are_created_once_validated_and_opened_by_the_ui() {
		let mut state = state();
		assert!(state.create_post(Id(10), "Title", "Body").is_none());
		assert!(state.create_post(Id(20), "", "Body").is_none());
		assert!(state.create_post(Id(20), "Title", " ").is_none());
		let Some(Command::CreatePost { request, .. }) =
			state.create_post(Id(20), " Title ", "Body")
		else {
			panic!("forum post command expected");
		};
		assert!(!state.can_create_post(Id(20)), "One post at a time");
		state.apply_post(
			Id(20),
			request.wrapping_add(1),
			Ok(channel(30, Some(Id(20)), 11)),
		);
		assert!(state.posting.pending.is_some());
		state.apply_post(Id(20), request, Ok(channel(30, Some(Id(10)), 11)));
		assert!(state.posting.error.is_some());
		assert!(state.channels.iter().all(|c| c.id != Id(30)));
		let Some(Command::CreatePost { request, .. }) = state.create_post(Id(20), "Title", "Body")
		else {
			panic!("forum post command expected");
		};
		state.apply_post(Id(20), request, Ok(channel(30, Some(Id(20)), 11)));
		assert_eq!(state.posting.created.take(), Some(Id(30)));
		assert!(state.channels.iter().any(|c| c.id == Id(30)));
		assert!(state.select(Id(30)).is_some());
		let command = state.create_post(Id(20), "Title", "Body").unwrap();
		state.command_rejected(command);
		assert!(state.posting.pending.is_none());
		assert!(state.posting.error.is_some());
	}
}
