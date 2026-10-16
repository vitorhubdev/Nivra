//! Bounded, callback-owned D3D11 staging. No capture, device selection or extra worker.
#![allow(unsafe_code)]

use super::{MAX_FRAME_HEIGHT, MAX_FRAME_WIDTH, MAX_RAW_BYTES};
use windows_capture::frame::Frame;
use windows_capture_api::Win32::Graphics::{
	Direct3D11::{
		D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC,
		D3D11_USAGE_STAGING, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
	},
	Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
};

const FAILED: &str = "Captured frame could not be read";

#[derive(Default)]
pub(super) struct Reader {
	staging: Option<Cached>,
}

struct Cached {
	texture: ID3D11Texture2D,
	device: ID3D11Device,
	desc: D3D11_TEXTURE2D_DESC,
}

impl Reader {
	pub(super) fn read<'a>(&'a mut self, frame: &'a Frame) -> Result<Mapped<'a>, &'static str> {
		self.map(
			frame.device(),
			frame.device_context(),
			frame.as_raw_texture(),
			*frame.desc(),
		)
	}

	fn map<'a>(
		&'a mut self,
		device: &ID3D11Device,
		context: &'a ID3D11DeviceContext,
		source: &ID3D11Texture2D,
		desc: D3D11_TEXTURE2D_DESC,
	) -> Result<Mapped<'a>, &'static str> {
		if desc.Width == 0
			|| desc.Height == 0
			|| desc.Width > MAX_FRAME_WIDTH
			|| desc.Height > MAX_FRAME_HEIGHT
			|| desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
			|| desc.MipLevels != 1
			|| desc.ArraySize != 1
			|| desc.SampleDesc.Count != 1
		{
			return Err(FAILED);
		}
		let recreate = self.staging.as_ref().is_none_or(|cached| {
			cached.device != *device
				|| cached.desc.Width != desc.Width
				|| cached.desc.Height != desc.Height
				|| cached.desc.Format != desc.Format
		});
		if recreate {
			// Release the old allocation first: resizing cannot retain two 4K surfaces.
			self.staging = None;
			let staging_desc = D3D11_TEXTURE2D_DESC {
				Usage: D3D11_USAGE_STAGING,
				BindFlags: 0,
				CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
				MiscFlags: 0,
				..desc
			};
			let mut texture = None;
			// SAFETY: Descriptor is single-sample BGRA within the existing 4K cap;
			// the device is the capture source's own device, never a different adapter.
			unsafe { device.CreateTexture2D(&staging_desc, None, Some(&mut texture)) }
				.map_err(|_| FAILED)?;
			self.staging = Some(Cached {
				texture: texture.ok_or(FAILED)?,
				device: device.clone(),
				desc: staging_desc,
			});
		}
		let texture = &self.staging.as_ref().ok_or(FAILED)?.texture;
		let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
		// SAFETY: Source and staging have matching geometry/format on the same device.
		// The callback is the exclusive user of this immediate context here. The guard
		// borrows this reader, so neither reuse nor destruction can precede Unmap.
		unsafe {
			context.CopyResource(texture, source);
			context
				.Map(texture, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
				.map_err(|_| FAILED)?;
		}
		let guard = Mapped {
			texture,
			context,
			mapped,
			length: 0,
		};
		guard.validate(desc.Width, desc.Height)
	}
}

pub(super) struct Mapped<'a> {
	texture: &'a ID3D11Texture2D,
	context: &'a ID3D11DeviceContext,
	mapped: D3D11_MAPPED_SUBRESOURCE,
	length: usize,
}

impl Mapped<'_> {
	fn validate(mut self, width: u32, height: u32) -> Result<Self, &'static str> {
		let length = (self.mapped.RowPitch as usize)
			.checked_mul(height as usize)
			.ok_or(FAILED)?;
		if self.mapped.pData.is_null() || self.mapped.RowPitch < width * 4 || length > MAX_RAW_BYTES
		{
			return Err(FAILED); // Drop still unmaps on every validation error.
		}
		self.length = length;
		Ok(self)
	}

	pub(super) fn row_pitch(&self) -> u32 {
		self.mapped.RowPitch
	}

	pub(super) fn bytes(&self) -> &[u8] {
		// SAFETY: Validated mapped BGRA rows; no mutable slice escapes. The borrow is
		// shorter than the guard's lifetime, which owns the matching Unmap.
		unsafe { std::slice::from_raw_parts(self.mapped.pData.cast(), self.length) }
	}
}

impl Drop for Mapped<'_> {
	fn drop(&mut self) {
		// SAFETY: Constructed only after a successful Map, on its capture callback.
		unsafe { self.context.Unmap(self.texture, 0) };
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use windows_capture_api::Win32::{
		Foundation::HMODULE,
		Graphics::{
			Direct3D::{D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP},
			Direct3D11::{
				D3D11_CPU_ACCESS_WRITE, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ_WRITE,
				D3D11_SDK_VERSION, D3D11_SUBRESOURCE_DATA, D3D11_USAGE_DEFAULT, D3D11CreateDevice,
			},
			Dxgi::Common::DXGI_SAMPLE_DESC,
		},
	};

	fn device(driver: D3D_DRIVER_TYPE) -> (ID3D11Device, ID3D11DeviceContext) {
		let (mut device, mut context) = (None, None);
		// SAFETY: Synthetic D3D device only. No screen, window, microphone or account.
		unsafe {
			D3D11CreateDevice(
				None,
				driver,
				HMODULE::default(),
				D3D11_CREATE_DEVICE_BGRA_SUPPORT,
				None,
				D3D11_SDK_VERSION,
				Some(&mut device),
				None,
				Some(&mut context),
			)
			.unwrap();
		}
		(device.unwrap(), context.unwrap())
	}

	fn source(
		device: &ID3D11Device,
		width: u32,
		height: u32,
		salt: u8,
	) -> (ID3D11Texture2D, D3D11_TEXTURE2D_DESC, Vec<u8>) {
		let pixels: Vec<u8> = (0..width as usize * height as usize * 4)
			.map(|i| ((i * 17 + salt as usize) % 251) as u8)
			.collect();
		let desc = D3D11_TEXTURE2D_DESC {
			Width: width,
			Height: height,
			MipLevels: 1,
			ArraySize: 1,
			Format: DXGI_FORMAT_B8G8R8A8_UNORM,
			SampleDesc: DXGI_SAMPLE_DESC {
				Count: 1,
				Quality: 0,
			},
			Usage: D3D11_USAGE_DEFAULT,
			..Default::default()
		};
		let data = D3D11_SUBRESOURCE_DATA {
			pSysMem: pixels.as_ptr().cast(),
			SysMemPitch: width * 4,
			SysMemSlicePitch: width * height * 4,
		};
		let mut texture = None;
		// SAFETY: Pixel vector covers all initialized rows and outlives CreateTexture2D.
		unsafe {
			device
				.CreateTexture2D(&desc, Some(&data), Some(&mut texture))
				.unwrap()
		};
		(texture.unwrap(), desc, pixels)
	}

	fn assert_rows(mapped: &Mapped<'_>, desc: D3D11_TEXTURE2D_DESC, expected: &[u8]) {
		for (actual, expected) in mapped
			.bytes()
			.chunks_exact(mapped.row_pitch() as usize)
			.zip(expected.chunks_exact(desc.Width as usize * 4))
		{
			assert_eq!(&actual[..expected.len()], expected);
		}
	}

	#[test]
	fn staging_reuses_storage_and_tracks_pixels_resize_and_device() {
		let (device, context) = device(D3D_DRIVER_TYPE_WARP);
		let mut reader = Reader::default();
		let mut previous = None;
		for (width, height, salt) in [
			(17, 9, 1),
			(17, 9, 2),
			(33, 7, 3),
			(1920, 1080, 4),
			(3840, 2160, 5),
		] {
			let (source, desc, expected) = source(&device, width, height, salt);
			{
				let mapped = reader.map(&device, &context, &source, desc).unwrap();
				assert_rows(&mapped, desc, &expected);
			}
			let cached = &reader.staging.as_ref().unwrap().texture;
			if salt == 2 {
				assert_eq!(cached, previous.as_ref().unwrap());
			}
			if salt == 3 {
				assert_ne!(cached, previous.as_ref().unwrap());
			}
			previous = Some(cached.clone());
		}
		let (other_device, other_context) = self::device(D3D_DRIVER_TYPE_WARP);
		let (source, desc, expected) = source(&other_device, 3840, 2160, 6);
		assert_rows(
			&reader
				.map(&other_device, &other_context, &source, desc)
				.unwrap(),
			desc,
			&expected,
		);
		assert_ne!(reader.staging.as_ref().unwrap().texture, previous.unwrap());
	}

	#[test]
	fn invalid_mapping_unmaps_and_bounds_precede_copy() {
		let (device, context) = device(D3D_DRIVER_TYPE_WARP);
		let (source, desc, expected) = source(&device, 17, 9, 7);
		let mut reader = Reader::default();
		// Simulate failed post-map validation and immediately reuse the texture.
		let mapped = reader.map(&device, &context, &source, desc).unwrap();
		assert!(mapped.validate(MAX_FRAME_WIDTH, 9).is_err());
		assert_rows(
			&reader.map(&device, &context, &source, desc).unwrap(),
			desc,
			&expected,
		);
		for (width, height) in [
			(0, 9),
			(17, 0),
			(MAX_FRAME_WIDTH + 1, 9),
			(17, MAX_FRAME_HEIGHT + 1),
		] {
			assert!(
				reader
					.map(
						&device,
						&context,
						&source,
						D3D11_TEXTURE2D_DESC {
							Width: width,
							Height: height,
							..desc
						}
					)
					.is_err()
			);
		}
	}

	fn legacy(
		device: &ID3D11Device,
		context: &ID3D11DeviceContext,
		source: &ID3D11Texture2D,
		desc: D3D11_TEXTURE2D_DESC,
	) -> (ID3D11Texture2D, D3D11_MAPPED_SUBRESOURCE) {
		let mut texture = None;
		let desc = D3D11_TEXTURE2D_DESC {
			Usage: D3D11_USAGE_STAGING,
			BindFlags: 0,
			MiscFlags: 0,
			CPUAccessFlags: (D3D11_CPU_ACCESS_READ.0 | D3D11_CPU_ACCESS_WRITE.0) as u32,
			..desc
		};
		let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
		// SAFETY: Independent benchmark of the former allocate/copy/map-per-frame path,
		// using the same validated synthetic source and matching caller-owned Unmap.
		unsafe {
			device
				.CreateTexture2D(&desc, None, Some(&mut texture))
				.unwrap();
			let texture = texture.as_ref().unwrap();
			context.CopyResource(texture, source);
			context
				.Map(texture, 0, D3D11_MAP_READ_WRITE, 0, Some(&mut mapped))
				.unwrap();
		}
		(texture.unwrap(), mapped)
	}

	#[test]
	#[ignore = "synthetic GPU readback benchmark; no screen capture; run explicitly with --nocapture"]
	fn synthetic_staging_readback_benchmark() {
		let (device, context) = device(D3D_DRIVER_TYPE_HARDWARE);
		for (width, height) in [(1920, 1080), (3840, 2160)] {
			let (source, desc, _) = source(&device, width, height, 3);
			let mut reader = Reader::default();
			let mut timings = [Vec::new(), Vec::new()];
			for batch in 0..6 {
				// Alternate order to avoid crediting one path for GPU clock warmup.
				for index in if batch % 2 == 0 { [0, 1] } else { [1, 0] } {
					let started = std::time::Instant::now();
					for _ in 0..30 {
						if index == 0 {
							let (texture, mapped) = legacy(&device, &context, &source, desc);
							let guard = Mapped {
								texture: &texture,
								context: &context,
								mapped,
								length: 0,
							}
							.validate(width, height)
							.unwrap();
							std::hint::black_box((
								guard.bytes()[0],
								guard.bytes()[guard.bytes().len() - 1],
							));
						} else {
							let guard = reader.map(&device, &context, &source, desc).unwrap();
							std::hint::black_box((
								guard.bytes()[0],
								guard.bytes()[guard.bytes().len() - 1],
							));
						}
					}
					if batch != 0 {
						timings[index].push(started.elapsed().as_secs_f64() * 1000.0 / 30.0);
					}
				}
			}
			for samples in &mut timings {
				samples.sort_by(f64::total_cmp);
			}
			println!(
				"{width}x{height}: allocate/map_rw_ms={:.4}; reuse/map_read_ms={:.4}; delta_percent={:.2}",
				timings[0][2],
				timings[1][2],
				(timings[1][2] / timings[0][2] - 1.0) * 100.0
			);
		}
	}
}
