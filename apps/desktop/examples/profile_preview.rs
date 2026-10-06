//! Offline native framebuffer capture; no account, filesystem cache, or network adapters.
use eframe::egui;
#[path = "../src/server_settings_demo.rs"]
mod server_settings_demo;
#[allow(dead_code)] // The shared fixture's CLI check is called by the desktop binary.
#[path = "../src/slash_demo.rs"]
mod slash_demo;
use std::{
	path::PathBuf,
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	},
	thread::JoinHandle,
	time::{Duration, Instant},
};

struct Preview {
	messaging: ui::MessagingUi,
	state: client_core::State,
	output: PathBuf,
	thumbnail: bool,
	/// Opens the synthetic video player after the first frame's channel reset.
	start_video: bool,
	frames: u8,
	/// Wheel distance and pointer position injected over the first frames, for pages below the fold.
	scroll: Option<(f32, egui::Pos2)>,
	requested: bool,
	screenshot: Option<std::sync::mpsc::Receiver<Arc<egui::ColorImage>>>,
	writer: Option<JoinHandle<Result<(), String>>>,
	saved: Arc<AtomicBool>,
	started: Instant,
}

impl eframe::App for Preview {
	fn persist_egui_memory(&self) -> bool {
		false
	}

	fn raw_input_hook(&mut self, _: &egui::Context, raw_input: &mut egui::RawInput) {
		if let Some((distance, at)) = self.scroll.filter(|_| (1..=3).contains(&self.frames)) {
			raw_input.events.push(egui::Event::PointerMoved(at));
			raw_input.events.push(egui::Event::MouseWheel {
				unit: egui::MouseWheelUnit::Point,
				phase: egui::TouchPhase::Move,
				delta: egui::vec2(0.0, -distance / 3.0),
				modifiers: egui::Modifiers::NONE,
			});
		}
	}

	fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
		let ctx = ui.ctx().clone();
		ui::design::paint_backdrop(&ctx);
		// Only synthetic fixtures execute these commands; no service adapters exist here.
		for command in self.messaging.show(ui, &mut self.state) {
			let event = match command {
				client_core::Command::ApplicationCommands {
					channel,
					guild,
					request,
				} => client_core::Event::ApplicationCommands {
					channel,
					request,
					result: Ok(slash_demo::catalog(guild)),
				},
				client_core::Command::Interaction(request) => {
					slash_demo::respond(&mut self.state, request);
					continue;
				}
				client_core::Command::ServerAdmin {
					guild,
					request,
					action,
				} => server_settings_demo::execute_admin(&self.state, guild, request, *action),
				client_core::Command::ServerSettings {
					guild,
					request,
					edit,
				} => server_settings_demo::execute(&self.state, guild, request, edit),
				client_core::Command::ServerAction { action, request } => {
					server_settings_demo::execute_action(&mut self.state, action, request)
				}
				_ => continue,
			};
			self.state.apply(client_core::Envelope {
				generation: self.state.generation,
				event,
			});
		}
		// The desktop bridge draws the fetched text body after the main surface.
		self.messaging.show_preview(&ctx);
		// The timeline resets its player when the selected channel changes on the
		// first frame, so the fixture opens the video only after that reset.
		if self.start_video && self.frames == 0 {
			self.start_video = false;
			if let Some(message) = self.state.timeline.get(model::Id(601)).cloned()
				&& let Some(attachment) = message.attachments.first().cloned()
			{
				let player = self.messaging.video();
				player.begin(&message, &attachment, false);
				let _ = player.accept_frame(&ctx, synthetic_video_frame());
				player.state = ui::VideoState::Paused;
				player.position = 1.2;
				player.duration = 3.0;
			}
		}
		for request in std::mem::take(&mut self.messaging.extensions.requests) {
			if let ui::ExtensionRequest::PreviewTheme { theme, image } = request {
				ui::design::set_extension_theme(theme.as_deref());
				ui::design::set_background_image(&ctx, image);
				ui::design::apply(&ctx);
			}
		}
		if self.requested && self.writer.is_none() {
			let screenshot = self
				.screenshot
				.as_ref()
				.and_then(|receiver| receiver.try_recv().ok());
			if let Some(image) = screenshot {
				let output = self.output.clone();
				let thumbnail = self.thumbnail;
				self.writer = Some(std::thread::spawn(move || {
					if image.size[0] > 4096 || image.size[1] > 4096 {
						return Err("Screenshot exceeds the 4096-pixel dimension limit".into());
					}
					let pixels: Vec<u8> = image
						.pixels
						.iter()
						.flat_map(|pixel| pixel.to_srgba_unmultiplied())
						.collect();
					let image = image::RgbaImage::from_raw(
						image.size[0] as u32,
						image.size[1] as u32,
						pixels,
					)
					.ok_or("Invalid screenshot pixel count")?;
					let image = image::DynamicImage::ImageRgba8(image);
					let image = if thumbnail {
						image.thumbnail(640, 360)
					} else {
						image
					};
					image
						.save_with_format(&output, image::ImageFormat::Png)
						.map_err(|error| error.to_string())
				}));
			}
		}
		if self.writer.as_ref().is_some_and(JoinHandle::is_finished) {
			match self.writer.take().unwrap().join() {
				Ok(Ok(())) => {
					self.saved.store(true, Ordering::Release);
					println!(
						"Saved offline native framebuffer: {}",
						self.output.display()
					);
				}
				Ok(Err(error)) => eprintln!("Screenshot save failed: {error}"),
				Err(_) => eprintln!("Screenshot worker failed"),
			}
			ctx.send_viewport_cmd(egui::ViewportCommand::Close);
			return;
		}
		if self.started.elapsed() > Duration::from_secs(20) {
			eprintln!("Native screenshot callback did not complete within 20 seconds");
			ctx.send_viewport_cmd(egui::ViewportCommand::Close);
			return;
		}
		self.frames = self.frames.saturating_add(1);
		if self.frames >= 5 && self.started.elapsed() >= Duration::from_secs(1) && !self.requested {
			self.requested = true;
			let (send, receive) = std::sync::mpsc::sync_channel(1);
			self.screenshot = Some(receive);
			let wake = ctx.clone();
			ctx.request_screenshot(move |image| {
				let _ = send.try_send(image);
				wake.request_repaint();
			});
		}
		ctx.request_repaint_after(Duration::from_millis(100));
	}
}

/// Synthetic member list for the identity captures; names and statuses are invented.
fn showcase_members(state: &mut client_core::State) {
	use model::{ClientPlatforms, Id, Member, MemberList, MemberSlot, RichActivity, User};
	for guild in state.permissions.guilds.values_mut() {
		if let Some(roles) = &mut guild.roles {
			roles.extend([
				model::permissions::Role {
					id: Id(9001),
					bits: 0,
					name: "Founders".into(),
					color: 0xe78284,
					position: 2,
					hoist: true,
				},
				model::permissions::Role {
					id: Id(9002),
					bits: 0,
					name: "Community".into(),
					color: 0xe5c769,
					position: 1,
					hoist: true,
				},
			]);
		}
	}
	let channel = state.selected.expect("selected fixture channel");
	let guild = state.channel(channel).and_then(|channel| channel.guild);
	let person = |id: u64,
	              name: &str,
	              status: &str,
	              roles: Vec<Id>,
	              custom: Option<&str>,
	              activity: Option<RichActivity>| {
		Member {
			user: User {
				id: Id(id),
				name: name.into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
			},
			nick: None,
			roles,
			status: Some(status.into()),
			custom_status: custom.map(str::to_owned),
			activities: activity.into_iter().collect(),
			clients: ClientPlatforms::default(),
		}
	};
	let members = vec![
		person(
			1,
			"You (synthetic)",
			"online",
			vec![Id(9002)],
			Some("Building quiet software for loud places"),
			None,
		),
		person(
			2,
			"Robin (synthetic)",
			"idle",
			vec![Id(9001)],
			None,
			Some(RichActivity {
				kind: 0,
				name: "Stardew Valley".into(),
				details: Some("Tending the synthetic farm".into()),
				state: Some("Spring - Day 12".into()),
				image: None,
				small_image: None,
				ends_at: None,
				started_at: None,
			}),
		),
		person(
			9003,
			"Alex (synthetic)",
			"online",
			vec![],
			Some("Sipping synthetic coffee"),
			None,
		),
		person(9004, "Sam (synthetic)", "dnd", vec![], None, None),
		person(9005, "Taylor (synthetic)", "offline", vec![], None, None),
	];
	state.members = Some(MemberList {
		guild,
		channel,
		request: 0,
		total: members.len() as u64,
		start: 0,
		slots: members
			.into_iter()
			.map(|member| Some(MemberSlot::Person(member)))
			.collect(),
		lazy: false,
		groups: vec![],
		ranges: vec![],
		freshness: model::Freshness::Fresh,
	});
}

/// Original synthetic frame for the offline player capture; no decoded media bytes.
fn synthetic_video_frame() -> egui::ColorImage {
	let (width, height) = (320usize, 180usize);
	let mut image = egui::ColorImage::filled([width, height], egui::Color32::BLACK);
	for y in 0..height {
		for x in 0..width {
			let mut color =
				egui::Color32::from_rgb((x * 255 / width) as u8, (y * 255 / height) as u8, 190);
			let (cx, cy) = (x as i32 - 96, y as i32 - 90);
			if cx * cx + cy * cy < 44 * 44 {
				color = egui::Color32::from_rgb(250, 181, 98);
			}
			if (248..width).contains(&x) && (24..height - 24).contains(&y) && (y / 12) % 2 == 0 {
				color = egui::Color32::from_rgb(235, 238, 242);
			}
			image.pixels[y * width + x] = color;
		}
	}
	image
}

fn prime_profile(state: &mut client_core::State) {
	if let Some(client_core::Command::EditProfile { user, request, .. }) = state.load_own_profile()
	{
		let profile = ui::synthetic_own_profile(state.user.as_ref().unwrap());
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::ProfileEdited {
				user,
				request,
				result: Ok(Box::new(profile)),
			},
		});
	}
}

fn prime_extension_chat(state: &mut client_core::State) {
	let channel = state.selected.expect("selected fixture channel");
	let messages = [
		"Welcome to our little corner of the internet.",
		"A place for good conversations and late-night ideas.",
		"**Game night** starts at 8. Everyone is welcome!",
		"I'll bring the playlist. Any requests?",
		"Something with a little more synth, please.",
		"Hey everyone, ready for game night?",
	]
	.into_iter()
	.enumerate()
	.map(|(index, content)| {
		let mut message = test_support::message(600 + index as u64, channel);
		message.content = content.into();
		message.attachments.clear();
		message.embeds.clear();
		message.reactions = Some(vec![]);
		message
	})
	.collect();
	state.timeline.clear();
	state
		.timeline
		.seed_cache(messages)
		.expect("valid synthetic conversation");
}

// Fixture packages are checked-in inputs; execution never calls desktop adapters.
fn extension_fixture(
	id: &str,
) -> Result<
	(
		extensions::Package,
		extensions::Invocation,
		Option<extensions::Output>,
	),
	Box<dyn std::error::Error>,
> {
	let bytes: &[u8] = match id {
		"serein-ocean" => include_bytes!("../../../extensions/ocean.serein-extension"),
		"message-delete-protector" => include_bytes!(
			"../../../examples/extensions/packages/message-delete-protector.nivra-extension"
		),
		"serein-midnight" => include_bytes!("../../../extensions/midnight.serein-extension"),
		"serein-rose" => include_bytes!("../../../extensions/rose.serein-extension"),
		"serein-forest" => include_bytes!("../../../extensions/forest.serein-extension"),
		"serein-latte" => include_bytes!("../../../extensions/latte.serein-extension"),
		"golden-theme" => include_bytes!("../../../extensions/golden.serein-extension"),
		"black-theme" => include_bytes!("../../../extensions/katana.serein-extension"),
		"obsidian-theme" => include_bytes!("../../../extensions/obsidian.serein-extension"),
		"teal-theme" => include_bytes!("../../../extensions/teal.serein-extension"),
		"emoji-sticker-images" => include_bytes!(
			"../../../examples/extensions/packages/emoji-sticker-images.nivra-extension"
		),
		_ => return Err("Unknown fixture extension".into()),
	};
	let package = extensions::parse_package(bytes)?;
	let invocation = extensions::Invocation {
		action: "activate".into(),
		..Default::default()
	};
	let output = if package.theme.is_none() {
		Some(extensions::invoke(&package, &invocation)?)
	} else {
		None
	};
	Ok((package, invocation, output))
}

fn seed_catalog(extensions: &mut ui::ExtensionUi, themes: bool) {
	if themes {
		let packages: [(&[u8], &str); 6] = [
			(
				include_bytes!("../../../extensions/ocean.serein-extension"),
				"",
			),
			(
				include_bytes!("../../../extensions/obsidian.serein-extension"),
				"Obsidian violet surfaces and lavender accents.",
			),
			(
				include_bytes!("../../../extensions/forest.serein-extension"),
				"Calm forest greens and fresh leafy accents.",
			),
			(
				include_bytes!("../../../extensions/latte.serein-extension"),
				"Warm coffee tones and a creamy caramel accent.",
			),
			(
				include_bytes!("../../../extensions/rose.serein-extension"),
				"Soft rose accents.",
			),
			(
				include_bytes!("../../../extensions/midnight.serein-extension"),
				"Deep, quiet surfaces.",
			),
		];
		let mut entries: Vec<_> = packages
			.into_iter()
			.enumerate()
			.map(|(index, (bytes, description))| {
				let package = extensions::parse_package(bytes).expect("valid synthetic theme");
				ui::ExtensionEntry {
					cover_image: None,
					local_theme: index == 0,
					manifest: package.manifest,
					theme_preview: package.theme,
					description: description.into(),
					preview: None,
					reviewed: true,
					sha256: "a".repeat(64),
					download_bytes: bytes.len() as u64,
					enabled: index < 2,
					cleanup_pending: false,
					update_available: false,
					update_manifest: None,
				}
			})
			.collect();
		entries[0].manifest.name = "My ocean".into();
		entries[0].manifest.author = "You".into();
		let image =
			image::load_from_memory(include_bytes!("../../../extensions/previews/ocean.png"))
				.expect("valid synthetic cover")
				.to_rgba8();
		entries[0].cover_image = Some(Arc::new(egui::ColorImage::from_rgba_unmultiplied(
			[image.width() as usize, image.height() as usize],
			image.as_raw(),
		)));
		extensions.active_theme = Some(entries[0].manifest.id.clone());
		extensions.set_entries(entries);
		return;
	}
	let catalog = extensions::parse_catalog(include_bytes!("../../../extensions/catalog.json"))
		.expect("valid fixture catalog");
	extensions.set_entries(
		catalog
			.entries
			.into_iter()
			.map(|entry| ui::ExtensionEntry {
				cover_image: None,
				local_theme: false,
				manifest: entry.manifest,
				description: entry.description,
				preview: entry.preview,
				theme_preview: None,
				reviewed: true,
				sha256: entry.sha256,
				download_bytes: entry.download_bytes,
				enabled: false,
				cleanup_pending: false,
				update_available: false,
				update_manifest: None,
			})
			.collect(),
	);
	let previews = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../extensions/previews");
	{
		let (id, filename) = ("serein-ocean", "ocean.png");
		let image = image::open(previews.join(filename))
			.expect("valid fixture preview")
			.to_rgba8();
		let size = [image.width() as usize, image.height() as usize];
		extensions.preview_fixture_image(
			id.into(),
			egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
		);
	}
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let args: Vec<_> = std::env::args().skip(1).collect();
	let value = |prefix: &str| args.iter().find_map(|arg| arg.strip_prefix(prefix));
	if !args.iter().any(|arg| arg == "--demo") {
		return Err("Usage: profile_preview --demo --output=PATH.png [--page=overview|voice-call|text-preview|video|image-viewer|forum|stickers|slash-commands|slash-command-search|slash-command-options|profile|profile-card|member-tags|dm-tags|account|appearance|chat|notifications|voice|keybinds|help|extensions|server|server-engagement|server-stickers] [--state=default|voice|video] [--command=help|weather] [--themes] [--extension=ID] [--thumbnail] [--width=1120] [--height=760] [--light] [--timeline-text] [--poll] [--poll-presence]".into());
	}
	let output = PathBuf::from(value("--output=").ok_or("Missing --output=PATH.png")?);
	let page = value("--page=").unwrap_or("profile").to_owned();
	if !matches!(
		page.as_str(),
		"overview"
			| "voice-call"
			| "text-preview"
			| "video" | "image-viewer"
			| "forum" | "profile"
			| "stickers"
			| "slash-commands"
			| "slash-command-search"
			| "slash-command-options"
			| "profile-card"
			| "member-tags"
			| "dm-tags"
			| "account"
			| "appearance"
			| "chat" | "notifications"
			| "voice" | "help"
			| "general"
			| "keybinds"
			| "extensions"
			| "server"
			| "server-engagement"
			| "server-stickers"
	) {
		return Err("Page must be overview, voice-call, text-preview, video, image-viewer, profile, profile-card, member-tags, dm-tags, account, appearance, chat, notifications, voice, keybinds, help, extensions, slash-commands, slash-command-search, slash-command-options, server, server-engagement or server-stickers".into());
	}
	let state_kind = value("--state=").unwrap_or("default").to_owned();
	if !matches!(state_kind.as_str(), "default" | "voice" | "video") {
		return Err("State fixture must be default, voice or video".into());
	}
	let slash_command = value("--command=").unwrap_or("help").to_owned();
	if !matches!(slash_command.as_str(), "help" | "weather") {
		return Err("Command fixture must be help or weather".into());
	}
	let width: f32 = value("--width=").unwrap_or("1120").parse()?;
	let height: f32 = value("--height=").unwrap_or("760").parse()?;
	let scroll = value("--scroll=")
		.map(str::parse::<f32>)
		.transpose()?
		.map(|distance| (distance, egui::pos2(width * 0.6, height * 0.5)));
	if !(500.0..=1920.0).contains(&width) || !(520.0..=1200.0).contains(&height) {
		return Err("Viewport must be 500-1920 by 520-1200".into());
	}
	let light = args.iter().any(|arg| arg == "--light");
	let theme_editor = value("--theme-editor=").map(str::to_owned);
	let theme_preview = args.iter().any(|arg| arg == "--theme-preview");
	let thumbnail = args.iter().any(|arg| arg == "--thumbnail");
	let extension = value("--extension=").map(str::to_owned);
	let fixture = extension.as_deref().map(extension_fixture).transpose()?;
	let saved = Arc::new(AtomicBool::new(false));
	let completed = saved.clone();
	eframe::run_native(
		"Nivra · offline preview",
		eframe::NativeOptions {
			viewport: egui::ViewportBuilder::default()
				.with_inner_size([width, height])
				.with_decorations(false),
			renderer: eframe::Renderer::Wgpu,
			persist_window: false,
			..Default::default()
		},
		Box::new(move |cc| {
			ui::fonts::install(&cc.egui_ctx);
			ui::design::apply(&cc.egui_ctx);
			cc.egui_ctx.set_theme(if light {
				egui::ThemePreference::Light
			} else {
				egui::ThemePreference::Dark
			});
			let mut state = if matches!(
				page.as_str(),
				"slash-commands" | "slash-command-search" | "slash-command-options"
			) {
				slash_demo::preview()
			} else if state_kind == "voice" {
				test_support::voice_demo_state()
			} else if state_kind == "video" {
				test_support::video_demo_state()
			} else if page == "forum" {
				test_support::forum_gallery_state()
			} else {
				test_support::demo_state()
			};
			if args.iter().any(|arg| arg == "--timeline-text") {
				// Text-only fixture for timeline density shots: six short rows with
				// alternating authors in the selected channel (synthetic, disclosed).
				let channel = state.selected.unwrap();
				state.timeline.clear();
				state.older_exhausted = true;
				for i in 0..6u64 {
					let mut m = test_support::message(100 + i, channel);
					m.id = model::Id(
						((1_788_998_100_000u64 + i * 60_000 - 1_420_070_400_000) << 22) | 1,
					);
					m.content = format!("Synthetic timeline row {i} for density comparison.");
					state.timeline.insert(m, false, false).unwrap();
				}
			}
			if args.iter().any(|arg| arg == "--poll") {
				// Synthetic poll card on the third row; offline and disclosed.
				let poll_message = state.timeline.iter().nth(2).map(|m| m.id);
				if let Some(message) = poll_message {
					let _ = state
						.timeline
						.set_poll(message, Some(test_support::synthetic_poll()));
				}
			}
			if args.iter().any(|arg| arg == "--poll-presence") {
				// Presence-only marker for the pre-card rendering of the same row.
				let poll_message = state.timeline.iter().nth(2).map(|m| m.id);
				if let Some(message) = poll_message
					&& let Some(mut row) = state.timeline.get(message).cloned()
				{
					row.extra_content.poll = true;
					let _ = state.timeline.insert(row, false, false);
				}
			}
			if page == "profile" {
				prime_profile(&mut state);
			}
			if page == "overview" {
				showcase_members(&mut state);
			}
			if page == "member-tags" {
				let user = test_support::message(1, model::Id(20)).author;
				state.members = Some(model::MemberList {
					channel: model::Id(20),
					guild: Some(model::Id(10)),
					request: 0,
					total: 1,
					lazy: false,
					groups: vec![],
					ranges: vec![],
					freshness: model::Freshness::Fresh,
					start: 0,
					slots: vec![Some(model::MemberSlot::Person(model::Member {
						user,
						nick: None,
						roles: vec![],
						status: Some("online".into()),
						custom_status: Some("Building a quieter place".into()),
						activities: vec![],
						clients: model::ClientPlatforms {
							mobile: Some(model::ClientPresence::Online),
							..Default::default()
						},
					}))],
				});
			} else if page == "dm-tags" {
				let _ = state.select(model::Id(22));
			}
			let mut messaging = ui::MessagingUi::default();
			if args.iter().any(|arg| arg == "--compact") {
				messaging.compact_timeline = true;
			}
			messaging.tray_available = platform::tray::supported();
			messaging.startup_available = platform::startup::available();
			messaging.startup_enabled = args.iter().any(|arg| arg == "--startup-enabled");
			messaging.startup_minimized = args.iter().any(|arg| arg == "--startup-minimized");
			if page == "overview" {
				// The wide People pane is part of the normal layout for this capture.
				messaging.reading_preferences.show_members = true;
			} else if matches!(
				page.as_str(),
				"text-preview" | "image-viewer" | "video" | "forum"
			) {
				// These captures have no synthetic member list; keep the timeline full width.
				messaging.reading_preferences.show_members = false;
			}
			if matches!(page.as_str(), "overview" | "video" | "voice-call") {
				// The base surface is the capture; the video player opens after the first
				// frame and the voice fixture already stages its synthetic call.
			} else if page == "text-preview" {
				messaging.set_preview(ui::text_preview::TextPreview {
					filename: "release-notes.md".into(),
					format: ui::text_preview::PreviewFormat::Markdown,
					text: "# Synthetic release notes\n\nThis preview is the real bounded dialog the desktop opens after fetching a text, Markdown or code attachment. Every line below is invented for the capture.\n\n- **Bold**, *italics*, `inline code` and [links](https://example.com) all render on the render thread only.\n- Oversized files stop at the disclosed window and offer *Show more*; nothing is written to disk.\n\n> Attachments are fetched once, decoded in memory and never executed.\n\n```rust\nfn main() {\n    println!(\"offline fixture\");\n}\n```\n".into(),
					truncated: false,
					shown: ui::text_preview::PREVIEW_WINDOW_CHARS,
				});
			} else if page == "image-viewer" {
				messaging.preview_image_viewer(model::Id(500), model::Id(700));
			} else if matches!(page.as_str(), "member-tags" | "dm-tags") {
				// State is primed above; the normal offline messaging surface renders the list.
			} else if page == "slash-commands" {
				messaging.preview_slash_commands();
			} else if page == "slash-command-search" {
				// A partial name: the flat "commands matching" list with per-row icons.
				let channel = state.selected.expect("synthetic command conversation");
				state
					.drafts
					.insert(channel, format!("/{}", &slash_command[..2]));
				messaging.preview_slash_commands();
			} else if page == "slash-command-options" {
				let channel = state.selected.expect("synthetic command conversation");
				state.drafts.insert(channel, format!("/{slash_command}"));
				messaging.preview_slash_command_options(&mut state);
			} else if page == "stickers" {
				test_support::seed_stickers(&mut state);
				messaging.preview_sticker_picker();
			} else if page == "profile-card" {
				state.demo = false;
				messaging.preview_profile(test_support::message(1, model::Id(20)).author);
			} else if let Some((package, _invocation, result)) = fixture {
				prime_extension_chat(&mut state);
				if let Some(theme) = package.theme.as_ref() {
					ui::design::set_extension_theme(Some(theme));
					ui::design::apply(&cc.egui_ctx);
				}
				if let Some(output) = result {
					messaging.image_sharing_enabled = output.image_sharing;
					if output.image_sharing {
						test_support::seed_stickers(&mut state);
						messaging.preview_sticker_picker();
					}
					if output.preserve_deleted_messages {
						let channel = state.selected.unwrap();
						state.apply(client_core::Envelope {
							generation: state.generation,
							event: client_core::Event::Delete {
								channel,
								id: model::Id(601),
							},
						});
					}
				}
			} else if page.starts_with("server") {
				server_settings_demo::open(&mut state, &mut messaging);
				if page == "server-engagement" {
					messaging.preview_server_engagement();
				} else if page == "server-stickers"
					&& let Some(client_core::Command::ServerAdmin {
						guild,
						request,
						action,
					}) = messaging.preview_server_admin(&mut state, model::Id(10), "stickers")
				{
					let event =
						server_settings_demo::execute_admin(&state, guild, request, *action);
					state.apply(client_core::Envelope {
						generation: state.generation,
						event,
					});
				}
			} else if !args.iter().any(|arg| arg == "--no-settings") {
				messaging.preview_settings(
					if page == "extensions" && args.iter().any(|arg| arg == "--themes") {
						"themes"
					} else {
						&page
					},
				);
				if page == "extensions" {
					seed_catalog(
						&mut messaging.extensions,
						args.iter().any(|arg| arg == "--themes"),
					);
					messaging
						.extensions
						.preview_themes(args.iter().any(|arg| arg == "--themes"));
					if theme_preview {
						prime_extension_chat(&mut state);
						let package = extensions::parse_package(include_bytes!(
							"../../../extensions/katana.serein-extension"
						))?;
						messaging.extensions.receive_theme_edit(
							Box::new(package),
							None,
							None,
							false,
							true,
						);
					}
					if let Some(tab) = &theme_editor {
						let mut package = extensions::parse_package(include_bytes!(
							"../../../extensions/ocean.serein-extension"
						))
						.expect("valid theme fixture");
						package.manifest.name = "My ocean".into();
						package.manifest.author = "You".into();
						messaging.extensions.receive_theme_edit(
							Box::new(package),
							None,
							None,
							true,
							false,
						);
						let bytes = include_bytes!("../../../extensions/previews/ocean.png");
						let pixels = image::load_from_memory(bytes)
							.expect("valid fixture image")
							.to_rgba8();
						messaging.extensions.receive_theme_image(
							bytes.to_vec(),
							Arc::new(egui::ColorImage::from_rgba_unmultiplied(
								[pixels.width() as usize, pixels.height() as usize],
								pixels.as_raw(),
							)),
						);
						messaging.extensions.preview_theme_editor_tab(tab);
					}
				}
			}
			Ok(Box::new(Preview {
				messaging,
				state,
				output,
				thumbnail,
				start_video: page == "video",
				frames: 0,
				scroll,
				requested: false,
				screenshot: None,
				writer: None,
				saved: completed,
				started: Instant::now(),
			}))
		}),
	)?;
	if !saved.load(Ordering::Acquire) {
		return Err("No screenshot saved".into());
	}
	Ok(())
}
