//! Sustained WireGuard check for the owner's provider. Ignored by default: it needs a real endpoint.
//! Run with `NIVRA_WG_CONF=<path to .conf>` and
//! `cargo test -p protect --test live_wireguard_sustained -- --ignored --nocapture`.
//! One long-lived UDP association (the path live voice uses) sends a DNS query every 200 ms for
//! `NIVRA_WG_SECONDS` seconds (default 150) and reports the first error or silence, so a tunnel
//! that dies after about a minute shows its exact time. The configuration is never printed.
use model::protect::{Draft, Protocol, Scope};
use std::{
	net::SocketAddr,
	time::{Duration, Instant},
};
use zeroize::Zeroizing;

const DNS: &str = "1.1.1.1:53";

fn query(id: u16) -> Vec<u8> {
	let mut bytes = id.to_be_bytes().to_vec();
	bytes.extend_from_slice(&[0x01, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
	bytes.extend_from_slice(&[
		7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm', 0,
	]);
	bytes.extend_from_slice(&[0, 1, 0, 1]);
	bytes
}

#[tokio::test]
#[ignore = "needs a real WireGuard configuration in NIVRA_WG_CONF"]
async fn live_wireguard_survives_a_long_udp_association() {
	let path = std::env::var("NIVRA_WG_CONF").expect("set NIVRA_WG_CONF to the .conf path");
	let seconds: u64 = std::env::var("NIVRA_WG_SECONDS")
		.ok()
		.and_then(|value| value.parse().ok())
		.unwrap_or(150);
	let text = Zeroizing::new(std::fs::read_to_string(path).expect("read configuration"));
	let draft = Draft {
		protocol: Protocol::WireGuard,
		scope: Scope::Complete,
		wireguard: text,
		..Draft::default()
	};
	// Ultra mode on, printing what the tunnel reports, so the run also shows the monitoring itself.
	protect::set_trace_sink(|scope, message| println!("[{scope}] {message}"));
	protect::set_ultra(true);
	let profile = protect::prepare(&draft).await.expect("prepare route");
	protect::apply(Some(profile), None).expect("apply route");
	let destination: SocketAddr = DNS.parse().unwrap();
	let socket = protect::Datagram::connect(destination, Scope::Complete)
		.await
		.expect("open UDP association");
	let started = Instant::now();
	let (mut sent, mut received, mut longest_gap) = (0u32, 0u32, Duration::ZERO);
	let mut last_reply = started;
	let mut buffer = [0u8; 512];
	let mut tick = tokio::time::interval(Duration::from_millis(200));
	let mut next_report = 10u64;
	while started.elapsed() < Duration::from_secs(seconds) {
		tokio::select! {
			_ = tick.tick() => {
				sent += 1;
				if let Err(error) = socket.send(&query(sent as u16)).await {
					println!("SEND ERROR at {:.1}s: {:?} {error}", started.elapsed().as_secs_f32(), error.kind());
					break;
				}
			}
			result = socket.recv(&mut buffer) => match result {
				Ok(_) => {
					received += 1;
					longest_gap = longest_gap.max(last_reply.elapsed());
					last_reply = Instant::now();
				}
				Err(error) => {
					println!("RECV ERROR at {:.1}s: {:?} {error}", started.elapsed().as_secs_f32(), error.kind());
					break;
				}
			}
		}
		if started.elapsed().as_secs() >= next_report {
			println!(
				"t={}s sent={sent} received={received} silence={:.1}s",
				next_report,
				last_reply.elapsed().as_secs_f32()
			);
			next_report += 10;
		}
	}
	println!(
		"DONE after {:.1}s sent={sent} received={received} longest_gap={:.2}s",
		started.elapsed().as_secs_f32(),
		longest_gap.as_secs_f32()
	);
	assert!(
		started.elapsed() >= Duration::from_secs(seconds),
		"the association ended early"
	);
	assert!(received > sent / 2, "most queries went unanswered");
}
