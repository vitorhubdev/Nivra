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

/// Distinct CJK scalars carried to the first probe. Longer text sets `overflow`
/// instead of dropping characters, so coverage is never certified from a
/// partial set.
const DRAWN_CHARS: usize = 64;

/// Blocks that need a system CJK face: kana, Han, Hangul and their forms.
/// Arabic Presentation Forms-B (U+FE70–U+FEFF) and Yi (U+A000–U+A4CF) are not CJK.
fn is_cjk(c: char) -> bool {
	matches!(
		c as u32,
		0x1100..=0x11ff
			| 0x2e80..=0x2fdf
			| 0x2ff0..=0x303f
			| 0x3040..=0x30ff
			| 0x3100..=0x318f
			| 0x3190..=0x31ff
			| 0x3200..=0x33ff
			| 0x3400..=0x4dbf
			| 0x4e00..=0x9fff
			| 0xa960..=0xa97f
			| 0xac00..=0xd7af
			| 0xd7b0..=0xd7ff
			| 0xf900..=0xfaff
			| 0xfe30..=0xfe6f
			| 0xff00..=0xffef
			| 0x20000..=0x323af
	)
}

#[derive(Default)]
struct CjkScan {
	checked: Vec<Weak<egui::text::LayoutJob>>,
	/// Scalars carried to the first probe, before any face is loaded.
	drawn: Vec<char>,
	/// A drawn CJK scalar has no loaded face.
	uncovered: bool,
	/// More distinct scalars appeared than `drawn` can carry; coverage cannot be
	/// certified from a partial set, so the probe stays conservative.
	overflow: bool,
}

impl CjkScan {
	/// Records one job and reports whether it drew CJK text. `faces` is the
	/// cached set, so a later script is checked with a cmap lookup on this thread.
	fn text(
		&mut self,
		job: &Arc<egui::text::LayoutJob>,
		faces: Option<&[system::LoadedFont]>,
	) -> bool {
		let index = match self
			.checked
			.binary_search_by_key(&(Arc::as_ptr(job) as usize), |entry| {
				entry.as_ptr() as usize
			}) {
			Ok(_) => return false,
			Err(index) => index,
		};
		let mut found = false;
		if !job.text.is_ascii() {
			for c in job.text.chars().filter(|c| is_cjk(*c)) {
				found = true;
				match faces {
					Some(faces) => {
						if !faces.iter().any(|font| font.covers(c)) {
							self.uncovered = true;
						}
					}
					None if self.drawn.contains(&c) => {}
					None if self.drawn.len() < DRAWN_CHARS => self.drawn.push(c),
					None => self.overflow = true,
				}
			}
		}
		// Register the job before returning: a covered CJK job must not be
		// rediscovered on every later pass, or the UI repaints forever.
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
		found
	}

	fn shape(&mut self, shape: &egui::Shape, faces: Option<&[system::LoadedFont]>) -> bool {
		match shape {
			egui::Shape::Text(text) => self.text(&text.galley.job, faces),
			egui::Shape::Vec(shapes) => {
				// Scan every child: each job must be registered, so this cannot short-circuit.
				let mut found = false;
				for shape in shapes {
					found |= self.shape(shape, faces);
				}
				found
			}
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

/// Fonts already loaded by the probe, kept in the context so a later script can
/// be checked against them without touching the disk again.
const CJK_FONTS: &str = "nivra.system-cjk-fonts";

fn loaded_fonts(ctx: &Context) -> Option<Arc<Vec<system::LoadedFont>>> {
	ctx.data(|data| data.get_temp(Id::unique(CJK_FONTS)))
}

fn store_loaded_fonts(ctx: &Context, fonts: Arc<Vec<system::LoadedFont>>) {
	ctx.data_mut(|data| data.insert_temp(Id::unique(CJK_FONTS), fonts));
}

/// Install once during application creation, before the first UI pass.
pub fn install(ctx: &Context) {
	ctx.set_fonts(definitions());
	let scan = Mutex::new(CjkScan::default());
	ctx.on_end_pass(
		"CJK fallback",
		Arc::new(move |ui| {
			let ctx = ui.ctx().clone();
			// `Notified` ends the probe; `Checking` means a worker is already on it.
			if matches!(
				status(&ctx),
				Some(SystemCjk::Notified | SystemCjk::Checking)
			) {
				return;
			}
			let fonts = loaded_fonts(&ctx);
			let needed = {
				let mut scan = scan.lock().expect("CJK scan");
				let layers: Vec<_> = ctx.memory(|memory| memory.layer_ids().collect());
				ctx.graphics(|graphics| {
					let mut needed = false;
					'layers: for layer in &layers {
						if let Some(list) = graphics.get(*layer) {
							for entry in list.all_entries() {
								needed |=
									scan.shape(&entry.shape, fonts.as_deref().map(Vec::as_slice));
								if scan.uncovered {
									break 'layers;
								}
							}
						}
					}
					needed
				})
			};
			if !needed {
				return;
			}
			// A later script can arrive after the first probe. Checking the cached
			// faces is a cmap lookup, so it stays on this thread; only the first
			// probe reads the disk, and that runs on a worker.
			let Some(_) = fonts else {
				let (drawn, overflow) = {
					let mut scan = scan.lock().expect("CJK scan");
					(
						std::mem::take(&mut scan.drawn),
						std::mem::take(&mut scan.overflow),
					)
				};
				if drawn.is_empty() {
					return;
				}
				set_status(&ctx, SystemCjk::Checking);
				let worker = ctx.clone();
				let spawned =
					std::thread::Builder::new()
						.name("cjk-font".into())
						.spawn(move || {
							install_system_fonts(&worker, &drawn, overflow);
							worker.request_repaint();
						});
				if spawned.is_err() {
					record_probe_failure(&ctx);
				}
				return;
			};
			let uncovered = {
				let mut scan = scan.lock().expect("CJK scan");
				let uncovered = scan.uncovered || scan.overflow;
				scan.uncovered = false;
				scan.overflow = false;
				scan.drawn.clear();
				uncovered
			};
			let new_status = if uncovered {
				SystemCjk::Missing
			} else {
				SystemCjk::Available
			};
			// Only a changed state needs another frame; a covered pass must settle.
			if status(&ctx) != Some(new_status) {
				set_status(&ctx, new_status);
				ctx.request_repaint();
			}
		}),
	);
	crate::design::weights_installed(ctx);
}

/// Loads the system CJK faces and records whether the drawn text is covered.
fn install_system_fonts(ctx: &Context, drawn: &[char], overflow: bool) {
	let fonts = Arc::new(system::load_fallbacks());
	if !fonts.is_empty() {
		ctx.set_fonts(definitions_with(&fonts));
	}
	let status = if overflow {
		// A partial scalar set cannot certify coverage.
		SystemCjk::Missing
	} else {
		coverage_status(&fonts, drawn)
	};
	set_status(ctx, status);
	store_loaded_fonts(ctx, fonts);
}

/// A worker could not be started: never read font files on the render thread.
/// The one-time notice is the safe fallback on a process this constrained.
fn record_probe_failure(ctx: &Context) {
	set_status(ctx, SystemCjk::Missing);
}

/// `Available` when every CJK scalar that was drawn has an installed face.
/// A machine with only a Japanese face must not warn about Korean text it never
/// showed; the notice follows the text that actually reached the screen.
fn coverage_status(fonts: &[system::LoadedFont], drawn: &[char]) -> SystemCjk {
	if drawn
		.iter()
		.all(|c| fonts.iter().any(|font| font.covers(*c)))
	{
		SystemCjk::Available
	} else {
		SystemCjk::Missing
	}
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
		assert!(!scan.text(&job, None));
		assert!(!scan.text(&job, None));
		assert_eq!(scan.checked.len(), 1);
		assert_eq!(Arc::strong_count(&job), 1);
		let old = Arc::downgrade(&job);
		Arc::make_mut(&mut job).text = "日本語 中文 한국어".into();
		assert!(
			scan.text(&job, None),
			"editing an already checked job must detect CJK"
		);
		assert!(
			old.upgrade().is_none(),
			"the replaced allocation must be gone"
		);
		assert!(
			scan.checked
				.iter()
				.any(|entry| entry.ptr_eq(&Arc::downgrade(&job))),
			"the edited job must be registered under its new identity"
		);
		for index in 0..CHECKED_JOBS * 2 {
			let job = Arc::new(egui::text::LayoutJob {
				text: format!("Synthetic {index}"),
				..Default::default()
			});
			assert!(!scan.text(&job, None));
			assert!(scan.checked.len() <= CHECKED_JOBS);
			assert!(
				scan.checked.capacity()
					* (size_of::<egui::text::LayoutJob>() + 3 * size_of::<usize>())
					<= CHECKED_BYTES
			);
		}
		assert!(scan.checked.iter().all(|entry| entry.upgrade().is_none()));
		assert!(
			scan.text(&job, None),
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
		// The real search on this machine: a face that was loaded must render its
		// sample and a missing one must degrade to replacement glyphs, never panic.
		let fonts = system::load_fallbacks();
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
			let covered = fonts.iter().any(|font| font.covers(script.sample()));
			if covered {
				assert!(
					widths[index] > 0.0,
					"{script:?} loaded but no glyph: {widths:?}"
				);
			} else {
				assert_eq!(
					widths[index], 0.0,
					"{script:?} not loaded but a glyph was found: {widths:?}"
				);
			}
		}
	}

	#[test]
	fn the_scan_remembers_the_cjk_scalars_that_were_drawn() {
		let mut scan = CjkScan::default();
		let job = Arc::new(egui::text::LayoutJob {
			text: "Latin 日本語 한국어".into(),
			..Default::default()
		});
		assert!(scan.text(&job, None));
		assert!(scan.drawn.contains(&'日'));
		assert!(scan.drawn.contains(&'한'));
		assert!(!scan.drawn.contains(&'L'));
	}

	#[test]
	fn a_covered_cjk_job_is_registered_and_not_rescanned() {
		let mut scan = CjkScan::default();
		let job = Arc::new(egui::text::LayoutJob {
			text: "日本語".into(),
			..Default::default()
		});
		assert!(scan.text(&job, None), "the first pass reports the CJK job");
		assert!(
			!scan.text(&job, None),
			"a registered job must not be rediscovered and repaint forever"
		);
		assert_eq!(scan.checked.len(), 1);
	}

	#[test]
	fn more_scalars_than_the_probe_carries_set_overflow_instead_of_dropping_them() {
		let mut scan = CjkScan::default();
		let text: String = (0..DRAWN_CHARS + 1)
			.filter_map(|offset| char::from_u32(0x4e00 + offset as u32))
			.collect();
		let job = Arc::new(egui::text::LayoutJob {
			text,
			..Default::default()
		});
		assert!(scan.text(&job, None));
		assert_eq!(scan.drawn.len(), DRAWN_CHARS);
		assert!(scan.overflow, "a partial set cannot certify coverage");
	}

	#[test]
	fn a_repeated_scalar_at_capacity_does_not_overflow() {
		let mut scan = CjkScan::default();
		let full: String = (0..DRAWN_CHARS)
			.filter_map(|offset| char::from_u32(0x4e00 + offset as u32))
			.collect();
		let repeat = Arc::new(egui::text::LayoutJob {
			text: format!("{full}{}", '\u{4e00}'),
			..Default::default()
		});
		assert!(scan.text(&repeat, None));
		assert_eq!(scan.drawn.len(), DRAWN_CHARS);
		assert!(!scan.overflow, "a repeated scalar is not new coverage");
		let extra = Arc::new(egui::text::LayoutJob {
			text: char::from_u32(0x4e00 + DRAWN_CHARS as u32)
				.expect("next Han scalar")
				.to_string(),
			..Default::default()
		});
		assert!(scan.text(&extra, None));
		assert!(scan.overflow, "a new scalar past the cap overflows");
	}

	#[test]
	fn arabic_presentation_forms_and_yi_are_not_cjk() {
		assert!(!is_cjk('\u{fe70}'), "Arabic Presentation Forms-B");
		assert!(!is_cjk('\u{feff}'), "zero-width no-break space");
		assert!(!is_cjk('\u{a000}'), "Yi syllable");
		assert!(!is_cjk('\u{a4cf}'), "Yi radical");
		assert!(is_cjk('あ'), "kana");
		assert!(is_cjk('语'), "Han");
		assert!(is_cjk('한'), "Hangul");
		assert!(is_cjk('\u{ff01}'), "fullwidth form");
		assert!(is_cjk('\u{20000}'), "Han extension B");
	}

	#[test]
	fn only_the_drawn_scripts_decide_whether_to_warn() {
		let inter = system::LoadedFont {
			name: "system:inter:0".into(),
			data: FontData::from_owned(
				std::fs::read(concat!(
					env!("CARGO_MANIFEST_DIR"),
					"/../../assets/fonts/Inter-Regular.ttf"
				))
				.expect("bundled Inter"),
			),
		};
		assert_eq!(
			coverage_status(std::slice::from_ref(&inter), &['H']),
			SystemCjk::Available,
			"a face that covers the drawn text must not warn"
		);
		assert_eq!(
			coverage_status(&[inter], &['日']),
			SystemCjk::Missing,
			"a drawn Han scalar without a face must warn"
		);
		assert_eq!(
			coverage_status(&[], &['日']),
			SystemCjk::Missing,
			"no installed face at all must warn"
		);
	}

	#[test]
	fn a_later_script_is_checked_against_the_loaded_fonts_without_touching_the_disk() {
		let ctx = Context::default();
		install(&ctx);
		// Pretend the first probe found no usable face at all: the next CJK text
		// must still be evaluated instead of being silently ignored.
		store_loaded_fonts(&ctx, Arc::new(Vec::new()));
		set_status(&ctx, SystemCjk::Available);
		let output = ctx.run_ui(Default::default(), |ui| {
			ui.label("한국어");
		});
		output.drop_without_applying_deltas();
		assert_eq!(status(&ctx), Some(SystemCjk::Missing));
	}

	#[test]
	fn a_failed_worker_records_a_failure_instead_of_reading_fonts_on_the_draw_thread() {
		let ctx = Context::default();
		install(&ctx);
		record_probe_failure(&ctx);
		assert_eq!(status(&ctx), Some(SystemCjk::Missing));
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
