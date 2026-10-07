//! Bundled OFL fallback faces. CJK comes from the operating system.
//!
//! Inter leads proportional text and two heavier faces provide Discord-style
//! emphasis (egui has no synthetic bold); Noto Sans Arabic and Noto Sans Math
//! stay embedded because no stock desktop ships them everywhere. Japanese,
//! Chinese and Korean text no longer embeds a face: a worker reads the
//! installed system fonts once (see [`system`]), installs the faces that cover
//! kana, Han and Hangul into egui's fallback families, and warns once when no
//! such font is installed.
use egui::{Context, FontData, FontDefinitions, FontFamily, Id};
use std::sync::{Arc, Mutex, Weak};

mod system;

const ARABIC: &[u8] = include_bytes!("../../../assets/fonts/NotoSansArabic.ttf");
const MATH: &[u8] = include_bytes!("../../../assets/fonts/NotoSansMath-Regular.otf");
const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../../../assets/fonts/Inter-Medium.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf");

const CHECKED_JOBS: usize = 512;
const CHECKED_BYTES: usize = 128 * 1024;
// Weak references retain only the fixed-size Arc allocation, never text or meshes.
const _: () = assert!(
	CHECKED_JOBS * (size_of::<egui::text::LayoutJob>() + 3 * size_of::<usize>()) <= CHECKED_BYTES
);

#[derive(Default)]
struct CjkScan {
	checked: Vec<Weak<egui::text::LayoutJob>>,
}

impl CjkScan {
	fn text(&mut self, job: &Arc<egui::text::LayoutJob>) -> bool {
		let index = match self
			.checked
			.binary_search_by_key(&(Arc::as_ptr(job) as usize), |entry| {
				entry.as_ptr() as usize
			}) {
			Ok(_) => return false,
			Err(index) => index,
		};
		if !job.text.is_ascii() && job.text.chars().any(|c| matches!(c as u32, 0x1100..=0x11ff | 0x2e80..=0xa4cf | 0xa960..=0xa97f | 0xac00..=0xd7af | 0xd7b0..=0xd7ff | 0xf900..=0xfaff | 0xfe30..=0xffef | 0x20000..=0x323af)) {
			return true;
		}
		// Keep allocation identities alive so allocator address reuse cannot hide new text.
		// Arc::make_mut also dissociates these weak references before editing a job.
		if self.checked.capacity() == 0 {
			self.checked.reserve_exact(CHECKED_JOBS);
		}
		// ponytail: clear the fixed cache at capacity; unusually busy views rescan text.
		let index = if self.checked.len() == CHECKED_JOBS {
			self.checked.clear();
			0
		} else {
			index
		};
		self.checked.insert(index, Arc::downgrade(job));
		false
	}

	fn shape(&mut self, shape: &egui::Shape) -> bool {
		match shape {
			egui::Shape::Text(text) => self.text(&text.galley.job),
			egui::Shape::Vec(shapes) => shapes.iter().any(|shape| self.shape(shape)),
			_ => false,
		}
	}
}

/// State of the one-time CJK probe, stored per egui context so it dies with it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SystemCjk {
	/// A CJK glyph was drawn; the installed fonts are being checked off-thread.
	Checking,
	/// An installed font covers every script; nothing to report.
	Available,
	/// CJK text was drawn but at least one script has no installed font.
	Missing,
	/// The notice was shown; it must not repeat in this window.
	Notified,
}

const CJK_STATUS: &str = "nivra.system-cjk";

fn status(ctx: &Context) -> Option<SystemCjk> {
	ctx.data(|data| data.get_temp(Id::unique(CJK_STATUS)))
}

pub(crate) fn set_status(ctx: &Context, value: SystemCjk) {
	ctx.data_mut(|data| data.insert_temp(Id::unique(CJK_STATUS), value));
}

/// True once per context, when CJK text was drawn with no installed font behind it.
pub(crate) fn take_cjk_notice(ctx: &Context) -> bool {
	if status(ctx) == Some(SystemCjk::Missing) {
		set_status(ctx, SystemCjk::Notified);
		return true;
	}
	false
}

/// Install once during application creation, before the first UI pass.
pub fn install(ctx: &Context) {
	ctx.set_fonts(definitions());
	let scan = Mutex::new(CjkScan::default());
	ctx.on_end_pass(
		"CJK fallback",
		std::sync::Arc::new(move |ui| {
			let ctx = ui.ctx().clone();
			if status(&ctx).is_some() {
				return;
			}
			let needed = {
				let mut scan = scan.lock().expect("CJK scan");
				let layers: Vec<_> = ctx.memory(|memory| memory.layer_ids().collect());
				let needed = ctx.graphics(|graphics| {
					layers.iter().any(|layer| {
						graphics.get(*layer).is_some_and(|list| {
							list.all_entries().any(|entry| scan.shape(&entry.shape))
						})
					})
				});
				if needed {
					*scan = CjkScan::default();
				}
				needed
			};
			if !needed {
				return;
			}
			set_status(&ctx, SystemCjk::Checking);
			// Reading installed font files is disk work; never do it on the draw thread.
			let worker = ctx.clone();
			let spawned = std::thread::Builder::new()
				.name("cjk-font".into())
				.spawn(move || {
					install_system_fonts(&worker);
					worker.request_repaint();
				});
			if spawned.is_err() {
				install_system_fonts(&ctx);
				ctx.request_repaint();
			}
		}),
	);
	crate::design::weights_installed(ctx);
}

/// Loads the system CJK faces and records whether every script is covered.
fn install_system_fonts(ctx: &Context) {
	let (fonts, missing) = system::load_fallbacks();
	if !fonts.is_empty() {
		ctx.set_fonts(definitions_with(&fonts));
	}
	let status = if missing.is_empty() {
		SystemCjk::Available
	} else {
		SystemCjk::Missing
	};
	set_status(ctx, status);
}

fn latin(data: &'static [u8]) -> FontData {
	let mut font = FontData::from_static(data);
	font.tweak.hinting = Some(false);
	font.tweak.subpixel_binning = Some(true);
	font
}

/// The bundled faces plus the system CJK faces that were found.
fn definitions_with(fallbacks: &[system::LoadedFont]) -> FontDefinitions {
	let mut definitions = definitions();
	for font in fallbacks {
		definitions
			.font_data
			.insert(font.name.clone(), font.data.clone().into());
		for family in [
			FontFamily::Proportional,
			FontFamily::Monospace,
			FontFamily::Name(crate::design::MEDIUM.into()),
			FontFamily::Name(crate::design::SEMIBOLD.into()),
		] {
			definitions
				.families
				.entry(family)
				.or_default()
				.push(font.name.clone());
		}
	}
	definitions
}

fn definitions() -> FontDefinitions {
	let mut definitions = FontDefinitions::default();
	// Inter leads proportional text; two heavier faces provide Discord-style emphasis
	// (egui has no synthetic bold). Each weight family falls back to egui's defaults.
	let weights = [
		(FontFamily::Proportional, "Inter", INTER),
		(
			FontFamily::Name(crate::design::MEDIUM.into()),
			"Inter Medium",
			INTER_MEDIUM,
		),
		(
			FontFamily::Name(crate::design::SEMIBOLD.into()),
			"Inter SemiBold",
			INTER_SEMIBOLD,
		),
	];
	let defaults = definitions.families[&FontFamily::Proportional].clone();
	for (family, name, data) in weights {
		definitions
			.font_data
			.insert(name.into(), latin(data).into());
		let list = definitions.families.entry(family).or_default();
		list.retain(|existing| !defaults.contains(existing));
		list.insert(0, name.into());
		list.extend(defaults.iter().cloned());
	}
	for (name, data) in [
		("Noto Sans Arabic", FontData::from_static(ARABIC)),
		("Noto Sans Math", FontData::from_static(MATH)),
	] {
		definitions.font_data.insert(name.into(), data.into());
		for family in [
			FontFamily::Proportional,
			FontFamily::Monospace,
			FontFamily::Name(crate::design::MEDIUM.into()),
			FontFamily::Name(crate::design::SEMIBOLD.into()),
		] {
			definitions
				.families
				.entry(family)
				.or_default()
				.push(name.into());
		}
	}
	definitions
}

#[cfg(test)]
mod tests {
	use super::*;
	use egui::FontId;
	use skrifa::MetadataProvider;

	#[test]
	fn cjk_scan_reuses_immutable_jobs_without_retaining_their_text() {
		let mut scan = CjkScan::default();
		let mut job = Arc::new(egui::text::LayoutJob {
			text: "Latin — čeština العربية".into(),
			..Default::default()
		});
		assert!(!scan.text(&job));
		assert!(!scan.text(&job));
		assert_eq!(scan.checked.len(), 1);
		assert_eq!(Arc::strong_count(&job), 1);
		Arc::make_mut(&mut job).text = "日本語 中文 한국어".into();
		assert!(
			scan.text(&job),
			"editing an already checked job must detect CJK"
		);
		assert!(scan.checked[0].upgrade().is_none());
		for index in 0..CHECKED_JOBS * 2 {
			let job = Arc::new(egui::text::LayoutJob {
				text: format!("Synthetic {index}"),
				..Default::default()
			});
			assert!(!scan.text(&job));
			assert!(scan.checked.len() <= CHECKED_JOBS);
			assert!(
				scan.checked.capacity()
					* (size_of::<egui::text::LayoutJob>() + 3 * size_of::<usize>())
					<= CHECKED_BYTES
			);
		}
		assert!(scan.checked.iter().all(|entry| entry.upgrade().is_none()));
		assert!(
			scan.text(&job),
			"cache rollover must not suppress new CJK text"
		);
	}

	#[test]
	fn cjk_text_starts_the_system_font_probe() {
		let ctx = Context::default();
		install(&ctx);
		for _ in 0..3 {
			ctx.run_ui(Default::default(), |ui| {
				ui.label("Synthetic Latin text");
			})
			.drop_without_applying_deltas();
			assert!(status(&ctx).is_none(), "no CJK text, no probe");
		}
		let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
		loop {
			ctx.run_ui(Default::default(), |ui| {
				ui.label("日本語");
			})
			.drop_without_applying_deltas();
			if matches!(
				status(&ctx),
				Some(SystemCjk::Available | SystemCjk::Missing)
			) {
				break;
			}
			assert!(
				std::time::Instant::now() < deadline,
				"the system font probe never finished"
			);
			std::thread::sleep(std::time::Duration::from_millis(5));
		}
	}

	#[test]
	fn a_missing_system_font_is_noticed_once_per_context() {
		let ctx = Context::default();
		install(&ctx);
		assert!(!take_cjk_notice(&ctx));
		assert_eq!(status(&ctx), None);
		set_status(&ctx, SystemCjk::Missing);
		assert!(take_cjk_notice(&ctx));
		assert_eq!(status(&ctx), Some(SystemCjk::Notified));
		assert!(!take_cjk_notice(&ctx));
	}

	#[test]
	fn definitions_keep_inter_arabic_and_math_but_embed_no_cjk_face() {
		let definitions = definitions();
		for name in ["Inter", "Inter Medium", "Inter SemiBold"] {
			assert!(definitions.font_data.contains_key(name), "{name} missing");
		}
		assert!(definitions.font_data.contains_key("Noto Sans Arabic"));
		assert!(definitions.font_data.contains_key("Noto Sans Math"));
		for family in [
			FontFamily::Proportional,
			FontFamily::Monospace,
			FontFamily::Name(crate::design::MEDIUM.into()),
			FontFamily::Name(crate::design::SEMIBOLD.into()),
		] {
			for name in &definitions.families[&family] {
				let data = &definitions.font_data[name];
				let font = skrifa::FontRef::from_index(data.bytes(), data.index)
					.expect("valid bundled font");
				for c in "日本語かなカナ中文汉字繁體한국어".chars() {
					assert!(
						font.charmap()
							.map(c)
							.is_none_or(|id| id == skrifa::GlyphId::NOTDEF),
						"{name} claims CJK scalar {c:?}; those come from the system"
					);
				}
			}
		}
	}

	#[test]
	fn bundled_fallbacks_cover_non_cjk_multilingual_text() {
		let total =
			ARABIC.len() + MATH.len() + INTER.len() + INTER_MEDIUM.len() + INTER_SEMIBOLD.len();
		assert!(
			total <= 4 * 1024 * 1024,
			"bundled faces grew: {total} bytes"
		);
		let definitions = definitions();
		for family in [FontFamily::Proportional, FontFamily::Monospace] {
			let faces: Vec<_> = definitions.families[&family]
				.iter()
				.map(|name| {
					let data = &definitions.font_data[name];
					skrifa::FontRef::from_index(data.bytes(), data.index)
						.expect("valid bundled font")
				})
				.collect();
			for c in "Hello, العربية مَرْحَبًا 𝖘𝖓𝖎𝖎𝖝. é e\u{301}".chars()
			{
				assert!(
					faces.iter().any(|face| {
						face.charmap()
							.map(c)
							.is_some_and(|id| id != skrifa::GlyphId::NOTDEF)
					}),
					"missing glyph: {c} ({c:?})"
				);
			}
		}
	}

	#[test]
	fn system_fonts_render_cjk_when_the_os_has_them() {
		// The real search on this machine: covered scripts must have a glyph and
		// uncovered ones must degrade to replacement glyphs, never a panic.
		let (fonts, missing) = system::load_fallbacks();
		let ctx = Context::default();
		ctx.set_fonts(definitions_with(&fonts));
		let mut widths = [0.0_f32; 3];
		let output = ctx.run_ui(Default::default(), |ui| {
			ui.fonts_mut(|fonts| {
				let font = FontId::proportional(14.0);
				for (index, script) in system::SCRIPTS.into_iter().enumerate() {
					widths[index] = fonts.glyph_width(&font, script.sample());
				}
			});
		});
		output.drop_without_applying_deltas();
		for (index, script) in system::SCRIPTS.into_iter().enumerate() {
			if missing.contains(&script) {
				assert_eq!(
					widths[index], 0.0,
					"{script:?} reported missing but a glyph was found: {widths:?}"
				);
			} else {
				assert!(
					widths[index] > 0.0,
					"{script:?} reported covered but no glyph: {widths:?}"
				);
			}
		}
	}

	#[test]
	fn latin_faces_are_rasterized_without_truetype_hinting() {
		for data in [INTER, INTER_MEDIUM, INTER_SEMIBOLD] {
			let font = skrifa::FontRef::new(data).expect("valid bundled font");
			for table in ["glyf", "fpgm", "prep"] {
				let tag = skrifa::Tag::new(table.as_bytes().try_into().unwrap());
				assert!(
					font.table_data(tag).is_some(),
					"Inter must be the TrueType build, missing `{table}`",
				);
			}
		}
		let ctx = Context::default();
		install(&ctx);
		crate::design::apply(&ctx);
		for theme in [egui::Theme::Dark, egui::Theme::Light] {
			let options = &ctx.style_of(theme).visuals.text_options;
			assert!(options.subpixel_binning);
			assert!(!options.font_hinting);
		}
		assert_eq!(
			ctx.style_of(egui::Theme::Dark)
				.visuals
				.text_options
				.color_transfer_function,
			egui::epaint::FontColorTransferFunction::Gamma(0.5)
		);
		let tweaks = definitions()
			.font_data
			.iter()
			.filter(|(name, _)| name.starts_with("Inter"))
			.map(|(_, data)| (data.tweak.hinting, data.tweak.subpixel_binning))
			.collect::<Vec<_>>();
		assert_eq!(tweaks, vec![(Some(false), Some(true)); 3]);
	}
}
