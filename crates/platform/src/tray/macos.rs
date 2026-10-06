use super::{Event, Events, TrayLabels};
use objc2::{
	AllocAnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained, sel,
};
use objc2_app_kit::{NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem};
use objc2_foundation::{
	MainThreadMarker, NSData, NSObject, NSObjectProtocol, NSSize, NSString, ns_string,
};
use std::sync::Arc;

/// Nivra's own mark, rasterized from `assets/brand/nivra-mark.svg` as a menu bar template:
/// only its alpha matters, so macOS tints it for light, dark and highlighted menu bars.
const MARK: &[u8] = include_bytes!("../../../../assets/brand/nivra-tray.png");
/// Menu bar images are measured in points; the render is larger so Retina scales stay sharp.
const MARK_POINTS: f64 = 18.0;

struct State {
	window: Arc<winit::window::Window>,
	events: Events,
	wake: Box<dyn Fn()>,
}

define_class!(
	// SAFETY: NSObject has no subclassing requirements. All state and callbacks stay on
	// the main thread, and the target lives until every menu item is disconnected.
	#[unsafe(super = NSObject)]
	#[name = "NivraTrayTarget"]
	#[thread_kind = MainThreadOnly]
	#[ivars = State]
	struct Target;

	// SAFETY: NSObjectProtocol has no additional requirements.
	unsafe impl NSObjectProtocol for Target {}

	impl Target {
		#[unsafe(method(showNivra:))]
		fn show(&self, _sender: &NSMenuItem) {
			self.emit(Event::Show);
		}

		#[unsafe(method(quitNivra:))]
		fn quit(&self, _sender: &NSMenuItem) {
			self.emit(Event::Quit);
		}
	}
);

impl Target {
	fn emit(&self, event: Event) {
		let state = self.ivars();
		// Restore before Quit as well, so the app can display its unsaved-draft prompt.
		state.window.set_visible(true);
		state.window.set_minimized(false);
		state.window.focus_window();
		state.events.push(event);
		(state.wake)();
	}
}

/// Main-thread ownership prevents cross-thread callbacks and destruction.
pub struct Tray {
	item: Retained<NSStatusItem>,
	menu: Retained<NSMenu>,
	target: Retained<Target>,
	show_item: Retained<NSMenuItem>,
	quit_item: Retained<NSMenuItem>,
}

impl Tray {
	pub fn new(
		window: Arc<winit::window::Window>,
		wake: impl Fn() + 'static,
		labels: TrayLabels,
	) -> Result<Self, &'static str> {
		let mtm = MainThreadMarker::new().ok_or("The menu bar icon requires the main thread.")?;
		let target = Target::alloc(mtm).set_ivars(State {
			window,
			events: Events::default(),
			wake: Box::new(wake),
		});
		// SAFETY: NSObject init initializes this allocated NSObject subclass.
		let target = unsafe { msg_send![super(target), init] };
		let menu = NSMenu::new(mtm);
		menu.setAutoenablesItems(false);
		let show = NSString::from_str(&labels.show);
		let quit = NSString::from_str(&labels.quit);
		let empty = NSString::from_str("");
		// SAFETY: both selectors are implemented above with the menu action signature.
		// The target lives until every menu item is disconnected on drop.
		let (show_item, quit_item) = unsafe {
			let show_item = NSMenuItem::initWithTitle_action_keyEquivalent(
				NSMenuItem::alloc(mtm),
				&show,
				Some(sel!(showNivra:)),
				&empty,
			);
			show_item.setTarget(Some(&target));
			let quit_item = NSMenuItem::initWithTitle_action_keyEquivalent(
				NSMenuItem::alloc(mtm),
				&quit,
				Some(sel!(quitNivra:)),
				&empty,
			);
			quit_item.setTarget(Some(&target));
			(show_item, quit_item)
		};
		menu.addItem(&show_item);
		menu.addItem(&quit_item);
		let tray = Self {
			// NSVariableStatusItemLength: fit the symbol or fallback title.
			item: NSStatusBar::systemStatusBar().statusItemWithLength(-1.0),
			menu,
			target,
			show_item,
			quit_item,
		};
		let button = tray
			.item
			.button(mtm)
			.ok_or("The macOS menu bar is unavailable.")?;
		let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(MARK));
		if let Some(image) = image {
			image.setTemplate(true);
			image.setSize(NSSize::new(MARK_POINTS, MARK_POINTS));
			button.setImage(Some(&image));
		} else {
			button.setTitle(ns_string!("Nivra"));
		}
		button.setToolTip(Some(ns_string!("Nivra")));
		tray.item.setMenu(Some(&tray.menu));
		Ok(tray)
	}

	pub fn take_event(&self) -> Option<Event> {
		self.target.ivars().events.take()
	}

	/// Replaces both menu rows with the labels for the current interface language.
	pub fn set_labels(&self, labels: &TrayLabels) {
		self.show_item.setTitle(&NSString::from_str(&labels.show));
		self.quit_item.setTitle(&NSString::from_str(&labels.quit));
	}
}

impl Drop for Tray {
	fn drop(&mut self) {
		self.item.setMenu(None);
		for item in self.menu.itemArray() {
			// SAFETY: disconnect even a menu item retained by AppKit's active menu tracking.
			unsafe { item.setTarget(None) };
		}
		self.menu.removeAllItems();
		NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
	}
}
