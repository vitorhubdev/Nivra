//! A bounded page of active forum posts fetched on demand, mirroring the archive page budget.
use crate::{Channel, EmbedMedia, Id, Reaction, ReactionEmoji};
use std::time::Duration;

pub const PAGE_SIZE: usize = 25;
pub const MAX_BYTES: usize = 128 * 1024;
/// How many posts one forum may pull in before the list stops offering more.
pub const MAX_POSTS: usize = 200;

/// Discord caps a forum at 20 tags and a post at 5 applied ones.
pub const MAX_TAGS: usize = 20;
pub const MAX_APPLIED_TAGS: usize = 5;
pub const MAX_TAG_NAME: usize = 50;

/// A card shows this many starter images; the rest fold into a "+N" badge.
pub const MAX_PREVIEW_IMAGES: usize = 4;
pub const MAX_PREVIEW_AUTHOR: usize = 512;
pub const MAX_PREVIEW_EXCERPT: usize = 1024;
pub const MAX_PREVIEW_REACTIONS: usize = 20;

/// One tag a forum or media channel offers its posts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
	pub id: Id,
	pub name: String,
	/// Only members who can manage threads may apply a moderated tag.
	pub moderated: bool,
	pub emoji_id: Option<Id>,
	/// A Unicode emoji, or the name of the custom emoji in `emoji_id`.
	pub emoji_name: Option<String>,
}

/// How a forum lays out its posts by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
	#[default]
	List,
	Gallery,
}

/// Which posts a forum lists first by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
	#[default]
	Activity,
	Created,
}

/// Forum metadata carried by a container (the tags it offers and its post defaults) or by a
/// post (the tags applied to it).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tags {
	pub available: Vec<Tag>,
	pub applied: Vec<Id>,
	/// The post is pinned in its forum's list, shown as Discord's pin badge.
	pub pinned: bool,
	/// The container requires at least one tag on every new post.
	pub required: bool,
	/// The emoji members react to posts with from the post list.
	pub reaction: Option<ReactionEmoji>,
	pub layout: Layout,
	pub sort: Sort,
	/// Unofficial `default_tag_setting`: a post must carry every selected tag, not just one.
	pub match_all: bool,
}
impl Tags {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.available.capacity() * size_of::<Tag>()
			+ self
				.available
				.iter()
				.map(|tag| {
					tag.name.capacity() + tag.emoji_name.as_ref().map_or(0, String::capacity)
				})
				.sum::<usize>()
			+ self.applied.capacity() * size_of::<Id>()
			+ self
				.reaction
				.as_ref()
				.and_then(|emoji| emoji.name.as_ref())
				.map_or(0, String::capacity)
	}
	pub fn is_empty(&self) -> bool {
		*self == Self::default()
	}
}

/// One image of a post card's starter mosaic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StarterImage {
	pub media: EmbedMedia,
	/// Attachment identity and declared size, so the mosaic can open the real attachment
	/// in the viewer and its download menu; zero for embed artwork.
	pub id: Id,
	pub size: u64,
	/// The attachment was marked as a spoiler; the card blurs it.
	pub spoiler: bool,
	/// An animated image such as a GIF; the card shows a badge.
	pub animated: bool,
	/// A video; the card shows a play badge over its poster.
	pub video: bool,
}

/// What a post card shows of its starter message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Starter {
	pub author_id: Id,
	pub author: String,
	/// Role membership supplied with the starter, for the author's name color.
	pub roles: Vec<Id>,
	pub webhook: bool,
	pub excerpt: String,
	/// The first images of the starter, in order; bounded by `MAX_PREVIEW_IMAGES`.
	pub images: Vec<StarterImage>,
	/// Every image the starter carries; the card folds the tail into a "+N" badge.
	pub image_count: u16,
	/// Reactions on the starter; the card shows the forum's default one or the most used.
	pub reactions: Vec<Reaction>,
}
impl Default for Starter {
	fn default() -> Self {
		Self {
			author_id: Id(0),
			author: String::new(),
			roles: Vec::new(),
			webhook: false,
			excerpt: String::new(),
			images: Vec::new(),
			image_count: 0,
			reactions: Vec::new(),
		}
	}
}
impl Starter {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.author.capacity()
			+ self.roles.capacity() * size_of::<Id>()
			+ self.excerpt.capacity()
			+ self.images.capacity() * size_of::<StarterImage>()
			+ self
				.images
				.iter()
				.map(|image| EmbedMedia::bytes(&image.media))
				.sum::<usize>()
			+ crate::reactions::reaction_bytes(&self.reactions)
			+ self
				.reactions
				.capacity()
				.saturating_sub(self.reactions.len())
				* size_of::<Reaction>()
	}
	pub fn valid(&self) -> bool {
		self.author_id.0 > 0
			&& self.author.len() <= MAX_PREVIEW_AUTHOR
			&& self.roles.len() <= crate::permissions::MAX_MEMBER_ROLES
			&& self.excerpt.len() <= MAX_PREVIEW_EXCERPT
			&& self.images.len() <= MAX_PREVIEW_IMAGES
			&& self.images.len() <= usize::from(self.image_count)
			&& self
				.images
				.iter()
				.all(|image| image.media.valid() && image.media.url.is_some())
			&& self.reactions.len() <= MAX_PREVIEW_REACTIONS
			&& crate::reactions::valid_reactions(&self.reactions)
	}
}

/// Why the per-forum search page was replaced by the documented active list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FallbackReason {
	/// The service answered with this non-success HTTP status.
	Status(u16),
	/// The search page exceeded its wire budget before it could decode.
	Oversized,
	/// The search page did not decode into a valid page.
	Decode,
}

/// What the fallback cost, for the diagnostic log and the slow-list notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FallbackReport {
	pub reason: FallbackReason,
	/// Response bytes involved in the decision (declared size, wire cap or decoded body).
	pub bytes: usize,
	/// Wall time the fallback route took.
	pub elapsed: Duration,
}
impl FallbackReport {
	/// One log line: the forum id is hashed and no service text, URL or content appears.
	pub fn log_line(&self, forum: Id) -> String {
		let reason = match self.reason {
			FallbackReason::Status(status) => format!("status:{status}"),
			FallbackReason::Oversized => "oversized".to_owned(),
			FallbackReason::Decode => "decode".to_owned(),
		};
		format!(
			"forum fallback: forum={} reason={reason} bytes={} elapsed_ms={}",
			short_hash(forum),
			self.bytes,
			self.elapsed.as_millis()
		)
	}
	/// A fallback that took long enough to deserve a discreet explanation.
	pub fn slow(&self) -> bool {
		self.elapsed > Duration::from_secs(3)
	}
}

/// Stable, non-reversible 8-hex label for an id in logs; never the raw snowflake.
fn short_hash(id: Id) -> String {
	let mut hash = 0xcbf2_9ce4_8422_2325u64;
	for byte in id.0.to_le_bytes() {
		hash ^= u64::from(byte);
		hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
	}
	format!("{:08x}", (hash >> 32) as u32)
}

pub struct Page {
	pub threads: Vec<Channel>,
	pub more: bool,
	/// The starter message of each listed post, when the service sent one.
	pub previews: Vec<(Id, Starter)>,
	/// Set when this page came from the documented guild-wide active list instead of the
	/// per-forum search route.
	pub fallback: Option<FallbackReport>,
}
impl Page {
	pub fn bytes(&self) -> usize {
		self.threads.capacity().saturating_sub(self.threads.len()) * size_of::<Channel>()
			+ self.threads.iter().map(Channel::bytes).sum::<usize>()
			+ self.previews.capacity().saturating_sub(self.previews.len())
				* size_of::<(Id, Starter)>()
			+ self
				.previews
				.iter()
				.map(|(_, starter)| starter.bytes())
				.sum::<usize>()
	}
	pub fn valid(&self, parent: Id, guild: Id) -> bool {
		parent.0 > 0
			&& guild.0 > 0
			&& self.threads.len() <= PAGE_SIZE
			&& self.bytes() <= MAX_BYTES
			&& (!self.more || !self.threads.is_empty())
			&& self.threads.iter().enumerate().all(|(i, thread)| {
				thread.id.0 > 0
					&& thread.id != parent
					&& thread.guild == Some(guild)
					&& thread.parent_id == Some(parent)
					&& thread.name.len() <= 512
					&& matches!(thread.kind, 10 | 11)
					&& self.threads[..i].iter().all(|other| other.id != thread.id)
			}) && self.previews.len() <= self.threads.len()
			&& self.previews.iter().all(|(id, starter)| {
				starter.valid() && self.threads.iter().any(|thread| thread.id == *id)
			})
	}
}

/// One card-only snapshot of a forum's post list for the on-disk cache: titles, authors,
/// tags, counters, the card excerpt and image metadata. Never a full message body.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CachedPage {
	pub posts: Vec<CachedPost>,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CachedPost {
	pub id: Id,
	#[serde(default)]
	pub name: String,
	#[serde(default)]
	pub last_message: Option<Id>,
	#[serde(default)]
	pub message_count: Option<u32>,
	#[serde(default)]
	pub applied: Vec<Id>,
	#[serde(default)]
	pub pinned: bool,
	#[serde(default)]
	pub starter: Option<CachedStarter>,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CachedStarter {
	pub author_id: Id,
	#[serde(default)]
	pub author: String,
	#[serde(default)]
	pub roles: Vec<Id>,
	#[serde(default)]
	pub webhook: bool,
	#[serde(default)]
	pub excerpt: String,
	#[serde(default)]
	pub images: Vec<CachedImage>,
	#[serde(default)]
	pub image_count: u16,
	#[serde(default)]
	pub reactions: Vec<Reaction>,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CachedImage {
	pub media: EmbedMedia,
	/// Attachment identity, absent for embed artwork.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub id: Option<Id>,
	#[serde(default)]
	pub size: u64,
	#[serde(default)]
	pub spoiler: bool,
	#[serde(default)]
	pub animated: bool,
	#[serde(default)]
	pub video: bool,
}

impl CachedPage {
	/// Snapshot the card fields of one forum's loaded posts, bounded by `MAX_POSTS`.
	pub fn from_posts(posts: impl IntoIterator<Item = (Channel, Option<Starter>)>) -> Self {
		let posts = posts
			.into_iter()
			.take(MAX_POSTS)
			.map(|(channel, starter)| CachedPost {
				id: channel.id,
				name: channel.name.chars().take(512).collect(),
				last_message: channel.last_message,
				message_count: channel.message_count,
				applied: channel
					.tags
					.as_deref()
					.map_or(Vec::new(), |tags| tags.applied.clone()),
				pinned: channel.tags.as_deref().is_some_and(|tags| tags.pinned),
				starter: starter.map(|starter| CachedStarter {
					author_id: starter.author_id,
					author: starter.author.chars().take(MAX_PREVIEW_AUTHOR).collect(),
					roles: starter.roles,
					webhook: starter.webhook,
					excerpt: starter.excerpt.chars().take(MAX_PREVIEW_EXCERPT).collect(),
					images: starter
						.images
						.into_iter()
						.map(|image| CachedImage {
							media: image.media,
							id: (image.id.0 > 0).then_some(image.id),
							size: image.size,
							spoiler: image.spoiler,
							animated: image.animated,
							video: image.video,
						})
						.collect(),
					image_count: starter.image_count,
					reactions: starter.reactions,
				}),
			})
			.collect();
		Self { posts }
	}

	/// The channels a cached list seeds into navigation; only card fields are restored.
	pub fn channels(&self, parent: Id, guild: Id) -> Vec<Channel> {
		self.posts
			.iter()
			.map(|post| {
				let tags = (post.pinned || !post.applied.is_empty()).then(|| {
					Box::new(Tags {
						applied: post.applied.clone(),
						pinned: post.pinned,
						..Tags::default()
					})
				});
				Channel {
					id: post.id,
					guild: Some(guild),
					parent_id: Some(parent),
					kind: 11,
					name: post.name.clone(),
					position: 0,
					recipients: Vec::new(),
					last_message: post.last_message,
					icon: None,
					member_list_id: None,
					message_count: post.message_count,
					tags,
				}
			})
			.collect()
	}

	/// The starter previews a cached list restores.
	pub fn previews(&self) -> Vec<(Id, Starter)> {
		self.posts
			.iter()
			.filter_map(|post| {
				post.starter.as_ref().map(|starter| {
					(
						post.id,
						Starter {
							author_id: starter.author_id,
							author: starter.author.clone(),
							roles: starter.roles.clone(),
							webhook: starter.webhook,
							excerpt: starter.excerpt.clone(),
							images: starter
								.images
								.iter()
								.map(|image| StarterImage {
									media: image.media.clone(),
									id: image.id.unwrap_or(Id(0)),
									size: image.size,
									spoiler: image.spoiler,
									animated: image.animated,
									video: image.video,
								})
								.collect(),
							image_count: starter.image_count,
							reactions: starter.reactions.clone(),
						},
					)
				})
			})
			.collect()
	}

	/// Bounds of a snapshot that may be trusted from disk or memory.
	pub fn valid(&self) -> bool {
		self.posts.len() <= MAX_POSTS
			&& self.posts.iter().enumerate().all(|(i, post)| {
				post.id.0 > 0
					&& post.name.len() <= 512
					&& post.applied.len() <= MAX_APPLIED_TAGS
					&& post.applied.iter().all(|id| id.0 > 0)
					&& self.posts[..i].iter().all(|other| other.id != post.id)
					&& post.starter.as_ref().is_none_or(|starter| {
						starter.author_id.0 > 0
							&& starter.author.len() <= MAX_PREVIEW_AUTHOR
							&& starter.roles.len() <= crate::permissions::MAX_MEMBER_ROLES
							&& starter.excerpt.len() <= MAX_PREVIEW_EXCERPT
							&& starter.images.len() <= MAX_PREVIEW_IMAGES
							&& starter.images.len() <= usize::from(starter.image_count)
							&& starter
								.images
								.iter()
								.all(|image| image.media.valid() && image.media.url.is_some())
							&& starter.reactions.len() <= MAX_PREVIEW_REACTIONS
							&& crate::reactions::valid_reactions(&starter.reactions)
					})
			})
	}
}
