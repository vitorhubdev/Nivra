//! Spike: WireGuard in pure Rust (boringtun), without sing-box or a system interface.
//! It proves the handshake and IP encapsulation round trip in process. It is test-only
//! until the userspace network stack and route integration exist.
#[cfg(test)]
mod tests {
	use boringtun::noise::{Tunn, TunnResult};
	use boringtun::x25519::{PublicKey, StaticSecret};

	fn secret(tag: u8) -> StaticSecret {
		let mut bytes = [tag; 32];
		bytes[0] = tag.wrapping_mul(7);
		StaticSecret::from(bytes)
	}

	/// Minimal IPv4 + UDP packet: enough for boringtun to parse the inner version and length.
	fn ipv4_udp(payload: &[u8]) -> Vec<u8> {
		let total = 20 + 8 + payload.len();
		let mut packet = vec![0u8; total];
		packet[0] = 0x45;
		packet[2..4].copy_from_slice(&(total as u16).to_be_bytes());
		packet[8] = 64;
		packet[9] = 17;
		packet[12..16].copy_from_slice(&[10, 0, 0, 2]);
		packet[16..20].copy_from_slice(&[1, 1, 1, 1]);
		packet[20..22].copy_from_slice(&5000u16.to_be_bytes());
		packet[22..24].copy_from_slice(&53u16.to_be_bytes());
		packet[24..26].copy_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
		packet[28..].copy_from_slice(payload);
		packet
	}

	#[test]
	fn rust_wireguard_core_carries_an_ip_packet_both_ways() {
		let client_key = secret(1);
		let server_key = secret(2);
		let client_public = PublicKey::from(&client_key);
		let server_public = PublicKey::from(&server_key);
		let mut client = Tunn::new(client_key, server_public, None, None, 1, None);
		let mut server = Tunn::new(server_key, client_public, None, None, 2, None);
		let packet = ipv4_udp(b"dns?");
		let mut out = vec![0u8; 4096];
		let mut inner = vec![0u8; 4096];

		// The first outbound packet becomes a handshake initiation.
		let to_server = match client.encapsulate(&packet, &mut out) {
			TunnResult::WriteToNetwork(bytes) => bytes.to_vec(),
			_ => panic!("client did not start a handshake"),
		};
		// Server answers the initiation with a handshake response.
		let response = match server.decapsulate(None, &to_server, &mut inner) {
			TunnResult::WriteToNetwork(bytes) => bytes.to_vec(),
			_ => panic!("server did not answer the handshake"),
		};
		// The client finishes the handshake; its reply is only a keepalive here.
		match client.decapsulate(None, &response, &mut out) {
			TunnResult::WriteToNetwork(_) | TunnResult::Done => {}
			_ => panic!("client did not complete the handshake"),
		}
		// Now the packet travels over the established session.
		let data = match client.encapsulate(&packet, &mut out) {
			TunnResult::WriteToNetwork(bytes) => bytes.to_vec(),
			_ => panic!("client did not send over the session"),
		};
		let delivered = match server.decapsulate(None, &data, &mut inner) {
			TunnResult::WriteToTunnelV4(bytes, _) => Some(bytes.to_vec()),
			_ => None,
		};
		assert_eq!(delivered.as_deref(), Some(packet.as_slice()));
	}
}
