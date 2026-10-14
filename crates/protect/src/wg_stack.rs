//! Spike, milestone 1: a TCP connection runs inside a WireGuard tunnel, in process.
//! Two smoltcp stacks exchange TCP segments through boringtun sessions over an in-memory
//! datagram link. No sockets, no interface, no administrator. Test-only until the UDP
//! transport, the SOCKS5 bridge and route integration exist.
#[cfg(test)]
mod tests {
	use boringtun::noise::{Tunn, TunnResult};
	use boringtun::x25519::{PublicKey, StaticSecret};
	use smoltcp::iface::{Config, Interface, SocketSet};
	use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
	use smoltcp::socket::tcp;
	use smoltcp::time::Instant;
	use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr};
	use std::collections::VecDeque;

	const MTU: usize = 1420;
	const PORT: u16 = 9000;
	/// Virtual clock steps before the test gives up. Each step is one millisecond.
	const MAX_STEPS: i64 = 20_000;

	/// The IP packets between one stack and its tunnel. Nothing touches hardware.
	#[derive(Default)]
	struct Pipe {
		/// Decapsulated IP packets waiting to be read by the stack.
		to_stack: VecDeque<Vec<u8>>,
		/// IP packets the stack produced and the tunnel has not encapsulated yet.
		from_stack: VecDeque<Vec<u8>>,
	}

	struct Rx(Vec<u8>);

	impl RxToken for Rx {
		fn consume<R, F>(self, f: F) -> R
		where
			F: FnOnce(&[u8]) -> R,
		{
			f(&self.0)
		}
	}

	struct Tx<'a>(&'a mut VecDeque<Vec<u8>>);

	impl TxToken for Tx<'_> {
		fn consume<R, F>(self, len: usize, f: F) -> R
		where
			F: FnOnce(&mut [u8]) -> R,
		{
			let mut packet = vec![0u8; len];
			let result = f(&mut packet);
			self.0.push_back(packet);
			result
		}
	}

	impl Device for Pipe {
		type RxToken<'a> = Rx;
		type TxToken<'a> = Tx<'a>;

		fn receive(&mut self, _: Instant) -> Option<(Rx, Tx<'_>)> {
			let packet = self.to_stack.pop_front()?;
			Some((Rx(packet), Tx(&mut self.from_stack)))
		}

		fn transmit(&mut self, _: Instant) -> Option<Tx<'_>> {
			Some(Tx(&mut self.from_stack))
		}

		fn capabilities(&self) -> DeviceCapabilities {
			let mut caps = DeviceCapabilities::default();
			caps.medium = Medium::Ip;
			caps.max_transmission_unit = MTU;
			caps
		}
	}

	/// What one call into boringtun produced, copied out so the buffer can be reused.
	enum Produced {
		Network(Vec<u8>),
		Tunnel(Vec<u8>),
		Nothing,
	}

	fn owned(result: TunnResult<'_>) -> Produced {
		match result {
			TunnResult::WriteToNetwork(bytes) => Produced::Network(bytes.to_vec()),
			TunnResult::WriteToTunnelV4(bytes, _) | TunnResult::WriteToTunnelV6(bytes, _) => {
				Produced::Tunnel(bytes.to_vec())
			}
			TunnResult::Done | TunnResult::Err(_) => Produced::Nothing,
		}
	}

	/// One end of the tunnel: a WireGuard peer, its IP stack and its datagram outbox.
	struct Node {
		tunn: Tunn,
		pipe: Pipe,
		iface: Interface,
		sockets: SocketSet<'static>,
		/// Encrypted UDP datagrams to deliver to the other end.
		outbox: VecDeque<Vec<u8>>,
	}

	impl Node {
		fn new(secret: StaticSecret, peer: PublicKey, address: [u8; 4], index: u32) -> Self {
			let tunn = Tunn::new(secret, peer, None, None, index, None);
			let mut pipe = Pipe::default();
			let mut iface =
				Interface::new(Config::new(HardwareAddress::Ip), &mut pipe, Instant::ZERO);
			iface.update_ip_addrs(|addrs| {
				addrs
					.push(IpCidr::new(
						IpAddress::v4(address[0], address[1], address[2], address[3]),
						24,
					))
					.expect("address slot");
			});
			Self {
				tunn,
				pipe,
				iface,
				sockets: SocketSet::new(vec![]),
				outbox: VecDeque::new(),
			}
		}

		/// Encapsulates every IP packet the stack produced, queueing UDP datagrams.
		fn send_stack_packets(&mut self) {
			while let Some(packet) = self.pipe.from_stack.pop_front() {
				let mut buffer = vec![0u8; packet.len() + 256];
				if let Produced::Network(datagram) =
					owned(self.tunn.encapsulate(&packet, &mut buffer))
				{
					self.outbox.push_back(datagram);
				}
			}
		}

		/// Runs one stack poll, then encapsulates what it sent.
		fn poll(&mut self, now: Instant) {
			self.iface.poll(now, &mut self.pipe, &mut self.sockets);
			self.send_stack_packets();
		}

		/// Handles one UDP datagram from the other end. Handshake replies, keepalives and
		/// queued packets released by a finished handshake all come back out of boringtun.
		fn ingest(&mut self, datagram: &[u8]) {
			let mut buffer = vec![0u8; 65_536];
			let mut input: &[u8] = datagram;
			// Bounded: each round either drains a queued packet or ends the loop.
			for _ in 0..1_000 {
				match owned(self.tunn.decapsulate(None, input, &mut buffer)) {
					Produced::Network(reply) => self.outbox.push_back(reply),
					Produced::Tunnel(packet) => self.pipe.to_stack.push_back(packet),
					Produced::Nothing => return,
				}
				// An empty datagram asks boringtun for the next queued packet.
				input = &[];
			}
		}
	}

	fn secret(tag: u8) -> StaticSecret {
		let mut bytes = [tag; 32];
		bytes[0] = tag.wrapping_mul(7);
		StaticSecret::from(bytes)
	}

	#[test]
	fn tcp_connection_runs_inside_the_wireguard_tunnel() {
		let client_key = secret(11);
		let server_key = secret(22);
		let client_public = PublicKey::from(&client_key);
		let server_public = PublicKey::from(&server_key);
		let mut client = Node::new(client_key, server_public, [10, 0, 0, 1], 1);
		let mut server = Node::new(server_key, client_public, [10, 0, 0, 2], 2);

		let listener = server.sockets.add(tcp::Socket::new(
			tcp::SocketBuffer::new(vec![0; 4096]),
			tcp::SocketBuffer::new(vec![0; 4096]),
		));
		server
			.sockets
			.get_mut::<tcp::Socket>(listener)
			.listen(PORT)
			.expect("listen");

		let connection = client.sockets.add(tcp::Socket::new(
			tcp::SocketBuffer::new(vec![0; 4096]),
			tcp::SocketBuffer::new(vec![0; 4096]),
		));
		{
			let context = client.iface.context();
			client
				.sockets
				.get_mut::<tcp::Socket>(connection)
				.connect(context, (IpAddress::v4(10, 0, 0, 2), PORT), 49_152)
				.expect("connect");
		}

		let mut sent = false;
		let mut echoed = Vec::new();
		for step in 0..MAX_STEPS {
			let now = Instant::from_millis(step);
			client.poll(now);
			server.poll(now);

			// Deliver every datagram to the other end. Replies come back on the next step.
			let to_server: Vec<Vec<u8>> = client.outbox.drain(..).collect();
			for datagram in to_server {
				server.ingest(&datagram);
			}
			let to_client: Vec<Vec<u8>> = server.outbox.drain(..).collect();
			for datagram in to_client {
				client.ingest(&datagram);
			}

			// The server echoes whatever it receives.
			let server_socket = server.sockets.get_mut::<tcp::Socket>(listener);
			if server_socket.can_recv() {
				let mut buffer = [0u8; 256];
				if let Ok(read) = server_socket.recv_slice(&mut buffer) {
					let _ = server_socket.send_slice(&buffer[..read]);
				}
			}

			let client_socket = client.sockets.get_mut::<tcp::Socket>(connection);
			if !sent && client_socket.can_send() {
				client_socket.send_slice(b"ping").expect("send");
				sent = true;
			}
			if client_socket.can_recv() {
				let mut buffer = [0u8; 256];
				if let Ok(read) = client_socket.recv_slice(&mut buffer) {
					echoed.extend_from_slice(&buffer[..read]);
				}
			}
			if echoed == b"ping" {
				return;
			}
		}
		panic!(
			"TCP echo did not complete through the tunnel; client echoed {:?}",
			String::from_utf8_lossy(&echoed)
		);
	}
}
