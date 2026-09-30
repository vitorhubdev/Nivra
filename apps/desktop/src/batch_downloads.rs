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

#[derive(Clone, Copy, PartialEq, Eq)]
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
#[derive(Default)]
pub struct BatchDownloads {
	shared: Arc<std::sync::Mutex<Shared>>,
	cancelled: Arc<AtomicBool>,
	wake_cancel: Arc<Notify>,
	worker: Option<std::thread::JoinHandle<()>>,
}

fn unique_dest(folder: &Path, filename: &str, reserved: &mut Vec<String>) -> (PathBuf, String) {
	let safe = platform::save::safe_filename(filename);
	let (stem, extension) = match safe.rsplit_once('.') {
		Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => {
			(stem.to_owned(), Some(extension.to_owned()))
		}
		_ => (safe.clone(), None),
	};
	for n in 0..=99 {
		let named = if n == 0 {
			safe.clone()
		} else {
			match &extension {
				Some(extension) => format!("{stem} ({n}).{extension}"),
				None => format!("{stem} ({n})"),
			}
		};
		if reserved.iter().any(|taken| taken == &named) {
			continue;
		}
		let candidate = folder.join(&named);
		if candidate.exists() {
			continue;
		}
		reserved.push(named.clone());
		return (candidate, named);
	}
	let fallback = format!("{safe}-{}", reserved.len() + 1);
	reserved.push(fallback.clone());
	(folder.join(&fallback), fallback)
}

pub fn open_folder(folder: &Path) {
	#[cfg(target_os = "windows")]
	let _ = std::process::Command::new("explorer").arg(folder).spawn();
	#[cfg(target_os = "macos")]
	let _ = std::process::Command::new("open").arg(folder).spawn();
	#[cfg(target_os = "linux")]
	let _ = std::process::Command::new("xdg-open").arg(folder).spawn();
	#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
	let _ = folder;
}

/// Download stored items at `indices` into `folder`, one at a time, updating the
/// shared snapshot. Reuses the `downloads` transfer, client limits and size caps.
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
		.redirect(reqwest::redirect::Policy::none())
		.connect_timeout(Duration::from_secs(15))
		.read_timeout(Duration::from_secs(30))
		.timeout(Duration::from_secs(300))
		.build()
		.map_err(|_| "Download unavailable")
		.ok();
	let Some(client) = client else {
		let mut shared = shared.lock().expect("batch download state");
		for index in &indices {
			if shared.files[*index].status == FileStatus::Queued {
				shared.files[*index].status = FileStatus::Failed;
				shared.files[*index].error = Some("Download unavailable");
			}
		}
		shared.active = false;
		shared.finished = true;
		context.request_repaint();
		return;
	};
	let mut reserved = Vec::new();
	for index in indices {
		if cancelled.load(Ordering::Acquire) {
			let mut shared = shared.lock().expect("batch download state");
			for file in shared.files.iter_mut() {
				if file.status == FileStatus::Queued || file.status == FileStatus::Active {
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
		let result = match attachment.and_then(|attachment| {
			super::downloads::original_url(&attachment)
				.map(|url| (url, attachment.filename, attachment.size))
		}) {
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
				}
				Err("Cancelled") => {
					shared.files[index].status = FileStatus::Cancelled;
					for file in shared.files.iter_mut() {
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
	/// Start a batch; at most one runs at a time. Folder is picked once on the
	/// native UI thread, then files download sequentially off-thread.
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
		{
			let mut shared = self.locked();
			shared.folder = None;
			shared.items = attachments.clone();
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
			shared.active = true;
			shared.finished = false;
			shared.announced = false;
			shared.closed = false;
		}
		self.cancelled.store(false, Ordering::Release);
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
					{
						let mut shared = shared.lock().expect("batch download state");
						shared.folder = Some(folder.clone());
					}
					worker_context.request_repaint();
					let client = reqwest::Client::builder()
						.no_proxy()
						.redirect(reqwest::redirect::Policy::none())
						.connect_timeout(Duration::from_secs(15))
						.read_timeout(Duration::from_secs(30))
						.timeout(Duration::from_secs(300))
						.build()
						.map_err(|_| "Download unavailable")
						.ok();
					let Some(client) = client else {
						let mut shared = shared.lock().expect("batch download state");
						for file in &mut shared.files {
							if file.status == FileStatus::Queued {
								file.status = FileStatus::Failed;
								file.error = Some("Download unavailable");
							}
						}
						shared.active = false;
						shared.finished = true;
						worker_context.request_repaint();
						return;
					};
					let mut reserved = Vec::new();
					for (index, attachment) in attachments.iter().enumerate() {
						if cancelled.load(Ordering::Acquire) {
							let mut shared = shared.lock().expect("batch download state");
							for file in shared.files.iter_mut().skip(index) {
								if file.status == FileStatus::Queued
									|| file.status == FileStatus::Active
								{
									file.status = FileStatus::Cancelled;
								}
							}
							break;
						}
						{
							let mut shared = shared.lock().expect("batch download state");
							shared.files[index].status = FileStatus::Active;
						}
						worker_context.request_repaint();
						let result = match super::downloads::original_url(attachment) {
							None => Err("Attachment download unavailable"),
							Some(url) => {
								let (destination, named) =
									unique_dest(&folder, &attachment.filename, &mut reserved);
								{
									let mut shared = shared.lock().expect("batch download state");
									shared.files[index].name = named;
								}
								super::downloads::download(
									&client,
									url,
									&destination,
									attachment.size,
									false,
									&cancelled,
									&wake_cancel,
									&|status| {
										if let super::downloads::Status::Downloading {
											received,
											total,
										} = status
										{
											let mut shared =
												shared.lock().expect("batch download state");
											shared.files[index].received = received;
											shared.files[index].total = total;
											worker_context.request_repaint();
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
								}
								Err("Cancelled") => {
									shared.files[index].status = FileStatus::Cancelled;
									for file in shared.files.iter_mut().skip(index + 1) {
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
						worker_context.request_repaint();
					}
					{
						let mut shared = shared.lock().expect("batch download state");
						shared.active = false;
						shared.finished = true;
					}
					worker_context.request_repaint();
				});
			});
		if let Ok(worker) = worker {
			// Reap a previous finished worker; only one batch runs at a time.
			if let Some(previous) = self.worker.take() {
				let _ = previous.join();
			}
			self.worker = Some(worker);
		} else {
			let mut shared = self.locked();
			shared.active = false;
			shared.finished = true;
		}
		context.request_repaint();
	}
	/// Retry failed files with the already-picked folder; picks nothing new.
	pub fn retry(&mut self, runtime: &tokio::runtime::Handle, context: &eframe::egui::Context) {
		let folder = self.folder();
		let Some(folder) = folder else {
			return;
		};
		if self.is_active() {
			return;
		}
		let indices: Vec<usize> = {
			let mut shared = self.locked();
			let indices: Vec<usize> = shared
				.files
				.iter()
				.enumerate()
				.filter(|(_, file)| file.status == FileStatus::Failed)
				.map(|(index, _)| index)
				.collect();
			if indices.is_empty() {
				return;
			}
			for index in &indices {
				shared.files[*index].status = FileStatus::Queued;
				shared.files[*index].received = 0;
			}
			shared.active = true;
			shared.finished = false;
			shared.announced = false;
			indices
		};
		self.cancelled.store(false, Ordering::Release);
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
		if let Ok(worker) = worker {
			if let Some(previous) = self.worker.take() {
				let _ = previous.join();
			}
			self.worker = Some(worker);
		} else {
			let mut shared = self.locked();
			shared.active = false;
			shared.finished = true;
		}
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
		let mut shared = self.locked();
		*shared = Shared::default();
	}
}

#[cfg(test)]
mod tests {
	use super::unique_dest;

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
}
