//! Opt-out native tray icon. Minimizing keeps its normal window behavior; the application
//! decides what closing does: hide when supported, otherwise ask the compositor to minimize.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Event {
	Show = 1,
	Unavailable = 2,
	Quit = 4,
	#[cfg(target_os = "linux")]
	Minimize = 8,
}

pub const fn supported() -> bool {
	cfg!(any(
		target_os = "windows",
		target_os = "macos",
		target_os = "linux"
	))
}

/// Call state shown on the tray icon, like Discord's tray variants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Voice {
	#[default]
	Idle,
	Connected,
	Muted,
	Deafened,
}

/// Square straight-alpha RGBA tray icon: the app icon scaled to `size`, with a red dot for
/// unread mentions (top right) and a call dot (bottom right: green, or red with a slash when
/// muted and a bar when deafened). Each dot is cut out of the icon so it reads at 16 px.
pub fn status_icon(png: &[u8], size: u32, mentions: bool, voice: Voice) -> Option<Vec<u8>> {
	let base = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?;
	let mut pixels = base
		.resize_exact(size, size, image::imageops::FilterType::Lanczos3)
		.into_rgba8()
		.into_raw();
	let scale = size as f32 / 32.0;
	const GREEN: [u8; 3] = [0x23, 0xa5, 0x5a];
	const RED: [u8; 3] = [0xf2, 0x3f, 0x43];
	let mut dots = Vec::new();
	if mentions {
		dots.push(((26.0, 6.0), 5.5, RED, None));
	}
	match voice {
		Voice::Idle => {}
		Voice::Connected => dots.push(((24.5, 24.5), 7.0, GREEN, None)),
		Voice::Muted => dots.push(((24.5, 24.5), 7.0, RED, Some(true))),
		Voice::Deafened => dots.push(((24.5, 24.5), 7.0, RED, Some(false))),
	}
	for y in 0..size {
		for x in 0..size {
			let index = ((y * size + x) * 4) as usize;
			let pixel = &mut pixels[index..index + 4];
			for &((cx, cy), radius, color, glyph) in &dots {
				let (cx, cy, radius) = (cx * scale, cy * scale, radius * scale);
				// 4x4 supersampling for smooth edges at tray sizes.
				let cover = |inside: &dyn Fn(f32, f32) -> bool| {
					let hits = (0..16)
						.filter(|sample| {
							let px = x as f32 + (sample % 4) as f32 * 0.25 + 0.125;
							let py = y as f32 + (sample / 4) as f32 * 0.25 + 0.125;
							inside(px, py)
						})
						.count();
					hits as f32 / 16.0
				};
				let within =
					|r: f32| move |px: f32, py: f32| (px - cx).powi(2) + (py - cy).powi(2) <= r * r;
				let gap = cover(&within(radius + 1.6 * scale));
				pixel[3] = (f32::from(pixel[3]) * (1.0 - gap)) as u8;
				let fill = cover(&within(radius));
				let bar = 1.1 * scale;
				let mark = match glyph {
					None => 0.0,
					Some(true) => cover(&|px, py| {
						let (dx, dy) = (px - cx, py - cy);
						(dx + dy).abs() <= bar * std::f32::consts::SQRT_2
							&& dx.abs() <= radius * 0.55
							&& dy.abs() <= radius * 0.55
					}),
					Some(false) => {
						cover(&|px, py| (py - cy).abs() <= bar && (px - cx).abs() <= radius * 0.55)
					}
				};
				if fill > 0.0 {
					let ink: [f32; 3] =
						std::array::from_fn(|i| f32::from(color[i]) * (1.0 - mark) + 255.0 * mark);
					let under = f32::from(pixel[3]) / 255.0;
					let alpha = fill + under * (1.0 - fill);
					for (channel, ink) in pixel[..3].iter_mut().zip(ink) {
						let blended =
							(ink * fill + f32::from(*channel) * under * (1.0 - fill)) / alpha;
						*channel = blended.round().clamp(0.0, 255.0) as u8;
					}
					pixel[3] = (alpha * 255.0).round() as u8;
				}
			}
		}
	}
	Some(pixels)
}

#[cfg(any(target_os = "windows", target_os = "macos", test))]
#[derive(Default)]
struct Events(std::cell::Cell<u8>);

#[cfg(any(target_os = "windows", target_os = "macos", test))]
impl Events {
	fn push(&self, event: Event) {
		self.0.set(self.0.get() | event as u8);
	}
	fn take(&self) -> Option<Event> {
		let event = [Event::Quit, Event::Unavailable, Event::Show]
			.into_iter()
			.find(|event| self.0.get() & *event as u8 != 0)?;
		self.0.set(self.0.get() & !(event as u8));
		Some(event)
	}
}

#[cfg(target_os = "linux")]
#[path = "tray/linux.rs"]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::Tray;

#[cfg(target_os = "windows")]
pub use native::Tray;

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
#[path = "tray/macos.rs"]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::Tray;

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
pub struct Tray;

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
impl Tray {
	pub fn new(
		_window: std::sync::Arc<winit::window::Window>,
		_wake: impl Fn() + 'static,
	) -> Result<Self, &'static str> {
		Err("The tray icon is unavailable on this platform.")
	}
	pub fn take_event(&self) -> Option<Event> {
		None
	}
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod native {
	use super::{Event, Events};
	use std::{cell::Cell, rc::Rc, sync::Arc};
	use windows::{
		Win32::{
			Foundation::{HANDLE, HWND, LPARAM, LRESULT, POINT, WPARAM},
			System::Threading::GetCurrentThreadId,
			UI::{Shell::*, WindowsAndMessaging::*},
		},
		core::w,
	};
	use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

	const ID: usize = 0x5352;
	const SHOW: usize = 1;
	const QUIT: usize = 2;
	const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
	const UNAVAILABLE: &str = "The Windows tray is unavailable. The window will remain accessible.";

	/// UI-thread-owned registration: no worker, timer or allocating event queue.
	/// The retained window and Rc prevent cross-thread drop or a dangling subclass callback.
	pub struct Tray {
		state: Rc<State>,
		_window: Arc<winit::window::Window>,
	}

	struct State {
		icon: Cell<NOTIFYICONDATAW>,
		/// Status icon created by `set_icon`; the window's own icon is only borrowed.
		owned: Cell<Option<HICON>>,
		menu: HMENU,
		previous: WNDPROC,
		restart: u32,
		hooked: Cell<bool>,
		enabled: Cell<bool>,
		present: Cell<bool>,
		events: Events,
		wake: Box<dyn Fn()>,
	}

	impl Tray {
		pub fn new(
			window: Arc<winit::window::Window>,
			wake: impl Fn() + 'static,
		) -> Result<Self, &'static str> {
			let RawWindowHandle::Win32(handle) =
				window.window_handle().map_err(|_| UNAVAILABLE)?.as_raw()
			else {
				return Err(UNAVAILABLE);
			};
			let hwnd = HWND(handle.hwnd.get() as *mut _);
			// SAFETY: the retained winit window owns hwnd. Hooks must run on its UI thread.
			let previous = unsafe {
				if GetWindowThreadProcessId(hwnd, None) != GetCurrentThreadId()
					|| !GetPropW(hwnd, w!("Nivra.TrayState")).is_invalid()
				{
					return Err(UNAVAILABLE);
				}
				let previous = GetWindowLongPtrW(hwnd, GWLP_WNDPROC);
				if previous == 0 {
					return Err(UNAVAILABLE);
				}
				std::mem::transmute::<isize, WNDPROC>(previous)
			};
			// SAFETY: these fixed names register messages, without taking ownership of pointers.
			let (notification, restart) = unsafe {
				(
					RegisterWindowMessageW(w!("Nivra.TrayCallback")),
					RegisterWindowMessageW(w!("TaskbarCreated")),
				)
			};
			if notification == 0 || restart == 0 {
				return Err(UNAVAILABLE);
			}
			// SAFETY: request a borrowed window icon; the fallback is a shared system icon.
			let icon = unsafe {
				let handle = HICON(
					SendMessageW(hwnd, WM_GETICON, Some(WPARAM(ICON_SMALL2 as usize)), None).0
						as *mut _,
				);
				if handle.is_invalid() {
					LoadIconW(None, IDI_APPLICATION).map_err(|_| UNAVAILABLE)?
				} else {
					handle
				}
			};
			// SAFETY: creates a menu owned by State, released on every success/error path.
			let menu = unsafe { CreatePopupMenu() }.map_err(|_| UNAVAILABLE)?;
			let mut data = NOTIFYICONDATAW {
				cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
				hWnd: hwnd,
				uID: ID as u32,
				uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
				uCallbackMessage: notification,
				hIcon: icon,
				Anonymous: NOTIFYICONDATAW_0 {
					uVersion: NOTIFYICON_VERSION_4,
				},
				..Default::default()
			};
			for (slot, unit) in data.szTip.iter_mut().zip("Nivra".encode_utf16()) {
				*slot = unit;
			}
			let tray = Self {
				state: Rc::new(State {
					icon: Cell::new(data),
					owned: Cell::new(None),
					menu,
					previous,
					restart,
					hooked: Cell::new(false),
					enabled: Cell::new(true),
					present: Cell::new(false),
					events: Events::default(),
					wake: Box::new(wake),
				}),
				_window: window,
			};
			// SAFETY: menu and hwnd are live UI-thread handles; Rc keeps callback state stable.
			unsafe {
				AppendMenuW(menu, MF_STRING, SHOW, w!("Show Nivra")).map_err(|_| UNAVAILABLE)?;
				AppendMenuW(menu, MF_STRING, QUIT, w!("Quit")).map_err(|_| UNAVAILABLE)?;
				SetMenuDefaultItem(menu, SHOW as u32, 0).map_err(|_| UNAVAILABLE)?;
				let reference = Rc::into_raw(tray.state.clone());
				if SetPropW(
					hwnd,
					w!("Nivra.TrayState"),
					Some(HANDLE(reference.cast_mut().cast())),
				)
				.is_err()
				{
					drop(Rc::from_raw(reference));
					return Err(UNAVAILABLE);
				}
				if SetWindowLongPtrW(hwnd, GWLP_WNDPROC, callback as *const () as isize) == 0 {
					let _ = RemovePropW(hwnd, w!("Nivra.TrayState"));
					drop(Rc::from_raw(reference));
					return Err(UNAVAILABLE);
				}
			}
			tray.state.hooked.set(true);
			if !tray.state.add_icon() {
				return Err(UNAVAILABLE);
			}
			Ok(tray)
		}

		pub fn take_event(&self) -> Option<Event> {
			self.state.events.take()
		}

		/// Replaces the tray picture with square straight-alpha RGBA pixels and sets the hover
		/// text. False leaves the previous icon in place.
		pub fn set_icon(&self, rgba: &[u8], size: u32, tooltip: &str) -> bool {
			let side = size as usize;
			if size == 0 || size > 256 || rgba.len() != side * side * 4 {
				return false;
			}
			let stride = side.div_ceil(16) * 2;
			let mut mask = vec![0u8; stride * side];
			let mut pixels = vec![0u8; side * side * 4];
			for (index, rgba) in rgba.as_chunks::<4>().0.iter().enumerate() {
				let alpha = u32::from(rgba[3]);
				if alpha == 0 {
					mask[(index / side) * stride + (index % side) / 8] |= 0x80 >> (index % 8);
				}
				let premultiply = |value: u8| (u32::from(value) * alpha / 255) as u8;
				pixels[index * 4..index * 4 + 4].copy_from_slice(&[
					premultiply(rgba[2]),
					premultiply(rgba[1]),
					premultiply(rgba[0]),
					rgba[3],
				]);
			}
			// SAFETY: the buffers hold exactly a size x size WORD-aligned AND mask and BGRA pixels.
			let Ok(icon) = (unsafe {
				CreateIcon(
					None,
					size as i32,
					size as i32,
					1,
					32,
					mask.as_ptr(),
					pixels.as_ptr(),
				)
			}) else {
				return false;
			};
			let mut data = self.state.icon.get();
			data.hIcon = icon;
			data.szTip = [0; 128];
			for (slot, unit) in data.szTip.iter_mut().take(127).zip(tooltip.encode_utf16()) {
				*slot = unit;
			}
			self.state.icon.set(data);
			if self.state.present.get() {
				// SAFETY: modifies only this application's registered notification icon.
				let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
			}
			if let Some(previous) = self.state.owned.replace(Some(icon)) {
				// SAFETY: the shell copied the new icon; the replaced one is owned and unused.
				let _ = unsafe { DestroyIcon(previous) };
			}
			true
		}
	}

	impl State {
		fn add_icon(&self) -> bool {
			let icon = self.icon.get();
			// SAFETY: this initialized descriptor contains only live borrowed handles and fixed text.
			unsafe {
				if !Shell_NotifyIconW(NIM_ADD, &icon).as_bool() {
					return false;
				}
				if !Shell_NotifyIconW(NIM_SETVERSION, &icon).as_bool() {
					let _ = Shell_NotifyIconW(NIM_DELETE, &icon);
					return false;
				}
			}
			self.present.set(true);
			true
		}
		fn remove_icon(&self) {
			if self.present.replace(false) {
				// SAFETY: hwnd/uID identify only this application's owned notification icon.
				let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &self.icon.get()) };
			}
		}
		fn restore(&self) {
			let hwnd = self.icon.get().hWnd;
			// SAFETY: called while the retained window/subclass is live on its owning thread.
			unsafe {
				let _ = ShowWindow(hwnd, SW_RESTORE);
				let _ = SetForegroundWindow(hwnd);
			}
		}
		fn emit(&self, event: Event) {
			self.events.push(event);
			(self.wake)();
		}
		fn menu(&self) {
			let mut cursor = POINT::default();
			let hwnd = self.icon.get().hWnd;
			// SAFETY: live owned menu/window and stack cursor; TrackPopupMenu runs a nested UI loop.
			let command = unsafe {
				if GetCursorPos(&mut cursor).is_err() {
					return;
				}
				let _ = SetForegroundWindow(hwnd);
				let command = TrackPopupMenu(
					self.menu,
					TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
					cursor.x,
					cursor.y,
					None,
					hwnd,
					None,
				)
				.0 as usize;
				let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
				command
			};
			self.activate(command);
		}
		fn activate(&self, command: usize) {
			if let Some(event) = match command {
				SHOW => Some(Event::Show),
				QUIT => Some(Event::Quit),
				_ => None,
			} {
				self.restore();
				self.emit(event);
			}
		}
	}

	impl Drop for Tray {
		fn drop(&mut self) {
			self.state.enabled.set(false);
			if self.state.hooked.get() {
				// SAFETY: Rc makes Tray !Send; only restore our hook if it is still atop the chain.
				unsafe {
					let hwnd = self.state.icon.get().hWnd;
					if GetWindowLongPtrW(hwnd, GWLP_WNDPROC) == callback as *const () as isize
						&& SetWindowLongPtrW(
							hwnd,
							GWLP_WNDPROC,
							self.state.previous.unwrap() as *const () as isize,
						) != 0
					{
						self.state.hooked.set(false);
						let _ = RemovePropW(hwnd, w!("Nivra.TrayState"));
						drop(Rc::from_raw(Rc::as_ptr(&self.state)));
					}
				}
				// A newer hook may still call ours: retain disabled state until WM_NCDESTROY.
			}
			self.state.remove_icon();
		}
	}
	impl Drop for State {
		fn drop(&mut self) {
			// SAFETY: this state owns the menu; callback Rc copies keep it alive during nested menus.
			let _ = unsafe { DestroyMenu(self.menu) };
			if let Some(icon) = self.owned.take() {
				// SAFETY: created by `set_icon` and owned here; the notification icon is gone.
				let _ = unsafe { DestroyIcon(icon) };
			}
		}
	}

	unsafe extern "system" fn callback(
		hwnd: HWND,
		message: u32,
		wparam: WPARAM,
		lparam: LPARAM,
	) -> LRESULT {
		// SAFETY: the UI-thread registration installed this property before replacing WNDPROC.
		let pointer = unsafe { GetPropW(hwnd, w!("Nivra.TrayState")).0.cast::<State>() };
		if pointer.is_null() {
			// SAFETY: no owned state is accessible; use the OS default rather than a stale pointer.
			return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
		}
		// SAFETY: Tray retains this Rc until the subclass is removed. Retain a temporary Rc so
		// nested menu dispatch may disable/drop Tray without invalidating the current callback.
		let state = unsafe {
			Rc::increment_strong_count(pointer);
			Rc::from_raw(pointer)
		};
		if state.enabled.get() && message == state.icon.get().uCallbackMessage {
			match lparam.0 as u32 & 0xffff {
				NIN_SELECT | NIN_KEYSELECT => {
					state.restore();
					state.emit(Event::Show);
				}
				WM_CONTEXTMENU => state.menu(),
				_ => {}
			}
			return LRESULT(0);
		}
		if state.enabled.get() && message == state.restart {
			state.present.set(false);
			if !state.add_icon() {
				state.restore();
				state.emit(Event::Unavailable);
			}
		}
		if message == WM_NCDESTROY {
			let hooked = state.hooked.replace(false);
			state.remove_icon();
			// SAFETY: remove only our property as the window is destroyed.
			let _ = unsafe { RemovePropW(hwnd, w!("Nivra.TrayState")) };
			if hooked {
				// SAFETY: the destroyed window cannot dispatch again; release its registration Rc.
				unsafe {
					drop(Rc::from_raw(pointer));
				}
			}
		}
		// SAFETY: every unhandled message follows the original winit subclass chain.
		unsafe { CallWindowProcW(state.previous, hwnd, message, wparam, lparam) }
	}

	#[cfg(test)]
	mod tests {
		use super::*;
		use winit::{event_loop::EventLoop, platform::windows::EventLoopBuilderExtWindows};

		#[test]
		#[ignore = "Requires an interactive Windows shell; creates only a synthetic test window/icon"]
		#[allow(deprecated)]
		fn native_minimize_restore_restart_quit_and_cleanup() {
			let mut builder = EventLoop::builder();
			builder.with_any_thread(true);
			let event_loop = builder.build().unwrap();
			let window = Arc::new(
				event_loop
					.create_window(
						winit::window::Window::default_attributes()
							.with_title("Nivra synthetic tray test")
							.with_inner_size(winit::dpi::LogicalSize::new(320., 200.))
							.with_visible(false),
					)
					.unwrap(),
			);
			let wakes = Rc::new(Cell::new(0));
			let wake = wakes.clone();
			let tray = Tray::new(window.clone(), move || wake.set(wake.get() + 1)).unwrap();
			let icon = tray.state.icon.get();
			let hwnd = icon.hWnd;
			assert!(Tray::new(window.clone(), || {}).is_err());
			// SAFETY: every API here targets only this test-owned synthetic window/menu/icon.
			unsafe {
				let _ = ShowWindow(hwnd, SW_MINIMIZE);
				assert!(IsWindowVisible(hwnd).as_bool());
				assert!(IsIconic(hwnd).as_bool());
				let _ = SendMessageW(
					hwnd,
					icon.uCallbackMessage,
					None,
					Some(LPARAM(((ID as u32) << 16 | NIN_KEYSELECT) as isize)),
				);
				assert!(IsWindowVisible(hwnd).as_bool());
				assert!(!IsIconic(hwnd).as_bool());
				assert_eq!(tray.take_event(), Some(Event::Show));
				let status = vec![255u8; 32 * 32 * 4];
				assert!(tray.set_icon(&status, 32, "Nivra — test status"));
				assert!(tray.set_icon(&status, 32, "Nivra — replaced"));
				assert!(!tray.set_icon(&status[..16], 32, "wrong size"));
				assert!(tray.state.owned.get().is_some());
				tray.state.remove_icon();
				let _ = SendMessageW(hwnd, tray.state.restart, None, None);
				assert!(tray.state.present.get());
				assert_eq!(GetMenuItemID(tray.state.menu, 1), QUIT as u32);
				tray.state.activate(QUIT);
				assert_eq!(tray.take_event(), Some(Event::Quit));
				assert!(IsWindow(Some(hwnd)).as_bool()); // Quit is an app event, never forced destruction.
				let _ = ShowWindow(hwnd, SW_MINIMIZE);
				assert!(IsWindowVisible(hwnd).as_bool());
				drop(tray);
				assert!(IsWindowVisible(hwnd).as_bool());
				assert!(GetPropW(hwnd, w!("Nivra.TrayState")).is_invalid());
				assert_ne!(
					GetWindowLongPtrW(hwnd, GWLP_WNDPROC),
					callback as *const () as isize
				);
				assert!(!Shell_NotifyIconW(NIM_MODIFY, &icon).as_bool());
				// Startup can minimize before the asynchronous preference enables the tray.
				let _ = ShowWindow(hwnd, SW_MINIMIZE);
				let late_tray = Tray::new(window.clone(), || {}).unwrap();
				assert!(IsIconic(hwnd).as_bool());
				assert!(IsWindowVisible(hwnd).as_bool());
				drop(late_tray);
				assert!(IsWindowVisible(hwnd).as_bool());
				assert!(IsIconic(hwnd).as_bool());
			}
			assert_eq!(wakes.get(), 2);
			window.set_visible(false);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn plain_png() -> Vec<u8> {
		let image = image::RgbaImage::from_pixel(64, 64, image::Rgba([40, 120, 220, 255]));
		let mut png = Vec::new();
		image::DynamicImage::ImageRgba8(image)
			.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
			.unwrap();
		png
	}

	#[test]
	fn status_icon_marks_mentions_and_call_state_in_the_corners() {
		let png = plain_png();
		let at = |pixels: &[u8], x: usize, y: usize| {
			let index = (y * 32 + x) * 4;
			[
				pixels[index],
				pixels[index + 1],
				pixels[index + 2],
				pixels[index + 3],
			]
		};
		let idle = status_icon(&png, 32, false, Voice::Idle).unwrap();
		assert_eq!(idle.len(), 32 * 32 * 4);
		assert_eq!(at(&idle, 26, 6), [40, 120, 220, 255]);
		assert_eq!(at(&idle, 24, 24), [40, 120, 220, 255]);

		let pinged = status_icon(&png, 32, true, Voice::Connected).unwrap();
		assert_eq!(
			at(&pinged, 26, 6),
			[0xf2, 0x3f, 0x43, 255],
			"red mention dot"
		);
		assert_eq!(
			at(&pinged, 21, 21),
			[0x23, 0xa5, 0x5a, 255],
			"green call dot"
		);
		assert_eq!(
			at(&pinged, 16, 16),
			[40, 120, 220, 255],
			"the icon stays visible"
		);
		assert!(
			at(&pinged, 16, 25)[3] < 255,
			"the dot is cut out of the icon"
		);

		let muted = status_icon(&png, 32, false, Voice::Muted).unwrap();
		let deafened = status_icon(&png, 32, false, Voice::Deafened).unwrap();
		assert_eq!(
			at(&muted, 21, 21),
			[0xf2, 0x3f, 0x43, 255],
			"red when muted"
		);
		assert_eq!(at(&muted, 24, 24), [255, 255, 255, 255], "muted slash");
		assert_eq!(at(&deafened, 24, 24), [255, 255, 255, 255], "deafened bar");
		assert_ne!(muted, deafened, "muted and deafened look different");
		assert!(status_icon(b"not a png", 32, false, Voice::Idle).is_none());
	}

	#[test]
	fn clicks_coalesce_without_losing_quit_or_failure() {
		let events = Events::default();
		events.push(Event::Quit);
		for _ in 0..1000 {
			events.push(Event::Show);
		}
		events.push(Event::Unavailable);
		assert_eq!(events.take(), Some(Event::Quit));
		assert_eq!(events.take(), Some(Event::Unavailable));
		assert_eq!(events.take(), Some(Event::Show));
		assert_eq!(events.take(), None);
	}
}
