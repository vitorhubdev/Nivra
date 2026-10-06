use super::{Event, TrayLabels};
use ksni::{TrayMethods, menu::StandardItem};
use std::sync::{
	Arc, Mutex,
	atomic::{AtomicU8, Ordering},
};
use std::time::Duration;
use tokio::sync::oneshot;

struct Events {
	bits: AtomicU8,
	// Pending -> ready, or permanently unavailable until this registration is replaced.
	availability: AtomicU8,
	wake: Box<dyn Fn() + Send + Sync>,
	restore: Box<dyn Fn() + Send + Sync>,
}

impl Events {
	fn push(&self, event: Event) {
		if event == Event::Unavailable {
			self.availability.store(2, Ordering::Release);
		}
		// A hidden window may receive no frames, so the UI would never see this event.
		if event != Event::Minimize {
			(self.restore)();
		}
		if self.bits.fetch_or(event as u8, Ordering::Relaxed) & event as u8 == 0 {
			(self.wake)();
		}
	}
}

/// One cancellable registration on the application's runtime, never a UI-thread D-Bus call.
pub struct Tray {
	events: Arc<Events>,
	_stop: oneshot::Sender<()>,
	runtime: tokio::runtime::Handle,
	labels: Arc<Mutex<TrayLabels>>,
	handle: Arc<tokio::sync::Mutex<Option<ksni::Handle<Item>>>>,
}

impl Tray {
	/// `restore` runs on the tray worker for events that need a visible window, before `wake`.
	pub fn new(
		wake: impl Fn() + Send + Sync + 'static,
		restore: impl Fn() + Send + Sync + 'static,
		labels: TrayLabels,
	) -> Result<Self, &'static str> {
		let runtime = tokio::runtime::Handle::try_current()
			.map_err(|_| "The tray requires the application runtime.")?;
		let events = Arc::new(Events {
			bits: AtomicU8::new(0),
			availability: AtomicU8::new(0),
			wake: Box::new(wake),
			restore: Box::new(restore),
		});
		let shared_labels = Arc::new(Mutex::new(labels.clone()));
		let handle_slot: Arc<tokio::sync::Mutex<Option<ksni::Handle<Item>>>> =
			Arc::new(tokio::sync::Mutex::new(None));
		let (stop, mut stopped) = oneshot::channel();
		let worker_events = events.clone();
		let worker_labels = shared_labels.clone();
		let worker_handle = handle_slot.clone();
		runtime.spawn(async move {
			let Ok(icon) = image::load_from_memory_with_format(
				include_bytes!("../../../../packaging/linux/hicolor/32x32/apps/nivra.png"),
				image::ImageFormat::Png,
			) else {
				worker_events.push(Event::Unavailable);
				return;
			};
			let mut pixels = icon.into_rgba8().into_raw();
			for pixel in pixels.as_chunks_mut::<4>().0 {
				pixel.rotate_right(1); // StatusNotifier pixmaps use ARGB, not RGBA.
			}
			let item = Item {
				events: worker_events.clone(),
				pixels,
				labels: worker_labels,
			};
			let registration = item.disable_dbus_name(true).spawn();
			let result = tokio::select! {
				_ = &mut stopped => return,
				result = tokio::time::timeout(Duration::from_secs(3), registration) => result,
			};
			let Ok(Ok(handle)) = result else {
				worker_events.push(Event::Unavailable);
				return;
			};
			// ksni subscribes to watcher changes after registration. Recheck after
			// subscribing so a host lost during registration cannot leave a phantom tray.
			let host_present = tokio::select! {
				_ = &mut stopped => false,
				result = tokio::time::timeout(Duration::from_secs(3), async {
					let bus = zbus::Connection::session().await?;
					let dbus = zbus::fdo::DBusProxy::new(&bus).await?;
					dbus.name_has_owner("org.kde.StatusNotifierWatcher".try_into().unwrap()).await
				}) => matches!(result, Ok(Ok(true))),
			};
			if !host_present || handle.is_closed() {
				worker_events.push(Event::Unavailable);
				let _ = tokio::time::timeout(Duration::from_secs(2), handle.shutdown()).await;
				return;
			}
			*worker_handle.lock().await = Some(handle.clone());
			if worker_events
				.availability
				.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
				.is_ok()
			{
				(worker_events.wake)();
			}
			let _ = stopped.await;
			let _ = tokio::time::timeout(Duration::from_secs(2), handle.shutdown()).await;
		});
		Ok(Self {
			events,
			_stop: stop,
			runtime,
			labels: shared_labels,
			handle: handle_slot,
		})
	}
	pub fn is_available(&self) -> bool {
		self.events.availability.load(Ordering::Acquire) == 1
	}

	/// Replaces the menu rows with the labels for the current interface language.
	pub fn set_labels(&self, labels: &TrayLabels) {
		if let Ok(mut current) = self.labels.lock() {
			*current = labels.clone();
		}
		let handle = self.handle.clone();
		let labels = labels.clone();
		self.runtime.spawn(async move {
			if let Some(handle) = handle.lock().await.clone() {
				let labels = labels.clone();
				let _ = handle
					.update(move |item| {
						if let Ok(mut current) = item.labels.lock() {
							*current = labels;
						}
					})
					.await;
			}
		});
	}

	pub fn take_event(&self) -> Option<Event> {
		[
			Event::Quit,
			Event::Minimize,
			Event::Unavailable,
			Event::Show,
		]
		.into_iter()
		.find(|event| {
			self.events
				.bits
				.fetch_and(!(*event as u8), Ordering::Relaxed)
				& *event as u8
				!= 0
		})
	}
}

struct Item {
	events: Arc<Events>,
	pixels: Vec<u8>,
	labels: Arc<Mutex<TrayLabels>>,
}

impl ksni::Tray for Item {
	fn id(&self) -> String {
		"nivra".into()
	}
	fn title(&self) -> String {
		"Nivra".into()
	}
	fn icon_pixmap(&self) -> Vec<ksni::Icon> {
		vec![ksni::Icon {
			width: 32,
			height: 32,
			data: self.pixels.clone(),
		}]
	}
	fn activate(&mut self, _x: i32, _y: i32) {
		self.events.push(Event::Show);
	}
	fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
		let labels = self
			.labels
			.lock()
			.map(|labels| labels.clone())
			.unwrap_or_else(|_| TrayLabels::english());
		vec![
			StandardItem {
				label: labels.show,
				activate: Box::new(|item: &mut Self| item.events.push(Event::Show)),
				..Default::default()
			}
			.into(),
			StandardItem {
				label: labels.minimize,
				activate: Box::new(|item: &mut Self| item.events.push(Event::Minimize)),
				..Default::default()
			}
			.into(),
			StandardItem {
				label: labels.quit,
				activate: Box::new(|item: &mut Self| item.events.push(Event::Quit)),
				..Default::default()
			}
			.into(),
		]
	}
	fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
		self.events.push(Event::Unavailable);
		false
	}
}
