//! Sequential batch download: one folder picked once, one file at a time.
//!
//! Transfers, client limits and size caps reuse `downloads` (`download`,
//! `original_url`); this module only owns the queue, the shared snapshot the
//! timeline manager renders, and cancel/retry/open-folder actions.
use model::Attachment;
use std::{
	path::{Path, PathBuf},
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	},
	time::Duration,
};
use tokio::sync::Notify;

/// Selection cap enforced by the floating bar before requests reach here.
const MAX_FILES: usize = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
	Queued,
	Active,
	Done,
	Failed,
	Cancelled,
}

#[derive(Clone)]
pub struct FileState {
	pub name: String,
	pub received: u64,
	pub total: u64,
	pub status: FileStatus,
	pub error: Option<&'static str>,
}

#[derive(Default)]
struct Shared {
	folder: Option<PathBuf>,
	files: Vec<FileState>,
	/// Source attachments by file index; retry re-resolves from here.
	items: Vec<Attachment>,
	/// Worker alive (folder pick or transfers running).
	active: bool,
	/// Batch reached a terminal state; the desktop announces it once.
	finished: bool,
	announced: bool,
	/// Folder pick cancelled before any transfer.
	closed: bool,
}
impl Shared {
	fn fail_pending(&mut self, error: &'static str) {
		for file in &mut self.files {
			if matches!(file.status, FileStatus::Queued | FileStatus::Active) {
				file.status = FileStatus::Failed;
				file.error = Some(error);
			}
		}
		self.active = false;
		self.finished = true;
		self.announced = false;
	}

	fn prepare_retry(&mut self) -> Vec<usize> {
		let indices: Vec<usize> = self
			.files
			.iter_mut()
			.enumerate()
			.filter_map(|(index, file)| {
				if file.status != FileStatus::Failed {
					return None;
				}
				file.status = FileStatus::Queued;
				file.received = 0;
				file.error = None;
				Some(index)
			})
			.collect();
		if !indices.is_empty() {
			self.active = true;
			self.finished = false;
			self.announced = false;
		}
		indices
	}
}

#[derive(Default)]
pub struct BatchDownloads {
	shared: Arc<std::sync::Mutex<Shared>>,
	cancelled: Arc<AtomicBool>,
	wake_cancel: Arc<Notify>,
	worker: Option<std::thread::JoinHandle<()>>,
}

fn unique_dest(folder: &Path, filename: &str, reserved: &mut Vec<String>) -> (PathBuf, String) {
	let safe = platform::save::safe_filename(filename);
	let named = ui::batch_select::first_free_filename(&safe, |name| {
		reserved.iter().any(|taken| taken == name) || folder.join(name).exists()
	})
	.unwrap_or_else(|| format!("{safe}-{}", reserved.len() + 1));
	reserved.push(named.clone());
	(folder.join(&named), named)
}

pub fn open_folder(folder: &Path) {
	#[cfg(target_os = "windows")]
	let _ = platform::processes::hidden_command("explorer")
		.arg(folder)
		.spawn();
	#[cfg(target_os = "macos")]
	let _ = std::process::Command::new("open").arg(folder).spawn();
	#[cfg(target_os = "linux")]
	let _ = std::process::Command::new("xdg-open").arg(folder).spawn();
	#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
	let _ = folder;
}

fn resolve_item_url(attachment: &Attachment) -> Option<(url::Url, String, u64)> {
	if let Some(url) = super::downloads::original_url(attachment) {
		Some((url, attachment.filename.clone(), attachment.size))
	} else {
		let media = &attachment.media;
		let url = [media.proxy_url.as_deref(), media.url.as_deref()]
			.into_iter()
			.flatten()
			.find_map(|source| {
				crate::avatars::embed_url(source, super::downloads::DOWNLOAD_EDGE)
					.and_then(|u| url::Url::parse(&u).ok())
			})?;
		Some((url, attachment.filename.clone(), attachment.size))
	}
}

/// Both the initial transfer and retries use this same bounded transfer path.
async fn run_indices(
	shared: &Arc<std::sync::Mutex<Shared>>,
	cancelled: &Arc<AtomicBool>,
	wake_cancel: &Arc<Notify>,
	context: &eframe::egui::Context,
	folder: &Path,
	indices: Vec<usize>,
) {
	let client = reqwest::Client::builder()
		.no_proxy()
		.pool_max_idle_per_host(0)
		.proxy(protect::proxy(model::protect::Scope::Videos))
		.redirect(reqwest::redirect::Policy::none())
		.connect_timeout(Duration::from_secs(15))
		.read_timeout(Duration::from_secs(30))
		.timeout(Duration::from_secs(300))
		.build()
		.ok();
	let Some(client) = client else {
		shared
			.lock()
			.expect("batch download state")
			.fail_pending("Download unavailable");
		context.request_repaint();
		return;
	};
	let mut reserved = Vec::new();
	for index in indices {
		if cancelled.load(Ordering::Acquire) {
			let mut shared = shared.lock().expect("batch download state");
			for file in &mut shared.files {
				if matches!(file.status, FileStatus::Queued | FileStatus::Active) {
					file.status = FileStatus::Cancelled;
				}
			}
			break;
		}
		let attachment: Option<Attachment> = {
			let mut shared = shared.lock().expect("batch download state");
			shared.files[index].status = FileStatus::Active;
			shared.items.get(index).cloned()
		};
		context.request_repaint();
		let result = match attachment.as_ref().and_then(resolve_item_url) {
			None => Err("Attachment download unavailable"),
			Some((url, filename, size)) => {
				let (destination, named) = unique_dest(folder, &filename, &mut reserved);
				{
					let mut shared = shared.lock().expect("batch download state");
					shared.files[index].name = named;
				}
				super::downloads::download(
					&client,
					url,
					&destination,
					size,
					false,
					cancelled,
					wake_cancel,
					&|status| {
						if let super::downloads::Status::Downloading { received, total } = status {
							let mut shared = shared.lock().expect("batch download state");
							shared.files[index].received = received;
							shared.files[index].total = total;
							context.request_repaint();
						}
					},
				)
				.await
			}
		};
		{
			let mut shared = shared.lock().expect("batch download state");
			match result {
				Ok(()) => {
					shared.files[index].status = FileStatus::Done;
					shared.files[index].received = shared.files[index].total;
					shared.files[index].error = None;
				}
				Err("Cancelled") => {
					shared.files[index].status = FileStatus::Cancelled;
					for file in &mut shared.files {
						if file.status == FileStatus::Queued {
							file.status = FileStatus::Cancelled;
						}
					}
					break;
				}
				Err(error) => {
					shared.files[index].status = FileStatus::Failed;
					shared.files[index].error = Some(error);
				}
			}
		}
		context.request_repaint();
	}
	{
		let mut shared = shared.lock().expect("batch download state");
		shared.active = false;
		shared.finished = true;
	}
	context.request_repaint();
}

impl BatchDownloads {
	fn locked(&self) -> std::sync::MutexGuard<'_, Shared> {
		self.shared.lock().expect("batch download state")
	}

	fn reset_cancellation(&mut self) {
		// Notify retains a permit when Cancel is clicked during the folder picker
		// or between transfers. Resetting only the boolean cancels the next batch.
		// A fresh pair also isolates a late cancellation from the previous worker.
		self.cancelled = Arc::new(AtomicBool::new(false));
		self.wake_cancel = Arc::new(Notify::new());
	}

	fn accept_worker(&mut self, worker: std::io::Result<std::thread::JoinHandle<()>>) {
		match worker {
			Ok(worker) => {
				if let Some(previous) = self.worker.replace(worker)
					&& previous.is_finished()
				{
					let _ = previous.join();
				}
				// A previous worker may have published its terminal state but still
				// be leaving request_repaint. Never wait for it on the render thread.
			}
			Err(_) => self.locked().fail_pending("Download unavailable"),
		}
	}

	pub fn is_active(&self) -> bool {
		self.locked().active
	}
	/// Snapshot for the timeline manager: folder, files, worker alive, terminal.
	pub fn snapshot(&self) -> (Option<PathBuf>, Vec<FileState>, bool, bool) {
		let shared = self.locked();
		(
			shared.folder.clone(),
			shared.files.clone(),
			shared.active,
			shared.finished,
		)
	}
	/// Folder pick cancelled before transfers; the manager should close.
	pub fn take_closed(&self) -> bool {
		std::mem::replace(&mut self.locked().closed, false)
	}
	/// Terminal batch not yet announced (toast + reopen manager once).
	pub fn take_finished(&self) -> bool {
		let mut shared = self.locked();
		if shared.finished && !shared.announced {
			shared.announced = true;
			true
		} else {
			false
		}
	}
	pub fn folder(&self) -> Option<PathBuf> {
		self.locked().folder.clone()
	}
	/// Pick one folder and download sequentially off-thread, with at most one active batch.
	pub fn start(
		&mut self,
		attachments: Vec<Attachment>,
		runtime: &tokio::runtime::Handle,
		context: &eframe::egui::Context,
		parent: Arc<winit::window::Window>,
	) {
		if attachments.is_empty() || self.is_active() {
			return;
		}
		let attachments: Vec<Attachment> = attachments.into_iter().take(MAX_FILES).collect();
		let count = attachments.len();
		{
			let mut shared = self.locked();
			shared.folder = None;
			shared.files = attachments
				.iter()
				.map(|attachment| FileState {
					name: attachment.filename.clone(),
					received: 0,
					total: attachment.size,
					status: FileStatus::Queued,
					error: None,
				})
				.collect();
			shared.items = attachments;
			shared.active = true;
			shared.finished = false;
			shared.announced = false;
			shared.closed = false;
		}
		self.reset_cancellation();
		let shared = self.shared.clone();
		let cancelled = self.cancelled.clone();
		let wake_cancel = self.wake_cancel.clone();
		let runtime = runtime.clone();
		let worker_context = context.clone();
		let worker = std::thread::Builder::new()
			.name("nivra-batch-download".into())
			.spawn(move || {
				runtime.block_on(async {
					let folder = platform::save::select_folder(parent).await;
					let Some(folder) = folder else {
						let mut shared = shared.lock().expect("batch download state");
						shared.active = false;
						shared.closed = true;
						worker_context.request_repaint();
						return;
					};
					shared.lock().expect("batch download state").folder = Some(folder.clone());
					worker_context.request_repaint();
					run_indices(
						&shared,
						&cancelled,
						&wake_cancel,
						&worker_context,
						&folder,
						(0..count).collect(),
					)
					.await;
				});
			});
		self.accept_worker(worker);
		context.request_repaint();
	}
	/// Retry failed files with the already-picked folder; picks nothing new.
	pub fn retry(&mut self, runtime: &tokio::runtime::Handle, context: &eframe::egui::Context) {
		let Some(folder) = self.folder() else {
			return;
		};
		if self.is_active() {
			return;
		}
		let indices = self.locked().prepare_retry();
		if indices.is_empty() {
			return;
		}
		self.reset_cancellation();
		let shared = self.shared.clone();
		let cancelled = self.cancelled.clone();
		let wake_cancel = self.wake_cancel.clone();
		let runtime = runtime.clone();
		let worker_context = context.clone();
		let worker = std::thread::Builder::new()
			.name("nivra-batch-retry".into())
			.spawn(move || {
				runtime.block_on(async {
					run_indices(
						&shared,
						&cancelled,
						&wake_cancel,
						&worker_context,
						&folder,
						indices,
					)
					.await;
				});
			});
		self.accept_worker(worker);
		context.request_repaint();
	}
	pub fn cancel(&mut self) {
		self.cancelled.store(true, Ordering::Release);
		self.wake_cancel.notify_one();
	}
	/// Forget a terminal batch so a new Download starts fresh.
	pub fn dismiss(&mut self) {
		if self.is_active() {
			return;
		}
		*self.locked() = Shared::default();
	}
}

impl Drop for BatchDownloads {
	fn drop(&mut self) {
		self.cancel();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::{future::Future, task::Context};

	fn notification_ready(notify: &Notify) -> bool {
		let mut future = std::pin::pin!(notify.notified());
		future
			.as_mut()
			.poll(&mut Context::from_waker(std::task::Waker::noop()))
			.is_ready()
	}

	fn file(status: FileStatus) -> FileState {
		FileState {
			name: "test.bin".into(),
			received: 42,
			total: 100,
			status,
			error: None,
		}
	}

	#[test]
	fn batch_names_stay_unique_before_any_file_exists() {
		let folder = std::env::temp_dir().join(format!("nivra-batch-names-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&folder);
		std::fs::create_dir_all(&folder).unwrap();
		let mut reserved = Vec::new();
		let (first, first_name) = unique_dest(&folder, "pasted-image.png", &mut reserved);
		let (second, second_name) = unique_dest(&folder, "pasted-image.png", &mut reserved);
		assert_ne!(first, second);
		assert_eq!(first_name, "pasted-image.png");
		assert_eq!(second_name, "pasted-image (1).png");
		let _ = std::fs::remove_dir_all(&folder);
	}

	#[test]
	fn cancel_before_a_waiter_does_not_cancel_the_next_batch() {
		let mut batch = BatchDownloads::default();
		let old_flag = batch.cancelled.clone();
		let old_wake = batch.wake_cancel.clone();
		batch.cancel();
		batch.reset_cancellation();
		assert!(old_flag.load(Ordering::Acquire));
		assert!(notification_ready(&old_wake));
		assert!(!batch.cancelled.load(Ordering::Acquire));
		assert!(!notification_ready(&batch.wake_cancel));
	}

	#[test]
	fn late_previous_cancellation_is_isolated_from_the_new_transfer() {
		let mut batch = BatchDownloads::default();
		let old_flag = batch.cancelled.clone();
		let old_wake = batch.wake_cancel.clone();
		batch.reset_cancellation();
		old_flag.store(true, Ordering::Release);
		old_wake.notify_one();
		assert!(!batch.cancelled.load(Ordering::Acquire));
		assert!(!notification_ready(&batch.wake_cancel));
		batch.cancel();
		assert!(notification_ready(&batch.wake_cancel));
	}

	#[test]
	fn spawn_failure_finishes_pending_files_and_announces_once() {
		let mut batch = BatchDownloads::default();
		{
			let mut shared = batch.locked();
			shared.active = true;
			shared.files = vec![
				file(FileStatus::Queued),
				file(FileStatus::Active),
				file(FileStatus::Done),
				file(FileStatus::Cancelled),
			];
		}
		batch.accept_worker(Err(std::io::Error::other("synthetic spawn failure")));
		let (_, files, active, finished) = batch.snapshot();
		assert!(!active);
		assert!(finished);
		assert_eq!(files[0].status, FileStatus::Failed);
		assert_eq!(files[1].status, FileStatus::Failed);
		assert_eq!(files[0].error, Some("Download unavailable"));
		assert_eq!(files[2].status, FileStatus::Done);
		assert_eq!(files[3].status, FileStatus::Cancelled);
		assert!(batch.take_finished());
		assert!(!batch.take_finished());
	}

	#[test]
	fn retry_clears_old_errors_and_progress_without_repeating_completed_files() {
		let mut shared = Shared {
			files: vec![file(FileStatus::Failed), file(FileStatus::Done)],
			finished: true,
			announced: true,
			..Shared::default()
		};
		shared.files[0].error = Some("Attachment download failed");
		assert_eq!(shared.prepare_retry(), vec![0]);
		assert_eq!(shared.files[0].status, FileStatus::Queued);
		assert_eq!(shared.files[0].received, 0);
		assert_eq!(shared.files[0].error, None);
		assert_eq!(shared.files[1].status, FileStatus::Done);
		assert_eq!(shared.files[1].received, 42);
		assert!(shared.active);
		assert!(!shared.finished);
		assert!(!shared.announced);
	}

	#[test]
	fn retry_without_failed_files_does_not_start_a_worker() {
		let mut shared = Shared {
			files: vec![file(FileStatus::Done), file(FileStatus::Cancelled)],
			finished: true,
			..Shared::default()
		};
		assert!(shared.prepare_retry().is_empty());
		assert!(!shared.active);
		assert!(shared.finished);
	}

	#[test]
	fn dropping_the_owner_cancels_the_transfer_without_joining_it() {
		let batch = BatchDownloads::default();
		let cancelled = batch.cancelled.clone();
		let wake = batch.wake_cancel.clone();
		drop(batch);
		assert!(cancelled.load(Ordering::Acquire));
		assert!(notification_ready(&wake));
	}
}
