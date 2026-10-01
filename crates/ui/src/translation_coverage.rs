//! Render-based translation coverage.
//!
//! Each surface is drawn with the interface language stored on the context, then
//! `i18n::drain_untranslated_keys()` must come back empty: every string the frame
//! asked the catalog for exists in pt-BR and es. This checks behaviour (what the
//! frame asks for) instead of the wording of source files.

use client_core::State;
use egui::Context;
use model::Language;

fn context(language: Language) -> Context {
	let ctx = Context::default();
	crate::design::apply(&ctx);
	crate::i18n::store_interface_language(&ctx, language);
	ctx
}

fn frame(ctx: &Context, width: f32, height: f32, mut build: impl FnMut(&mut egui::Ui)) {
	let mut output = ctx.run_ui(
		egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(width, height),
			)),
			focused: true,
			..Default::default()
		},
		|ui| build(ui),
	);
	output.textures_delta.clear();
	output.drop_without_applying_deltas();
}

const LANGUAGES: [Language; 2] = [Language::PortugueseBrazil, Language::Spanish];

/// The chat shell: sidebar, header, timeline and composer.
#[test]
fn chat_shell_renders_translated_in_both_languages() {
	for language in LANGUAGES {
		let ctx = context(language);
		let mut view = crate::MessagingUi::default();
		view.language = language;
		let mut state: State = test_support::demo_state();
		let _ = crate::i18n::drain_untranslated_keys();
		frame(&ctx, 1280.0, 820.0, |ui| {
			view.show(ui, &mut state);
		});
		let missing = crate::i18n::drain_untranslated_keys();
		assert!(missing.is_empty(), "{language:?} missing {missing:?}");
	}
}

/// A conversation with a message, an attachment and an embed card.
#[test]
fn conversation_surfaces_render_translated_in_both_languages() {
	for language in LANGUAGES {
		let ctx = context(language);
		let mut view = crate::MessagingUi::default();
		view.language = language;
		let mut state: State = test_support::demo_state();
		let channel = state.selected.expect("the fixture selects a conversation");
		let mut message = test_support::message(4242, channel);
		message.attachments.push(model::Attachment {
			id: model::Id(9),
			filename: "captura.mp4".into(),
			description: None,
			content_type: Some("video/mp4".into()),
			size: 2048,
			media: model::EmbedMedia {
				width: 640,
				height: 360,
				..Default::default()
			},
			spoiler: false,
			duration_ms: None,
			waveform: Vec::new(),
		});
		message.embeds = vec![model::Embed {
			kind: "link".into(),
			url: Some("https://example.com/artigo".into()),
			title: Some("Um artigo".into()),
			description: Some("Resumo do artigo.".into()),
			..Default::default()
		}];
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::Message(message),
		});
		let _ = crate::i18n::drain_untranslated_keys();
		frame(&ctx, 1280.0, 820.0, |ui| {
			view.show(ui, &mut state);
		});
		let missing = crate::i18n::drain_untranslated_keys();
		assert!(missing.is_empty(), "{language:?} missing {missing:?}");
	}
}

/// The voice stage, where call notices, participants and controls are painted.
#[test]
fn voice_surface_renders_translated_in_both_languages() {
	for language in LANGUAGES {
		let ctx = context(language);
		let mut view = crate::MessagingUi::default();
		view.language = language;
		let mut state: State = test_support::call_demo_state();
		let (channel, request) = state
			.voice
			.active
			.as_ref()
			.map(|call| (call.channel, call.request))
			.expect("the call fixture has an active call");
		view.screen.context = Some((state.generation, channel, request));
		let _ = crate::i18n::drain_untranslated_keys();
		let mut commands = vec![];
		frame(&ctx, 1280.0, 820.0, |ui| {
			view.call_bar(ui, &mut state, &mut commands);
		});
		let missing = crate::i18n::drain_untranslated_keys();
		assert!(missing.is_empty(), "{language:?} missing {missing:?}");
	}
}

/// The guard itself: a key the catalog does not know must be reported, otherwise
/// the surface tests above would pass while a string stays English.
#[test]
fn the_guard_reports_a_key_without_translation() {
	let ctx = context(Language::PortugueseBrazil);
	let _ = crate::i18n::drain_untranslated_keys();
	frame(&ctx, 240.0, 120.0, |ui| {
		ui.label(crate::tr_ui!(ui, "Guard probe with no translation"));
	});
	assert_eq!(
		crate::i18n::drain_untranslated_keys(),
		vec!["Guard probe with no translation".to_owned()]
	);
}
