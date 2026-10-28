//! Desktop ownership for watching other participants' screen shares, up to the
//! shared multi-watch cap. Every explicitly watched stream negotiates its own view;
//! frames are decoded off the render thread and never written to disk.
use client_core::{Command, Event, State, screen, voice};
use discord_voice::{Identity, Status};
use eframe::egui;
use model::Id;
use std::{
	collections::VecDeque,
	sync::{
		Arc, Mutex,
		atomic::{AtomicBool, Ordering},
	},
	time::{Duration, Instant},
};
use tokio::{runtime::Runtime, sync::watch, task::JoinHandle};
use zeroize::Zeroizing;

const SIGNAL_TIMEOUT: Duration = Duration::from_secs(30);

/// Replace an undisplayed frame in place; UI ownership moves out through the existing slot.
#[allow(clippy::chunks_exact_to_as_chunks)] // Match egui's conversion loop.
fn store_frame(
	picture: &mut Option<egui::ColorImage>,
	frame: discord_voice::RemoteFrame<'_>,
) -> bool {
	let size = [frame.width as usize, frame.height as usize];
	if size.contains(&0)
		|| size[0]
			.checked_mul(size[1])
			.and_then(|pixels| pixels.checked_mul(4))
			!= Some(frame.rgba.len())
	{
		return false;
	}
	if let Some(image) = picture.as_mut() {
		image.size = size;
		image.source_size = egui::vec2(frame.width as f32, frame.height as f32);
		let pixels = frame.rgba.len() / 4;
		if image.pixels.capacity() > 1024 * 1024 / size_of::<egui::Color32>()
			&& image.pixels.capacity() > pixels.saturating_mul(4)
		{
			// Release a previous large resolution while keeping ordinary resize reuse.
			image.pixels = Vec::with_capacity(pixels);
		}
		if pixels > image.pixels.capacity() {
			// Do not double a previous resolution's allocation beyond the frame bound.
			image.pixels.reserve_exact(pixels - image.pixels.len());
		}
		// egui's pixel conversion is optimized in debug builds too; keep large
		// scalar application loops out of that path with bounded conversion scratch.
		#[cfg(debug_assertions)]
		{
			image.pixels.resize(pixels, egui::Color32::TRANSPARENT);
			for (output, input) in image
				.as_raw_mut()
				.chunks_mut(64 * 1024)
				.zip(frame.rgba.chunks(64 * 1024))
			{
				let converted =
					egui::ColorImage::from_rgba_unmultiplied([input.len() / 4, 1], input);
				output.copy_from_slice(converted.as_raw());
			}
		}
		#[cfg(not(debug_assertions))]
		{
			image.pixels.clear();
			image.pixels.extend(frame.rgba.chunks_exact(4).map(|pixel| {
				egui::Color32::from_rgba_unmultiplied(pixel[0], pixel[1], pixel[2], pixel[3])
			}));
		}
	} else {
		*picture = Some(egui::ColorImage::from_rgba_unmultiplied(size, frame.rgba));
	}
	true
}

/// Whether a decoded stream frame reaches a visible surface: the floating window or the
/// selected channel of a non-minimized main window.
fn stream_visible(pip: bool, selected: Option<Id>, channel: Id, minimized: bool) -> bool {
	pip || (selected == Some(channel) && !minimized)
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Context {
	generation: u64,
	channel: Id,
	request: u64,
	stream_request: u64,
	streamer: Id,
}
impl Context {
	/// Same call session and streamer: `stream_request` differs per negotiation.
	fn matches(&self, generation: u64, channel: Id, request: u64, streamer: Id) -> bool {
		self.generation == generation
			&& self.channel == channel
			&& self.request == request
			&& self.streamer == streamer
	}
}
/// Tell the gateway to release one negotiated view.
fn stop_command(context: Context) -> Command {
	Command::Voice(voice::Command::StopWatching {
		channel: context.channel,
		request: context.request,
		stream_request: context.stream_request,
	})
}
struct Pending {
	context: Context,
	user: Id,
	peer: Option<Id>,
	session: Zeroizing<String>,
	identity: Arc<Identity>,
	audio: Option<std::sync::mpsc::SyncSender<discord_voice::Frame>>,
	rtc: Option<(Id, Id)>,
	server: Option<(voice::Secret, String)>,
	started: Instant,
	/// The view failed before connecting; torn down on the next poll.
	ended: Option<&'static str>,
}
#[derive(Clone, Copy)]
enum Notice {
	Status(&'static str),
	Failed(&'static str),
}
struct Live {
	context: Context,
	task: JoinHandle<()>,
	events: watch::Receiver<Option<Notice>>,
	frames: Arc<Mutex<Option<egui::ColorImage>>>,
	status: &'static str,
	/// The streamer stopped or Discord failed the view; torn down on the next poll.
	ended: Option<&'static str>,
	/// When the current view was requested, for the local diagnostics timeline.
	requested_at: Instant,
	first_frame_logged: bool,
}

#[derive(Default)]
pub(super) struct Watch {
	pending: Vec<Pending>,
	lives: Vec<Live>,
	/// Voice commands produced faster than the single-command lane drains: one is
	/// returned per poll while the rest wait here. A non-empty outbox schedules the
	/// next frame, so every queued stop and start completes within a few polls.
	outbox: VecDeque<Command>,
	sequence: u64,
	status: &'static str,
	/// Why the last view stopped; shown as a stage notice until the next request or hang-up.
	notice: &'static str,
	/// Whether a decoded frame could be seen (stage or PiP). The decoder threads
	/// check this before waking the UI, so hidden streams never schedule
	/// full-window redraws; the bounded slots keep the newest frames meanwhile.
	visible: Arc<AtomicBool>,
}
impl Watch {
	/// Retire transports, not the user's watch choices. The next poll renegotiates
	/// only streams still wanted in the same call, behind any required teardown.
	pub fn route_changed(&mut self) {
		for pending in self.pending.drain(..) {
			let context = pending.context;
			let unsent = self.outbox.iter().any(|command| matches!(command,
				Command::Voice(voice::Command::WatchStream { channel, request, stream_request, .. })
				if *channel == context.channel && *request == context.request && *stream_request == context.stream_request));
			if !unsent {
				self.outbox.push_back(stop_command(context));
			}
		}
		self.outbox.retain(|command| {
			!matches!(command, Command::Voice(voice::Command::WatchStream { .. }))
		});
		for live in self.lives.drain(..) {
			live.task.abort();
			self.outbox.push_back(stop_command(live.context));
		}
		self.visible.store(false, Ordering::Release);
	}
	pub fn stop(&mut self) {
		self.pending.clear();
		self.outbox.clear();
		for live in self.lives.drain(..) {
			live.task.abort();
		}
	}
	/// One queued voice command per poll; a non-empty outbox schedules the next frame.
	fn drain(&mut self, ctx: &egui::Context) -> Option<Command> {
		if self.outbox.is_empty() {
			return None;
		}
		ctx.request_repaint();
		self.outbox.pop_front()
	}
	/// Take negotiation secrets before the UI reduces the event. Nothing is persisted.
	pub fn observe(&mut self, state: &State, event: &mut Event) {
		let Event::Voice(voice::Event::Watch {
			channel,
			request,
			stream_request,
			streamer,
			event,
		}) = event
		else {
			return;
		};
		if let Some(pending) = self.pending.iter_mut().find(|pending| {
			pending
				.context
				.matches(state.generation, *channel, *request, *streamer)
				&& pending.context.stream_request == *stream_request
		}) {
			match event {
				screen::Event::Created {
					rtc_server,
					rtc_channel,
				} => {
					pending.rtc = Some((*rtc_server, *rtc_channel));
				}
				screen::Event::Server { token, endpoint } => {
					match (token.take(), endpoint.take()) {
						(Some(token), Some(endpoint)) => pending.server = Some((token, endpoint)),
						// A null endpoint means Discord is still allocating the stream server.
						(_, None) => pending.server = None,
						(None, Some(_)) => {
							pending.ended = Some("Discord omitted the stream connection token");
						}
					}
				}
				screen::Event::Deleted { reason } => {
					pending.ended = Some(reason.unwrap_or("The stream ended"));
				}
				screen::Event::Failed(message) => pending.ended = Some(message),
			}
			return;
		}
		if let Some(live) = self.lives.iter_mut().find(|live| {
			live.context
				.matches(state.generation, *channel, *request, *streamer)
				&& live.context.stream_request == *stream_request
		}) {
			match event {
				screen::Event::Deleted { reason } => {
					live.ended = Some(reason.unwrap_or("The stream ended"));
				}
				screen::Event::Failed(message) => live.ended = Some(message),
				_ => {}
			}
		}
	}
	pub fn poll(
		&mut self,
		runtime: &Runtime,
		state: &mut State,
		ui: &mut ui::MessagingUi,
		ctx: &egui::Context,
		call: Option<super::screen::Call<'_>>,
		audio: Option<std::sync::mpsc::SyncSender<discord_voice::Frame>>,
	) -> Option<Command> {
		// Every explicitly watched stream of the connected call, oldest first. Empty
		// while demoing or reconnecting, so stale sessions end below and resume after.
		let wanted: Vec<(u64, Id, u64, Id)> = if state.demo {
			Vec::new()
		} else {
			state
				.voice
				.active
				.as_ref()
				.filter(|active| {
					matches!(
						active.phase,
						voice::Phase::Connected | voice::Phase::Waiting
					)
				})
				.map(|active| {
					active
						.watching_all()
						.iter()
						.map(|streamer| {
							(state.generation, active.channel, active.request, *streamer)
						})
						.collect()
				})
				.unwrap_or_default()
		};
		// Sessions nobody wants anymore (unwatched, hung up, reconnecting) end here.
		// Their teardown commands queue in the outbox, one draining per poll.
		let mut index = 0;
		while index < self.pending.len() {
			let context = self.pending[index].context;
			if wanted
				.iter()
				.any(|(generation, channel, request, streamer)| {
					context.matches(*generation, *channel, *request, *streamer)
				}) {
				index += 1;
			} else {
				let pending = self.pending.remove(index);
				self.outbox.push_back(stop_command(pending.context));
				platform::diagnostics::info(&format!(
					"watch: dropped pending view of {}",
					pending.context.streamer.0
				));
			}
		}
		let mut index = 0;
		while index < self.lives.len() {
			let context = self.lives[index].context;
			if wanted
				.iter()
				.any(|(generation, channel, request, streamer)| {
					context.matches(*generation, *channel, *request, *streamer)
				}) {
				index += 1;
			} else {
				let live = self.lives.remove(index);
				live.task.abort();
				self.outbox.push_back(stop_command(live.context));
				platform::diagnostics::info(&format!(
					"watch: stopped view of {} after {} ms",
					live.context.streamer.0,
					live.requested_at.elapsed().as_millis()
				));
			}
		}
		if state.voice.active.is_none() {
			self.notice = "";
		}
		if state.demo {
			ui.voice_stream_status = if wanted.is_empty() {
				""
			} else {
				"Offline preview · no stream is received"
			};
			return self.drain(ctx);
		}
		// One view per wanted stream: negotiations run side by side, each with its
		// own gateway slot. Starts queue in the outbox behind any pending stops.
		for (generation, channel, request, streamer) in &wanted {
			let known = self.pending.iter().any(|pending| {
				pending
					.context
					.matches(*generation, *channel, *request, *streamer)
			}) || self.lives.iter().any(|live| {
				live.context
					.matches(*generation, *channel, *request, *streamer)
			});
			if known {
				continue;
			}
			let Some(call) = call.as_ref().filter(|call| {
				call.generation == *generation
					&& call.channel == *channel
					&& call.request == *request
			}) else {
				ui.voice_stream_status = "Connect the call before watching a stream";
				return self.drain(ctx);
			};
			self.sequence = self.sequence.wrapping_add(1);
			let context = Context {
				generation: *generation,
				channel: *channel,
				request: *request,
				stream_request: self.sequence,
				streamer: *streamer,
			};
			self.pending.push(Pending {
				context,
				user: call.user,
				peer: call.peer,
				session: Zeroizing::new(call.session.to_owned()),
				identity: call.identity.clone(),
				audio: audio.clone(),
				rtc: None,
				server: None,
				started: Instant::now(),
				ended: None,
			});
			self.status = "Requesting the stream…";
			self.notice = "";
			platform::diagnostics::info(&format!("watch: stream {} requested", streamer.0));
			self.outbox
				.push_back(Command::Voice(voice::Command::WatchStream {
					channel: *channel,
					request: *request,
					stream_request: context.stream_request,
					streamer: *streamer,
				}));
		}
		for pending in &mut self.pending {
			if pending.ended.is_some() {
				continue;
			}
			if pending.started.elapsed() >= SIGNAL_TIMEOUT {
				pending.ended = Some("Discord did not provide the stream connection; try again");
				ctx.request_repaint();
			} else {
				ctx.request_repaint_after(SIGNAL_TIMEOUT.saturating_sub(pending.started.elapsed()));
			}
		}
		let mut index = 0;
		while index < self.pending.len() {
			if self.pending[index].ended.is_some()
				|| self.pending[index].rtc.is_none()
				|| self.pending[index].server.is_none()
			{
				index += 1;
				continue;
			}
			let pending = self.pending.remove(index);
			let context = pending.context;
			platform::diagnostics::info(&format!(
				"watch: negotiation with {} complete after {} ms",
				context.streamer.0,
				pending.started.elapsed().as_millis()
			));
			if let Err(error) = self.start(runtime, pending, ctx) {
				state.unwatch_stream(context.streamer);
				self.outbox.push_back(stop_command(context));
				self.notice = error;
				platform::diagnostics::warn(&format!(
					"watch: view of {} failed to start: {error}",
					context.streamer.0
				));
			}
		}
		for live in &mut self.lives {
			match *live.events.borrow_and_update() {
				Some(Notice::Status(status)) => live.status = status,
				Some(Notice::Failed(error)) => live.ended = Some(error),
				None => {}
			}
			if live.task.is_finished() && live.ended.is_none() {
				live.ended = Some("The stream connection ended");
			}
		}
		// Views the service ended (or that failed) leave the set one by one.
		let mut index = 0;
		while index < self.pending.len() {
			if let Some(message) = self.pending[index].ended {
				let pending = self.pending.remove(index);
				state.unwatch_stream(pending.context.streamer);
				self.outbox.push_back(stop_command(pending.context));
				self.notice = message;
				platform::diagnostics::warn(&format!(
					"watch: pending view of {} ended: {message}",
					pending.context.streamer.0
				));
			} else {
				index += 1;
			}
		}
		let mut index = 0;
		while index < self.lives.len() {
			if let Some(message) = self.lives[index].ended {
				let live = self.lives.remove(index);
				live.task.abort();
				state.unwatch_stream(live.context.streamer);
				self.outbox.push_back(stop_command(live.context));
				self.status = message;
				self.notice = message;
				platform::diagnostics::warn(&format!(
					"watch: view of {} ended after {} ms: {message}",
					live.context.streamer.0,
					live.requested_at.elapsed().as_millis()
				));
			} else {
				index += 1;
			}
		}
		// The set may have shrunk above; the grid below follows what is wanted now.
		let wanted: Vec<(u64, Id, u64, Id)> = state
			.voice
			.active
			.as_ref()
			.filter(|active| {
				matches!(
					active.phase,
					voice::Phase::Connected | voice::Phase::Waiting
				)
			})
			.map(|active| {
				active
					.watching_all()
					.iter()
					.map(|streamer| (state.generation, active.channel, active.request, *streamer))
					.collect()
			})
			.unwrap_or_default();
		let primary = wanted.last().copied();
		let minimized = ctx.input(|input| input.viewport().minimized.unwrap_or(false));
		// Streams nobody can see (another channel open, or the window minimized) are
		// not uploaded to the GPU: their frames are dropped here and the next visible
		// ones are shown. The decoder threads read the same flag before waking the UI,
		// so hidden frames never schedule full-window redraws either.
		let visible = !self.lives.is_empty()
			&& state.voice.active.as_ref().is_some_and(|active| {
				stream_visible(
					ui.voice_stream_pip,
					state.selected,
					active.channel,
					minimized,
				)
			});
		self.visible.store(visible, Ordering::Release);
		for live in &mut self.lives {
			let image = live.frames.try_lock().ok().and_then(|mut slot| slot.take());
			if let Some(image) = image.filter(|_| visible) {
				let streamer = live.context.streamer;
				if let Some((_, texture)) = ui
					.voice_stream_views
					.iter_mut()
					.find(|(id, _)| *id == streamer)
				{
					texture.set(image, egui::TextureOptions::LINEAR);
				} else {
					ui.voice_stream_views.push((
						streamer,
						ctx.load_texture(
							format!("remote-stream-{}", streamer.0),
							image,
							egui::TextureOptions::LINEAR,
						),
					));
				}
				live.status = "Watching the stream";
				if primary.map(|(_, _, _, primary)| primary) == Some(streamer) {
					if !live.first_frame_logged {
						live.first_frame_logged = true;
						platform::diagnostics::info(&format!(
							"watch: first frame of {} after {} ms",
							streamer.0,
							live.requested_at.elapsed().as_millis()
						));
					}
					if ui.voice_stream_pip {
						ctx.request_repaint_of(ui::stream_pip_viewport());
					}
				}
			}
		}
		ui.voice_stream_views.retain(|(id, _)| {
			self.pending
				.iter()
				.any(|pending| pending.context.streamer == *id)
				|| self.lives.iter().any(|live| live.context.streamer == *id)
		});
		// The primary tile, PiP and fullscreen share one handle to the primary texture.
		ui.voice_stream_view = primary.and_then(|(_, _, _, streamer)| {
			ui.voice_stream_views
				.iter()
				.find(|(id, _)| *id == streamer)
				.map(|(_, texture)| texture.clone())
		});
		// The primary view drives the shared status line.
		if let Some((_, _, _, streamer)) = primary {
			if self
				.pending
				.iter()
				.any(|pending| pending.context.streamer == streamer)
			{
				self.status = "Requesting the stream…";
			} else if let Some(live) = self
				.lives
				.iter()
				.find(|live| live.context.streamer == streamer)
			{
				self.status = live.status;
			}
		}
		ui.voice_stream_status = if primary.is_some() {
			self.status
		} else {
			self.notice
		};
		self.drain(ctx)
	}
	fn start(
		&mut self,
		runtime: &Runtime,
		mut pending: Pending,
		ctx: &egui::Context,
	) -> Result<(), &'static str> {
		let (rtc_server, rtc_channel) = pending.rtc.take().ok_or("Missing stream RTC identity")?;
		let (token, endpoint) = pending.server.take().ok_or("Missing stream server")?;
		let session = voice::Secret::new(pending.session.to_string())
			.map_err(|_| "Invalid stream voice session")?;
		let credentials = voice::VoiceConnection {
			channel: rtc_channel,
			guild: Some(rtc_server),
			user: pending.user,
			peer: pending.peer,
			session,
			token,
			endpoint,
			request: pending.context.stream_request,
		};
		let frames = Arc::new(Mutex::new(None));
		let slot = frames.clone();
		let wake = ctx.clone();
		let visible = self.visible.clone();
		// A fresh view starts woken: the first poll corrects this flag, while starting
		// hidden would delay the first frame until an unrelated repaint.
		visible.store(true, Ordering::Release);
		let streamer = pending.context.streamer.0;
		let sink: discord_voice::VideoSink = Arc::new(move |frame: discord_voice::RemoteFrame| {
			if frame.user != streamer {
				return;
			}
			if slot
				.lock()
				.is_ok_and(|mut slot| store_frame(&mut slot, frame))
				&& visible.load(Ordering::Acquire)
			{
				wake.request_repaint();
			}
		});
		let (send, events) = watch::channel(None);
		let wake = ctx.clone();
		let identity = pending.identity;
		let audio = pending.audio;
		let task = runtime.spawn(async move {
			let (status_send, status_wake) = (send.clone(), wake.clone());
			let result =
				discord_voice::watch_stream(credentials, identity, sink, audio, move |event| {
					let status = match event {
						Status::Connecting => "Connecting to the stream…",
						Status::Discovering => "Checking the stream network…",
						Status::TransportReady | Status::Securing => "Securing the stream…",
						Status::WaitingForPeer => "Waiting for the streamer…",
						Status::Ready { .. } => "Stream secured · waiting for video",
						// Why a group key transition was refused; logged, no state change.
						Status::Note(reason) => {
							platform::diagnostics::info(&format!("watch key transition: {reason}"));
							return Ok(());
						}
						Status::RemoteAudio
						| Status::Speaking(_)
						| Status::CameraAvailable(_)
						| Status::Ping(_)
						| Status::TransportOnly
						| Status::Resuming { .. }
						| Status::Closed { .. } => {
							return Ok(());
						}
					};
					status_send.send_replace(Some(Notice::Status(status)));
					status_wake.request_repaint();
					Ok(())
				})
				.await;
			if let Err(error) = result {
				send.send_replace(Some(Notice::Failed(error)));
			}
			wake.request_repaint();
		});
		self.status = "Connecting to the stream…";
		self.lives.push(Live {
			context: pending.context,
			task,
			events,
			frames,
			status: "Connecting to the stream…",
			ended: None,
			requested_at: Instant::now(),
			first_frame_logged: false,
		});
		Ok(())
	}
}
impl Drop for Watch {
	fn drop(&mut self) {
		self.stop();
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn frame(width: u32, height: u32, rgba: &[u8]) -> discord_voice::RemoteFrame<'_> {
		discord_voice::RemoteFrame {
			user: 7,
			width,
			height,
			rgba,
		}
	}

	#[test]
	fn stream_visibility_needs_pip_or_the_selected_unminimized_channel() {
		let channel = Id(20);
		assert!(stream_visible(true, None, channel, true));
		assert!(stream_visible(true, Some(Id(21)), channel, true));
		assert!(stream_visible(false, Some(channel), channel, false));
		assert!(!stream_visible(false, Some(channel), channel, true));
		assert!(!stream_visible(false, Some(Id(21)), channel, false));
		assert!(!stream_visible(false, None, channel, false));
	}

	#[test]
	fn hidden_views_start_unwoken_until_poll_marks_them_visible() {
		let watch = Watch::default();
		assert!(!watch.visible.load(Ordering::Acquire));
		watch.visible.store(true, Ordering::Release);
		assert!(watch.visible.load(Ordering::Acquire));
	}

	#[test]
	fn two_watched_streams_queue_one_command_each_and_unwatching_drops_one() {
		let runtime = tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.unwrap();
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		state.demo = false;
		let channel = Id(25);
		state.start_call(channel, false).unwrap();
		let request = state.voice.active.as_ref().unwrap().request;
		let call = state.voice.active.as_mut().unwrap();
		call.phase = voice::Phase::Waiting;
		for user in [Id(3), Id(4)] {
			call.participants.push(voice::Participant {
				user,
				muted: false,
				deafened: false,
				server_muted: false,
				server_deafened: false,
				video: false,
				streaming: true,
			});
		}
		assert!(state.watch_stream(Id(3)).is_some());
		assert!(state.watch_stream(Id(4)).is_some());
		let mut ui = ui::MessagingUi::default();
		let mut watch = Watch::default();
		let generation = state.generation;
		let media = || crate::screen::Call {
			generation,
			channel,
			request,
			user: Id(1),
			peer: None,
			session: "synthetic-session",
			identity: discord_voice::Identity::generate(),
		};
		let watch_command = |watch: &mut Watch, state: &mut State, ui: &mut ui::MessagingUi| {
			watch.poll(&runtime, state, ui, &ctx, Some(media()), None)
		};
		// One gateway command per poll: the negotiations queue behind each other.
		let Some(Command::Voice(voice::Command::WatchStream {
			streamer,
			stream_request: first,
			..
		})) = watch_command(&mut watch, &mut state, &mut ui)
		else {
			panic!("first stream negotiates");
		};
		assert_eq!(streamer, Id(3));
		let Some(Command::Voice(voice::Command::WatchStream { streamer, .. })) =
			watch_command(&mut watch, &mut state, &mut ui)
		else {
			panic!("second stream negotiates");
		};
		assert_eq!(streamer, Id(4));
		assert_eq!(watch.pending.len(), 2);
		assert!(watch.outbox.is_empty());
		// A route change preserves intent, releases both old views, and starts fresh IDs.
		watch.route_changed();
		assert_eq!(state.voice.active.as_ref().unwrap().watching_all().len(), 2);
		assert!(
			matches!(watch_command(&mut watch, &mut state, &mut ui), Some(Command::Voice(voice::Command::StopWatching { stream_request, .. })) if stream_request == first)
		);
		assert!(matches!(
			watch_command(&mut watch, &mut state, &mut ui),
			Some(Command::Voice(voice::Command::StopWatching { .. }))
		));
		let Some(Command::Voice(voice::Command::WatchStream {
			stream_request: first,
			streamer: Id(3),
			..
		})) = watch_command(&mut watch, &mut state, &mut ui)
		else {
			panic!("first view resumes");
		};
		assert!(matches!(
			watch_command(&mut watch, &mut state, &mut ui),
			Some(Command::Voice(voice::Command::WatchStream {
				streamer: Id(4),
				..
			}))
		));
		assert!(watch.outbox.is_empty());
		// Forgetting one stream tears down only its negotiation.
		state.unwatch_stream(Id(3));
		let Some(Command::Voice(voice::Command::StopWatching { stream_request, .. })) =
			watch_command(&mut watch, &mut state, &mut ui)
		else {
			panic!("dropped view is released");
		};
		assert_eq!(stream_request, first);
		assert_eq!(watch.pending.len(), 1);
		assert!(watch.outbox.is_empty());
	}

	#[test]
	fn repeated_route_changes_cancel_unsent_starts_instead_of_growing_the_outbox() {
		let mut watch = Watch::default();
		let identity = Identity::generate();
		for attempt in 1..=100 {
			for streamer in 1..=4 {
				let context = Context {
					generation: 0,
					channel: Id(25),
					request: 1,
					stream_request: attempt * 4 + streamer,
					streamer: Id(streamer),
				};
				watch.pending.push(Pending {
					context,
					user: Id(10),
					peer: None,
					session: Zeroizing::new("synthetic".into()),
					identity: identity.clone(),
					audio: None,
					rtc: None,
					server: None,
					started: Instant::now(),
					ended: None,
				});
				watch
					.outbox
					.push_back(Command::Voice(voice::Command::WatchStream {
						channel: context.channel,
						request: context.request,
						stream_request: context.stream_request,
						streamer: context.streamer,
					}));
			}
			watch.route_changed();
			assert!(watch.pending.is_empty());
			assert!(
				watch.outbox.is_empty(),
				"unsent requests require no network teardown"
			);
		}
	}

	#[test]
	fn watch_reuses_dropped_frames_and_delivers_only_updates() {
		let mut picture = None;
		assert!(picture.take().is_none());
		assert!(store_frame(
			&mut picture,
			frame(2, 1, &[1, 2, 3, 255, 4, 5, 6, 128])
		));
		let pixels = picture.as_ref().unwrap().pixels.as_ptr();
		for _ in 0..20 {
			assert!(store_frame(
				&mut picture,
				frame(2, 1, &[9, 8, 7, 255, 6, 5, 4, 128])
			));
			assert_eq!(picture.as_ref().unwrap().pixels.as_ptr(), pixels);
		}
		let latest = picture.take().unwrap();
		assert_eq!(
			latest.pixels,
			egui::ColorImage::from_rgba_unmultiplied([2, 1], &[9, 8, 7, 255, 6, 5, 4, 128]).pixels
		);
		assert!(picture.take().is_none());
		picture = Some(latest);
		assert!(store_frame(&mut picture, frame(1, 1, &[3, 2, 1, 0])));
		assert_eq!(picture.as_ref().unwrap().pixels.as_ptr(), pixels);
		let latest = picture.take().unwrap();
		assert_eq!(latest.size, [1, 1]);
		assert_eq!(latest.source_size, egui::vec2(1.0, 1.0));
		assert_eq!(latest.pixels, vec![egui::Color32::TRANSPARENT]);
	}

	#[test]
	fn watch_preserves_pixels_owned_by_pending_egui_uploads() {
		let mut picture = None;
		let ctx = egui::Context::default();
		let first = [1, 2, 3, 255, 4, 5, 6, 128];
		assert!(store_frame(&mut picture, frame(2, 1, &first)));
		// Discard Context's initial font texture allocation so only this upload is inspected.
		ctx.tex_manager().write().take_delta().clear();
		let mut texture = ctx.load_texture(
			"synthetic-watch",
			picture.take().unwrap(),
			egui::TextureOptions::LINEAR,
		);
		assert!(picture.is_none());
		assert!(store_frame(&mut picture, frame(1, 1, &[10, 20, 30, 255])));
		let mut delta = ctx.tex_manager().write().take_delta();
		assert_eq!(delta.set.len(), 1);
		let uploads = &delta.set[&texture.id()];
		assert_eq!(uploads.len(), 1);
		assert!(uploads[0].is_whole());
		let egui::ImageData::Color(pending) = &uploads[0].image;
		assert_eq!(pending.size, [2, 1]);
		assert_eq!(
			pending.pixels,
			egui::ColorImage::from_rgba_unmultiplied([2, 1], &first).pixels
		);
		assert_eq!(picture.as_ref().unwrap().size, [1, 1]);
		delta.clear();
		texture.set(picture.take().unwrap(), egui::TextureOptions::LINEAR);
		assert!(store_frame(&mut picture, frame(2, 1, &first)));
		let mut updated = ctx.tex_manager().write().take_delta();
		assert_eq!(updated.set.len(), 1);
		let uploads = &updated.set[&texture.id()];
		assert_eq!(uploads.len(), 1);
		assert!(uploads[0].is_whole());
		let egui::ImageData::Color(pending) = &uploads[0].image;
		assert_eq!(pending.size, [1, 1]);
		assert_eq!(pending.pixels, vec![egui::Color32::from_rgb(10, 20, 30)]);
		updated.clear();
		drop(texture);
		ctx.tex_manager().write().take_delta().clear();
	}

	#[test]
	fn watch_invalid_frames_preserve_the_last_valid_picture() {
		let mut picture = None;
		assert!(store_frame(&mut picture, frame(1, 1, &[1, 2, 3, 255])));
		let pixels = picture.as_ref().unwrap().pixels.as_ptr();
		for invalid in [
			frame(0, 1, &[]),
			frame(1, 0, &[]),
			frame(1, 1, &[0; 3]),
			frame(u32::MAX, u32::MAX, &[]),
		] {
			assert!(!store_frame(&mut picture, invalid));
			assert_eq!(picture.as_ref().unwrap().pixels.as_ptr(), pixels);
			assert_eq!(picture.as_ref().unwrap().size, [1, 1]);
		}
		assert!(store_frame(&mut picture, frame(3, 1, &[0; 12])));
		assert_eq!(picture.take().unwrap().pixels.len(), 3);
	}

	#[test]
	fn watch_releases_large_pixel_capacity_after_a_resolution_drop() {
		let mut picture = None;
		let rgba = vec![255; 1920 * 1080 * 4];
		assert!(store_frame(&mut picture, frame(1920, 1080, &rgba)));
		assert_eq!(picture.as_ref().unwrap().pixels.len(), 1920 * 1080);
		assert!(store_frame(
			&mut picture,
			frame(16, 16, &[255; 16 * 16 * 4])
		));
		let latest = picture.take().unwrap();
		assert_eq!(latest.size, [16, 16]);
		assert_eq!(latest.pixels.len(), 16 * 16);
		assert!(latest.pixels.capacity() <= 16 * 16 * 4);
	}

	#[test]
	fn watch_resolution_growth_does_not_double_the_pixel_capacity() {
		let mut picture = None;
		let rgba = vec![255; 1920 * 1080 * 4];
		assert!(store_frame(
			&mut picture,
			frame(1600, 900, &rgba[..1600 * 900 * 4])
		));
		assert!(store_frame(&mut picture, frame(1920, 1080, &rgba)));
		let latest = picture.take().unwrap();
		assert_eq!(latest.size, [1920, 1080]);
		assert_eq!(latest.pixels.len(), 1920 * 1080);
		assert!(latest.pixels.capacity() <= 1920 * 1080);
	}

	#[test]
	fn watch_reused_pixels_match_egui_across_scratch_boundaries() {
		let mut picture = None;
		let mut rgba = vec![0; 129 * 129 * 4];
		assert!(store_frame(&mut picture, frame(129, 129, &rgba)));
		let pixels = picture.as_ref().unwrap().pixels.as_ptr();
		for (index, pixel) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
			pixel.copy_from_slice(&[index as u8, 70, 210, [255, 128, 0, 32][index % 4]]);
		}
		assert!(store_frame(&mut picture, frame(129, 129, &rgba)));
		let latest = picture.take().unwrap();
		assert_eq!(latest.pixels.as_ptr(), pixels);
		assert_eq!(
			latest.pixels,
			egui::ColorImage::from_rgba_unmultiplied([129, 129], &rgba).pixels
		);
	}

	/// Run this test executable directly under `/usr/bin/time -l` for process peak RSS.
	/// Set NIVRA_WATCH_FRAME_LEGACY=1 for the original allocation-per-frame path.
	/// Set NIVRA_WATCH_FRAME_UPLOAD_EVERY=3 to include a pending upload every third frame.
	/// Both modes include the same synthetic 1080p input; no device or transport starts.
	#[test]
	#[ignore = "synthetic release CPU/RSS workload; run with --release --ignored --nocapture"]
	fn watch_frame_memory_workload() {
		let legacy = std::env::var_os("NIVRA_WATCH_FRAME_LEGACY").is_some();
		let uploads_every = std::env::var("NIVRA_WATCH_FRAME_UPLOAD_EVERY")
			.ok()
			.map(|text| {
				text.parse::<usize>()
					.expect("upload interval must be an integer")
			})
			.unwrap_or(0);
		let rgba = vec![255; 1920 * 1080 * 4];
		let mut samples = Vec::new();
		for run in 0..6 {
			let mut old = None;
			let mut picture = None;
			let mut uploading = None;
			let started = Instant::now();
			for index in 0..120 {
				if legacy {
					old = Some(egui::ColorImage::from_rgba_unmultiplied(
						[1920, 1080],
						&rgba,
					));
				} else {
					assert!(store_frame(&mut picture, frame(1920, 1080, &rgba)));
				}
				if uploads_every > 0 && index % uploads_every == 0 {
					uploading = if legacy { old.take() } else { picture.take() };
				} else {
					uploading = None;
				}
				std::hint::black_box((&old, &picture, &uploading));
			}
			std::hint::black_box(uploading);
			if run > 0 {
				samples.push(started.elapsed().as_secs_f64() * 1000.0);
			}
		}
		samples.sort_by(f64::total_cmp);
		println!(
			"watch_frame_memory_workload legacy={legacy} frames=120 uploads_every={uploads_every} samples_ms={samples:?} median_ms={:.3} frame_bytes={}",
			samples[2],
			rgba.len()
		);
	}
}
