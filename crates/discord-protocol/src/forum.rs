//! Active forum posts from the thread search route; archived rows belong to the archive page.
use crate::ChannelDto;
use model::{
	EmbedMedia, Id, Patch,
	forum::{
		MAX_APPLIED_TAGS, MAX_PREVIEW_IMAGES, MAX_TAG_NAME, MAX_TAGS, Page, Starter, StarterImage,
		Tag, Tags,
	},
};
use serde::{Deserialize, Deserializer, de::Visitor};

/// Forum channel flag: every new post must carry at least one tag.
const REQUIRE_TAG: u64 = 1 << 4;

/// Keeps the first `N` entries of a list and skips the rest, so an oversized tag list never
/// rejects the snapshot carrying it; null reads as empty.
fn capped<'de, D: Deserializer<'de>, T: Deserialize<'de>, const N: usize>(
	d: D,
) -> Result<Vec<T>, D::Error> {
	struct Capped<T, const N: usize>(std::marker::PhantomData<T>);
	impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for Capped<T, N> {
		type Value = Vec<T>;
		fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
			f.write_str("a list of forum tags")
		}
		fn visit_unit<E>(self) -> Result<Self::Value, E> {
			Ok(Vec::new())
		}
		fn visit_none<E>(self) -> Result<Self::Value, E> {
			Ok(Vec::new())
		}
		fn visit_seq<A: serde::de::SeqAccess<'de>>(
			self,
			mut seq: A,
		) -> Result<Self::Value, A::Error> {
			let mut items = Vec::new();
			while items.len() < N {
				match seq.next_element()? {
					Some(item) => items.push(item),
					None => return Ok(items),
				}
			}
			while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
			Ok(items)
		}
	}
	d.deserialize_any(Capped::<T, N>(std::marker::PhantomData))
}

#[derive(Deserialize)]
struct TagDto {
	id: Id,
	#[serde(default)]
	name: String,
	#[serde(default)]
	moderated: bool,
	#[serde(default)]
	emoji_id: Option<Id>,
	#[serde(default)]
	emoji_name: Option<String>,
}

/// A forum's `available_tags`, bounded while decoding.
pub struct TagList(Vec<TagDto>);
impl<'de> Deserialize<'de> for TagList {
	fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		capped::<_, _, MAX_TAGS>(d).map(Self)
	}
}

/// A post's `applied_tags`, bounded while decoding.
pub struct AppliedTags(Vec<Id>);
impl<'de> Deserialize<'de> for AppliedTags {
	fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		capped::<_, _, MAX_APPLIED_TAGS>(d).map(Self)
	}
}

/// A forum's `default_reaction_emoji`.
#[derive(Deserialize)]
pub struct DefaultReaction {
	#[serde(default)]
	emoji_id: Option<Id>,
	#[serde(default)]
	emoji_name: Option<String>,
}
impl DefaultReaction {
	fn into_model(self) -> Option<model::ReactionEmoji> {
		let emoji = model::ReactionEmoji {
			id: self.emoji_id.filter(|id| id.0 > 0),
			name: self
				.emoji_name
				.map(|name| name.chars().take(32).collect::<String>())
				.filter(|name| !name.is_empty()),
		};
		emoji.valid().then_some(emoji)
	}
}

/// A forum's post defaults as they arrive on the wire.
#[derive(Default)]
pub(crate) struct Defaults {
	pub reaction: Option<DefaultReaction>,
	pub layout: Option<u8>,
	pub sort: Option<u8>,
	pub tag_setting: Option<String>,
}

/// Reduce wire tags to the model: containers keep what they offer, posts what they apply.
pub(crate) fn tags(
	kind: u8,
	available: Option<TagList>,
	applied: Option<AppliedTags>,
	flags: u64,
	defaults: Defaults,
) -> Option<Box<Tags>> {
	let mut tags = Tags::default();
	if matches!(kind, 15 | 16) {
		tags.reaction = defaults.reaction.and_then(DefaultReaction::into_model);
		tags.layout = if defaults.layout == Some(2) {
			model::forum::Layout::Gallery
		} else {
			model::forum::Layout::List
		};
		tags.sort = if defaults.sort == Some(1) {
			model::forum::Sort::Created
		} else {
			model::forum::Sort::Activity
		};
		tags.match_all = defaults.tag_setting.as_deref() == Some("match_all");
		for tag in available.map(|list| list.0).unwrap_or_default() {
			if tag.id.0 == 0 || tags.available.iter().any(|known| known.id == tag.id) {
				continue;
			}
			tags.available.push(Tag {
				id: tag.id,
				name: tag.name.trim().chars().take(MAX_TAG_NAME).collect(),
				moderated: tag.moderated,
				emoji_id: tag.emoji_id.filter(|id| id.0 > 0),
				emoji_name: tag
					.emoji_name
					.map(|name| name.chars().take(32).collect::<String>())
					.filter(|name| !name.is_empty()),
			});
		}
		tags.required = flags & REQUIRE_TAG != 0;
	} else if matches!(kind, 10..=12) {
		for id in applied.map(|list| list.0).unwrap_or_default() {
			if id.0 > 0 && !tags.applied.contains(&id) {
				tags.applied.push(id);
			}
		}
	}
	(!tags.is_empty()).then(|| Box::new(tags))
}

/// Channel updates carry whole objects; any tag-bearing field replaces the known tags.
pub(crate) fn patched_tags(
	kind: Patch<u8>,
	available: Patch<TagList>,
	applied: Patch<AppliedTags>,
	flags: &Patch<u64>,
	defaults: Defaults,
) -> Patch<Box<Tags>> {
	let Patch::Value(kind) = kind else {
		return Patch::Absent;
	};
	if matches!(available, Patch::Absent)
		&& matches!(applied, Patch::Absent)
		&& !matches!(flags, Patch::Value(_))
	{
		return Patch::Absent;
	}
	fn value<T>(patch: Patch<T>) -> Option<T> {
		match patch {
			Patch::Value(value) => Some(value),
			_ => None,
		}
	}
	let flags = match flags {
		Patch::Value(flags) => *flags,
		_ => 0,
	};
	tags(kind, value(available), value(applied), flags, defaults).map_or(Patch::Null, Patch::Value)
}

/// A source URL names an animated picture when its path ends in a GIF or APNG container.
fn animated(source: &Option<String>) -> bool {
	source.as_ref().is_some_and(|url| {
		let path = url.split(['?', '#']).next().unwrap_or(url);
		path.ends_with(".gif") || path.ends_with(".gifv") || path.ends_with(".apng")
	})
}

/// What a card shows of a starter: its first images and its reactions.
fn preview(message: crate::MessageDto) -> Option<(Id, Starter)> {
	let message = message.into_model();
	let mut images = Vec::with_capacity(MAX_PREVIEW_IMAGES);
	let mut image_count = 0u16;
	let mut push = |media: EmbedMedia, id: Id, size: u64, spoiler: bool, video: bool| {
		if !media.valid() || (media.url.is_none() && media.proxy_url.is_none()) {
			return;
		}
		image_count = image_count.saturating_add(1);
		if images.len() < MAX_PREVIEW_IMAGES {
			let animated = !video && animated(&media.url);
			images.push(StarterImage {
				media,
				id,
				size,
				spoiler,
				animated,
				video,
			});
		}
	};
	for attachment in message.attachments {
		if attachment.is_video() {
			push(
				attachment.media,
				attachment.id,
				attachment.size,
				attachment.spoiler,
				true,
			);
		} else if attachment.is_image() {
			push(
				attachment.media,
				attachment.id,
				attachment.size,
				attachment.spoiler,
				false,
			);
		}
	}
	for embed in message.embeds {
		if let Some(media) = embed.image.or(embed.thumbnail) {
			push(media, Id(0), 0, false, false);
		}
	}
	let mut reactions = message.reactions.unwrap_or_default();
	reactions.sort_by_key(|reaction| std::cmp::Reverse(reaction.count));
	reactions.truncate(model::forum::MAX_PREVIEW_REACTIONS);
	let excerpt = if message.content.contains("||") {
		"Spoiler content - open the post to reveal".into()
	} else {
		message.content.chars().take(256).collect()
	};
	let starter = Starter {
		author_id: message.author.id,
		author: message
			.author_nick
			.unwrap_or_else(|| message.author.name.clone()),
		roles: message.author_roles,
		webhook: message.author.webhook,
		excerpt,
		images,
		image_count,
		reactions,
	};
	(!starter.images.is_empty() || !starter.reactions.is_empty() || !starter.author.is_empty())
		.then_some((message.channel, starter))
}
pub const MAX_WIRE: usize = 1024 * 1024;
/// The guild-wide fallback lists every visible thread, so it needs the snapshot budget.
pub const GUILD_MAX_WIRE: usize = 2 * 1024 * 1024;

#[derive(Deserialize)]
pub struct Reply {
	#[serde(deserialize_with = "crate::search::list::<_,_,25>")]
	threads: Vec<Thread>,
	#[serde(default)]
	has_more: bool,
	/// Unofficial: the starter message of each listed post, used for its card preview.
	#[serde(default, deserialize_with = "capped::<_,_,25>")]
	first_messages: Vec<crate::MessageDto>,
}
#[derive(Deserialize)]
struct Thread {
	#[serde(flatten)]
	channel: ChannelDto,
	#[serde(default)]
	thread_metadata: Option<Metadata>,
}
#[derive(Deserialize)]
struct Metadata {
	#[serde(default)]
	archived: bool,
	#[serde(default)]
	pinned: bool,
}

impl Reply {
	pub fn into_page(self, parent: Id, guild: Id) -> Result<Page, &'static str> {
		let invalid = "Invalid forum post page";
		let mut threads = Vec::with_capacity(self.threads.len());
		for thread in self.threads {
			let metadata = thread.thread_metadata;
			// An archived row here would duplicate the archive page and mislead the post list.
			if metadata.as_ref().is_some_and(|metadata| metadata.archived) {
				return Err(invalid);
			}
			let mut channel =
				crate::threads::into_thread(thread.channel, guild).map_err(|_| invalid)?;
			// The pin badge rides the same optional container a post's applied tags use.
			if metadata.as_ref().is_some_and(|metadata| metadata.pinned) {
				let mut tags = channel.tags.take().unwrap_or_default();
				tags.pinned = true;
				channel.tags = Some(tags);
			}
			threads.push(channel);
		}
		let mut previews: Vec<(Id, Starter)> = Vec::new();
		for (id, starter) in self.first_messages.into_iter().filter_map(preview) {
			if threads.iter().any(|thread| thread.id == id)
				&& !previews.iter().any(|(known, _)| *known == id)
			{
				previews.push((id, starter));
			}
		}
		let page = Page {
			threads,
			more: self.has_more,
			previews,
		};
		if !page.valid(parent, guild) {
			return Err(invalid);
		}
		Ok(page)
	}
}

/// The documented guild-wide active list, used when the per-forum search route is unavailable.
#[derive(Deserialize)]
pub struct GuildActive {
	#[serde(deserialize_with = "crate::threads::list")]
	threads: Vec<ChannelDto>,
}

impl GuildActive {
	pub fn into_page(self, parent: Id, guild: Id) -> Result<Page, &'static str> {
		let invalid = "Invalid forum post page";
		let mut threads = Vec::new();
		for thread in self.threads {
			if thread.parent_id != Some(parent) {
				continue;
			}
			threads.push(crate::threads::into_thread(thread, guild).map_err(|_| invalid)?);
		}
		threads.sort_by_key(|thread| std::cmp::Reverse(thread.last_message.unwrap_or(thread.id)));
		threads.truncate(model::forum::PAGE_SIZE);
		threads.shrink_to_fit();
		let page = Page {
			threads,
			more: false,
			previews: Vec::new(),
		};
		if !page.valid(parent, guild) {
			return Err(invalid);
		}
		Ok(page)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;
	#[test]
	fn forum_pages_validate_scope_state_and_capacity() {
		let thread = |id: u64, kind: u8, archived: bool| json!({"id":id.to_string(),"guild_id":"1","parent_id":"2","type":kind,"name":"Synthetic","thread_metadata":{"archived":archived}});
		let decode = |threads, more| {
			crate::decode::<Reply>(
				&serde_json::to_vec(
					&json!({"threads":threads,"has_more":more,"members":[{"private":"ignored"}],"total_results":2}),
				)
				.unwrap(),
			)
		};
		let page = decode(vec![thread(3, 11, false), thread(9, 11, false)], true)
			.unwrap()
			.into_page(Id(2), Id(1))
			.unwrap();
		assert_eq!(
			page.threads.iter().map(|t| t.id).collect::<Vec<_>>(),
			vec![Id(3), Id(9)]
		);
		assert!(page.more);
		// A missing metadata block still lists; only an archived row is rejected.
		let mut bare = thread(3, 11, false);
		bare["thread_metadata"] = json!(null);
		assert!(
			decode(vec![bare], false)
				.unwrap()
				.into_page(Id(2), Id(1))
				.is_ok()
		);
		assert!(
			decode(vec![thread(3, 11, true)], false)
				.unwrap()
				.into_page(Id(2), Id(1))
				.is_err()
		);
		assert!(
			decode(Vec::<serde_json::Value>::new(), true)
				.unwrap()
				.into_page(Id(2), Id(1))
				.is_err()
		);
		for (key, value) in [
			("guild_id", json!("7")),
			("parent_id", json!("7")),
			("type", json!(12)),
		] {
			let mut invalid = thread(3, 11, false);
			invalid[key] = value;
			assert!(
				decode(vec![invalid], false)
					.unwrap()
					.into_page(Id(2), Id(1))
					.is_err()
			);
		}
		// The guild-wide fallback keeps only this forum's newest rows.
		let guild_active = |threads| {
			crate::decode::<GuildActive>(
				&serde_json::to_vec(&json!({ "threads": threads, "members": [] })).unwrap(),
			)
			.unwrap()
		};
		let mut elsewhere = thread(4, 11, false);
		elsewhere["parent_id"] = json!("5");
		let page = guild_active(vec![thread(3, 11, false), elsewhere])
			.into_page(Id(2), Id(1))
			.unwrap();
		assert_eq!(
			page.threads.iter().map(|t| t.id).collect::<Vec<_>>(),
			vec![Id(3)]
		);
		assert!(!page.more);
		assert!(
			guild_active(vec![thread(3, 12, false)])
				.into_page(Id(2), Id(1))
				.is_err()
		);
		let crowded: Vec<_> = (3..=32).map(|id| thread(id, 11, false)).collect();
		assert_eq!(
			guild_active(crowded)
				.into_page(Id(2), Id(1))
				.unwrap()
				.threads
				.len(),
			model::forum::PAGE_SIZE
		);
		assert!(decode(vec![thread(3, 11, false); 26], false).is_err());
		assert!(
			decode(vec![thread(3, 11, false); 2], false)
				.unwrap()
				.into_page(Id(2), Id(1))
				.is_err()
		);
	}

	fn message(id: u64, channel: u64, images: usize) -> serde_json::Value {
		let attachments: Vec<_> = (0..images)
			.map(|image| {
				json!({
					"id": format!("{}", 700 + image),
					"filename": format!("synthetic-{image}.png"),
					"content_type": "image/png",
					"size": 4096,
					"url": format!("https://cdn.example/attachments/1/{}/synthetic-{image}.png", 700 + image),
					"proxy_url": format!("https://media.example/attachments/1/{}/synthetic-{image}.png", 700 + image),
					"width": 800,
					"height": 600
				})
			})
			.collect();
		json!({
			"id": id.to_string(),
			"channel_id": channel.to_string(),
			"author": {"id": "5", "username": "Synthetic", "discriminator": "0"},
			"content": "A synthetic starter",
			"attachments": attachments,
			"reactions": [{"count": 3, "me": false, "emoji": {"id": null, "name": "🔥"}}]
		})
	}

	#[test]
	fn starter_previews_keep_the_first_images_and_every_count() {
		for (images, expected, count) in [
			(0usize, 0usize, 0u16),
			(1, 1, 1),
			(2, 2, 2),
			(4, 4, 4),
			(9, 4, 9),
		] {
			let body = json!({
				"threads": [{
					"id": "9", "guild_id": "1", "parent_id": "2", "type": 11,
					"name": "Synthetic", "thread_metadata": {"archived": false, "pinned": true},
					"applied_tags": ["31", "32"]
				}],
				"members": [],
				"has_more": false,
				"first_messages": [message(9, 9, images)]
			});
			let page = crate::decode::<Reply>(&serde_json::to_vec(&body).unwrap())
				.unwrap()
				.into_page(Id(2), Id(1))
				.unwrap();
			assert_eq!(page.previews.len(), 1);
			let (post, starter) = &page.previews[0];
			assert_eq!(*post, Id(9));
			assert_eq!(starter.images.len(), expected);
			assert_eq!(starter.image_count, count);
			assert_eq!(starter.author, "Synthetic");
			assert_eq!(starter.excerpt, "A synthetic starter");
			assert_eq!(starter.reactions.len(), 1);
			let tags = page.threads[0].tags.as_deref().unwrap();
			assert_eq!(tags.applied, vec![Id(31), Id(32)]);
			assert!(tags.pinned, "the search row's pin badge is kept");
		}
	}

	#[test]
	fn forum_container_offers_bounded_tags_and_defaults() {
		let channel = |layout: Option<u8>, sort: Option<u8>| {
			let body = json!({
				"id": "2", "guild_id": "1", "type": 15, "name": "Synthetic forum",
				"flags": 16,
				"available_tags": [
					{"id": "31", "name": " Help ", "moderated": true, "emoji_name": "🔥"},
					{"id": "32", "name": "Outros", "emoji_id": "77", "emoji_name": "custom"},
					{"id": "31", "name": "duplicate"}
				],
				"default_reaction_emoji": {"emoji_id": null, "emoji_name": "🔥"},
				"default_forum_layout": layout,
				"default_sort_order": sort,
				"default_tag_setting": "match_all"
			});
			crate::decode::<crate::ChannelDto>(&serde_json::to_vec(&body).unwrap())
				.unwrap()
				.into_model()
		};
		let tags = channel(Some(2), Some(1)).tags.expect("tags");
		assert!(tags.required);
		assert!(tags.match_all);
		assert_eq!(tags.layout, model::forum::Layout::Gallery);
		assert_eq!(tags.sort, model::forum::Sort::Created);
		assert_eq!(tags.available.len(), 2);
		assert_eq!(tags.available[0].name, "Help");
		assert_eq!(tags.available[0].emoji_name.as_deref(), Some("🔥"));
		assert_eq!(tags.available[1].emoji_id, Some(Id(77)));
		assert_eq!(tags.reaction.unwrap().name.as_deref(), Some("🔥"));
		let list = channel(None, None).tags.expect("tags");
		assert_eq!(list.layout, model::forum::Layout::List);
		assert_eq!(list.sort, model::forum::Sort::Activity);
	}

	#[test]
	fn an_image_heavy_full_page_fits_the_wire_and_model_budgets() {
		let threads: Vec<_> = (0..25)
			.map(|index| {
				let id = 900 + index;
				json!({
					"id": id.to_string(), "guild_id": "1", "parent_id": "2", "type": 11,
					"name": "Synthetic post with a reasonably long title",
					"thread_metadata": {"archived": false},
					"applied_tags": ["31", "32", "33", "34", "35"]
				})
			})
			.collect();
		let first_messages: Vec<_> = (900..925).map(|id| message(id, id, 9)).collect();
		let body = json!({
			"threads": threads,
			"members": [],
			"has_more": false,
			"first_messages": first_messages
		});
		let bytes = serde_json::to_vec(&body).unwrap();
		let page = crate::decode::<Reply>(&bytes)
			.unwrap_or_else(|_| panic!("{} wire bytes must decode", bytes.len()))
			.into_page(Id(2), Id(1))
			.unwrap_or_else(|_| panic!("{} wire bytes must be a valid page", bytes.len()));
		assert_eq!(page.previews.len(), 25);
		assert_eq!(page.previews[0].1.images.len(), 4);
		assert_eq!(page.previews[0].1.image_count, 9);
		println!(
			"Synthetic 25-post page: {} wire bytes, {} model bytes (budgets {} and {})",
			bytes.len(),
			page.bytes(),
			MAX_WIRE,
			model::forum::MAX_BYTES
		);
		assert!(
			page.bytes() <= model::forum::MAX_BYTES,
			"{} model bytes need a bigger page budget",
			page.bytes()
		);
	}
}
