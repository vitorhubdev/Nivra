//! Provider-bounded web media player used from chat embeds.
//! Default play is incognito. "Open with login" uses a dedicated profile that never
//! shares the Discord login webview, accepts downloads, or grants device permissions.
use std::{path::Path, sync::Arc};

pub const HEADER_HEIGHT: f32 = 56.0;

pub fn normalize(value: &str) -> Option<String> {
	model::web_media::normalize(value)
}

#[cfg(not(target_os = "linux"))]
pub struct WebMediaView {
	view: wry::WebView,
	_context: Option<wry::WebContext>,
}
#[cfg(not(target_os = "linux"))]
impl WebMediaView {
	pub fn open(
		parent: Arc<winit::window::Window>,
		value: &str,
		persist: Option<&Path>,
		_wake: impl Fn() + Send + Sync + 'static,
	) -> Result<Self, &'static str> {
		let url = normalize(value).ok_or("Unsupported web video provider or URL")?;
		#[cfg(windows)]
		if let Some(dir) = persist {
			std::fs::create_dir_all(dir).map_err(|_| "Could not create the web media profile")?;
		}
		let persist_on = persist.is_some();
		#[cfg(windows)]
		let mut context = persist.map(|dir| wry::WebContext::new(Some(dir.to_path_buf())));
		#[cfg(not(windows))]
		let context = None::<wry::WebContext>;
		#[allow(unused_mut)]
		let mut builder = {
			#[cfg(windows)]
			{
				match context.as_mut() {
					Some(context) => wry::WebViewBuilder::new_with_web_context(context),
					None => wry::WebViewBuilder::new(),
				}
			}
			#[cfg(not(windows))]
			{
				wry::WebViewBuilder::new()
			}
		};
		builder = builder
			.with_url(&url)
			.with_incognito(!persist_on)
			.with_visible(true)
			.with_focused(true)
			.with_autoplay(true)
			.with_devtools(false)
			.with_permission_handler(|kind| {
				if matches!(kind, wry::PermissionKind::Autoplay) {
					wry::PermissionResponse::Allow
				} else {
					wry::PermissionResponse::Deny
				}
			})
			.with_navigation_handler(|url| model::web_media::navigation_allowed(&url))
			.with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
			.with_download_started_handler(|_, _| false);
		#[cfg(windows)]
		{
			use wry::WebViewBuilderExtWindows;
			builder = builder.with_browser_extensions_enabled(false);
		}
		#[cfg(target_os = "macos")]
		if persist_on {
			use wry::WebViewBuilderExtDarwin;
			// WKWebView has no data_directory; this is the 0.57 replacement API.
			// The identifier is a fixed 16-byte data-store tag.
			builder = builder.with_data_store_identifier(*b"nivra-web-media1");
		}
		let view = builder
			.with_bounds(bounds(&parent))
			.build_as_child(parent.as_ref())
			.map_err(|_| "Could not create the isolated web media player")?;
		view.set_visible(true)
			.map_err(|_| "Could not show the web media player")?;
		let _ = view.focus();
		Ok(Self {
			view,
			_context: context,
		})
	}
	pub fn embedded(&self) -> bool {
		true
	}
	pub fn closed(&self) -> bool {
		false
	}
	pub fn pump(&self) {}
	pub fn resize(&self, parent: &winit::window::Window) {
		let _ = self.view.set_bounds(bounds(parent));
	}
}
#[cfg(not(target_os = "linux"))]
fn bounds(parent: &winit::window::Window) -> wry::Rect {
	let size = parent.inner_size();
	let header = (HEADER_HEIGHT as f64 * parent.scale_factor()).round() as u32;
	wry::Rect {
		position: wry::dpi::PhysicalPosition::new(0, header as i32).into(),
		size: wry::dpi::PhysicalSize::new(size.width, size.height.saturating_sub(header)).into(),
	}
}

#[cfg(target_os = "linux")]
pub struct WebMediaView {
	view: webkit6::WebView,
	window: gtk4::Window,
	display: gtk4::gdk::Display,
	closed: std::rc::Rc<std::cell::Cell<bool>>,
}
#[cfg(target_os = "linux")]
impl WebMediaView {
	pub fn open(
		_parent: Arc<winit::window::Window>,
		value: &str,
		persist: Option<&Path>,
		wake: impl Fn() + Send + Sync + 'static,
	) -> Result<Self, &'static str> {
		use webkit6::{glib, prelude::*};
		gtk4::init().map_err(|_| "Linux web media window unavailable")?;
		crate::ensure_gtk_application_id();
		let url = normalize(value).ok_or("Unsupported web video provider or URL")?;
		let session = if let Some(dir) = persist {
			let data = dir.join("data");
			let cache = dir.join("cache");
			std::fs::create_dir_all(&data).map_err(|_| "Could not create the web media profile")?;
			std::fs::create_dir_all(&cache)
				.map_err(|_| "Could not create the web media profile")?;
			webkit6::NetworkSession::builder()
				.data_directory(
					data.to_str()
						.ok_or("Web media profile path is not valid UTF-8")?,
				)
				.cache_directory(
					cache
						.to_str()
						.ok_or("Web media profile path is not valid UTF-8")?,
				)
				.build()
		} else {
			let session = webkit6::NetworkSession::new_ephemeral();
			session.set_persistent_credential_storage_enabled(false);
			session
		};
		session.set_tls_errors_policy(webkit6::TLSErrorsPolicy::Fail);
		session.connect_download_started(|_, download| download.cancel());
		let settings = webkit6::Settings::new();
		settings.set_enable_developer_extras(false);
		settings.set_enable_write_console_messages_to_stdout(false);
		settings.set_javascript_can_open_windows_automatically(false);
		settings.set_javascript_can_access_clipboard(false);
		settings.set_enable_media_stream(false);
		settings.set_enable_webrtc(false);
		let view = webkit6::WebView::builder()
			.network_session(&session)
			.settings(&settings)
			.build();
		view.set_hexpand(true);
		view.set_vexpand(true);
		view.connect_decide_policy(|_, decision, kind| {
			let allowed = match kind {
				webkit6::PolicyDecisionType::NavigationAction => decision
					.downcast_ref::<webkit6::NavigationPolicyDecision>()
					.and_then(|decision| decision.navigation_action())
					.and_then(|action| action.request())
					.and_then(|request| request.uri())
					.is_some_and(|uri| model::web_media::navigation_allowed(&uri)),
				webkit6::PolicyDecisionType::Response => decision
					.downcast_ref::<webkit6::ResponsePolicyDecision>()
					.is_some_and(|response| {
						response.is_mime_type_supported()
							&& (!response.is_main_frame_main_resource()
								|| response
									.request()
									.and_then(|request| request.uri())
									.is_some_and(|uri| model::web_media::navigation_allowed(&uri)))
					}),
				_ => false,
			};
			if allowed {
				decision.use_();
			} else {
				decision.ignore();
			}
			true
		});
		view.connect_create(|_, _| None);
		view.connect_permission_request(|_, request| {
			request.deny();
			true
		});
		view.connect_run_file_chooser(|_, request| {
			request.cancel();
			true
		});
		view.connect_context_menu(|_, _, _| true);
		let window = gtk4::Window::builder()
			.title("Web media preview · Nivra")
			.default_width(960)
			.default_height(720)
			.child(&view)
			.build();
		let closed = std::rc::Rc::new(std::cell::Cell::new(false));
		let closed_request = closed.clone();
		let weak_view = view.downgrade();
		let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
		let close_wake = wake.clone();
		window.connect_close_request(move |_| {
			closed_request.set(true);
			if let Some(view) = weak_view.upgrade() {
				view.stop_loading();
				view.terminate_web_process();
			}
			close_wake();
			glib::Propagation::Proceed
		});
		let crashed = closed.clone();
		let crash_wake = wake.clone();
		view.connect_web_process_terminated(move |_, _| {
			crashed.set(true);
			crash_wake();
		});
		window.present();
		view.grab_focus();
		view.load_uri(&url);
		let display = gtk4::prelude::WidgetExt::display(&window);
		Ok(Self {
			view,
			window,
			display,
			closed,
		})
	}
	pub fn embedded(&self) -> bool {
		false
	}
	pub fn closed(&self) -> bool {
		self.closed.get()
	}
	pub fn resize(&self, _parent: &winit::window::Window) {}
	pub fn pump(&self) {
		use webkit6::glib;
		let context = glib::MainContext::default();
		let started = std::time::Instant::now();
		for _ in 0..16 {
			if started.elapsed() >= std::time::Duration::from_millis(2) || !context.pending() {
				break;
			}
			context.iteration(false);
		}
		use webkit6::prelude::*;
		self.display.flush();
	}
}
#[cfg(target_os = "linux")]
impl Drop for WebMediaView {
	fn drop(&mut self) {
		use webkit6::prelude::*;
		self.view.stop_loading();
		self.view.terminate_web_process();
		self.window.set_child(None::<&gtk4::Widget>);
		self.window.destroy();
		self.display.flush();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn platform_normalize_delegates_to_model() {
		for url in [
			"https://youtu.be/dQw4w9WgXcQ?si=tracking",
			"https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=ignored",
			"https://x.com/user/status/123?utm_source=ignored",
			"https://youtube.com.evil.test/watch?v=dQw4w9WgXcQ",
			"http://x.com/user/status/123",
		] {
			assert_eq!(normalize(url), model::web_media::normalize(url), "{url}");
		}
	}
}
