//! A bounded page of active forum posts fetched on demand, mirroring the archive page budget.
use crate::{Channel, EmbedMedia, Id, Reaction, ReactionEmoji};

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

pub struct Page {
	pub threads: Vec<Channel>,
	pub more: bool,
	/// The starter message of each listed post, when the service sent one.
	pub previews: Vec<(Id, Starter)>,
}
impl Page {
	pub fn bytes(&self) -> usize {
		self.threads.capacity().saturating_sub(self.threads.len()) * size_of::<Channel>()
			+ self.threads.iter().map(Channel::bytes).sum::<usize>()
			+ self.previews.capacity() * size_of::<Id>()
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
