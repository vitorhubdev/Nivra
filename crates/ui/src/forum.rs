//! Forum containers: a searchable post list with Discord-style cards and one-post creation.
use crate::{avatars::Avatars, design, icons};
use client_core::{Command, MAX_CONTENT, State, forum::MAX_TITLE};
use egui::RichText;
use model::{
	Channel, Id,
	archives::Kind,
	forum::{Layout, Sort, Starter, StarterImage, Tag},
};

/// Filter chips beside Sort & View, and the smaller ones on a post card.
const TAG_HEIGHT: f32 = 30.0;
/// Tags a card lists before folding the rest into a "+N" pill.
const CARD_TAGS: usize = 3;
/// Edge of the starter image beside a list card.
const PREVIEW: f32 = 72.0;
/// Narrowest gallery tile before a row drops a column.
const GALLERY_TILE: f32 = 300.0;
/// Space between gallery tiles, and between the cells of one mosaic.
const TILE_GAP: f32 = 12.0;
const MOSAIC_GAP: f32 = 3.0;
/// Height of the tag row every tile keeps so a gallery row lines up.
const CARD_TAG_HEIGHT: f32 = 24.0;
/// Share of a gallery tile's width its artwork spans; the rest holds the card text.
const TILE_ART: f32 = 0.58;
/// Layouts the user picked in this session, so a forum reopens the way it was left.
const REMEMBERED_LAYOUTS: usize = 64;

fn sort_label(ui: &egui::Ui, sort: Sort) -> &'static str {
	crate::tr_ui!(
		ui,
		match sort {
			Sort::Activity => "Recent activity",
			Sort::Created => "Creation date",
		}
	)
}

fn layout_label(ui: &egui::Ui, layout: Layout) -> &'static str {
	crate::tr_ui!(
		ui,
		match layout {
			Layout::List => "List",
			Layout::Gallery => "Gallery",
		}
	)
}

#[derive(Default)]
struct Draft {
	title: String,
	body: String,
	/// Forum tags applied to the new post, in the order they were picked.
	tags: Vec<Id>,
	focus: bool,
	submitted: bool,
}

#[derive(Default)]
pub struct ForumUi {
	forum: Option<Id>,
	query: String,
	sort: Sort,
	layout: Layout,
	/// Layouts this session changed, most recent first.
	layouts: Vec<(Id, Layout)>,
	/// Open starter image in the full-window viewer.
	viewing: Option<(Id, usize)>,
	/// Tags that filter the list; a post matches when it carries any of them, or all with
	/// `match_all`.
	tags: Vec<Id>,
	match_all: bool,
	draft: Option<Draft>,
	emoji: crate::emoji_picker::Picker,
}
impl ForumUi {
	fn remember_layout(&mut self, forum: Id, layout: Layout) {
		self.layouts.retain(|(known, _)| *known != forum);
		self.layouts.insert(0, (forum, layout));
		self.layouts.truncate(REMEMBERED_LAYOUTS);
	}

	fn remembered_layout(&self, forum: Id) -> Option<Layout> {
		self.layouts
			.iter()
			.find(|(known, _)| *known == forum)
			.map(|(_, layout)| *layout)
	}
}

/// Files chosen for the post's first message. The selection itself lives in the messaging
/// view's upload tray, so a forum never keeps a second copy of anything the user picked.
pub struct Staged<'a> {
	pub files: &'a [(String, u64)],
	pub textures: &'a [Option<egui::TextureHandle>],
	/// Set to ask the desktop shell for the native file chooser.
	pub choose: &'a mut bool,
	/// Set to the index of a card the user removed.
	pub remove: &'a mut Option<usize>,
	/// Set to drop the whole selection, as discarding the draft does.
	pub clear: &'a mut bool,
	pub busy: bool,
}

/// Post open target: an active thread, or an archived row admitted through the archive view.
enum Open {
	Active(Id),
	Archived(Id),
}

impl ForumUi {
	#[allow(clippy::too_many_arguments)] // Forum view carries all channel inputs.
	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		forum: Id,
		commands: &mut Vec<Command>,
		(session, staged, images): (&mut crate::scroll::Session, &mut Staged<'_>, &mut Avatars),
		(menu, view): (
			&mut crate::channel_menu::ChannelMenu,
			crate::shortcuts::ShortcutView<'_>,
		),
		language: model::Language,
	) {
		if self.forum != Some(forum) {
			self.forum = Some(forum);
			self.query.clear();
			self.tags.clear();
			self.viewing = None;
			// Each forum opens the way its moderators set it up; members may change it.
			let defaults = state.forum_defaults(forum).cloned().unwrap_or_default();
			self.sort = defaults.sort;
			self.layout = self.remembered_layout(forum).unwrap_or(defaults.layout);
			self.match_all = defaults.match_all;
			self.discard_draft(staged);
		}
		if let Some(draft) = &self.draft
			&& draft.submitted
			&& state.posting.pending.is_none()
			&& state.posting.error.is_none()
		{
			self.draft = None;
		}
		if self.draft.is_some()
			&& ui
				.ctx()
				.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
		{
			self.discard_draft(staged);
		}
		if let Some(command) = state.request_forum_posts(forum, false) {
			commands.push(command);
		}
		let colors = design::palette(ui);
		let mut open = None;
		let mut archive_request = None;
		let mut posts_request = false;
		let mut author_lookup = Vec::new();
		session
			.attach(
				ui,
				("forum", forum),
				egui::ScrollArea::vertical().auto_shrink([false, false]),
			)
			.show(ui, |ui| {
				egui::Frame::new()
					.inner_margin(egui::Margin::symmetric(16, 12))
					.show(ui, |ui| {
						ui.set_width(ui.available_width());
						ui.spacing_mut().item_spacing.y = 12.0;
						self.toolbar(ui, state, forum);
						if self.draft.is_some() {
							self.composer(ui, state, forum, commands, staged, images);
						}
						ui.horizontal(|ui| {
							ui.spacing_mut().item_spacing.x = 8.0;
							self.sort_menu(ui, forum);
							self.tag_filter(ui, state, forum, images);
						});
						// Only tags the forum still offers filter; a removed one matches nothing.
						let offered = state.forum_tags(forum);
						self.tags
							.retain(|id| offered.iter().any(|tag| tag.id == *id));
						let query = self.query.trim().to_lowercase();
						let (tags, all) = (&self.tags, self.match_all);
						let matches = |post: &Channel| {
							(query.is_empty() || post.name.to_lowercase().contains(&query))
								&& carries(post, tags, all)
						};
						let filtered = !query.is_empty() || !tags.is_empty();
						let mut posts: Vec<&Channel> = state.forum_posts(forum);
						if self.sort == Sort::Created {
							posts.sort_by_key(|post| std::cmp::Reverse(post.id));
						}
						let active: Vec<_> =
							posts.into_iter().filter(|post| matches(post)).collect();
						let archive = state
							.archives
							.as_ref()
							.filter(|view| view.parent == forum && view.kind == Kind::Public);
						let archived: Vec<&Channel> = archive
							.and_then(|view| view.page.as_ref())
							.map(|page| page.threads.iter().filter(|post| matches(post)).collect())
							.unwrap_or_default();
						let now = time::OffsetDateTime::now_utc();
						let loading = state.posts.parent == Some(forum) && state.posts.loading;
						if active.is_empty() && archived.is_empty() && !loading {
							ui.add_space(24.0);
							ui.vertical_centered(|ui| {
								ui.label(
									design::semibold(
										ui,
										if filtered {
											crate::tr_ui!(ui, "No posts match")
										} else {
											crate::tr_ui!(ui, "No posts loaded")
										},
										16.0,
									)
									.color(colors.text_strong),
								);
								ui.label(
									RichText::new(if !query.is_empty() {
										crate::tr_ui!(
											ui,
											"Press Enter to start a post with this title."
										)
									} else if filtered {
										crate::tr_ui!(
											ui,
											"No loaded post carries the selected tags."
										)
									} else {
										crate::tr_ui!(
											ui,
											"Nothing is posted here yet; archived posts load on request."
										)
									})
									.color(colors.muted),
								);
							});
						}
						let active: Vec<&Channel> = active
							.into_iter()
							.filter(|post| !archived.iter().any(|row| row.id == post.id))
							.collect();
						let hits = posts_view(ui, state, images, &active, self.layout, now);
						for (post, hit) in active.iter().zip(hits) {
							if let Some(starter) = state.post_preview(post.id)
								&& !starter.webhook && starter.roles.is_empty()
								&& author_lookup.len() < client_core::member_search::LIMIT
								&& !author_lookup.contains(&starter.author_id)
							{
								author_lookup.push(starter.author_id);
							}
							menu.context(&hit.response, state, post, view, language);
							if let Some(index) = hit.image {
								self.viewing = Some((post.id, index));
							} else if hit.response.clicked() {
								open = Some(Open::Active(post.id));
							}
						}
						posts_request = posts_footer(ui, state, forum);
						let hits = posts_view(ui, state, images, &archived, self.layout, now);
						for (post, hit) in archived.iter().zip(hits) {
							menu.context(&hit.response, state, post, view, language);
							if let Some(index) = hit.image {
								self.viewing = Some((post.id, index));
							} else if hit.response.clicked() {
								open = Some(Open::Archived(post.id));
							}
						}
						ui.add_space(4.0);
						archive_request = archive_footer(ui, state, forum, archive);
					});
			});
		if self.viewing.is_some() {
			let mut lightbox = super::attachments::DownloadUi::default();
			let mut opening = None;
			let keep = self.lightbox(ui, state, images, &mut lightbox, &mut opening);
			if !keep {
				self.viewing = None;
			}
		}
		if let Some(command) = state.request_author_members(&author_lookup) {
			commands.push(command);
		}
		if let Some(open) = open {
			match open {
				Open::Active(id) => {
					if let Some(command) = state.select(id) {
						commands.push(command);
					}
				}
				Open::Archived(id) => {
					if let Some(command) = state.open_archived_thread(id) {
						commands.push(command);
					}
				}
			}
		} else if let Some(before) = archive_request
			&& let Some(command) = state.request_archives(forum, Kind::Public, before)
		{
			commands.push(command);
		} else if posts_request {
			state.posts.error = None;
			if let Some(command) = state.request_forum_posts(forum, state.posts.loaded > 0) {
				commands.push(command);
			}
		}
	}

	fn toolbar(&mut self, ui: &mut egui::Ui, state: &State, forum: Id) {
		let colors = design::palette(ui);
		egui::Frame::new()
			.fill(colors.raised)
			.stroke(egui::Stroke::new(1.0, colors.border))
			.corner_radius(8)
			.inner_margin(egui::Margin::symmetric(12, 8))
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 8.0;
					let allowed = state.can_create_post(forum);
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						let button = ui.add_enabled(
							allowed && self.draft.is_none(),
							egui::Button::new(
								design::medium(ui, "New Post", 14.0).color(colors.accent_text),
							)
							.fill(colors.accent)
							.stroke(egui::Stroke::NONE)
							.corner_radius(8)
							.min_size(egui::vec2(0.0, 32.0)),
						);
						if button.clicked() {
							self.start_draft(String::new());
						}
						if !allowed && state.is_forum(forum) {
							button.on_disabled_hover_text(
								"Posting requires a connected session with permission to send here.",
							);
						}
						ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
							icons::inline(ui, icons::Icon::Search, 20.0, colors.muted);
							let input = ui.add(
								egui::TextEdit::singleline(&mut self.query)
									.char_limit(MAX_TITLE)
									.frame(egui::Frame::NONE)
									.hint_text(crate::tr_ui!(ui, "Search or create a post..."))
									.font(egui::TextStyle::Body)
									.desired_width(ui.available_width().max(60.0)),
							);
							let input =
								input.accessible_name("Search loaded posts or start a new one");
							if input.lost_focus()
								&& ui.input(|i| i.key_pressed(egui::Key::Enter))
								&& !self.query.trim().is_empty()
								&& allowed
							{
								let title = std::mem::take(&mut self.query);
								self.start_draft(title);
							}
						});
					});
				});
			});
	}

	fn start_draft(&mut self, title: String) {
		self.draft = Some(Draft {
			title: title.trim().chars().take(MAX_TITLE).collect(),
			body: String::new(),
			// Tags the list is filtered by are a good guess for what the post is about.
			tags: self
				.tags
				.iter()
				.take(model::forum::MAX_APPLIED_TAGS)
				.copied()
				.collect(),
			focus: true,
			submitted: false,
		});
	}

	fn sort_menu(&mut self, ui: &mut egui::Ui, forum: Id) {
		let colors = design::palette(ui);
		let view = format!(
			"{} · {}, {}",
			crate::tr_ui!(ui, "Sort & view"),
			sort_label(ui, self.sort),
			layout_label(ui, self.layout)
		);
		let button = ui
			.add(
				egui::Button::new(design::medium(ui, view, 13.0).color(colors.text))
					.fill(colors.raised)
					.stroke(egui::Stroke::new(1.0, colors.border))
					.corner_radius(8)
					.min_size(egui::vec2(0.0, 30.0)),
			)
			.on_hover_text(crate::tr_ui!(ui, "Sort and layout of this post list"));
		let mut changed = None;
		egui::Popup::menu(&button).show(|ui| {
			let motion = crate::anim::popup_alpha(ui.ctx(), ui.scope_id().with("menu-motion"));
			ui.set_opacity(motion);
			ui.set_min_width(200.0);
			ui.label(design::eyebrow(
				ui,
				crate::tr_ui!(ui, "Sort by"),
				colors.muted,
			));
			for sort in [Sort::Activity, Sort::Created] {
				if ui.radio(self.sort == sort, sort_label(ui, sort)).clicked() {
					self.sort = sort;
				}
			}
			ui.separator();
			ui.label(design::eyebrow(
				ui,
				crate::tr_ui!(ui, "View as"),
				colors.muted,
			));
			for layout in [Layout::List, Layout::Gallery] {
				if ui
					.radio(self.layout == layout, layout_label(ui, layout))
					.clicked()
				{
					self.layout = layout;
					changed = Some(layout);
				}
			}
		});
		if let Some(layout) = changed {
			self.remember_layout(forum, layout);
		}
	}

	/// Discord's tag bar: the tags that fit as toggles, then a menu holding all of them.
	fn tag_filter(&mut self, ui: &mut egui::Ui, state: &State, forum: Id, images: &mut Avatars) {
		let colors = design::palette(ui);
		let offered = state.forum_tags(forum);
		if offered.is_empty() {
			return;
		}
		let (rule, _) = ui.allocate_exact_size(egui::vec2(1.0, 20.0), egui::Sense::hover());
		ui.painter().rect_filled(rule, 0, colors.border);
		let menu_label = if self.tags.is_empty() {
			crate::tr_ui!(ui, "All").to_owned()
		} else {
			format!("{} ({})", crate::tr_ui!(ui, "Tags"), self.tags.len())
		};
		let menu_width = action_width(ui, &menu_label, TAG_HEIGHT, 1);
		let mut folded = false;
		for tag in offered {
			if pill_width(ui, tag, TAG_HEIGHT) + ui.spacing().item_spacing.x + menu_width
				> ui.available_width()
			{
				folded = true;
				break;
			}
			let selected = self.tags.contains(&tag.id);
			if tag_pill(
				ui,
				(images, state.demo),
				tag,
				selected,
				TAG_HEIGHT,
				egui::Sense::click(),
			)
			.clicked()
			{
				toggle(&mut self.tags, tag.id);
			}
		}
		let button = action_pill(
			ui,
			(None, Some(icons::Icon::ChevronDown)),
			&menu_label,
			TAG_HEIGHT,
			!self.tags.is_empty(),
		);
		let button = if folded {
			button.on_hover_text(crate::tr_ui!(ui, "More tags"))
		} else {
			button
		};
		egui::Popup::menu(&button)
			.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
			.show(|ui| {
				ui.set_max_width(360.0);
				ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
				ui.horizontal(|ui| {
					ui.label(
						design::semibold(ui, crate::tr_ui!(ui, "Select tags"), 15.0)
							.color(colors.muted),
					);
					count_badge(ui, self.tags.len());
				});
				ui.horizontal_wrapped(|ui| {
					for tag in offered {
						let selected = self.tags.contains(&tag.id);
						if tag_pill(
							ui,
							(images, state.demo),
							tag,
							selected,
							TAG_HEIGHT,
							egui::Sense::click(),
						)
						.clicked()
						{
							toggle(&mut self.tags, tag.id);
						}
					}
				});
				ui.horizontal(|ui| {
					ui.label(
						RichText::new(crate::tr_ui!(ui, "Match"))
							.size(13.0)
							.color(colors.muted),
					);
					ui.radio_value(&mut self.match_all, false, crate::tr_ui!(ui, "Any"))
						.on_hover_text(crate::tr_ui!(ui, "Show posts with any selected tag"));
					ui.radio_value(&mut self.match_all, true, crate::tr_ui!(ui, "All"))
						.on_hover_text(crate::tr_ui!(
							ui,
							"Show only posts with every selected tag"
						));
				});
				ui.separator();
				if ui
					.add_enabled(
						!self.tags.is_empty(),
						egui::Button::new(
							RichText::new(crate::tr_ui!(ui, "Clear tags")).color(colors.link),
						)
						.frame(false),
					)
					.clicked()
				{
					self.tags.clear();
				}
			});
	}

	/// Show the clicked starter image in the shared full-window viewer.
	fn lightbox(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		images: &mut Avatars,
		download: &mut crate::attachments::DownloadUi,
		opening: &mut Option<String>,
	) -> bool {
		let Some((post, index)) = self.viewing else {
			return false;
		};
		let Some(starter) = state.post_preview(post) else {
			return false;
		};
		if starter.images.is_empty() {
			return false;
		}
		let attachments: Vec<model::Attachment> = starter
			.images
			.iter()
			.enumerate()
			.map(starter_attachment)
			.collect();
		let current = attachments[index.min(attachments.len() - 1)].id;
		let Some(keep) = crate::attachments::viewer(
			ui,
			&attachments,
			current,
			images,
			download,
			opening,
			state.demo,
		) else {
			return false;
		};
		if let Some(index) = attachments
			.iter()
			.position(|attachment| attachment.id == keep)
		{
			self.viewing = Some((post, index));
		}
		true
	}

	/// Drop the draft and anything staged with it; a discarded post keeps no selection.
	fn discard_draft(&mut self, staged: &mut Staged<'_>) {
		if self.draft.take().is_some() && !staged.files.is_empty() {
			*staged.clear = true;
		}
	}

	/// Discord's post composer: one card holding the title, the first message, its images and
	/// the actions that send them together.
	fn composer(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		forum: Id,
		commands: &mut Vec<Command>,
		staged: &mut Staged<'_>,
		images: &mut Avatars,
	) {
		let colors = design::palette(ui);
		let files_allowed = state.can_attach_post(forum);
		let posting = state.posting.pending.is_some() || staged.busy;
		let error = state.posting.error;
		let emoji = &mut self.emoji;
		let Some(draft) = self.draft.as_mut() else {
			return;
		};
		let mut submit = false;
		let mut cancel = false;
		egui::Frame::new()
			.fill(colors.raised)
			.stroke(egui::Stroke::new(1.0, colors.border))
			.corner_radius(12)
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.spacing_mut().item_spacing.y = 0.0;
				egui::Frame::new()
					.inner_margin(egui::Margin::symmetric(16, 14))
					.show(ui, |ui| {
						ui.set_width(ui.available_width());
						ui.horizontal_top(|ui| {
							ui.spacing_mut().item_spacing.x = 10.0;
							if icons::button(ui, icons::Icon::Close, 22.0, "Discard this post")
								.clicked()
							{
								cancel = true;
							}
							const THUMB: f32 = 72.0;
							let fields = (ui.available_width() - THUMB - 10.0).max(160.0);
							ui.vertical(|ui| {
								ui.set_width(fields);
								ui.spacing_mut().item_spacing.y = 4.0;
								let title = ui.add(
									egui::TextEdit::singleline(&mut draft.title)
										.char_limit(MAX_TITLE)
										.frame(egui::Frame::NONE)
										.font(egui::FontId::new(
											20.0,
											design::semibold_family(ui.ctx()),
										))
										.text_color(colors.text_strong)
										.hint_text(
											design::semibold(ui, crate::tr_ui!(ui, "Title"), 20.0)
												.color(colors.muted),
										)
										.desired_width(f32::INFINITY),
								);
								let title = title.accessible_name("Post title");
								if draft.focus {
									title.request_focus();
									draft.focus = false;
								}
								let body = ui.add(
									egui::TextEdit::multiline(&mut draft.body)
										.char_limit(MAX_CONTENT)
										.frame(egui::Frame::NONE)
										.hint_text(
											RichText::new(crate::tr_ui!(ui, "Enter a message..."))
												.size(15.0)
												.color(colors.muted),
										)
										.desired_rows(3)
										.desired_width(f32::INFINITY),
								);
								body.accessible_name("First message of this post");
							});
							// Discord parks the image control beside the fields, not under them.
							ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
								let enabled = files_allowed
									&& !posting && staged.files.len()
									< client_core::MAX_ATTACHMENTS;
								let (rect, response) = ui.allocate_exact_size(
									egui::Vec2::splat(THUMB),
									if enabled {
										egui::Sense::click()
									} else {
										egui::Sense::hover()
									},
								);
								let hovered =
									enabled && (response.hovered() || response.has_focus());
								ui.painter().rect(
									rect,
									10,
									if hovered {
										colors.hover
									} else {
										colors.sidebar
									},
									egui::Stroke::new(1.0, colors.border),
									egui::StrokeKind::Inside,
								);
								icons::paint(
									ui.painter(),
									icons::Icon::Image,
									egui::Rect::from_center_size(
										rect.center(),
										egui::Vec2::splat(30.0),
									),
									if !enabled {
										colors.muted.gamma_multiply(0.5)
									} else if hovered {
										colors.text_strong
									} else {
										colors.text
									},
								);
								response.widget_info(|| {
									egui::WidgetInfo::labeled(
										egui::Role::Button,
										enabled,
										"Add images to this post",
									)
								});
								if response.clicked() {
									*staged.choose = true;
								}
								response.on_hover_text(if files_allowed {
									"Add images or files. Up to 10 files and 500 MB total; account limits may be lower."
								} else {
									"Attaching files is unavailable in this forum."
								});
							});
						});
						if !staged.files.is_empty() {
							ui.add_space(10.0);
							tray(ui, staged);
						}
						post_tags(ui, state, forum, &mut draft.tags, images);
					});
				let (rule, _) = ui.allocate_exact_size(
					egui::vec2(ui.available_width(), 1.0),
					egui::Sense::hover(),
				);
				ui.painter().rect_filled(rule, 0, colors.border);
				egui::Frame::new()
					.inner_margin(egui::Margin::symmetric(12, 10))
					.show(ui, |ui| {
						ui.set_width(ui.available_width());
						ui.horizontal(|ui| {
							ui.spacing_mut().item_spacing.x = 8.0;
							let mut inserted = None;
							emoji.unicode_button_with(ui, &mut inserted, false);
							if let Some(text) = inserted {
								// This composer tracks no caret, so a pick lands at the end.
								crate::emoji_picker::insert(
									&mut draft.body,
									&text,
									None,
									MAX_CONTENT,
								);
							}
							ui.with_layout(
								egui::Layout::right_to_left(egui::Align::Center),
								|ui| {
									let ready = state.can_create_post(forum)
										&& !draft.title.trim().is_empty()
										&& (!draft.body.trim().is_empty()
											|| !staged.files.is_empty());
									let post = ui.add_enabled(
										ready && !draft.submitted && !posting,
										egui::Button::new(
											design::medium(ui, "Post", 14.0)
												.color(colors.accent_text),
										)
										.fill(colors.accent)
										.stroke(egui::Stroke::NONE)
										.corner_radius(8)
										.min_size(egui::vec2(96.0, 34.0)),
									);
									submit = post.clicked();
									if posting {
										ui.label(
											RichText::new(crate::tr_ui!(ui, "Posting…"))
												.color(colors.muted),
										);
									} else if let Some(error) = error {
										ui.label(RichText::new(error).color(colors.danger));
									}
									ui.label(
										RichText::new(format!(
											"{}/{MAX_TITLE} · {}/{MAX_CONTENT}",
											draft.title.chars().count(),
											draft.body.chars().count()
										))
										.size(11.0)
										.color(colors.muted),
									);
								},
							);
						});
					});
			});
		if cancel {
			self.discard_draft(staged);
			state.posting.error = None;
		} else if submit {
			let names: Vec<&str> = staged.files.iter().map(|(name, _)| name.as_str()).collect();
			if let Some(command) = state.create_post_with_attachments(
				forum,
				&draft.title,
				&draft.body,
				&names,
				&draft.tags,
			) {
				draft.submitted = true;
				commands.push(command);
			}
		}
	}
}

/// Chosen files above the actions, in the same cards the message composer uses.
fn tray(ui: &mut egui::Ui, staged: &mut Staged<'_>) {
	egui::ScrollArea::horizontal()
		.id_salt("forum-post-attachments")
		.show(ui, |ui| {
			ui.horizontal(|ui| {
				ui.spacing_mut().item_spacing.x = 12.0;
				for (index, (filename, bytes)) in staged.files.iter().enumerate() {
					ui.push_id(index, |ui| {
						if crate::attachments::pending_card(
							ui,
							filename,
							*bytes,
							staged.textures.get(index).and_then(Option::as_ref),
							!staged.busy,
						) {
							*staged.remove = Some(index);
						}
					});
				}
			});
		});
}

/// One post's rendered card: its response plus the starter image a click opened.
struct CardHit {
	response: egui::Response,
	image: Option<usize>,
}

/// A post's title; read posts stay quiet and unread ones bright, like Discord.
fn title_row(ui: &mut egui::Ui, post: &Channel, unread: bool, changed: bool, size: f32) {
	let colors = design::palette(ui);
	ui.horizontal(|ui| {
		ui.spacing_mut().item_spacing.x = 8.0;
		if unread {
			let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
			ui.painter()
				.circle_filled(rect.center(), 4.0, colors.text_strong);
		}
		ui.add(
			egui::Label::new(if unread {
				design::semibold(ui, &post.name, size).color(colors.text_strong)
			} else {
				design::medium(ui, &post.name, size).color(colors.muted)
			})
			.truncate()
			.selectable(false),
		);
		// A background refresh marked this card as changed since the cached snapshot.
		if changed {
			ui.label(design::medium(ui, crate::tr_ui!(ui, "Updated"), 11.0).color(colors.accent));
		}
	});
}

/// "Author: starter text" under the title.
fn starter_row(ui: &mut egui::Ui, state: &State, post: &Channel, size: f32) {
	let colors = design::palette(ui);
	let Some(starter) = state.post_preview(post.id) else {
		ui.label(
			RichText::new(crate::tr_ui!(ui, "Latest message unavailable"))
				.size(size)
				.color(colors.muted),
		);
		return;
	};
	let excerpt = if starter.excerpt == "Spoiler content - open the post to reveal" {
		crate::tr_ui!(ui, "Spoiler content - open the post to reveal").to_owned()
	} else if starter.excerpt.trim().is_empty() {
		crate::tr_ui!(ui, "Attachment or non-text message").to_owned()
	} else {
		starter.excerpt.clone()
	};
	ui.horizontal(|ui| {
		ui.spacing_mut().item_spacing.x = 5.0;
		let author_color = state
			.forum_author_color(post.id, starter.author_id, starter.webhook, &starter.roles)
			.map_or(colors.text_strong, |rgb| {
				design::role_name_color(rgb, colors.raised, colors.text_strong)
			});
		if let Some((primary, secondary)) =
			state.forum_author_gradient(post.id, starter.author_id, starter.webhook, &starter.roles)
		{
			let mut job = design::role_gradient_job(
				ui,
				&starter.author,
				primary,
				secondary,
				colors.raised,
				colors.text_strong,
				size,
			);
			job.append(
				":",
				0.0,
				egui::text::TextFormat {
					font_id: egui::FontId::new(size, design::semibold_family(ui.ctx())),
					color: colors.text_strong,
					..Default::default()
				},
			);
			ui.label(job);
		} else {
			ui.label(
				design::semibold(ui, format!("{}:", starter.author), size).color(author_color),
			);
		}
		ui.add(
			egui::Label::new(RichText::new(excerpt).size(size).color(colors.text))
				.truncate()
				.selectable(false),
		);
	});
}

/// Reaction, reply count, new badge and age along a card's bottom edge.
fn stats_row(
	ui: &mut egui::Ui,
	state: &State,
	images: &mut Avatars,
	post: &Channel,
	(archived, unread): (bool, bool),
	now: time::OffsetDateTime,
) {
	let colors = design::palette(ui);
	ui.horizontal(|ui| {
		ui.spacing_mut().item_spacing.x = 6.0;
		if is_pinned(post) {
			icons::inline(ui, icons::Icon::Pin, 14.0, colors.muted);
		}
		if let Some(reaction) = state.post_reaction(post) {
			reaction_chip(ui, images, state.demo, reaction);
			ui.add_space(4.0);
		}
		if let Some(count) = post.message_count {
			icons::inline(ui, icons::Icon::Forum, 16.0, colors.muted);
			ui.label(design::medium(ui, count.to_string(), 13.0).color(colors.text));
		}
		if unread {
			ui.label(design::medium(ui, crate::tr_ui!(ui, "(New)"), 13.0).color(colors.accent));
		}
		ui.label(RichText::new("·").color(colors.muted));
		ui.label(
			RichText::new(format!(
				"{} {}",
				crate::tr_ui!(ui, "Posted"),
				ago_label(ui, post.id, now)
			))
			.size(13.0)
			.color(colors.muted),
		);
		if archived {
			ui.label(RichText::new("·").color(colors.muted));
			ui.label(
				RichText::new(crate::tr_ui!(ui, "Archived"))
					.size(13.0)
					.color(colors.muted),
			);
		}
	});
}

/// The reaction a card shows: an emoji and its count on a quiet pill.
fn reaction_chip(ui: &mut egui::Ui, images: &mut Avatars, demo: bool, reaction: &model::Reaction) {
	let colors = design::palette(ui);
	let height = 24.0;
	let galley = ui.painter().layout_no_wrap(
		reaction.count.to_string(),
		egui::FontId::new(12.0, design::semibold_family(ui.ctx())),
		if reaction.me {
			colors.text_strong
		} else {
			colors.text
		},
	);
	let emoji = 16.0;
	let (rect, response) = ui.allocate_exact_size(
		egui::vec2(8.0 + emoji + 6.0 + galley.size().x + 10.0, height),
		egui::Sense::hover(),
	);
	response.widget_info(|| {
		egui::WidgetInfo::labeled(
			egui::Role::Label,
			true,
			format!("{} {}", reaction.emoji.label(), reaction.count),
		)
	});
	if !ui.is_rect_visible(rect) {
		return;
	}
	ui.painter().rect(
		rect,
		7,
		if reaction.me {
			colors.accent.gamma_multiply(0.18)
		} else {
			colors.sidebar
		},
		egui::Stroke::new(
			1.0,
			if reaction.me {
				colors.accent
			} else {
				colors.border
			},
		),
		egui::StrokeKind::Inside,
	);
	paint_emoji(
		ui,
		images,
		demo,
		(reaction.emoji.id, reaction.emoji.name.as_deref()),
		egui::Rect::from_min_size(
			egui::pos2(rect.left() + 8.0, rect.center().y - emoji / 2.0),
			egui::Vec2::splat(emoji),
		),
	);
	let color = if reaction.me {
		colors.text_strong
	} else {
		colors.text
	};
	ui.painter().galley(
		egui::pos2(
			rect.left() + 8.0 + emoji + 6.0,
			rect.center().y - galley.size().y / 2.0,
		),
		galley,
		color,
	);
}

/// Paint a custom emoji from the image cache, or a Unicode one from the Twemoji atlas.
fn paint_emoji(
	ui: &egui::Ui,
	images: &mut Avatars,
	demo: bool,
	(id, name): (Option<Id>, Option<&str>),
	rect: egui::Rect,
) {
	let image = match id {
		Some(id) => images.custom_image(ui.ctx(), id, rect.width(), demo),
		None => name.and_then(|name| crate::emoji::image(ui.ctx(), name, rect.width())),
	};
	if let Some(image) = image {
		image.paint_at(ui, rect);
	}
}

/// Flip one tag in a selection, never exceeding the forum's limit or repeating.
fn toggle(tags: &mut Vec<Id>, id: Id) {
	if let Some(index) = tags.iter().position(|known| *known == id) {
		tags.remove(index);
	} else if tags.len() < model::forum::MAX_APPLIED_TAGS {
		tags.push(id);
	}
}

/// Does a post carry the selected tags? Every one with `all`, any of them otherwise.
fn carries(post: &Channel, selected: &[Id], all: bool) -> bool {
	if selected.is_empty() {
		return true;
	}
	let applied = post.tags.as_deref().map_or(&[][..], |tags| &tags.applied);
	if all {
		selected.iter().all(|id| applied.contains(id))
	} else {
		selected.iter().any(|id| applied.contains(id))
	}
}

fn action_width(ui: &egui::Ui, label: &str, height: f32, icons: usize) -> f32 {
	let text = ui
		.painter()
		.layout_no_wrap(label.to_owned(), tag_font(ui, height), egui::Color32::WHITE)
		.size()
		.x;
	(height * 0.45).round() * 2.0 + text + icons as f32 * ((height * 0.5).round() + 6.0)
}

/// A pill button with an optional icon before and after its label.
fn action_pill(
	ui: &mut egui::Ui,
	(leading, trailing): (Option<icons::Icon>, Option<icons::Icon>),
	label: &str,
	height: f32,
	active: bool,
) -> egui::Response {
	let colors = design::palette(ui);
	let count = usize::from(leading.is_some()) + usize::from(trailing.is_some());
	let (rect, response) = ui.allocate_exact_size(
		egui::vec2(action_width(ui, label, height, count), height),
		egui::Sense::click(),
	);
	let enabled = ui.is_enabled();
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
	if !ui.is_rect_visible(rect) {
		return response;
	}
	let hot = enabled && (response.hovered() || response.has_focus());
	let fill = if active || response.is_pointer_button_down_on() {
		colors.selected
	} else if hot {
		colors.hover
	} else {
		colors.raised
	};
	let text = match (enabled, active || hot) {
		(false, _) => colors.muted.gamma_multiply(0.5),
		(true, true) => colors.text_strong,
		(true, false) => colors.muted,
	};
	let painter = ui.painter();
	painter.rect(
		rect,
		height / 2.0,
		fill,
		egui::Stroke::new(1.0, colors.border),
		egui::StrokeKind::Inside,
	);
	if response.has_focus() {
		painter.rect_stroke(
			rect.expand(2.0),
			height / 2.0 + 2.0,
			egui::Stroke::new(2.0, colors.accent),
			egui::StrokeKind::Outside,
		);
	}
	let icon_size = (height * 0.5).round();
	let mut x = rect.left() + (height * 0.45).round();
	let icon_rect = |x: f32| {
		egui::Rect::from_min_size(
			egui::pos2(x, rect.center().y - icon_size / 2.0),
			egui::Vec2::splat(icon_size),
		)
	};
	if let Some(icon) = leading {
		icons::paint(painter, icon, icon_rect(x), text);
		x += icon_size + 6.0;
	}
	let galley = painter.layout_no_wrap(label.to_owned(), tag_font(ui, height), text);
	let width = galley.size().x;
	painter.galley(
		egui::pos2(x, rect.center().y - galley.size().y / 2.0),
		galley,
		text,
	);
	if let Some(icon) = trailing {
		icons::paint(painter, icon, icon_rect(x + width + 6.0), text);
	}
	response
}

/// The accent count beside "Select tags".
fn count_badge(ui: &mut egui::Ui, count: usize) {
	let colors = design::palette(ui);
	let galley = ui.painter().layout_no_wrap(
		count.to_string(),
		egui::FontId::new(12.0, design::semibold_family(ui.ctx())),
		colors.accent_text,
	);
	let (rect, _) = ui.allocate_exact_size(
		egui::vec2((galley.size().x + 10.0).max(20.0), 20.0),
		egui::Sense::hover(),
	);
	ui.painter().rect_filled(rect, 10, colors.accent);
	ui.painter().galley(
		rect.center() - galley.size() / 2.0,
		galley,
		colors.accent_text,
	);
}

fn tag_font(ui: &egui::Ui, height: f32) -> egui::FontId {
	egui::FontId::new(
		if height < TAG_HEIGHT { 12.0 } else { 14.0 },
		design::semibold_family(ui.ctx()),
	)
}

/// Does this emoji have artwork to paint: a custom emoji, or a Unicode one in the atlas?
fn has_emoji(ctx: &egui::Context, id: Option<Id>, name: Option<&str>) -> bool {
	id.is_some()
		|| name.is_some_and(|name| crate::emoji::lookup(name).is_some() && crate::emoji::ready(ctx))
}

fn pill_width(ui: &egui::Ui, tag: &Tag, height: f32) -> f32 {
	let text = ui
		.painter()
		.layout_no_wrap(tag.name.clone(), tag_font(ui, height), egui::Color32::WHITE)
		.size()
		.x;
	let emoji = if has_emoji(ui.ctx(), tag.emoji_id, tag.emoji_name.as_deref()) {
		(height * 0.6).round() + 6.0
	} else {
		0.0
	};
	(height * 0.45).round() * 2.0 + text + emoji
}

/// One rounded tag with its emoji: accent-filled when selected, quiet otherwise.
fn tag_pill(
	ui: &mut egui::Ui,
	(images, demo): (&mut Avatars, bool),
	tag: &Tag,
	selected: bool,
	height: f32,
	sense: egui::Sense,
) -> egui::Response {
	let colors = design::palette(ui);
	let (rect, response) =
		ui.allocate_exact_size(egui::vec2(pill_width(ui, tag, height), height), sense);
	let enabled = ui.is_enabled();
	let interactive = sense.senses_click();
	if interactive {
		response.widget_info(|| {
			egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, selected, &tag.name)
		});
	}
	if !ui.is_rect_visible(rect) {
		return response;
	}
	let hot = interactive && enabled && (response.hovered() || response.has_focus());
	let (fill, stroke, text) = if selected {
		(colors.accent, colors.accent, colors.accent_text)
	} else if hot {
		(colors.hover, colors.border, colors.text_strong)
	} else {
		(colors.raised, colors.border, colors.text_strong)
	};
	let text = if enabled {
		text
	} else {
		text.gamma_multiply(0.45)
	};
	ui.painter().rect(
		rect,
		height / 2.0,
		fill,
		egui::Stroke::new(1.0, stroke),
		egui::StrokeKind::Inside,
	);
	if response.has_focus() {
		ui.painter().rect_stroke(
			rect.expand(2.0),
			height / 2.0 + 2.0,
			egui::Stroke::new(2.0, colors.accent),
			egui::StrokeKind::Outside,
		);
	}
	let mut x = rect.left() + (height * 0.45).round();
	if has_emoji(ui.ctx(), tag.emoji_id, tag.emoji_name.as_deref()) {
		let size = (height * 0.6).round();
		paint_emoji(
			ui,
			images,
			demo,
			(tag.emoji_id, tag.emoji_name.as_deref()),
			egui::Rect::from_min_size(
				egui::pos2(x, rect.center().y - size / 2.0),
				egui::Vec2::splat(size),
			),
		);
		x += size + 6.0;
	}
	let galley = ui
		.painter()
		.layout_no_wrap(tag.name.clone(), tag_font(ui, height), text);
	ui.painter().galley(
		egui::pos2(x, rect.center().y - galley.size().y / 2.0),
		galley,
		text,
	);
	response
}

/// A post's tags in the forum's order, folding extras past `limit` into "+N" like Discord.
fn card_tags(ui: &mut egui::Ui, (images, demo): (&mut Avatars, bool), tags: &[&Tag], limit: usize) {
	ui.horizontal(|ui| {
		ui.spacing_mut().item_spacing.x = 6.0;
		for tag in tags.iter().take(limit) {
			tag_pill(
				ui,
				(images, demo),
				tag,
				false,
				CARD_TAG_HEIGHT,
				egui::Sense::hover(),
			);
		}
		if tags.len() > limit {
			let more = Tag {
				id: Id(0),
				name: format!("+{}", tags.len() - limit),
				moderated: false,
				emoji_id: None,
				emoji_name: None,
			};
			tag_pill(
				ui,
				(images, demo),
				&more,
				false,
				CARD_TAG_HEIGHT,
				egui::Sense::hover(),
			)
			.on_hover_text(
				tags[limit..]
					.iter()
					.map(|tag| tag.name.as_str())
					.collect::<Vec<_>>()
					.join(", "),
			);
		}
	});
}

/// The composer's tag picker: applied tags plus a menu of everything the forum offers.
fn post_tags(
	ui: &mut egui::Ui,
	state: &State,
	forum: Id,
	picked: &mut Vec<Id>,
	images: &mut Avatars,
) {
	let offered = state.forum_tags(forum);
	if offered.is_empty() {
		return;
	}
	let colors = design::palette(ui);
	let demo = state.demo;
	let required = state.forum_requires_tag(forum);
	picked.retain(|id| offered.iter().any(|tag| tag.id == *id));
	ui.add_space(6.0);
	ui.horizontal_wrapped(|ui| {
		ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
		let mut removed = None;
		for tag in offered.iter().filter(|tag| picked.contains(&tag.id)) {
			if tag_pill(
				ui,
				(images, demo),
				tag,
				true,
				CARD_TAG_HEIGHT,
				egui::Sense::click(),
			)
			.on_hover_text(crate::tr_ui!(ui, "Remove tag"))
			.clicked()
			{
				removed = Some(tag.id);
			}
		}
		picked.retain(|id| Some(*id) != removed);
		let full = picked.len() >= model::forum::MAX_APPLIED_TAGS;
		let button = ui
			.add_enabled_ui(!full, |ui| {
				action_pill(
					ui,
					(Some(icons::Icon::Plus), None),
					if picked.is_empty() {
						crate::tr_ui!(ui, "Add tags")
					} else {
						crate::tr_ui!(ui, "Add tag")
					},
					CARD_TAG_HEIGHT,
					false,
				)
			})
			.inner;
		let button = if full {
			button.on_disabled_hover_text(crate::tr_ui!(ui, "A post can carry up to 5 tags"))
		} else {
			button
		};
		if required && picked.is_empty() {
			ui.label(
				RichText::new(crate::tr_ui!(ui, "This forum requires a tag"))
					.size(12.0)
					.color(colors.muted),
			);
		}
		egui::Popup::menu(&button)
			.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
			.show(|ui| {
				ui.set_max_width(360.0);
				ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
				ui.horizontal(|ui| {
					ui.label(
						design::semibold(ui, crate::tr_ui!(ui, "Select tags"), 15.0)
							.color(colors.muted),
					);
					count_badge(ui, picked.len());
					ui.label(
						RichText::new(format!(
							"{} {}",
							crate::tr_ui!(ui, "Up to"),
							model::forum::MAX_APPLIED_TAGS
						))
						.size(12.0)
						.color(colors.muted),
					);
				});
				ui.horizontal_wrapped(|ui| {
					for tag in offered {
						let selected = picked.contains(&tag.id);
						let allowed = selected
							|| (picked.len() < model::forum::MAX_APPLIED_TAGS
								&& state.can_apply_tag(forum, tag));
						let pill = ui
							.add_enabled_ui(allowed, |ui| {
								tag_pill(
									ui,
									(images, demo),
									tag,
									selected,
									TAG_HEIGHT,
									egui::Sense::click(),
								)
							})
							.inner;
						let pill = if tag.moderated && !state.can_apply_tag(forum, tag) {
							pill.on_disabled_hover_text(crate::tr_ui!(
								ui,
								"Only moderators can apply this tag"
							))
						} else {
							pill
						};
						if pill.clicked() {
							toggle(picked, tag.id);
						}
					}
				});
			});
	});
}

fn describe(response: &egui::Response, post: &Channel, (archived, unread): (bool, bool)) {
	response.widget_info(|| {
		egui::WidgetInfo::labeled(
			egui::Role::Button,
			true,
			format!(
				"{}{}{}; {} replies",
				post.name,
				if unread { ", unread" } else { "" },
				if archived { ", archived" } else { "" },
				post.message_count
					.map_or("unknown".to_owned(), |n| n.to_string())
			),
		)
	});
}

/// One mosaic cell's click target.
struct MosaicCell {
	index: usize,
	rect: egui::Rect,
	response: egui::Response,
}

/// Cell rects for `count` starter images inside `size`, starting at `origin`.
fn mosaic_rects(origin: egui::Pos2, size: egui::Vec2, count: usize) -> Vec<egui::Rect> {
	let size = egui::vec2(size.x.max(1.0), size.y.max(1.0));
	let gap = MOSAIC_GAP;
	let left = ((size.x - gap) * 0.5).floor();
	let top = ((size.y - gap) * 0.5).floor();
	match count {
		0 => Vec::new(),
		1 => vec![egui::Rect::from_min_size(origin, size)],
		2 => vec![
			egui::Rect::from_min_size(origin, egui::vec2(left, size.y)),
			egui::Rect::from_min_size(
				origin + egui::vec2(left + gap, 0.0),
				egui::vec2(size.x - left - gap, size.y),
			),
		],
		3 => vec![
			egui::Rect::from_min_size(origin, egui::vec2(left, size.y)),
			egui::Rect::from_min_size(
				origin + egui::vec2(left + gap, 0.0),
				egui::vec2(size.x - left - gap, top),
			),
			egui::Rect::from_min_size(
				origin + egui::vec2(left + gap, top + gap),
				egui::vec2(size.x - left - gap, size.y - top - gap),
			),
		],
		_ => vec![
			egui::Rect::from_min_size(origin, egui::vec2(left, top)),
			egui::Rect::from_min_size(
				origin + egui::vec2(left + gap, 0.0),
				egui::vec2(size.x - left - gap, top),
			),
			egui::Rect::from_min_size(
				origin + egui::vec2(0.0, top + gap),
				egui::vec2(left, size.y - top - gap),
			),
			egui::Rect::from_min_size(
				origin + egui::vec2(left + gap, top + gap),
				egui::vec2(size.x - left - gap, size.y - top - gap),
			),
		],
	}
}

/// Paint the starter's first images as Discord's mosaic, cropped to fill each cell.
fn mosaic(
	ui: &mut egui::Ui,
	images: &mut Avatars,
	starter: &Starter,
	size: egui::Vec2,
	demo: bool,
) -> Vec<MosaicCell> {
	let arts = &starter.images;
	if arts.is_empty() {
		return Vec::new();
	}
	let size = egui::vec2(size.x.max(1.0), size.y.max(1.0));
	let rects = mosaic_rects(ui.cursor().min, size, arts.len());
	let mut cells = Vec::with_capacity(arts.len());
	for (index, rect) in rects.into_iter().enumerate() {
		let mut cell = ui.new_child(
			egui::UiBuilder::new()
				.max_rect(rect)
				.layout(egui::Layout::top_down(egui::Align::Min)),
		);
		let shown = images.show_media(
			&mut cell,
			&arts[index].media,
			rect.size(),
			demo,
			crate::avatars::Surface::Tile,
		);
		let response = cell.interact(
			shown.response.rect,
			shown.response.id.with(("mosaic", index)),
			egui::Sense::click(),
		);
		cells.push(MosaicCell {
			index,
			rect,
			response,
		});
	}
	ui.allocate_exact_size(size, egui::Sense::hover());
	cells
}

/// Spoiler covers, a play badge for video and the "+N" counter of a folded mosaic.
fn paint_mosaic_badges(ui: &egui::Ui, starter: &Starter, cells: &[MosaicCell]) {
	for cell in cells {
		let image = &starter.images[cell.index];
		if image.spoiler {
			ui.painter()
				.rect_filled(cell.rect, 8, egui::Color32::from_black_alpha(232));
			ui.painter().text(
				cell.rect.center(),
				egui::Align2::CENTER_CENTER,
				crate::tr_ui!(ui, "Spoiler"),
				egui::FontId::proportional(12.0),
				egui::Color32::WHITE,
			);
		}
		if image.video {
			let center = cell.rect.center();
			let radius = (cell.rect.width() * 0.18).clamp(8.0, 14.0);
			ui.painter()
				.circle_filled(center, radius, egui::Color32::from_black_alpha(150));
			let side = radius * 0.75;
			ui.painter().add(egui::Shape::convex_polygon(
				vec![
					center + egui::vec2(-side * 0.4, -side * 0.6),
					center + egui::vec2(-side * 0.4, side * 0.6),
					center + egui::vec2(side * 0.7, 0.0),
				],
				egui::Color32::WHITE,
				egui::Stroke::NONE,
			));
		}
		if image.animated {
			let badge = egui::Rect::from_min_size(
				cell.rect.left_bottom() + egui::vec2(6.0, -20.0),
				egui::vec2(30.0, 14.0),
			);
			ui.painter()
				.rect_filled(badge, 4, egui::Color32::from_black_alpha(170));
			ui.painter().text(
				badge.center(),
				egui::Align2::CENTER_CENTER,
				"GIF",
				egui::FontId::proportional(10.0),
				egui::Color32::WHITE,
			);
		}
	}
	let total = usize::from(starter.image_count);
	if total > cells.len()
		&& let Some(last) = cells.last()
	{
		ui.painter()
			.rect_filled(last.rect, 8, egui::Color32::from_black_alpha(168));
		ui.painter().text(
			last.rect.center(),
			egui::Align2::CENTER_CENTER,
			format!("+{}", total - cells.len()),
			egui::FontId::proportional(22.0),
			egui::Color32::WHITE,
		);
	}
}

/// List view: one full-width card per post, with the starter's first image on the right.
fn card(
	ui: &mut egui::Ui,
	state: &State,
	images: &mut Avatars,
	post: &Channel,
	flags: (bool, bool),
	now: time::OffsetDateTime,
) -> CardHit {
	let mut image = None;
	let response = ui
		.scope_builder(
			egui::UiBuilder::new()
				.id_salt(("post", post.id, flags.0))
				.sense(egui::Sense::click()),
			|ui| {
				let response = ui.response();
				design::interactive_card_frame(ui, &response)
					.inner_margin(egui::Margin::symmetric(16, 14))
					.show(ui, |ui| {
						ui.set_width(ui.available_width());
						let thumb = state
							.post_preview(post.id)
							.filter(|starter| !starter.images.is_empty());
						ui.horizontal_top(|ui| {
							ui.spacing_mut().item_spacing.x = 12.0;
							let width = ui.available_width()
								- if thumb.is_some() { PREVIEW + 12.0 } else { 0.0 };
							ui.vertical(|ui| {
								ui.set_width(width.max(120.0));
								ui.spacing_mut().item_spacing.y = 6.0;
								let tags = state.post_tags(post);
								if !tags.is_empty() {
									card_tags(ui, (images, state.demo), &tags, CARD_TAGS);
								}
								title_row(ui, post, flags.1, state.post_changed(post.id), 16.0);
								starter_row(ui, state, post, 14.0);
								stats_row(ui, state, images, post, flags, now);
							});
							// Discord shows the starter's first image at the card's right edge.
							if let Some(starter) = thumb {
								let art = egui::Vec2::splat(PREVIEW);
								// The next screen's cards warm their thumbnails before they scroll in.
								let clip = ui.clip_rect();
								let art_top = ui.cursor().min.y;
								if art_top >= clip.bottom()
									&& art_top <= clip.bottom() + clip.height().max(240.0)
								{
									images.prefetch_media(
										&starter.images[0].media,
										art,
										ui.ctx(),
										state.demo,
										crate::avatars::Surface::Tile,
									);
								}
								let (rect, _) = ui.allocate_exact_size(art, egui::Sense::hover());
								let mut cell = ui.new_child(
									egui::UiBuilder::new()
										.max_rect(rect)
										.layout(egui::Layout::top_down(egui::Align::Min)),
								);
								let shown = images.show_media(
									&mut cell,
									&starter.images[0].media,
									art,
									state.demo,
									crate::avatars::Surface::Tile,
								);
								let hit = cell.interact(
									shown.response.rect,
									shown.response.id.with("starter-preview"),
									egui::Sense::click(),
								);
								if hit.clicked() {
									image = Some(0);
								}
							}
						});
					});
			},
		)
		.response;
	describe(&response, post, flags);
	CardHit { response, image }
}

/// Gallery view: a tile led by the starter mosaic, or a quiet stand-in when it has none.
fn tile(
	ui: &mut egui::Ui,
	state: &State,
	images: &mut Avatars,
	post: &Channel,
	flags: (bool, bool),
	(width, now): (f32, time::OffsetDateTime),
) -> CardHit {
	let colors = design::palette(ui);
	let mut image = None;
	let response = ui
		.scope_builder(
			egui::UiBuilder::new()
				.id_salt(("tile", post.id, flags.0))
				.layout(egui::Layout::top_down(egui::Align::Min))
				.sense(egui::Sense::click()),
			|ui| {
				let response = ui.response();
				design::interactive_card_frame(ui, &response)
					.inner_margin(egui::Margin::same(8))
					.show(ui, |ui| {
						let inner = width - 18.0;
						ui.set_width(inner);
						ui.set_max_width(inner);
						ui.spacing_mut().item_spacing.y = 8.0;
						let art = egui::vec2(inner, (inner * TILE_ART).round());
						match state.post_preview(post.id) {
							Some(starter) if !starter.images.is_empty() => {
								// Cards one screen below the viewport warm their thumbnails now, at the
								// lowest priority, so scrolling never waits on the network.
								let clip = ui.clip_rect();
								let art_top = ui.cursor().min.y;
								if art_top >= clip.bottom()
									&& art_top <= clip.bottom() + clip.height().max(240.0)
								{
									for image in &starter.images {
										images.prefetch_media(
											&image.media,
											art,
											ui.ctx(),
											state.demo,
											crate::avatars::Surface::Tile,
										);
									}
								}
								let cells = mosaic(ui, images, starter, art, state.demo);
								paint_mosaic_badges(ui, starter, &cells);
								if let Some(cell) =
									cells.iter().find(|cell| cell.response.clicked())
								{
									image = Some(cell.index);
								}
							}
							_ => {
								let (rect, _) = ui.allocate_exact_size(art, egui::Sense::hover());
								ui.painter().rect_filled(rect, 8, colors.sidebar);
								icons::paint(
									ui.painter(),
									icons::Icon::Forum,
									egui::Rect::from_center_size(
										rect.center(),
										egui::Vec2::splat(36.0),
									),
									colors.muted.gamma_multiply(0.6),
								);
							}
						}
						egui::Frame::new()
							.inner_margin(egui::Margin::symmetric(6, 0))
							.show(ui, |ui| {
								ui.set_width(inner - 12.0);
								ui.spacing_mut().item_spacing.y = 6.0;
								// Every tile keeps a tag row so a gallery row lines up.
								let tags = state.post_tags(post);
								if tags.is_empty() {
									ui.allocate_exact_size(
										egui::vec2(1.0, CARD_TAG_HEIGHT),
										egui::Sense::hover(),
									);
								} else {
									card_tags(ui, (images, state.demo), &tags, 2);
								}
								title_row(ui, post, flags.1, state.post_changed(post.id), 15.0);
								starter_row(ui, state, post, 14.0);
								stats_row(ui, state, images, post, flags, now);
							});
					});
			},
		)
		.response;
	describe(&response, post, flags);
	CardHit { response, image }
}

/// Lay out one group of posts as list cards or gallery tiles.
fn posts_view(
	ui: &mut egui::Ui,
	state: &State,
	images: &mut Avatars,
	posts: &[&Channel],
	layout: Layout,
	now: time::OffsetDateTime,
) -> Vec<CardHit> {
	let flags = |post: &Channel| (false, state.post_unread(post));
	match layout {
		Layout::List => posts
			.iter()
			.map(|post| card(ui, state, images, post, flags(post), now))
			.collect(),
		Layout::Gallery => {
			// A scroll area can remember a wider content size from an earlier frame; the
			// visible clip is the truth, so a gallery row never spills sideways.
			let width = ui.clip_rect().width().min(ui.available_width());
			let columns =
				(((width + TILE_GAP) / (GALLERY_TILE + TILE_GAP)).floor() as usize).max(1);
			let tile_width = ((width - TILE_GAP * (columns - 1) as f32) / columns as f32).floor();
			let mut hits = Vec::with_capacity(posts.len());
			for row in posts.chunks(columns) {
				ui.horizontal_top(|ui| {
					ui.spacing_mut().item_spacing.x = TILE_GAP;
					for post in row {
						hits.push(tile(
							ui,
							state,
							images,
							post,
							flags(post),
							(tile_width, now),
						));
					}
				});
			}
			hits
		}
	}
}

/// A synthetic attachment record for one starter image, so the shared viewer can show it.
fn starter_attachment((index, image): (usize, &StarterImage)) -> model::Attachment {
	model::Attachment {
		id: if image.id.0 > 0 {
			image.id
		} else {
			Id(u64::MAX - index as u64)
		},
		filename: format!("starter-{index}.png"),
		description: None,
		content_type: Some("image/png".into()),
		size: image.size,
		media: image.media.clone(),
		spoiler: image.spoiler,
		duration_ms: None,
		waveform: Vec::new(),
	}
}
/// Active-post status under the list; returns true when the user asks for another page.
fn posts_footer(ui: &mut egui::Ui, state: &State, forum: Id) -> bool {
	let colors = design::palette(ui);
	if state.posts.parent != Some(forum) {
		return false;
	}
	let mut request = false;
	ui.horizontal_wrapped(|ui| {
		if state.posts.loading {
			ui.label(
				RichText::new(crate::tr_ui!(ui, "Loading posts…"))
					.size(13.0)
					.color(colors.muted),
			);
		} else if let Some(error) = state.posts.error {
			ui.label(RichText::new(error).size(13.0).color(colors.danger));
			request = ui
				.add_enabled(
					state.can_load_posts(forum),
					egui::Button::new(RichText::new(crate::tr_ui!(ui, "Retry")).size(13.0)),
				)
				.clicked();
		} else if state.posts.more {
			request = ui
				.add(
					egui::Button::new(
						RichText::new(crate::tr_ui!(ui, "Load more posts"))
							.size(13.0)
							.color(colors.link),
					)
					.frame(false),
				)
				.clicked();
		}
	});
	// A fallback that took long is explained once the list is on screen, discreetly.
	if state.posts.fallback.is_some_and(|report| report.slow()) {
		ui.label(
			RichText::new(crate::tr_ui!(
				ui,
				"This forum is large; loading another way…"
			))
			.size(12.0)
			.color(colors.muted),
		);
	}
	request
}

/// Archive controls under the list; returns a page cursor request when the user asks for one.
fn archive_footer(
	ui: &mut egui::Ui,
	state: &State,
	forum: Id,
	view: Option<&client_core::archives::View>,
) -> Option<Option<model::archives::Cursor>> {
	let colors = design::palette(ui);
	let allowed = state.can_archive(forum, Kind::Public);
	let mut request = None;
	ui.horizontal_wrapped(|ui| match view {
		None => {
			let button = ui.add_enabled(
				allowed,
				egui::Button::new(
					RichText::new(crate::tr_ui!(ui, "Load archived posts"))
						.size(13.0)
						.color(colors.link),
				)
				.frame(false),
			);
			if button.clicked() {
				request = Some(None);
			}
			if !allowed {
				ui.label(
					RichText::new(crate::tr_ui!(
						ui,
						"Archived posts need a connected session with history access."
					))
					.size(12.0)
					.color(colors.muted),
				);
			}
		}
		Some(view) if view.loading => {
			ui.label(
				RichText::new(crate::tr_ui!(ui, "Loading archived posts…"))
					.size(13.0)
					.color(colors.muted),
			);
		}
		Some(view) => {
			if let Some(error) = view.error {
				ui.label(RichText::new(error).size(13.0).color(colors.danger));
				if ui
					.add_enabled(
						allowed,
						egui::Button::new(RichText::new(crate::tr_ui!(ui, "Retry")).size(13.0)),
					)
					.clicked()
				{
					request = Some(view.before);
				}
			} else if let Some(page) = &view.page {
				if let Some(next) = page.next {
					if ui
						.add_enabled(
							allowed,
							egui::Button::new(
								RichText::new(crate::tr_ui!(ui, "Older archived posts"))
									.size(13.0)
									.color(colors.link),
							)
							.frame(false),
						)
						.clicked()
					{
						request = Some(Some(next));
					}
				} else {
					ui.label(
						RichText::new(crate::tr_ui!(ui, "No older archived posts reported."))
							.size(12.0)
							.color(colors.muted),
					);
				}
			}
		}
	});
	request
}

// Discord snowflakes carry milliseconds since 2015-01-01; all u64 IDs fit time's range.
/// The post's age as `(value, unit)`; `"now"` reads as just now, the rest take "ago".
fn ago_parts(id: Id, now: time::OffsetDateTime) -> (u64, &'static str) {
	let created =
		time::OffsetDateTime::from_unix_timestamp(((id.0 >> 22) / 1000) as i64 + 1_420_070_400)
			.expect("snowflake timestamp is in range");
	let seconds = (now - created).whole_seconds().max(0);
	match seconds {
		0..60 => (0, "now"),
		60..3_600 => ((seconds / 60) as u64, "m"),
		3_600..86_400 => ((seconds / 3_600) as u64, "h"),
		86_400..2_592_000 => ((seconds / 86_400) as u64, "d"),
		2_592_000..31_536_000 => ((seconds / 2_592_000) as u64, "mo"),
		_ => ((seconds / 31_536_000) as u64, "y"),
	}
}

/// "2m ago" in the interface language.
fn ago_label(ui: &egui::Ui, id: Id, now: time::OffsetDateTime) -> String {
	let (value, unit) = ago_parts(id, now);
	if unit == "now" {
		return crate::tr_ui!(ui, "just now").to_owned();
	}
	format!("{value}{unit} {}", crate::tr_ui!(ui, "ago"))
}

/// A pinned post keeps its Discord pin badge before the title.
fn is_pinned(post: &Channel) -> bool {
	post.tags.as_deref().is_some_and(|tags| tags.pinned)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Nothing is staged in these fixtures; the tray lives in the messaging view.
	#[derive(Default)]
	struct Scratch {
		choose: bool,
		remove: Option<usize>,
		clear: bool,
	}
	fn staged(scratch: &mut Scratch) -> Staged<'_> {
		Staged {
			files: &[],
			textures: &[],
			choose: &mut scratch.choose,
			remove: &mut scratch.remove,
			clear: &mut scratch.clear,
			busy: false,
		}
	}

	fn frame(ctx: &egui::Context, draw: impl FnMut(&mut egui::Ui)) {
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(720.0, 640.0),
				)),
				..Default::default()
			},
			draw,
		);
		output.drop_without_applying_deltas();
	}

	#[test]
	fn relative_times_round_down() {
		let ctx = egui::Context::default();
		let now = time::OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
		let at = |seconds_ago: i64| {
			Id((((1_800_000_000 - seconds_ago - 1_420_070_400) as u64) * 1000) << 22)
		};
		let label = |id| {
			let mut out = String::new();
			let output = ctx.run_ui(egui::RawInput::default(), |ui| {
				out = ago_label(ui, id, now);
			});
			output.drop_without_applying_deltas();
			out
		};
		assert_eq!(label(at(5)), "just now");
		assert_eq!(label(at(125)), "2m ago");
		assert_eq!(label(at(7_200)), "2h ago");
		assert_eq!(label(at(15 * 86_400)), "15d ago");
		assert_eq!(label(at(70 * 86_400)), "2mo ago");
		assert_eq!(label(at(800 * 86_400)), "2y ago");
	}

	fn rightmost(shape: &egui::Shape, max_x: &mut f32) {
		match shape {
			egui::Shape::Mesh(mesh) => {
				for vertex in &mesh.vertices {
					*max_x = max_x.max(vertex.pos.x);
				}
			}
			egui::Shape::Rect(rect) => *max_x = max_x.max(rect.rect.right()),
			egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| rightmost(shape, max_x)),
			_ => {}
		}
	}

	#[test]
	fn gallery_tiles_fit_the_conversation_panel() {
		let ctx = egui::Context::default();
		let mut view = crate::MessagingUi::default();
		view.reading_preferences.show_members = false;
		let mut state = test_support::forum_gallery_state();
		let mut max_x = 0.0;
		for frame in 0..3 {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1024.0, 768.0),
					)),
					..Default::default()
				},
				|ui| {
					let _ = view.show(ui, &mut state);
				},
			);
			max_x = 0.0;
			for shape in &output.shapes {
				rightmost(&shape.shape, &mut max_x);
			}
			output.drop_without_applying_deltas();
			println!("frame {frame}: content reaches {max_x}");
		}
		assert!(
			max_x <= 1024.0,
			"gallery content reaches {max_x} in a 1024-wide window"
		);
	}

	#[test]
	fn a_refreshed_card_shows_the_updated_marker() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		state.demo = false;
		state.gateway_connected = true;
		state.auth = client_core::auth::AuthState::Authenticated;
		assert!(state.select(Id(26)).is_none());
		state.request_forum_posts(Id(26), false).unwrap();
		// Seed from a cache where every post has an older last activity.
		let cached = model::forum::CachedPage::from_posts(
			state
				.forum_posts(Id(26))
				.into_iter()
				.map(|post| {
					let mut post = post.clone();
					let starter = state.post_preview(post.id).cloned();
					post.last_message = Some(Id(1));
					(post, starter)
				})
				.collect::<Vec<_>>(),
		);
		assert!(state.apply_forum_cache(Id(26), cached));
		// The fresh page advances one post and leaves the others as they were.
		let threads: Vec<Channel> = state
			.forum_posts(Id(26))
			.into_iter()
			.map(|post| {
				let mut post = post.clone();
				post.last_message = if post.id == Id(27) {
					Some(Id(999))
				} else {
					Some(Id(1))
				};
				post
			})
			.collect();
		state.apply_forum_posts(
			Id(26),
			state.posts.request,
			Ok(model::forum::Page {
				threads,
				more: false,
				previews: Vec::new(),
				fallback: None,
			}),
		);
		assert!(state.post_changed(Id(27)));
		assert!(!state.post_changed(Id(41)));
		let mut forum = ForumUi::default();
		let mut commands = Vec::new();
		let mut scratch = Scratch::default();
		let mut rendered = Vec::new();
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1120.0, 900.0),
				)),
				..Default::default()
			},
			|ui| {
				let mut staged = staged(&mut scratch);
				forum.show(
					ui,
					&mut state,
					Id(26),
					&mut commands,
					(
						&mut crate::scroll::Session::default(),
						&mut staged,
						&mut crate::avatars::Avatars::default(),
					),
					(
						&mut crate::channel_menu::ChannelMenu::default(),
						crate::shortcuts::ShortcutView::new(&Default::default(), true),
					),
					model::Language::English,
				);
			},
		);
		for shape in &output.shapes {
			labels(&shape.shape, &mut rendered);
		}
		output.drop_without_applying_deltas();
		assert!(
			rendered.iter().any(|text| text == "Updated"),
			"a changed card carries the marker: {rendered:?}"
		);
	}

	#[test]
	fn a_slow_fallback_shows_a_discreet_notice() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		state.posts.parent = Some(Id(26));
		state.posts.fallback = Some(model::forum::FallbackReport {
			reason: model::forum::FallbackReason::Oversized,
			bytes: 1024,
			elapsed: std::time::Duration::from_millis(3_200),
		});
		let mut forum = ForumUi::default();
		let mut commands = Vec::new();
		let mut scratch = Scratch::default();
		let mut rendered = |state: &mut client_core::State| {
			let mut text = Vec::new();
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1120.0, 900.0),
					)),
					..Default::default()
				},
				|ui| {
					let mut staged = staged(&mut scratch);
					forum.show(
						ui,
						state,
						Id(26),
						&mut commands,
						(
							&mut crate::scroll::Session::default(),
							&mut staged,
							&mut crate::avatars::Avatars::default(),
						),
						(
							&mut crate::channel_menu::ChannelMenu::default(),
							crate::shortcuts::ShortcutView::new(&Default::default(), true),
						),
						model::Language::English,
					);
				},
			);
			for shape in &output.shapes {
				labels(&shape.shape, &mut text);
			}
			output.drop_without_applying_deltas();
			text
		};
		let text = rendered(&mut state);
		assert!(
			text.iter()
				.any(|text| text == "This forum is large; loading another way…"),
			"a slow fallback explains itself: {text:?}"
		);
		// A fast fallback keeps the list quiet.
		state.posts.fallback = Some(model::forum::FallbackReport {
			elapsed: std::time::Duration::from_millis(120),
			..state.posts.fallback.unwrap()
		});
		let text = rendered(&mut state);
		assert!(
			!text
				.iter()
				.any(|text| text == "This forum is large; loading another way…"),
			"a fast fallback stays quiet"
		);
	}

	#[test]
	fn pinned_posts_keep_their_badge() {
		let mut post = Channel {
			id: Id(5),
			guild: Some(Id(1)),
			parent_id: Some(Id(2)),
			kind: 11,
			name: "Synthetic".into(),
			position: 0,
			recipients: vec![],
			last_message: None,
			icon: None,
			member_list_id: None,
			message_count: None,
			tags: Some(Box::new(model::forum::Tags {
				pinned: true,
				..Default::default()
			})),
		};
		assert!(is_pinned(&post));
		post.tags = None;
		assert!(!is_pinned(&post));
	}

	#[test]
	fn post_composer_stages_images_and_sends_them_with_the_first_message() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		state.gateway_connected = true;
		state.auth = client_core::auth::AuthState::Authenticated;
		assert!(state.select(Id(26)).is_none());
		assert!(
			state.can_attach_post(Id(26)),
			"the fixture forum allows files"
		);
		let mut forum = ForumUi::default();
		let mut commands = Vec::new();
		let mut scratch = Scratch::default();
		let files = [("synthetic.png".to_owned(), 2_048)];
		let textures = [None];
		let render = |forum: &mut ForumUi,
		              state: &mut client_core::State,
		              commands: &mut Vec<Command>,
		              scratch: &mut Scratch| {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(720.0, 640.0),
					)),
					..Default::default()
				},
				|ui| {
					let mut staged = Staged {
						files: &files,
						textures: &textures,
						choose: &mut scratch.choose,
						remove: &mut scratch.remove,
						clear: &mut scratch.clear,
						busy: false,
					};
					forum.show(
						ui,
						state,
						Id(26),
						commands,
						(
							&mut crate::scroll::Session::default(),
							&mut staged,
							&mut crate::avatars::Avatars::default(),
						),
						(
							&mut crate::channel_menu::ChannelMenu::default(),
							crate::shortcuts::ShortcutView::new(&Default::default(), true),
						),
						model::Language::English,
					);
				},
			);
			output.drop_without_applying_deltas();
		};
		render(&mut forum, &mut state, &mut commands, &mut scratch);
		forum.start_draft("Roadmap ideas".into());
		render(&mut forum, &mut state, &mut commands, &mut scratch);
		assert!(commands.is_empty(), "rendering never posts by itself");
		// An image without text is still a post; Discord accepts an empty starter body.
		let draft = forum.draft.as_ref().expect("composer stays open");
		let command = state
			.create_post_with_attachments(
				Id(26),
				&draft.title,
				&draft.body,
				&["synthetic.png"],
				&[],
			)
			.expect("staged files travel with the post");
		let Command::CreatePost {
			parent,
			attachments,
			..
		} = &command
		else {
			panic!("post creation command expected");
		};
		assert_eq!(
			(*parent, attachments.as_slice()),
			(Id(26), &["synthetic.png".to_owned()][..])
		);
		// Discarding the draft releases the selection instead of leaving it staged.
		state.posting.pending = None;
		forum.discard_draft(&mut staged(&mut scratch));
		assert!(forum.draft.is_none());
	}

	#[test]
	fn forum_pane_lists_posts_and_creates_then_opens_one() {
		for dark in [false, true] {
			let ctx = egui::Context::default();
			ctx.set_visuals(if dark {
				egui::Visuals::dark()
			} else {
				egui::Visuals::light()
			});
			let mut state = test_support::demo_state();
			state.gateway_connected = true;
			state.auth = client_core::auth::AuthState::Authenticated;
			assert!(state.select(Id(26)).is_none());
			assert!(state.is_forum(Id(26)));
			let posts = state.forum_posts(Id(26));
			assert!(posts.len() >= 3, "fixture ships several posts");
			assert!(posts.iter().all(|post| post.parent_id == Some(Id(26))));
			let mut forum = ForumUi::default();
			let mut scratch = Scratch::default();
			let mut commands = Vec::new();
			frame(&ctx, |ui| {
				forum.show(
					ui,
					&mut state,
					Id(26),
					&mut commands,
					(
						&mut crate::scroll::Session::default(),
						&mut staged(&mut scratch),
						&mut crate::avatars::Avatars::default(),
					),
					(
						&mut crate::channel_menu::ChannelMenu::default(),
						crate::shortcuts::ShortcutView::new(&Default::default(), true),
					),
					model::Language::English,
				)
			});
			assert!(
				commands.is_empty(),
				"Rendering never requests history or archives"
			);
			forum.query = "synthetic".into();
			frame(&ctx, |ui| {
				forum.show(
					ui,
					&mut state,
					Id(26),
					&mut commands,
					(
						&mut crate::scroll::Session::default(),
						&mut staged(&mut scratch),
						&mut crate::avatars::Avatars::default(),
					),
					(
						&mut crate::channel_menu::ChannelMenu::default(),
						crate::shortcuts::ShortcutView::new(&Default::default(), true),
					),
					model::Language::English,
				)
			});
			assert!(commands.is_empty());
			forum.start_draft("Roadmap ideas".into());
			forum.draft.as_mut().unwrap().body = "First message".into();
			frame(&ctx, |ui| {
				forum.show(
					ui,
					&mut state,
					Id(26),
					&mut commands,
					(
						&mut crate::scroll::Session::default(),
						&mut staged(&mut scratch),
						&mut crate::avatars::Avatars::default(),
					),
					(
						&mut crate::channel_menu::ChannelMenu::default(),
						crate::shortcuts::ShortcutView::new(&Default::default(), true),
					),
					model::Language::English,
				)
			});
			let draft = forum.draft.as_mut().unwrap();
			let command = state
				.create_post(Id(26), &draft.title, &draft.body)
				.expect("fixture permissions allow posting");
			draft.submitted = true;
			let Command::CreatePost {
				parent,
				request,
				title,
				..
			} = &command
			else {
				panic!("post creation command expected");
			};
			assert_eq!((*parent, title.as_str()), (Id(26), "Roadmap ideas"));
			state.apply_post(
				Id(26),
				*request,
				Ok(Channel {
					id: Id(1_548_000_000_000_000_000),
					guild: Some(Id(10)),
					parent_id: Some(Id(26)),
					position: 0,
					name: title.clone(),
					kind: 11,
					recipients: vec![],
					last_message: None,
					member_list_id: None,
					message_count: Some(0),
					icon: None,
					tags: None,
				}),
			);
			assert_eq!(state.posting.created, Some(Id(1_548_000_000_000_000_000)));
			frame(&ctx, |ui| {
				forum.show(
					ui,
					&mut state,
					Id(26),
					&mut commands,
					(
						&mut crate::scroll::Session::default(),
						&mut staged(&mut scratch),
						&mut crate::avatars::Avatars::default(),
					),
					(
						&mut crate::channel_menu::ChannelMenu::default(),
						crate::shortcuts::ShortcutView::new(&Default::default(), true),
					),
					model::Language::English,
				)
			});
			assert!(
				forum.draft.is_none(),
				"A confirmed post closes the composer"
			);
			assert!(
				commands.is_empty(),
				"Opening the created post is the layout's job"
			);
			assert_eq!(
				state.forum_posts(Id(26))[0].id,
				Id(1_548_000_000_000_000_000)
			);
			assert!(matches!(
				state.select(Id(1_548_000_000_000_000_000)),
				Some(Command::History {
					channel: Id(1_548_000_000_000_000_000),
					..
				})
			));
			// A live session fetches the posts the gateway never delivered, exactly once.
			state.demo = false;
			let mut forum = ForumUi::default();
			frame(&ctx, |ui| {
				forum.show(
					ui,
					&mut state,
					Id(26),
					&mut commands,
					(
						&mut crate::scroll::Session::default(),
						&mut staged(&mut scratch),
						&mut crate::avatars::Avatars::default(),
					),
					(
						&mut crate::channel_menu::ChannelMenu::default(),
						crate::shortcuts::ShortcutView::new(&Default::default(), true),
					),
					model::Language::English,
				)
			});
			let index = commands
				.iter()
				.position(|command| matches!(command, Command::ForumPosts { .. }))
				.expect("the post list loads itself");
			let Command::ForumPosts {
				parent: Id(26),
				offset: 0,
				request,
				..
			} = commands.remove(index)
			else {
				panic!("the post list loads itself");
			};
			commands.clear();
			frame(&ctx, |ui| {
				forum.show(
					ui,
					&mut state,
					Id(26),
					&mut commands,
					(
						&mut crate::scroll::Session::default(),
						&mut staged(&mut scratch),
						&mut crate::avatars::Avatars::default(),
					),
					(
						&mut crate::channel_menu::ChannelMenu::default(),
						crate::shortcuts::ShortcutView::new(&Default::default(), true),
					),
					model::Language::English,
				)
			});
			assert!(
				commands.is_empty(),
				"A pending page is never re-requested: {:?}",
				commands
					.iter()
					.map(|command| match command {
						Command::ForumPosts { .. } => "posts",
						Command::MemberSearch(_) => "members",
						_ => "other",
					})
					.collect::<Vec<_>>()
			);
			state.apply_forum_posts(
				Id(26),
				request,
				Ok(model::forum::Page {
					threads: vec![Channel {
						id: Id(1_549_000_000_000_000_000),
						guild: Some(Id(10)),
						parent_id: Some(Id(26)),
						position: 0,
						name: "Fetched post".into(),
						kind: 11,
						recipients: vec![],
						last_message: None,
						member_list_id: None,
						message_count: Some(2),
						icon: None,
						tags: None,
					}],
					more: false,
					previews: Vec::new(),
					fallback: None,
				}),
			);
			forum.query.clear();
			frame(&ctx, |ui| {
				forum.show(
					ui,
					&mut state,
					Id(26),
					&mut commands,
					(
						&mut crate::scroll::Session::default(),
						&mut staged(&mut scratch),
						&mut crate::avatars::Avatars::default(),
					),
					(
						&mut crate::channel_menu::ChannelMenu::default(),
						crate::shortcuts::ShortcutView::new(&Default::default(), true),
					),
					model::Language::English,
				)
			});
			assert!(commands.is_empty(), "A loaded forum stays quiet");
			assert!(
				state
					.forum_posts(Id(26))
					.iter()
					.any(|post| post.id == Id(1_549_000_000_000_000_000)),
				"Fetched posts join the list"
			);
		}
	}

	fn preview_image(index: usize, kind: &str) -> StarterImage {
		StarterImage {
			media: model::EmbedMedia {
				url: Some(format!("https://cdn.example/{kind}-{index}.png")),
				proxy_url: None,
				width: 800,
				height: 600,
				placeholder: Vec::new(),
			},
			id: Id(9_000 + index as u64),
			size: 4096,
			spoiler: kind == "spoiler",
			animated: kind == "gif",
			video: kind == "video",
		}
	}

	fn starter_with(images: Vec<StarterImage>, image_count: u16) -> Starter {
		Starter {
			author_id: Id(987_654_321),
			author: "Synthetic author".into(),
			roles: vec![],
			webhook: false,
			excerpt: "Synthetic starter".into(),
			images,
			image_count,
			reactions: vec![model::Reaction {
				emoji: model::ReactionEmoji {
					id: None,
					name: Some("🔥".into()),
				},
				count: 3,
				me: false,
				me_burst: false,
			}],
		}
	}

	fn labels(shape: &egui::Shape, found: &mut Vec<String>) {
		match shape {
			egui::Shape::Text(text) => found.push(text.galley.job.text.clone()),
			egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| labels(shape, found)),
			_ => {}
		}
	}

	#[test]
	fn mosaic_cells_cover_the_art_area_without_overlap() {
		let origin = egui::pos2(10.0, 20.0);
		let size = egui::vec2(300.0, 174.0);
		let area = egui::Rect::from_min_size(origin, size);
		for (images, cells) in [(1usize, 1usize), (2, 2), (3, 3), (4, 4), (9, 4)] {
			let rects = mosaic_rects(origin, size, images.min(4));
			assert_eq!(rects.len(), cells, "{images} images");
			let covered: f32 = rects.iter().map(|rect| rect.width() * rect.height()).sum();
			assert!(
				covered >= area.width() * area.height() * 0.85,
				"{images} images cover only {covered}"
			);
			for (index, rect) in rects.iter().enumerate() {
				assert!(area.contains_rect(*rect), "{images} images: cell escapes");
				assert!(rect.width() > 1.0 && rect.height() > 1.0);
				for other in &rects[index + 1..] {
					assert!(!rect.intersects(*other), "{images} images: cells overlap");
				}
			}
		}
		// One image fills the area; four split it in a 2x2 that reaches both far corners.
		assert_eq!(mosaic_rects(origin, size, 1)[0], area);
		let four = mosaic_rects(origin, size, 4);
		assert_eq!(four[0].min, origin);
		assert_eq!(four[3].max, area.max);
		assert!(mosaic_rects(origin, size, 0).is_empty());
	}

	#[test]
	fn gallery_tiles_show_the_mosaic_badges_and_counts() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let mut forum = ForumUi::default();
		let mut commands = Vec::new();
		let mut scratch = Scratch::default();
		state.posts.remember_preview(
			Id(27),
			starter_with(
				vec![
					preview_image(0, "png"),
					preview_image(1, "gif"),
					preview_image(2, "video"),
					preview_image(3, "spoiler"),
				],
				9,
			),
		);
		let mut rendered = Vec::new();
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1120.0, 900.0),
				)),
				..Default::default()
			},
			|ui| {
				let mut staged = staged(&mut scratch);
				forum.show(
					ui,
					&mut state,
					Id(26),
					&mut commands,
					(
						&mut crate::scroll::Session::default(),
						&mut staged,
						&mut crate::avatars::Avatars::default(),
					),
					(
						&mut crate::channel_menu::ChannelMenu::default(),
						crate::shortcuts::ShortcutView::new(&Default::default(), true),
					),
					model::Language::English,
				);
			},
		);
		for shape in &output.shapes {
			labels(&shape.shape, &mut rendered);
		}
		output.drop_without_applying_deltas();
		assert_eq!(
			forum.layout,
			Layout::Gallery,
			"the fixture forum opens in its own layout"
		);
		assert!(
			rendered.iter().any(|text| text.contains("Sort & view")),
			"the sort and view control names the layout"
		);
		assert!(rendered.iter().any(|text| text == "+5"), "+N badge");
		assert!(rendered.iter().any(|text| text == "GIF"), "GIF badge");
		assert!(
			rendered.iter().any(|text| text == "Spoiler"),
			"spoiler cover"
		);
		assert!(
			rendered.iter().any(|text| text == "3"),
			"the most used reaction's count rides the card"
		);
		assert!(
			rendered.iter().any(|text| text == "Synthetic author:"),
			"the starter author replaces the latest message row"
		);
	}

	#[test]
	fn the_view_menu_switches_between_gallery_and_list() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let mut forum = ForumUi::default();
		let mut commands = Vec::new();
		let mut scratch = Scratch::default();
		let mut rendered = |forum: &mut ForumUi, state: &mut client_core::State| {
			let mut text = Vec::new();
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1120.0, 900.0),
					)),
					..Default::default()
				},
				|ui| {
					let mut staged = staged(&mut scratch);
					forum.show(
						ui,
						state,
						Id(26),
						&mut commands,
						(
							&mut crate::scroll::Session::default(),
							&mut staged,
							&mut crate::avatars::Avatars::default(),
						),
						(
							&mut crate::channel_menu::ChannelMenu::default(),
							crate::shortcuts::ShortcutView::new(&Default::default(), true),
						),
						model::Language::English,
					);
				},
			);
			for shape in &output.shapes {
				labels(&shape.shape, &mut text);
			}
			output.drop_without_applying_deltas();
			text
		};
		let text = rendered(&mut forum, &mut state);
		assert!(text.iter().any(|text| text.contains("Gallery")));
		// Changing the view keeps it for this forum only, the way Discord remembers it.
		forum.layout = Layout::List;
		forum.remember_layout(Id(26), Layout::List);
		let text = rendered(&mut forum, &mut state);
		assert!(text.iter().any(|text| text.contains("List")));
		assert!(!text.iter().any(|text| text.contains("Gallery")));
		let defaults = state.forum_defaults(Id(26)).cloned().unwrap_or_default();
		assert_eq!(defaults.layout, Layout::Gallery);
		assert_eq!(forum.remembered_layout(Id(26)), Some(Layout::List));
	}

	#[test]
	fn visible_cards_request_thumbnails_and_the_next_screen_prefetches() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		state.demo = false;
		state.gateway_connected = true;
		let template = state
			.channels
			.iter()
			.find(|channel| channel.parent_id == Some(Id(26)))
			.expect("the fixture forum has a post")
			.clone();
		state.channels.retain(|channel| {
			!(channel.parent_id == Some(Id(26)) && matches!(channel.kind, 11 | 12))
		});
		state.posts = client_core::forum::Posts::default();
		for index in 0..50u64 {
			let id = 2000 + index;
			let mut post = template.clone();
			post.id = Id(id);
			post.name = format!("Synthetic post {index}");
			post.last_message = Some(Id(2_000_000 - index));
			post.message_count = Some(index as u32);
			state.channels.push(post);
			state.posts.remember_preview(
				Id(id),
				starter_with(
					(0..4)
						.map(|image| {
							let mut preview = preview_image(image, "png");
							preview.media.url =
								Some(format!("https://cdn.example/post-{id}-{image}.png"));
							preview
						})
						.collect(),
					4,
				),
			);
		}
		state.invalidate_navigation();
		let mut forum = ForumUi::default();
		let mut commands = Vec::new();
		let mut scratch = Scratch::default();
		let mut images = crate::avatars::Avatars::default();
		let started = std::time::Instant::now();
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1120.0, 700.0),
				)),
				..Default::default()
			},
			|ui| {
				let mut staged = staged(&mut scratch);
				forum.show(
					ui,
					&mut state,
					Id(26),
					&mut commands,
					(
						&mut crate::scroll::Session::default(),
						&mut staged,
						&mut images,
					),
					(
						&mut crate::channel_menu::ChannelMenu::default(),
						crate::shortcuts::ShortcutView::new(&Default::default(), true),
					),
					model::Language::English,
				);
			},
		);
		output.drop_without_applying_deltas();
		let frame = started.elapsed();
		let requests = images.take_requests();
		let visible = requests
			.iter()
			.filter(|key| key.starts_with("media:i"))
			.count();
		let prefetched = requests
			.iter()
			.filter(|key| key.starts_with("media:p"))
			.count();
		println!(
			"Synthetic 50-post forum (4 images each): first frame {frame:?} debug headless, \
			 {visible} visible thumbnail requests, {prefetched} prefetched, {} total",
			requests.len()
		);
		assert!(
			frame < std::time::Duration::from_secs(2),
			"first frame took {frame:?}"
		);
		assert!(
			visible > 0,
			"visible cards request their thumbnails without a click: {requests:?}"
		);
		assert!(
			prefetched > 0,
			"the next screen pre-requests its thumbnails: {requests:?}"
		);
		assert!(
			prefetched <= 6,
			"{prefetched} prefetches in one frame is over the budget"
		);
		assert!(
			!requests.iter().any(|key| key.contains("post-2049-")),
			"a card many screens away stays cold"
		);
		assert!(
			requests.iter().any(|key| key.contains("post-2000-")),
			"the first card's thumbnail is requested"
		);
	}

	#[test]
	fn prefetch_budget_limits_off_screen_requests_per_frame() {
		let ctx = egui::Context::default();
		let mut images = crate::avatars::Avatars::default();
		for index in 0..20 {
			let media = model::EmbedMedia {
				url: Some(format!("https://cdn.example/prefetch-{index}.png")),
				proxy_url: None,
				width: 800,
				height: 600,
				placeholder: Vec::new(),
			};
			images.prefetch_media(
				&media,
				egui::vec2(200.0, 120.0),
				&ctx,
				false,
				crate::avatars::Surface::Tile,
			);
		}
		let requests = images.take_requests();
		assert_eq!(requests.len(), 6, "{requests:?}");
		assert!(requests.iter().all(|key| key.starts_with("media:p")));
	}

	#[test]
	fn tag_chips_render_with_emoji_and_fold_extra_tags() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		// The first fixture post carries all three offered tags: two chips and a "+1".
		if let Some(post) = state
			.channels
			.iter_mut()
			.find(|channel| channel.id == Id(27))
		{
			post.tags = Some(Box::new(model::forum::Tags {
				applied: vec![Id(31), Id(32), Id(33)],
				..Default::default()
			}));
		}
		let mut forum = ForumUi::default();
		let mut commands = Vec::new();
		let mut scratch = Scratch::default();
		let mut rendered = Vec::new();
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1120.0, 900.0),
				)),
				..Default::default()
			},
			|ui| {
				let mut staged = staged(&mut scratch);
				forum.show(
					ui,
					&mut state,
					Id(26),
					&mut commands,
					(
						&mut crate::scroll::Session::default(),
						&mut staged,
						&mut crate::avatars::Avatars::default(),
					),
					(
						&mut crate::channel_menu::ChannelMenu::default(),
						crate::shortcuts::ShortcutView::new(&Default::default(), true),
					),
					model::Language::English,
				);
			},
		);
		for shape in &output.shapes {
			labels(&shape.shape, &mut rendered);
		}
		output.drop_without_applying_deltas();
		assert!(
			rendered.iter().any(|text| text == "Synthetic help"),
			"an applied tag's chip renders: {rendered:?}"
		);
		assert!(rendered.iter().any(|text| text == "Synthetic ideas"));
		assert!(
			rendered.iter().any(|text| text == "+1"),
			"a tile folds tags past two into +N"
		);
		assert!(
			rendered.iter().any(|text| text == "All"),
			"the filter bar offers all tags"
		);
	}

	#[test]
	fn the_tag_filter_narrows_cards_and_all_restores_them() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let mut forum = ForumUi::default();
		let mut commands = Vec::new();
		let mut scratch = Scratch::default();
		let mut rendered = |forum: &mut ForumUi, state: &mut client_core::State| {
			let mut text = Vec::new();
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1120.0, 900.0),
					)),
					..Default::default()
				},
				|ui| {
					let mut staged = staged(&mut scratch);
					forum.show(
						ui,
						state,
						Id(26),
						&mut commands,
						(
							&mut crate::scroll::Session::default(),
							&mut staged,
							&mut crate::avatars::Avatars::default(),
						),
						(
							&mut crate::channel_menu::ChannelMenu::default(),
							crate::shortcuts::ShortcutView::new(&Default::default(), true),
						),
						model::Language::English,
					);
				},
			);
			for shape in &output.shapes {
				labels(&shape.shape, &mut text);
			}
			output.drop_without_applying_deltas();
			text
		};
		let text = rendered(&mut forum, &mut state);
		assert!(text.iter().any(|text| text == "A synthetic forum post"));
		assert!(
			text.iter()
				.any(|text| text == "Automatic model retraining on app data")
		);
		// Filtering by the first post's tag hides the other posts.
		forum.tags = vec![Id(31)];
		let text = rendered(&mut forum, &mut state);
		assert!(text.iter().any(|text| text == "A synthetic forum post"));
		assert!(
			!text
				.iter()
				.any(|text| text == "Automatic model retraining on app data"),
			"the tagged post alone survives the filter: {text:?}"
		);
		assert!(text.iter().any(|text| text.contains("Tags (1)")));
		// Clearing the selection brings every card back.
		forum.tags.clear();
		let text = rendered(&mut forum, &mut state);
		assert!(
			text.iter()
				.any(|text| text == "Automatic model retraining on app data")
		);
	}

	#[test]
	fn tag_selection_rules_toggle_limit_and_match() {
		let mut tags = Vec::new();
		toggle(&mut tags, Id(1));
		toggle(&mut tags, Id(2));
		assert_eq!(tags, vec![Id(1), Id(2)]);
		toggle(&mut tags, Id(1));
		assert_eq!(tags, vec![Id(2)]);
		for id in 10..20 {
			toggle(&mut tags, Id(id));
		}
		assert_eq!(tags.len(), model::forum::MAX_APPLIED_TAGS);
		let mut post = Channel {
			id: Id(5),
			guild: Some(Id(1)),
			parent_id: Some(Id(2)),
			kind: 11,
			name: "Synthetic".into(),
			position: 0,
			recipients: vec![],
			last_message: None,
			icon: None,
			member_list_id: None,
			message_count: None,
			tags: Some(Box::new(model::forum::Tags {
				applied: vec![Id(7), Id(8)],
				..Default::default()
			})),
		};
		assert!(carries(&post, &[], false));
		assert!(carries(&post, &[Id(7)], false));
		assert!(!carries(&post, &[Id(9)], false));
		assert!(carries(&post, &[Id(7), Id(8)], true));
		assert!(!carries(&post, &[Id(7), Id(9)], true));
		post.tags = None;
		assert!(!carries(&post, &[Id(7)], false));
		// Starting a post while filtering pre-fills the picked tags.
		let mut forum = ForumUi {
			tags: vec![Id(7), Id(8)],
			..Default::default()
		};
		forum.start_draft("Synthetic".into());
		assert_eq!(forum.draft.unwrap().tags, vec![Id(7), Id(8)]);
	}
}
