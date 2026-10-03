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
/// Returns `None` off Windows, for non-HEIC input, or when the system has no
/// HEIC codec (then the caller keeps the download fallback).
pub fn decode_heic(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
	#[cfg(target_os = "windows")]
	return self::native::decode(bytes);
	#[cfg(not(target_os = "windows"))]
	let _ = bytes;
	#[cfg(not(target_os = "windows"))]
	return None;
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod native {
	use windows::Win32::{
		Graphics::Imaging::{
			CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGRA, IWICImagingFactory,
			WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
		},
		System::Com::{
			CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
			CoUninitialize,
		},
		UI::Shell::SHCreateMemStream,
	};
	/// Largest edge WIC may return (matches the avatar canvas bound).
	const MAX_EDGE: u32 = 16384;
	/// Largest pixel count WIC may return (256 megapixels of RGBA).
	const MAX_AREA: u64 = 256 * 1024 * 1024;

	pub(super) fn decode(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
		if !super::is_heic(bytes) {
			return None;
		}
		// SAFETY: COM is initialized for this call below; every interface is
		// released on scope exit and the pixel buffer is owned.
		unsafe {
			let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
			let result = decode_inner(bytes);
			if initialized {
				CoUninitialize();
			}
			result
		}
	}

	fn decode_inner(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
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
			if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE {
				return None;
			}
			if u64::from(width) * u64::from(height) > MAX_AREA {
				return None;
			}
			let converter = factory.CreateFormatConverter().ok()?;
			converter
				.Initialize(
					&frame,
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
		assert!(decode_heic(&[]).is_none());
		assert!(decode_heic(b"definitely not an image").is_none());
	}
}
