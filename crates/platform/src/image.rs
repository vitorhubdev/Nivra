//! System image decoding for formats the bundled decoder cannot read.
//! iPhone photos (HEIC) decode through Windows Imaging Component on Windows;
//! everywhere else they fall back to the download affordance.

/// True when `bytes` look like ISO-BMFF HEIF/HEIC (a `ftyp` box with a HEIC-family brand).
pub fn is_heic(bytes: &[u8]) -> bool {
	if bytes.len() < 12 {
		return false;
	}
	if &bytes[4..8] != b"ftyp" {
		return false;
	}
	matches!(
		&bytes[8..12],
		b"heic"
			| b"heix" | b"hevc"
			| b"hevx" | b"heim"
			| b"heis" | b"hevm"
			| b"hevs" | b"mif1"
			| b"msf1"
	)
}

/// Decode HEIC bytes to straight-alpha RGBA with the OS decoder.
/// `max_edge`/`max_pixels` come from the caller: dimensions are rejected
/// before any pixel buffer is allocated (Codex PR #77 P1). Returns `None`
/// off Windows, for non-HEIC input, without a system codec, or over budget.
pub fn decode_heic(bytes: &[u8], max_edge: u32, max_pixels: u64) -> Option<(Vec<u8>, u32, u32)> {
	#[cfg(target_os = "windows")]
	return self::native::decode(bytes, max_edge, max_pixels);
	#[cfg(not(target_os = "windows"))]
	let _ = (bytes, max_edge, max_pixels);
	#[cfg(not(target_os = "windows"))]
	return None;
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod native {
	use windows::{
		Win32::{
			Graphics::Imaging::{
				CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGRA, IWICBitmapFrameDecode,
				IWICImagingFactory, IWICMetadataQueryReader, WICBitmapDitherTypeNone,
				WICBitmapPaletteTypeCustom, WICBitmapTransformFlipHorizontal,
				WICBitmapTransformFlipVertical, WICBitmapTransformOptions,
				WICBitmapTransformRotate0, WICBitmapTransformRotate90, WICBitmapTransformRotate180,
				WICBitmapTransformRotate270, WICDecodeMetadataCacheOnDemand,
			},
			System::{
				Com::StructuredStorage::{PROPVARIANT, PropVariantClear},
				Com::{
					CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance,
					CoInitializeEx, CoUninitialize,
				},
				Variant::VT_UI2,
			},
			UI::Shell::SHCreateMemStream,
		},
		core::{Interface, w},
	};
	/// EXIF orientation to the WIC transform that displays it upright, plus
	/// whether width and height swap (portrait iPhone photos, Codex PR #77 P2).
	pub(super) fn upright_transform(orientation: u16) -> (WICBitmapTransformOptions, bool) {
		let combine = |a: WICBitmapTransformOptions, b: WICBitmapTransformOptions| {
			WICBitmapTransformOptions(a.0 | b.0)
		};
		match orientation {
			2 => (WICBitmapTransformFlipHorizontal, false),
			3 => (WICBitmapTransformRotate180, false),
			4 => (WICBitmapTransformFlipVertical, false),
			5 => (
				combine(
					WICBitmapTransformRotate270,
					WICBitmapTransformFlipHorizontal,
				),
				true,
			),
			6 => (WICBitmapTransformRotate90, true),
			7 => (
				combine(WICBitmapTransformRotate90, WICBitmapTransformFlipHorizontal),
				true,
			),
			8 => (WICBitmapTransformRotate270, true),
			_ => (WICBitmapTransformRotate0, false),
		}
	}
	/// EXIF `System.Photo.Orientation` of a frame; missing or unreadable means upright.
	fn frame_orientation(frame: &IWICBitmapFrameDecode) -> u16 {
		// SAFETY: the PROPVARIANT is default-initialized, filled by WIC, read
		// only for a UI2 value, and always cleared before return.
		unsafe {
			let reader: IWICMetadataQueryReader = match frame.cast() {
				Ok(reader) => reader,
				Err(_) => return 1,
			};
			let mut property = PROPVARIANT::default();
			let orientation = reader
				.GetMetadataByName(w!("System.Photo.Orientation"), &mut property)
				.ok()
				.filter(|_| property.Anonymous.Anonymous.vt == VT_UI2)
				.map(|_| property.Anonymous.Anonymous.Anonymous.uiVal)
				.unwrap_or(1);
			let _ = PropVariantClear(&mut property);
			orientation
		}
	}

	pub(super) fn decode(
		bytes: &[u8],
		max_edge: u32,
		max_pixels: u64,
	) -> Option<(Vec<u8>, u32, u32)> {
		if !super::is_heic(bytes) {
			return None;
		}
		// SAFETY: COM is initialized for this call below; every interface is
		// released on scope exit and the pixel buffer is owned.
		unsafe {
			let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
			let result = decode_inner(bytes, max_edge, max_pixels);
			if initialized {
				CoUninitialize();
			}
			result
		}
	}

	fn decode_inner(bytes: &[u8], max_edge: u32, max_pixels: u64) -> Option<(Vec<u8>, u32, u32)> {
		// SAFETY: COM is initialized by the caller; all out-pointers are
		// stack-owned, the pixel buffer is owned, and every interface drops here.
		unsafe {
			let factory: IWICImagingFactory =
				CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
			let stream = SHCreateMemStream(Some(bytes))?;
			let decoder = factory
				.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
				.ok()?;
			let frame = decoder.GetFrame(0).ok()?;
			let mut width = 0;
			let mut height = 0;
			frame.GetSize(&mut width, &mut height).ok()?;
			if width == 0 || height == 0 || width > max_edge || height > max_edge {
				return None;
			}
			if u64::from(width) * u64::from(height) > max_pixels {
				return None;
			}
			let (transform, swap) = upright_transform(frame_orientation(&frame));
			let (width, height) = if swap {
				(height, width)
			} else {
				(width, height)
			};
			let flipper = factory.CreateBitmapFlipRotator().ok()?;
			flipper.Initialize(&frame, transform).ok()?;
			let converter = factory.CreateFormatConverter().ok()?;
			converter
				.Initialize(
					&flipper,
					&GUID_WICPixelFormat32bppBGRA,
					WICBitmapDitherTypeNone,
					None,
					0.0,
					WICBitmapPaletteTypeCustom,
				)
				.ok()?;
			let stride = width.checked_mul(4)?;
			let mut bgra = vec![0u8; stride as usize * height as usize];
			converter
				.CopyPixels(std::ptr::null(), stride, &mut bgra)
				.ok()?;
			// BGRA to straight-alpha RGBA for egui.
			for pixel in bgra.as_chunks_mut::<4>().0 {
				pixel.swap(0, 2);
			}
			Some((bgra, width, height))
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn heic_sniffing_accepts_heic_brands_and_rejects_common_images() {
		for brand in [
			*b"heic", *b"heix", *b"hevc", *b"hevx", *b"heim", *b"heis", *b"hevm", *b"hevs",
			*b"mif1", *b"msf1",
		] {
			let mut bytes = vec![0u8; 12];
			bytes[4..8].copy_from_slice(b"ftyp");
			bytes[8..12].copy_from_slice(&brand);
			assert!(is_heic(&bytes), "brand {brand:?}");
		}
		assert!(!is_heic(&[]));
		assert!(!is_heic(b"short"));
		assert!(!is_heic(b"\x89PNG\r\n\x1a\nftypheicXXXX"));
		assert!(!is_heic(b"\xff\xd8\xff\xe0\x00\x10ftypheic"));
		assert!(!is_heic(b"....ftypisom...."));
	}

	#[test]
	fn heic_decode_rejects_non_heic_without_touching_the_os_decoder() {
		assert!(decode_heic(&[], 16384, u64::MAX).is_none());
		assert!(decode_heic(b"definitely not an image", 16384, u64::MAX).is_none());
	}

	#[test]
	#[cfg(target_os = "windows")]
	fn upright_transform_maps_all_exif_orientations() {
		use super::native::upright_transform;
		use windows::Win32::Graphics::Imaging::{
			WICBitmapTransformFlipHorizontal, WICBitmapTransformFlipVertical,
			WICBitmapTransformRotate0, WICBitmapTransformRotate90, WICBitmapTransformRotate180,
			WICBitmapTransformRotate270,
		};
		// (orientation, expected transform bits, swaps dimensions)
		for (orientation, bits, swap) in [
			(1, WICBitmapTransformRotate0.0, false),
			(2, WICBitmapTransformFlipHorizontal.0, false),
			(3, WICBitmapTransformRotate180.0, false),
			(4, WICBitmapTransformFlipVertical.0, false),
			(
				5,
				WICBitmapTransformRotate270.0 | WICBitmapTransformFlipHorizontal.0,
				true,
			),
			(6, WICBitmapTransformRotate90.0, true),
			(
				7,
				WICBitmapTransformRotate90.0 | WICBitmapTransformFlipHorizontal.0,
				true,
			),
			(8, WICBitmapTransformRotate270.0, true),
			(0, WICBitmapTransformRotate0.0, false),
			(9, WICBitmapTransformRotate0.0, false),
		] {
			let (transform, swaps) = upright_transform(orientation);
			assert_eq!(transform.0, bits, "orientation {orientation} transform");
			assert_eq!(swaps, swap, "orientation {orientation} swap");
		}
	}
}
