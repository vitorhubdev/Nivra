//! Profile files contain only public metadata. Opaque originals live in the OS keyring.
//! All filesystem/keyring operations run on a blocking worker; tests never apply a route.
use model::protect::{Action, Draft, ProfileAction, Protocol, SavedProfile, Scope};
use serde::{Deserialize, Serialize};
use std::{io::Read, path::PathBuf, sync::mpsc, time::Duration};
use zeroize::Zeroizing;

const MAX_PROFILES: usize = 16;
const STORE_ERROR: &str = "Could not access protected profiles; no plaintext fallback was used";
const WG_TEST_BLOCKED: &str = "Disable PROTECT before testing another WireGuard tunnel";

/// An aborted async operation cannot abort an OS keyring call. The permit lives
/// inside that call, bounding even cancelled/generation-retired blocking workers.
async fn bounded_store<F, T>(operation: F) -> Result<T, ()>
where
	F: FnOnce() -> T + Send + 'static,
	T: Send + 'static,
{
	static GATE: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
		std::sync::OnceLock::new();
	let permit = GATE
		.get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(1)))
		.clone()
		.try_acquire_owned()
		.map_err(|_| ())?;
	tokio::task::spawn_blocking(move || {
		let _permit = permit;
		operation()
	})
	.await
	.map_err(|_| ())
}

fn decode_flag(bytes: &[u8]) -> Option<eframe::egui::ColorImage> {
	if bytes.len() > 64 * 1024 {
		return None;
	}
	let dimensions =
		image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png)
			.into_dimensions()
			.ok()?;
	if dimensions != (32, 24) {
		return None;
	}
	let pixels = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
		.ok()?
		.to_rgba8();
	Some(eframe::egui::ColorImage::from_rgba_unmultiplied(
		[32, 24],
		&pixels,
	))
}

#[derive(Serialize, Deserialize)]
struct Row {
	id: String,
	name: String,
	protocol: u8,
	scope: u8,
	secret_bytes: usize,
}
impl Row {
	fn visible(&self) -> Result<SavedProfile, &'static str> {
		if !valid_id(&self.id)
			|| self.name.is_empty()
			|| self.name.len() > 256
			|| self.secret_bytes > 8192
		{
			return Err(STORE_ERROR);
		}
		Ok(SavedProfile {
			id: self.id.clone(),
			name: self.name.clone(),
			protocol: protocol(self.protocol)?,
			scope: scope(self.scope)?,
			result: String::new(),
			location: String::new(),
		})
	}
}
fn valid_id(id: &str) -> bool {
	id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}
fn protocol(value: u8) -> Result<Protocol, &'static str> {
	Protocol::ALL
		.get(usize::from(value))
		.copied()
		.ok_or(STORE_ERROR)
}
fn scope(value: u8) -> Result<Scope, &'static str> {
	Scope::ALL
		.get(usize::from(value))
		.copied()
		.ok_or(STORE_ERROR)
}
fn folder() -> Result<PathBuf, &'static str> {
	let path = dirs::data_local_dir()
		.ok_or(STORE_ERROR)?
		.join("nivra")
		.join("protect");
	std::fs::create_dir_all(&path).map_err(|_| STORE_ERROR)?;
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
			.map_err(|_| STORE_ERROR)?;
	}
	Ok(path)
}
fn rows() -> Result<Vec<Row>, &'static str> {
	let mut rows = Vec::new();
	for (index, entry) in std::fs::read_dir(folder()?)
		.map_err(|_| STORE_ERROR)?
		.enumerate()
	{
		if index >= MAX_PROFILES * 4 {
			return Err("PROTECT profile folder exceeds its file limit");
		}
		let path = entry.map_err(|_| STORE_ERROR)?.path();
		if path.extension().is_none_or(|ext| ext != "json") {
			continue;
		}
		if rows.len() >= MAX_PROFILES {
			return Err("At most 16 saved PROTECT profiles are supported");
		}
		let mut bytes = Vec::new();
		std::fs::File::open(&path)
			.map_err(|_| STORE_ERROR)?
			.take(1025)
			.read_to_end(&mut bytes)
			.map_err(|_| STORE_ERROR)?;
		if bytes.len() > 1024 {
			return Err(STORE_ERROR);
		}
		let row: Row = serde_json::from_slice(&bytes).map_err(|_| STORE_ERROR)?;
		row.visible()?;
		if path.file_stem().and_then(|s| s.to_str()) != Some(&row.id) {
			return Err(STORE_ERROR);
		}
		rows.push(row);
	}
	rows.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
	Ok(rows)
}
fn visible_rows() -> Result<Vec<SavedProfile>, &'static str> {
	rows()?.iter().map(Row::visible).collect()
}
fn encode(draft: &Draft) -> Result<Zeroizing<Vec<u8>>, &'static str> {
	if draft.host.len() > 253
		|| draft.username.len() > 255
		|| draft.password.len() > 255
		|| draft.wireguard.len() > 4096
	{
		return Err("Profile fields exceed their limits");
	}
	let mut bytes = Zeroizing::new(Vec::new());
	bytes.push(
		Protocol::ALL
			.iter()
			.position(|p| p == &draft.protocol)
			.ok_or(STORE_ERROR)? as u8,
	);
	bytes.push(
		Scope::ALL
			.iter()
			.position(|s| s == &draft.scope)
			.ok_or(STORE_ERROR)? as u8,
	);
	bytes.extend_from_slice(&draft.port.to_le_bytes());
	for field in [
		&*draft.host,
		&*draft.username,
		&*draft.password,
		&*draft.wireguard,
	] {
		bytes.extend_from_slice(&(field.len() as u16).to_le_bytes());
		bytes.extend_from_slice(field.as_bytes());
	}
	Ok(bytes)
}
fn decode(bytes: &[u8]) -> Result<Draft, &'static str> {
	let mut cursor = std::io::Cursor::new(bytes);
	let mut head = [0; 4];
	cursor.read_exact(&mut head).map_err(|_| STORE_ERROR)?;
	let mut fields = Vec::with_capacity(4);
	for limit in [253, 255, 255, 4096] {
		let mut size = [0; 2];
		cursor.read_exact(&mut size).map_err(|_| STORE_ERROR)?;
		let len = usize::from(u16::from_le_bytes(size));
		if len > limit {
			return Err(STORE_ERROR);
		}
		let start = cursor.position() as usize;
		let end = start
			.checked_add(len)
			.filter(|end| *end <= bytes.len())
			.ok_or(STORE_ERROR)?;
		fields.push(Zeroizing::new(
			std::str::from_utf8(&bytes[start..end])
				.map_err(|_| STORE_ERROR)?
				.to_owned(),
		));
		cursor.set_position(end as u64);
	}
	if cursor.position() as usize != bytes.len() {
		return Err(STORE_ERROR);
	}
	let mut fields = fields.into_iter();
	Ok(Draft {
		protocol: protocol(head[0])?,
		scope: scope(head[1])?,
		port: u16::from_le_bytes([head[2], head[3]]),
		host: fields.next().ok_or(STORE_ERROR)?.to_string(),
		username: fields.next().ok_or(STORE_ERROR)?,
		password: fields.next().ok_or(STORE_ERROR)?,
		wireguard: fields.next().ok_or(STORE_ERROR)?,
	})
}
fn load(row: &Row) -> Result<Draft, &'static str> {
	let bytes = platform::protect_profiles::load(&row.id).map_err(|_| STORE_ERROR)?;
	if bytes.len() != row.secret_bytes {
		return Err(STORE_ERROR);
	}
	decode(&bytes)
}
fn save(name: String, draft: Draft) -> Result<(), &'static str> {
	if rows()?.len() >= MAX_PROFILES {
		return Err("At most 16 profiles can be saved; remove one first");
	}
	let name: String = name
		.trim()
		.chars()
		.filter(|c| !c.is_control())
		.take(64)
		.collect();
	if name.is_empty() {
		return Err("Give this profile a name before saving");
	}
	let bytes = encode(&draft)?;
	let mut random = [0; 16];
	getrandom::fill(&mut random).map_err(|_| STORE_ERROR)?;
	let id: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
	let path = folder()?;
	platform::protect_profiles::save(&id, &bytes).map_err(|_| STORE_ERROR)?;
	let row = Row {
		id: id.clone(),
		name,
		protocol: bytes[0],
		scope: bytes[1],
		secret_bytes: bytes.len(),
	};
	let result = (|| {
		use std::io::Write;
		let public = model::protect::public_wireguard(&draft.wireguard);
		if draft.protocol == Protocol::WireGuard {
			std::fs::OpenOptions::new()
				.write(true)
				.create_new(true)
				.open(path.join(format!("{id}.conf")))
				.map_err(|_| STORE_ERROR)?
				.write_all(
					format!(
						"# Public reference only. Private keys are in the Nivra OS credential store.\n{public}"
					)
					.as_bytes(),
				)
				.map_err(|_| STORE_ERROR)?;
		}
		let metadata = serde_json::to_vec(&row).map_err(|_| STORE_ERROR)?;
		let temporary = path.join(format!("{id}.tmp"));
		let mut metadata_file = std::fs::OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(&temporary)
			.map_err(|_| STORE_ERROR)?;
		metadata_file
			.write_all(&metadata)
			.map_err(|_| STORE_ERROR)?;
		metadata_file.sync_all().map_err(|_| STORE_ERROR)?;
		drop(metadata_file);
		std::fs::rename(temporary, path.join(format!("{id}.json"))).map_err(|_| STORE_ERROR)
	})();
	if result.is_err() {
		let _ = platform::protect_profiles::forget(&id);
		let _ = std::fs::remove_file(path.join(format!("{id}.conf")));
		let _ = std::fs::remove_file(path.join(format!("{id}.json")));
		let _ = std::fs::remove_file(path.join(format!("{id}.tmp")));
	}
	result
}

enum Event {
	Rows(Vec<SavedProfile>, &'static str),
	Loaded(Draft),
	Result(String, String),
	Location(String, String, Option<eframe::egui::ColorImage>),
	Finished(Result<&'static str, &'static str>),
}
#[derive(Default)]
pub struct Manager {
	started: bool,
	testing: bool,
	worker: Option<tokio::task::JoinHandle<()>>,
	receive: Option<mpsc::Receiver<Event>>,
	generation: Option<u64>,
}
impl Drop for Manager {
	fn drop(&mut self) {
		if let Some(worker) = &self.worker {
			worker.abort();
		}
	}
}
impl Manager {
	pub fn scope(&mut self, generation: u64) {
		if self.generation != Some(generation) {
			if let Some(worker) = self.worker.take() {
				worker.abort();
			}
			self.receive = None;
			self.started = false;
			self.testing = false;
			self.generation = Some(generation);
		}
	}
	pub fn poll(
		&mut self,
		runtime: &tokio::runtime::Runtime,
		ctx: &eframe::egui::Context,
		ui: &mut ui::MessagingUi,
	) {
		if !self.started {
			self.started = true;
			self.start(None, runtime, ctx, ui);
		}
		for _ in 0..64 {
			let event = match self.receive.as_ref().map(|r| r.try_recv()) {
				Some(Ok(event)) => Some(event),
				Some(Err(mpsc::TryRecvError::Disconnected)) => {
					self.worker = None;
					self.receive = None;
					self.testing = false;
					ui.set_protect_profile_status(
						"Profile worker stopped; retry the operation",
						false,
						false,
					);
					None
				}
				_ => None,
			};
			match event {
				Some(Event::Rows(rows, status)) => {
					ui.set_protect_profiles(rows);
					ui.set_protect_profile_status(status, true, self.testing);
				}
				Some(Event::Loaded(draft)) => ui.set_protect_draft(draft),
				Some(Event::Result(id, result)) => ui.set_protect_profile_result(&id, &result),
				Some(Event::Location(id, text, flag)) => {
					ui.set_protect_profile_location(ctx, &id, &text, flag)
				}
				Some(Event::Finished(result)) => {
					ui.set_protect_profile_status(result.unwrap_or_else(|e| e), false, false);
					self.worker = None;
					self.receive = None;
					self.testing = false;
				}
				None => break,
			}
		}
	}
	pub fn handle(
		&mut self,
		action: Action,
		runtime: &tokio::runtime::Runtime,
		ctx: &eframe::egui::Context,
		ui: &mut ui::MessagingUi,
	) -> Option<Action> {
		if self.testing && !matches!(action, Action::Profiles(_)) {
			ui.set_protect_operation_status(
				"Cancel profile tests before changing PROTECT",
				protect::enabled(),
				false,
			);
			return None;
		}
		let Action::Profiles(action) = action else {
			return Some(action);
		};
		if matches!(action, ProfileAction::CancelTests) {
			if self.testing {
				if let Some(worker) = self.worker.take() {
					worker.abort();
				}
				self.receive = None;
				self.testing = false;
				ui.set_protect_profile_status(
					"Tests cancelled; active route unchanged",
					false,
					false,
				);
			}
		} else if self.worker.is_none() {
			self.start(Some(action), runtime, ctx, ui);
		}
		None
	}
	fn start(
		&mut self,
		action: Option<ProfileAction>,
		runtime: &tokio::runtime::Runtime,
		ctx: &eframe::egui::Context,
		ui: &mut ui::MessagingUi,
	) {
		self.testing = matches!(action, Some(ProfileAction::TestAll { .. }));
		ui.set_protect_profile_status(
			if self.testing {
				"Testing saved profiles sequentially…"
			} else {
				"Opening protected profile store…"
			},
			true,
			self.testing,
		);
		let (send, receive) = mpsc::sync_channel(64);
		self.receive = Some(receive);
		let wake = ctx.clone();
		self.worker = Some(runtime.spawn(async move {
			let publish = |event| {
				let _ = send.try_send(event);
				wake.request_repaint();
			};
			let result = if let Some(ProfileAction::TestAll { location }) = action {
				match bounded_store(|| {
					rows().map(|rows| {
						rows.into_iter()
							.map(|r| {
								let draft = load(&r);
								(r.id, draft)
							})
							.collect::<Vec<_>>()
					})
				})
				.await
				{
					Ok(Ok(profiles)) => {
						for (id, draft) in profiles {
							let draft = match draft {
								Ok(draft) => draft,
								Err(error) => {
									publish(Event::Result(id, error.into()));
									continue;
								}
							};
							if draft.protocol == Protocol::WireGuard && protect::enabled() {
								publish(Event::Location(id.clone(), String::new(), None));
								publish(Event::Result(id, WG_TEST_BLOCKED.into()));
								continue;
							}
							let begin = std::time::Instant::now();
							publish(Event::Location(id.clone(), String::new(), None));
							let tested = tokio::time::timeout(Duration::from_secs(30), async {
								let profile = protect::prepare(&draft).await?;
								let result = protect::test(profile.clone()).await?;
								Ok::<_, &'static str>((result, profile))
							})
							.await
							.unwrap_or(Err("Test timed out"));
							let (status, tested_profile) = match tested {
								Ok((status, profile)) => (status, Some(profile)),
								Err(error) => (error, None),
							};
							publish(Event::Result(
								id.clone(),
								format!("{} · {} ms", status, begin.elapsed().as_millis()),
							));
							if location && let Some(profile) = tested_profile {
								match tokio::time::timeout(
									Duration::from_secs(22),
									protect::exit_location(profile),
								)
								.await
								{
									Ok(Ok(location)) => {
										let flag =
											location.flag_png.as_deref().and_then(decode_flag);
										publish(Event::Location(
											id,
											format!(
												"{} · {} ({})",
												location.ip,
												location.country,
												location.code.to_uppercase()
											),
											flag,
										));
									}
									Ok(Err(error)) => {
										publish(Event::Location(id, error.into(), None))
									}
									Err(_) => publish(Event::Location(
										id,
										"Location query timed out; route test result preserved"
											.into(),
										None,
									)),
								}
							}
						}
						Ok(
							"All saved profiles tested; active route unchanged. Probes do not prove a working call.",
						)
					}
					Ok(Err(error)) => Err(error),
					Err(_) => Err(STORE_ERROR),
				}
			} else {
				match bounded_store(move || -> Result<Event, &'static str> {
					match action {
						None => Ok(Event::Rows(visible_rows()?, "Saved profiles ready")),
						Some(ProfileAction::Save { name, draft }) => {
							save(name, draft)?;
							Ok(Event::Rows(
								visible_rows()?,
								"Profile saved in the OS credential store",
							))
						}
						Some(ProfileAction::Load(id)) => {
							let row = rows()?
								.into_iter()
								.find(|r| r.id == id)
								.ok_or(STORE_ERROR)?;
							Ok(Event::Loaded(load(&row)?))
						}
						Some(ProfileAction::Delete(id)) => {
							if !rows()?.iter().any(|r| r.id == id) {
								return Err(STORE_ERROR);
							}
							platform::protect_profiles::forget(&id).map_err(|_| STORE_ERROR)?;
							let path = folder()?;
							std::fs::remove_file(path.join(format!("{id}.json")))
								.map_err(|_| STORE_ERROR)?;
							let conf = path.join(format!("{id}.conf"));
							if conf.exists() {
								std::fs::remove_file(conf).map_err(|_| STORE_ERROR)?;
							}
							Ok(Event::Rows(
								visible_rows()?,
								"Saved profile removed; active route unchanged",
							))
						}
						_ => Err(STORE_ERROR),
					}
				})
				.await
				{
					Ok(Ok(event)) => {
						let status = match &event {
							Event::Rows(_, status) => *status,
							Event::Loaded(_) => "Profile loaded; test before enabling",
							_ => "Protected profile operation completed",
						};
						publish(event);
						Ok(status)
					}
					Ok(Err(e)) => Err(e),
					Err(_) => Err(STORE_ERROR),
				}
			};
			publish(Event::Finished(result));
		}));
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn cancellation_retains_the_blocking_store_slot_until_the_operation_exits() {
		let runtime = tokio::runtime::Runtime::new().unwrap();
		runtime.block_on(async {
			let (started, ready) = tokio::sync::oneshot::channel();
			let (release, wait) = std::sync::mpsc::sync_channel(1);
			let operation = tokio::spawn(bounded_store(move || {
				let _ = started.send(());
				let _ = wait.recv_timeout(Duration::from_secs(3));
			}));
			tokio::time::timeout(Duration::from_secs(2), ready)
				.await
				.unwrap()
				.unwrap();
			operation.abort();
			let _ = operation.await;
			assert!(bounded_store(|| ()).await.is_err());
			release.send(()).unwrap();
			tokio::time::timeout(Duration::from_secs(2), async {
				while bounded_store(|| ()).await.is_err() {
					tokio::task::yield_now().await;
				}
			})
			.await
			.unwrap();
		});
	}

	#[test]
	fn route_changes_require_cancelling_the_running_batch_first() {
		let runtime = tokio::runtime::Runtime::new().unwrap();
		let mut manager = Manager {
			started: false,
			testing: true,
			worker: None,
			receive: None,
			generation: None,
		};
		let mut ui = ui::MessagingUi::default();
		assert!(
			manager
				.handle(
					Action::Disable,
					&runtime,
					&eframe::egui::Context::default(),
					&mut ui
				)
				.is_none()
		);
		assert!(manager.testing);
		assert!(
			manager
				.handle(
					Action::Profiles(ProfileAction::CancelTests),
					&runtime,
					&eframe::egui::Context::default(),
					&mut ui
				)
				.is_none()
		);
		assert!(!manager.testing);
		assert!(matches!(
			manager.handle(
				Action::Disable,
				&runtime,
				&eframe::egui::Context::default(),
				&mut ui
			),
			Some(Action::Disable)
		));
	}

	#[test]
	fn flag_decode_requires_expected_dimensions_before_allocating_pixels() {
		for (width, accepted) in [(31, false), (32, true)] {
			let image = image::RgbaImage::new(width, 24);
			let mut bytes = std::io::Cursor::new(Vec::new());
			image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
			assert_eq!(decode_flag(bytes.get_ref()).is_some(), accepted);
		}
		assert!(decode_flag(&vec![0; 64 * 1024 + 1]).is_none());
	}
	#[test]
	fn private_profile_round_trip_and_bounds() {
		let mut draft = Draft {
			host: "example.invalid".into(),
			..Draft::default()
		};
		draft.password.push_str("secret");
		draft.wireguard.push_str("PrivateKey = hidden");
		let bytes = encode(&draft).unwrap();
		let restored = decode(&bytes).unwrap();
		assert_eq!(restored.password.as_str(), "secret");
		assert_eq!(restored.wireguard.as_str(), "PrivateKey = hidden");
		assert!(decode(&bytes[..bytes.len() - 1]).is_err());
		draft.wireguard.push_str(&"x".repeat(4096));
		assert!(encode(&draft).is_err());
	}
}
