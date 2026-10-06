//! Two-client voice loopback over a fake gateway/UDP server; DAVE on.
//! Tone from each fake microphone must arrive decoded on the other side.
//! No Discord connection, capture device, or speaker.
use super::*;
use client_core::voice::Secret;
use model::Id;
use tokio::net::TcpListener;

type TestSocket = WebSocketStream<TcpStream>;

async fn event(ws: &mut TestSocket, value: Value) {
	ws.send(Message::Text(value.to_string().into()))
		.await
		.unwrap();
}

async fn message(ws: &mut TestSocket) -> Message {
	loop {
		let message = ws.next().await.unwrap().unwrap();
		if let Message::Text(text) = &message {
			let value: Value = serde_json::from_str(text).unwrap();
			if value["op"] == 3 {
				event(ws, json!({"op":6,"d":{"t":value["d"]["t"]}})).await;
				continue;
			}
			// Speaking notifications and pings are traffic, not handshake steps.
			if value["op"] == 5 {
				continue;
			}
		}
		return message;
	}
}

async fn voice_connect(
	listener: &TcpListener,
	delivery: &crate::test_mls::Delivery,
	user: u64,
	ssrc: u32,
) -> (TestSocket, UdpSocket, SocketAddr, Vec<u8>) {
	let (tcp, _) = listener.accept().await.unwrap();
	let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
	let identify = message(&mut ws).await;
	let identify: Value = serde_json::from_str(identify.to_text().unwrap()).unwrap();
	assert_eq!(identify["op"], 0);
	assert_eq!(identify["d"]["user_id"], user.to_string());
	let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
	event(&mut ws, json!({"op":8,"d":{"heartbeat_interval":5000}})).await;
	event(
		&mut ws,
		json!({"op":2,"d":{"ssrc":ssrc,"ip":"127.0.0.1","port":udp.local_addr().unwrap().port(),"modes":[MODE]}}),
	)
	.await;
	let mut probe = [0; 74];
	let (length, client) = udp.recv_from(&mut probe).await.unwrap();
	assert_eq!(length, 74);
	probe[..4].copy_from_slice(&[0, 2, 0, 70]);
	probe[8..17].copy_from_slice(b"127.0.0.1");
	probe[72..].copy_from_slice(&client.port().to_be_bytes());
	udp.send_to(&probe, client).await.unwrap();
	let selected = message(&mut ws).await;
	let selected: Value = serde_json::from_str(selected.to_text().unwrap()).unwrap();
	assert_eq!(selected["op"], 1);
	ws.send(Message::Binary(
		[&[0, 1, 25], delivery.external.as_slice()].concat().into(),
	))
	.await
	.unwrap();
	event(&mut ws, json!({"op":11,"d":{"user_ids":["1","2"]}})).await;
	event(
		&mut ws,
		json!({"op":4,"d":{"mode":MODE,"secret_key":vec![7;32],"dave_protocol_version":1,"video_codec":"H264"}}),
	)
	.await;
	let package = match message(&mut ws).await {
		Message::Binary(package) => package.to_vec(),
		other => panic!("Expected voice key package, got {other:?}"),
	};
	assert_eq!(package[0], 26);
	(ws, udp, client, package)
}

fn credentials(user: u64, peer: u64) -> VoiceConnection {
	VoiceConnection {
		channel: Id(3),
		guild: Some(Id(4)),
		user: Id(user),
		peer: Some(Id(peer)),
		session: Secret::new("synthetic-session".into()).unwrap(),
		token: Secret::new("synthetic-token".into()).unwrap(),
		endpoint: "voice.discord.media".into(),
		request: 1,
	}
}

/// 440 Hz vs 660 Hz energy of one decoded frame batch.
fn tone_energy(frames: &[Frame], freq: f32) -> (f64, f64) {
	let (mut target, mut total) = (0.0f64, 0.0f64);
	for frame in frames {
		for (i, sample) in frame.iter().enumerate() {
			let s = f64::from(*sample);
			total += s * s;
			let reference =
				(2.0 * std::f64::consts::PI * f64::from(freq) * i as f64 / 48_000.0).sin();
			target += s * reference;
		}
	}
	(target * target, total)
}

fn tone(freq_hz: f32, phase: &mut f32) -> Frame {
	let mut frame = [0.0f32; 960];
	for sample in frame.iter_mut() {
		*sample = (*phase * 2.0 * std::f32::consts::PI).sin() * 0.5;
		*phase = (*phase + freq_hz / 48_000.0) % 1.0;
	}
	frame
}

#[tokio::test]
async fn two_voice_clients_exchange_decoded_audio_with_dave() {
	let result = timeout(Duration::from_secs(50), exchange(false)).await;
	result.expect("Synthetic voice exchange timed out");
}

#[tokio::test]
async fn ten_second_udp_blackout_recovers_without_rejoin() {
	let result = timeout(Duration::from_secs(55), exchange(true)).await;
	result.expect("Synthetic voice blackout recovery timed out");
}

async fn exchange(blackout: bool) {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let url = format!("ws://{}", listener.local_addr().unwrap());
	let delivery = crate::test_mls::Delivery::new();
	let (capture_a_tx, capture_a_rx) = std::sync::mpsc::sync_channel(8);
	let (playback_a_tx, playback_a_rx) = std::sync::mpsc::sync_channel(64);
	let (capture_b_tx, capture_b_rx) = std::sync::mpsc::sync_channel(8);
	let (playback_b_tx, playback_b_rx) = std::sync::mpsc::sync_channel(64);
	let (controls_a_tx, controls_a_rx) = tokio::sync::watch::channel(Controls::default());
	let (controls_b_tx, controls_b_rx) = tokio::sync::watch::channel(Controls::default());
	let _keep_controls = (controls_a_tx, controls_b_tx);
	let client_url = url.clone();
	let client_a = tokio::spawn(async move {
		run_inner(
			credentials(1, 2),
			capture_a_rx,
			playback_a_tx,
			controls_a_rx,
			None,
			None,
			None,
			|_| Ok(()),
			Identity::generate(),
			client_url,
			true,
		)
		.await
	});
	let client_b = tokio::spawn(async move {
		run_inner(
			credentials(2, 1),
			capture_b_rx,
			playback_b_tx,
			controls_b_rx,
			None,
			None,
			None,
			|_| Ok(()),
			Identity::generate(),
			url,
			true,
		)
		.await
	});
	let (mut ws_a, udp_a, addr_a, _package_a) = voice_connect(&listener, &delivery, 1, 41).await;
	let (mut ws_b, udp_b, addr_b, package_b) = voice_connect(&listener, &delivery, 2, 42).await;
	// One server group converges both members, mirroring the stream test: the
	// proposal adds B's key package, A commits it, and B joins via the welcome.
	let mut group = Dave::new(1, Some(2), 3).unwrap();
	group
		.session
		.set_external_sender(&delivery.external)
		.unwrap();
	let proposal = delivery.add_proposal(&group, &package_b);
	ws_a.send(Message::Binary(
		[&[0, 2, 27], proposal.as_slice()].concat().into(),
	))
	.await
	.unwrap();
	let committed = message(&mut ws_a).await.into_data();
	let (commit, welcome) = crate::test_mls::Delivery::split(&committed);
	ws_a.send(Message::Binary(
		[&[0, 3, 29, 0, 0], commit.as_slice()].concat().into(),
	))
	.await
	.unwrap();
	ws_b.send(Message::Binary(
		[&[0, 3, 30, 0, 0], welcome.as_slice()].concat().into(),
	))
	.await
	.unwrap();
	// Roster + mixer mapping both directions.
	event(&mut ws_a, json!({"op":11,"d":{"user_ids":["1","2"]}})).await;
	event(&mut ws_b, json!({"op":11,"d":{"user_ids":["1","2"]}})).await;
	event(
		&mut ws_a,
		json!({"op":5,"seq":3,"d":{"user_id":"2","ssrc":42,"speaking":1}}),
	)
	.await;
	event(
		&mut ws_b,
		json!({"op":5,"seq":3,"d":{"user_id":"1","ssrc":41,"speaking":1}}),
	)
	.await;
	// Feed tones: A speaks 440 Hz, B speaks 660 Hz.
	let feed_a = tokio::spawn(async move {
		let mut phase = 0.0f32;
		let mut tick = tokio::time::interval(Duration::from_millis(20));
		tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
		loop {
			tick.tick().await;
			if capture_a_tx.send(tone(450.0, &mut phase)).is_err() {
				break;
			}
		}
	});
	let feed_b = tokio::spawn(async move {
		let mut phase = 0.0f32;
		let mut tick = tokio::time::interval(Duration::from_millis(20));
		tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
		loop {
			tick.tick().await;
			if capture_b_tx.send(tone(650.0, &mut phase)).is_err() {
				break;
			}
		}
	});
	// Relay RTP between the peers until both directions are heard decoded.
	let transport = Encryption::new(&[7; 32]);
	let mut packet_a = [0; MAX_PACKET + 1];
	let mut packet_b = [0; MAX_PACKET + 1];
	let mut heard_a = Vec::new();
	let mut heard_b = Vec::new();
	timeout(Duration::from_secs(50), async {
		let mut drop_until = None;
		loop {
			let dropping = drop_until.is_some_and(|until| Instant::now() < until);
			tokio::select! {
				result = udp_a.recv_from(&mut packet_a) => {
					let (length, from) = result.unwrap();
					assert_eq!(from, addr_a);
					if length == 8 {
						continue;
					}
					if drop_until.is_some_and(|until| Instant::now() < until) {
						continue;
					}
					let rtp = transport.open(&packet_a[..length]).expect("A transport packet");
					assert_eq!(rtp.payload_type, 120);
					udp_b.send_to(&packet_a[..length], addr_b).await.unwrap();
				}
				result = udp_b.recv_from(&mut packet_b) => {
					let (length, from) = result.unwrap();
					assert_eq!(from, addr_b);
					if length == 8 {
						continue;
					}
					if drop_until.is_some_and(|until| Instant::now() < until) {
						continue;
					}
					let rtp = transport.open(&packet_b[..length]).expect("B transport packet");
					assert_eq!(rtp.payload_type, 120);
					udp_a.send_to(&packet_b[..length], addr_a).await.unwrap();
				}
				_ = message(&mut ws_a) => {},
				_ = message(&mut ws_b) => {},
			}
			while let Ok(frame) = playback_a_rx.try_recv() {
				if !dropping {
					heard_a.push(frame);
				}
			}
			while let Ok(frame) = playback_b_rx.try_recv() {
				if !dropping {
					heard_b.push(frame);
				}
			}
			if heard_a.len() >= 60 && heard_b.len() >= 60 {
				if blackout && drop_until.is_none() {
					// Baseline heard both ways; now drop all RTP for 10 s while
					// the gateway heartbeats continue. The call must survive.
					heard_a.clear();
					heard_b.clear();
					drop_until = Some(Instant::now() + Duration::from_secs(10));
				}
				if !blackout || drop_until.is_some_and(|until| Instant::now() >= until) {
					break;
				}
			}
		}
	})
	.await
	.expect("no decoded audio in either direction");
	feed_a.abort();
	feed_b.abort();
	// B must hear A's 440 Hz, A must hear B's 660 Hz.
	for (frames, freq, side) in [(&heard_a, 650.0, "A"), (&heard_b, 450.0, "B")] {
		let (target, total) = tone_energy(frames, freq);
		let (other, _) = tone_energy(frames, if freq == 450.0 { 650.0 } else { 450.0 });
		assert!(
			total > 1.0 && target > 10.0 * other,
			"{side} did not decode the peer tone: target={target} other={other} total={total}"
		);
	}
	if blackout {
		assert!(
			!client_a.is_finished() && !client_b.is_finished(),
			"a 10 s UDP blackout must not end either call"
		);
	}
	client_a.abort();
	client_b.abort();
}

#[test]
fn dave_proposals_must_add_the_peer_never_self() {
	use crate::crypto::Dave;
	let server = crate::test_mls::Delivery::new();
	let mut group = Dave::new(1, Some(2), 3).unwrap();
	group.session.set_external_sender(&server.external).unwrap();
	// Peer-add is the only valid external proposal shape here.
	let mut a = Dave::with_identity(1, Some(2), 3, Identity::generate()).unwrap();
	a.session.set_external_sender(&server.external).unwrap();
	let mut b = Dave::with_identity(2, Some(1), 3, Identity::generate()).unwrap();
	b.session.set_external_sender(&server.external).unwrap();
	let peer_add = server.add_proposal(&group, &b.key_package().unwrap());
	assert!(a.proposals(&peer_add).unwrap().is_some());
	// Self-add misroutes the choreography: the signer's own key is already in the
	// group, so MLS rejects it as a duplicate signature key. A fresh instance
	// proves the rejection (a failed attempt would poison group state).
	let mut a2 = Dave::with_identity(1, Some(2), 3, Identity::generate()).unwrap();
	a2.session.set_external_sender(&server.external).unwrap();
	let self_add = server.add_proposal(&group, &a2.key_package().unwrap());
	assert!(a2.proposals(&self_add).is_err());
}

/// Drives one synthetic voice session to the connected state, closes it with `code` and
/// reports what the transport did: `Ok(())` when it reconnected and sent resume op 7,
/// `Err(reason)` when it ended, with the app-visible reason.
async fn close_behaviour(code: Option<u16>) -> Result<(), &'static str> {
	use tokio_tungstenite::tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let url = format!("ws://{}", listener.local_addr().unwrap());
	let delivery = crate::test_mls::Delivery::new();
	let (_capture_tx, capture_rx) = std::sync::mpsc::sync_channel(8);
	let (playback_tx, _playback_rx) = std::sync::mpsc::sync_channel(8);
	let (_controls_tx, controls_rx) = tokio::sync::watch::channel(Controls::default());
	let (status_tx, mut status_rx) = tokio::sync::mpsc::channel(64);
	let client_url = url.clone();
	let mut client = tokio::spawn(async move {
		run_inner(
			credentials(1, 2),
			capture_rx,
			playback_tx,
			controls_rx,
			None,
			None,
			None,
			move |status| status_tx.try_send(status).map_err(|_| ()),
			Identity::generate(),
			client_url,
			true,
		)
		.await
	});
	let (mut ws, _udp, _addr, _package) = voice_connect(&listener, &delivery, 1, 41).await;
	// The close must arrive after SESSION_DESCRIPTION so resume is actually allowed.
	loop {
		match timeout(Duration::from_secs(8), status_rx.recv()).await {
			// Wait for the last notice of op 4 handling so the close races nothing.
			Ok(Some(Status::Securing)) => break,
			Ok(Some(_)) => {}
			Ok(None) | Err(_) => panic!("client never reached the secured state"),
		}
	}
	if let Some(code) = code {
		ws.send(Message::Close(Some(CloseFrame {
			code: CloseCode::from(code),
			reason: "".into(),
		})))
		.await
		.unwrap();
		// Keep the socket alive until the outcome; dropping it with unread data would send
		// an RST that races the flushed close frame and looks like a network drop.
	} else {
		// Without a close frame this is the RFC's abnormal closure (1006): the socket ends.
		drop(ws);
	}
	match close_disposition(code) {
		CloseDisposition::Resume => {
			// The session survives: a new socket must carry resume op 7.
			let resumed = match timeout(Duration::from_secs(10), listener.accept()).await {
				Ok(accepted) => accepted.unwrap(),
				Err(_) => {
					if let Ok(result) = client.await {
						panic!("resume connection timed out; client ended with {result:?}");
					}
					panic!("resume connection timed out; client is still running");
				}
			};
			let mut resumed = tokio_tungstenite::accept_async(resumed.0).await.unwrap();
			let frame = timeout(Duration::from_secs(5), message(&mut resumed))
				.await
				.expect("resume frame timed out");
			let value: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
			assert_eq!(value["op"], 7, "expected resume op 7: {value}");
			client.abort();
			Ok(())
		}
		_ => {
			let result = match timeout(Duration::from_secs(10), &mut client).await {
				Ok(result) => result.expect("client task panicked"),
				Err(_) => {
					client.abort();
					let mut pending = Vec::new();
					while let Ok(status) = status_rx.try_recv() {
						pending.push(match status {
							Status::Connecting => "connecting",
							Status::Discovering => "discovering",
							Status::TransportReady => "transport-ready",
							Status::CameraAvailable(_) => "camera",
							Status::Securing => "securing",
							Status::WaitingForPeer => "waiting-for-peer",
							Status::Ready { .. } => "ready",
							Status::Ping(_) => "ping",
							Status::RemoteAudio => "remote-audio",
							Status::Speaking(_) => "speaking",
							Status::TransportOnly => "transport-only",
							Status::Resuming { .. } => "resuming",
							Status::Closed { .. } => "closed",
						});
					}
					panic!("terminal close timed out with pending {pending:?}");
				}
			};
			let error = result.expect_err("a terminal close must end the transport");
			if let Some(code) = code {
				assert!(
					error.contains(&code.to_string()),
					"code {code} must be named in {error:?}"
				);
			}
			Err(error)
		}
	}
}

#[tokio::test]
async fn close_codes_drive_resume_or_terminal_end() {
	// 1000/1001 preserve the session; a dropped socket is the 1006 abnormal closure.
	// 4015 is the crashed voice server. Every terminal code names itself.
	for code in [Some(1000), Some(1001), None, Some(4015)] {
		close_behaviour(code).await.expect("resume path");
	}
	for code in [Some(4006), Some(4009), Some(4014), Some(4021), Some(4022)] {
		close_behaviour(code).await.expect_err("terminal path");
	}
}

#[test]
fn close_code_table_matches_the_discord_documentation() {
	assert_eq!(close_disposition(None), CloseDisposition::Resume);
	for code in [1000, 1001, 1006, 4015] {
		assert_eq!(close_disposition(Some(code)), CloseDisposition::Resume);
	}
	for code in [4006, 4009] {
		assert_eq!(
			close_disposition(Some(code)),
			CloseDisposition::SessionExpired
		);
	}
	assert_eq!(
		close_disposition(Some(4014)),
		CloseDisposition::Disconnected
	);
	assert_eq!(close_disposition(Some(4021)), CloseDisposition::RateLimited);
	assert_eq!(close_disposition(Some(4022)), CloseDisposition::Terminated);
}
