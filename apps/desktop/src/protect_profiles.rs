//! Profile files contain only public metadata. Opaque originals live in the OS keyring.
//! All filesystem/keyring operations run on a blocking worker; tests never apply a route.
use model::protect::{Action, Draft, ProfileAction, Protocol, SavedProfile, Scope};
use serde::{Deserialize, Serialize};
use std::{
	io::Read,
	path::{Path, PathBuf},
	sync::mpsc,
	time::Duration,
};
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
	rows_in(&folder()?)
}
/// One unreadable or tampered profile file is skipped, never fatal: otherwise it would hide every
/// other profile and block deleting it from the app.
fn rows_in(directory: &Path) -> Result<Vec<Row>, &'static str> {
	let mut rows = Vec::new();
	for (index, entry) in std::fs::read_dir(directory)
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
		if let Ok(row) = read_row(&path) {
			rows.push(row);
		}
	}
	rows.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
	Ok(rows)
}
fn read_row(path: &Path) -> Result<Row, &'static str> {
	let mut bytes = Vec::new();
	std::fs::File::open(path)
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
	Ok(row)
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
/// Removes one saved profile: its secret, its metadata and its public copy.
fn delete_row(id: &str) -> Result<(), &'static str> {
	if !rows()?.iter().any(|r| r.id == id) {
		return Err(STORE_ERROR);
	}
	platform::protect_profiles::forget(id).map_err(|_| STORE_ERROR)?;
	let path = folder()?;
	std::fs::remove_file(path.join(format!("{id}.json"))).map_err(|_| STORE_ERROR)?;
	let conf = path.join(format!("{id}.conf"));
	if conf.exists() {
		std::fs::remove_file(conf).map_err(|_| STORE_ERROR)?;
	}
	Ok(())
}
/// Importing the same file again replaces its profile instead of filling the list with copies.
fn remove_named(name: &str) -> Result<(), &'static str> {
	let name: String = name
		.trim()
		.chars()
		.filter(|c| !c.is_control())
		.take(64)
		.collect();
	for row in rows()?.into_iter().filter(|row| row.name == name) {
		delete_row(&row.id)?;
	}
	Ok(())
}
/// Present while PROTECT was on when Nivra last closed, so the next launch turns it on again.
fn startup_marker() -> Result<PathBuf, &'static str> {
	Ok(folder()?.join("start-enabled"))
}
pub fn startup_enabled() -> bool {
	startup_marker().is_ok_and(|path| path.exists())
}
pub fn set_startup_enabled(on: bool) {
	if let Ok(path) = startup_marker() {
		if on {
			let _ = std::fs::write(&path, b"1");
		} else {
			let _ = std::fs::remove_file(path);
		}
	}
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

fn snapshot_row(path: &std::path::Path) -> Result<Option<Row>, &'static str> {
	let file = match std::fs::File::open(path.join("current.json")) {
		Ok(file) => file,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
		Err(_) => return Err(STORE_ERROR),
	};
	let mut bytes = Vec::new();
	file.take(1025)
		.read_to_end(&mut bytes)
		.map_err(|_| STORE_ERROR)?;
	if bytes.len() > 1024 {
		return Err(STORE_ERROR);
	}
	let row: Row = serde_json::from_slice(&bytes).map_err(|_| STORE_ERROR)?;
	row.visible()?;
	Ok(Some(row))
}

fn remember_in(
	path: &std::path::Path,
	draft: &Draft,
	put: impl Fn(&str, &[u8]) -> Result<(), &'static str>,
	forget: impl Fn(&str),
) -> Result<(), &'static str> {
	use std::io::Write;
	std::fs::create_dir_all(path).map_err(|_| STORE_ERROR)?;
	if std::fs::read_dir(path)
		.map_err(|_| STORE_ERROR)?
		.take(9)
		.count()
		> 8
	{
		return Err("Remembered configuration folder exceeds its file limit");
	}
	let old = snapshot_row(path)?;
	let bytes = encode(draft)?;
	let mut random = [0; 16];
	getrandom::fill(&mut random).map_err(|_| STORE_ERROR)?;
	let id: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
	put(&id, &bytes)?;
	let row = Row {
		id: id.clone(),
		name: "Last configuration".into(),
		protocol: bytes[0],
		scope: bytes[1],
		secret_bytes: bytes.len(),
	};
	let result = (|| {
		std::fs::write(
			path.join(format!("{id}.conf")),
			model::protect::public_wireguard(&draft.wireguard),
		)
		.map_err(|_| STORE_ERROR)?;
		let mut file = std::fs::File::create(path.join("current.tmp")).map_err(|_| STORE_ERROR)?;
		file.write_all(&serde_json::to_vec(&row).map_err(|_| STORE_ERROR)?)
			.map_err(|_| STORE_ERROR)?;
		file.sync_all().map_err(|_| STORE_ERROR)?;
		drop(file);
		std::fs::rename(path.join("current.tmp"), path.join("current.json"))
			.map_err(|_| STORE_ERROR)
	})();
	if result.is_err() {
		forget(&id);
		let _ = std::fs::remove_file(path.join(format!("{id}.conf")));
		let _ = std::fs::remove_file(path.join("current.tmp"));
	} else if let Some(old) = old {
		forget(&old.id);
		let _ = std::fs::remove_file(path.join(format!("{}.conf", old.id)));
	}
	result
}

fn remember(draft: &Draft) -> Result<(), &'static str> {
	remember_in(
		&folder()?.join("remembered"),
		draft,
		|id, bytes| platform::protect_profiles::save(id, bytes).map_err(|_| STORE_ERROR),
		|id| {
			let _ = platform::protect_profiles::forget(id);
		},
	)
}

enum Event {
	Initial(
		Result<Vec<SavedProfile>, &'static str>,
		Result<Option<Draft>, &'static str>,
	),
	Rows(Vec<SavedProfile>, &'static str),
	Loaded(Draft, Result<(), &'static str>),
	Remembered,
	Result(String, String),
	Location(String, String, Option<eframe::egui::ColorImage>),
	Finished(Result<&'static str, &'static str>),
}
#[derive(Default)]
pub struct Manager {
	pending_save: Option<Draft>,
	started: bool,
	testing: bool,
	worker: Option<tokio::task::JoinHandle<()>>,
	receive: Option<mpsc::Receiver<Event>>,
	generation: Option<u64>,
	/// An imported profile waiting for the store to be free, saved under its file name.
	pending_named: Option<(String, Draft)>,
	/// Test the saved profiles (country, ping) once the current store operation ends.
	auto_test: bool,
	/// What the last launch left on: the configuration to turn on again, or none restorable.
	startup: Option<Option<Draft>>,
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
			self.pending_save = None;
			self.pending_named = None;
			self.auto_test = false;
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
		if self.worker.is_none()
			&& let Some((name, draft)) = self.pending_named.take()
		{
			// Saving also remembers it, and the new profile is tested right after.
			self.auto_test = true;
			self.start(Some(ProfileAction::Save { name, draft }), runtime, ctx, ui);
		}
		if self.worker.is_none()
			&& let Some(draft) = self.pending_save.take()
		{
			self.start(Some(ProfileAction::Remember(draft)), runtime, ctx, ui);
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
				Some(Event::Initial(rows, draft)) => {
					let has_rows = rows.as_ref().is_ok_and(|rows| !rows.is_empty());
					if let Ok(rows) = rows {
						ui.set_protect_profiles(rows);
					}
					let startup = startup_enabled();
					if startup {
						self.startup = Some(draft.as_ref().ok().and_then(|draft| draft.clone()));
					}
					if self.pending_save.is_none()
						&& let Ok(Some(draft)) = draft
					{
						ui.set_protect_draft(draft);
					}
					// Country and ping are collected without being asked, unless PROTECT is
					// about to come back on (WireGuard tests need it off).
					if !startup && has_rows {
						self.auto_test = true;
					}
				}
				Some(Event::Rows(rows, status)) => {
					ui.set_protect_profiles(rows);
					ui.set_protect_profile_status(status, true, self.testing);
				}
				Some(Event::Loaded(draft, _)) => ui.set_protect_draft(draft),
				Some(Event::Remembered) => {}
				Some(Event::Result(id, result)) => ui.set_protect_profile_result(&id, &result),
				Some(Event::Location(id, text, flag)) => {
					ui.set_protect_profile_location(ctx, &id, &text, flag)
				}
				Some(Event::Finished(result)) => {
					ui.set_protect_profile_status(result.unwrap_or_else(|e| e), false, false);
					self.worker = None;
					self.receive = None;
					self.testing = false;
					if std::mem::take(&mut self.auto_test) && !protect::enabled() {
						let location = ui.protect_locate();
						self.start(Some(ProfileAction::TestAll { location }), runtime, ctx, ui);
					}
				}
				None => break,
			}
		}
	}
	pub fn remember(&mut self, draft: Draft) {
		self.pending_save = Some(draft);
	}
	/// Saves an imported configuration as a profile named after its file.
	pub fn save_named(&mut self, name: String, draft: Draft) {
		self.pending_named = Some((name, draft));
	}
	/// The launch-time outcome, once: the configuration to enable, or `None` if there is none.
	pub fn take_startup(&mut self) -> Option<Option<Draft>> {
		self.startup.take()
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
						None => {
							let draft = snapshot_row(&folder()?.join("remembered"))
								.and_then(|row| row.map(|row| load(&row)).transpose());
							Ok(Event::Initial(visible_rows(), draft))
						}
						Some(ProfileAction::Remember(draft)) => {
							remember(&draft)?;
							Ok(Event::Remembered)
						}
						Some(ProfileAction::Save { name, draft }) => {
							remove_named(&name)?;
							save(name, draft.clone())?;
							let status = if remember(&draft).is_ok() {
								"Profile saved in the OS credential store"
							} else {
								"Named profile saved, but remembering the current configuration failed"
							};
							Ok(Event::Rows(visible_rows()?, status))
						}
						Some(ProfileAction::Load(id)) => {
							let row = rows()?
								.into_iter()
								.find(|r| r.id == id)
								.ok_or(STORE_ERROR)?;
							let draft = load(&row)?;
							let remembered = remember(&draft);
							Ok(Event::Loaded(draft, remembered))
						}
						Some(ProfileAction::Delete(id)) => {
							delete_row(&id)?;
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
							Event::Initial(_, Err(error)) | Event::Initial(Err(error), _) => *error,
							Event::Initial(_, Ok(Some(_))) => {
								"Configuration restored; enable PROTECT to apply it"
							}
							Event::Initial(_, Ok(None)) => "Saved profiles ready",
							Event::Remembered => {
								"Configuration saved securely; available after restart"
							}
							Event::Rows(_, status) => *status,
							Event::Loaded(_, Ok(())) => "Profile loaded; test before enabling",
							Event::Loaded(_, Err(_)) => {
								"Profile loaded, but remembering it failed; save again before closing"
							}
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

	struct SnapshotDirectory(PathBuf);
	impl SnapshotDirectory {
		fn new() -> Self {
			let mut nonce = [0; 16];
			getrandom::fill(&mut nonce).unwrap();
			let name: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
			Self(std::env::temp_dir().join(format!("nivra-protect-test-{name}")))
		}
	}
	impl Drop for SnapshotDirectory {
		fn drop(&mut self) {
			let _ = std::fs::remove_dir_all(&self.0);
		}
	}

	#[test]
	fn one_corrupt_profile_file_does_not_hide_the_others() {
		let directory = SnapshotDirectory::new();
		std::fs::create_dir_all(&directory.0).unwrap();
		let id = "0123456789abcdef0123456789abcdef";
		let good = serde_json::json!({
			"id": id, "name": "work", "protocol": 0, "scope": 0, "secret_bytes": 10
		});
		std::fs::write(directory.0.join(format!("{id}.json")), good.to_string()).unwrap();
		std::fs::write(
			directory.0.join("ffffffffffffffffffffffffffffffff.json"),
			b"not json",
		)
		.unwrap();
		let rows = rows_in(&directory.0).unwrap();
		assert_eq!(
			rows.len(),
			1,
			"the corrupt file is skipped, the good one listed"
		);
		assert_eq!(rows[0].id, id);
	}

	#[test]
	fn remembered_configuration_survives_reopen_and_replacement_without_plaintext_secrets() {
		let directory = SnapshotDirectory::new();
		let secrets = std::cell::RefCell::new(std::collections::HashMap::<String, Vec<u8>>::new());
		let mut draft = Draft {
			protocol: Protocol::WireGuard,
			scope: Scope::Complete,
			..Draft::default()
		};
		draft.wireguard.push_str("[Interface]\nPrivateKey = synthetic-private\nAddress = 10.0.0.2/32\n[Peer]\nPresharedKey = synthetic-preshared\nEndpoint = example.invalid:51820\n");
		let store = |id: &str, bytes: &[u8]| {
			secrets.borrow_mut().insert(id.into(), bytes.to_vec());
			Ok(())
		};
		let forget = |id: &str| {
			secrets.borrow_mut().remove(id);
		};
		remember_in(&directory.0, &draft, store, forget).unwrap();
		let first = snapshot_row(&directory.0).unwrap().unwrap();
		let restored = decode(&secrets.borrow()[&first.id]).unwrap();
		assert_eq!(restored.wireguard.as_str(), draft.wireguard.as_str());
		assert_eq!(restored.scope, Scope::Complete);
		for file in std::fs::read_dir(&directory.0).unwrap() {
			let bytes = std::fs::read(file.unwrap().path()).unwrap();
			let text = String::from_utf8(bytes).unwrap();
			assert!(!text.contains("synthetic-private"));
			assert!(!text.contains("synthetic-preshared"));
		}
		draft.scope = Scope::Streams;
		remember_in(&directory.0, &draft, store, forget).unwrap();
		let next = snapshot_row(&directory.0).unwrap().unwrap();
		assert_ne!(first.id, next.id);
		assert!(!secrets.borrow().contains_key(&first.id));
		assert!(!directory.0.join(format!("{}.conf", first.id)).exists());
		assert_eq!(secrets.borrow().len(), 1);
		assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 2);
		assert_eq!(
			decode(&secrets.borrow()[&next.id]).unwrap().scope,
			Scope::Streams
		);
	}

	#[test]
	fn failed_snapshot_publication_keeps_the_previous_configuration() {
		let directory = SnapshotDirectory::new();
		let secrets = std::cell::RefCell::new(std::collections::HashMap::<String, Vec<u8>>::new());
		let store = |id: &str, bytes: &[u8]| {
			secrets.borrow_mut().insert(id.into(), bytes.to_vec());
			Ok(())
		};
		let forget = |id: &str| {
			secrets.borrow_mut().remove(id);
		};
		let draft = Draft {
			host: "example.invalid".into(),
			..Draft::default()
		};
		remember_in(&directory.0, &draft, store, forget).unwrap();
		let first = snapshot_row(&directory.0).unwrap().unwrap();
		assert!(remember_in(&directory.0, &draft, |_, _| Err(STORE_ERROR), forget).is_err());
		assert_eq!(snapshot_row(&directory.0).unwrap().unwrap().id, first.id);
		std::fs::create_dir(directory.0.join("current.tmp")).unwrap();
		assert!(remember_in(&directory.0, &draft, store, forget).is_err());
		assert_eq!(snapshot_row(&directory.0).unwrap().unwrap().id, first.id);
		assert_eq!(secrets.borrow().len(), 1);
		assert!(secrets.borrow().contains_key(&first.id));
		std::fs::remove_dir(directory.0.join("current.tmp")).unwrap();
		std::fs::write(directory.0.join("current.json"), vec![b' '; 1025]).unwrap();
		assert!(snapshot_row(&directory.0).is_err());
	}

	#[test]
	fn a_broken_named_list_does_not_discard_a_restorable_configuration() {
		let runtime = tokio::runtime::Runtime::new().unwrap();
		let (send, receive) = mpsc::sync_channel(2);
		let draft = Draft {
			host: "restored.invalid".into(),
			..Draft::default()
		};
		send.send(Event::Initial(Err(STORE_ERROR), Ok(Some(draft))))
			.ok()
			.unwrap();
		send.send(Event::Finished(Err(STORE_ERROR))).ok().unwrap();
		let mut manager = Manager {
			started: true,
			receive: Some(receive),
			pending_save: None,
			testing: false,
			worker: None,
			generation: None,
			pending_named: None,
			auto_test: false,
			startup: None,
		};
		let mut ui = ui::MessagingUi::default();
		manager.poll(&runtime, &eframe::egui::Context::default(), &mut ui);
		assert_eq!(ui.protect_draft().host, "restored.invalid");
	}

	#[test]
	fn loading_a_valid_profile_survives_a_failed_remember_operation() {
		let runtime = tokio::runtime::Runtime::new().unwrap();
		let (send, receive) = mpsc::sync_channel(2);
		let draft = Draft {
			host: "loaded.invalid".into(),
			..Draft::default()
		};
		send.send(Event::Loaded(draft, Err(STORE_ERROR)))
			.ok()
			.unwrap();
		send.send(Event::Finished(Err(STORE_ERROR))).ok().unwrap();
		let mut manager = Manager {
			started: true,
			receive: Some(receive),
			pending_save: None,
			testing: false,
			worker: None,
			generation: None,
			pending_named: None,
			auto_test: false,
			startup: None,
		};
		let mut ui = ui::MessagingUi::default();
		manager.poll(&runtime, &eframe::egui::Context::default(), &mut ui);
		assert_eq!(ui.protect_draft().host, "loaded.invalid");
	}

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
			pending_save: None,
			started: false,
			testing: true,
			worker: None,
			receive: None,
			generation: None,
			pending_named: None,
			auto_test: false,
			startup: None,
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
