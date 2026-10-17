//! In-process WireGuard transport. boringtun runs the tunnel over one connected UDP socket and
//! smoltcp runs the guest TCP/IP stack. Nothing here touches system routes, adapters or DNS:
//! protected traffic leaves the process only as encrypted datagrams to the configured endpoint.
//! Every queue, socket buffer and socket count is bounded.
use super::trace;
use boringtun::{
	noise::{Tunn, TunnResult},
	x25519::{PublicKey, StaticSecret},
};
use smoltcp::{
	iface::{Config, Interface, SocketHandle, SocketSet},
	phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken},
	socket::{tcp, udp},
	time::Instant,
	wire::{HardwareAddress, IpAddress, IpCidr, IpEndpoint, IpListenEndpoint, Ipv4Address},
};
use std::{
	collections::VecDeque,
	net::{Ipv4Addr, SocketAddr, SocketAddrV4},
	sync::Arc,
	time::Duration,
};
use tokio::{
	io::DuplexStream,
	net::UdpSocket,
	sync::{Notify, mpsc, oneshot},
	task::JoinHandle,
};

const DEADLINE: Duration = super::DEADLINE;
const TICK: Duration = Duration::from_millis(250);
/// How often ultra diagnostics summarize the tunnel.
const SUMMARY: Duration = Duration::from_secs(5);
/// How often ultra diagnostics look up a name through the tunnel to prove it is alive.
const CANARY: Duration = Duration::from_secs(10);
const CANARY_HOST: &str = "discord.com";
/// Longest the driver sleeps without new work, so timers and retransmissions stay on time.
const MAX_IDLE: Duration = Duration::from_millis(20);
const CHUNK: usize = 16 * 1024;
/// Per-connection bridge depth: at most BRIDGE_CHUNKS × CHUNK bytes wait in each direction.
const BRIDGE_CHUNKS: usize = 32;
const TCP_BUFFER: usize = 64 * 1024;
const MAX_TCP: usize = 128;
const MAX_UDP: usize = 32;
const UDP_SLOTS: usize = 16;
/// Decrypted packets waiting for the stack; beyond this the tunnel drops, as UDP does.
const MAX_QUEUED: usize = 512;
const COMMANDS: usize = 64;
const INBOUND: usize = 256;
const IPV4_UDP_OVERHEAD: usize = 28;
const DNS_SERVER: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(1, 1, 1, 1), 53);
const ENGINE_STOPPED: &str = "PROTECT WireGuard engine stopped";
const SOCKET_LIMIT: &str = "PROTECT connection limit reached";
const OUTSIDE_ALLOWED: &str = "Destination is outside the WireGuard AllowedIPs";

/// One IPv4 network from `AllowedIPs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Cidr {
	network: u32,
	mask: u32,
}

impl Cidr {
	pub(super) fn parse(text: &str) -> Option<Self> {
		let (ip, prefix) = text.trim().split_once('/')?;
		let ip: Ipv4Addr = ip.parse().ok()?;
		let prefix: u8 = prefix.parse().ok()?;
		if prefix > 32 {
			return None;
		}
		let mask = if prefix == 0 {
			0
		} else {
			u32::MAX << (32 - prefix)
		};
		Some(Self {
			network: u32::from(ip) & mask,
			mask,
		})
	}

	fn contains(self, ip: Ipv4Addr) -> bool {
		u32::from(ip) & self.mask == self.network
	}
}

/// Everything the engine needs from a validated WireGuard configuration.
pub(super) struct Params {
	pub secret: StaticSecret,
	pub peer: PublicKey,
	pub preshared: Option<[u8; 32]>,
	pub keepalive: Option<u16>,
	pub address: Ipv4Addr,
	pub mtu: usize,
	pub allowed: Vec<Cidr>,
}

/// Link between the tunnel and the stack. Packets are plain IP; nothing is written to a device.
#[derive(Default)]
struct Link {
	rx: VecDeque<Vec<u8>>,
	tx: VecDeque<Vec<u8>>,
	mtu: usize,
}

struct RxTok(Vec<u8>);

impl RxToken for RxTok {
	fn consume<R, F>(self, f: F) -> R
	where
		F: FnOnce(&[u8]) -> R,
	{
		f(&self.0)
	}
}

struct TxTok<'a>(&'a mut VecDeque<Vec<u8>>);

impl TxToken for TxTok<'_> {
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

impl Device for Link {
	type RxToken<'a> = RxTok;
	type TxToken<'a> = TxTok<'a>;

	fn receive(&mut self, _: Instant) -> Option<(RxTok, TxTok<'_>)> {
		let packet = self.rx.pop_front()?;
		Some((RxTok(packet), TxTok(&mut self.tx)))
	}

	fn transmit(&mut self, _: Instant) -> Option<TxTok<'_>> {
		Some(TxTok(&mut self.tx))
	}

	fn capabilities(&self) -> DeviceCapabilities {
		let mut capabilities = DeviceCapabilities::default();
		capabilities.medium = Medium::Ip;
		capabilities.max_transmission_unit = self.mtu;
		capabilities
	}
}

/// Command from a bridge owner to the driver. Replies carry the application side of a bridge.
pub(super) enum Command {
	Tcp {
		destination: SocketAddrV4,
		reply: oneshot::Sender<Result<DuplexStream, &'static str>>,
	},
	Udp {
		reply: oneshot::Sender<Result<UdpBridge, &'static str>>,
	},
}

pub(super) type Datagram = (SocketAddrV4, Vec<u8>);

/// Application side of a UDP association: send `(destination, payload)`, receive `(source, payload)`.
pub(super) struct UdpBridge {
	pub tx: mpsc::Sender<Datagram>,
	pub rx: mpsc::Receiver<Datagram>,
}

struct Connect {
	deadline: tokio::time::Instant,
	reply: oneshot::Sender<Result<DuplexStream, &'static str>>,
	client: DuplexStream,
}

struct TcpConn {
	handle: SocketHandle,
	connect: Option<Connect>,
	/// Stack → application. Dropping it tells the pump the remote side is done.
	to_app: Option<mpsc::Sender<Vec<u8>>>,
	/// Application → stack.
	from_app: mpsc::Receiver<Vec<u8>>,
	/// Bytes accepted from the application that the socket has not taken yet.
	pending: Vec<u8>,
	app_closed: bool,
	fin: bool,
}

struct UdpConn {
	handle: SocketHandle,
	to_app: mpsc::Sender<Datagram>,
	from_app: mpsc::Receiver<Datagram>,
}

/// What the tunnel did since the last ultra summary. Counts and error kinds only.
#[derive(Default)]
struct Counters {
	sent: u64,
	send_failed: u64,
	received: u64,
	decrypt_errors: u64,
	last_error: Option<String>,
	inbound_full: u64,
	downlink_dropped: u64,
	uplink_blocked: u64,
	tcp_aborted: u64,
}

/// Guest IP stack and tunnel state. Only the driver task touches it.
pub(super) struct Stack {
	tunn: Tunn,
	link: Link,
	iface: Interface,
	sockets: SocketSet<'static>,
	tcp: Vec<TcpConn>,
	udp: Vec<UdpConn>,
	allowed: Vec<Cidr>,
	mtu: usize,
	next_port: u16,
	started: std::time::Instant,
	/// Encrypted datagrams waiting to leave for the endpoint.
	outbound: Vec<Vec<u8>>,
	scratch: Vec<u8>,
	wake: Arc<Notify>,
	counters: Counters,
	next_summary: std::time::Instant,
}

impl Stack {
	pub(super) fn new(params: Params, index: u32, wake: Arc<Notify>) -> Result<Self, &'static str> {
		let mut link = Link {
			mtu: params.mtu,
			..Link::default()
		};
		let mut iface = Interface::new(Config::new(HardwareAddress::Ip), &mut link, Instant::ZERO);
		let address = Ipv4Address::from(params.address.octets());
		let mut added = false;
		iface.update_ip_addrs(|addresses| {
			added = addresses
				.push(IpCidr::new(IpAddress::Ipv4(address), 32))
				.is_ok();
		});
		if !added || iface.routes_mut().add_default_ipv4_route(address).is_err() {
			return Err("PROTECT stack could not start");
		}
		let tunn = Tunn::new(
			params.secret,
			params.peer,
			params.preshared,
			params.keepalive,
			index,
			None,
		);
		Ok(Self {
			tunn,
			link,
			iface,
			sockets: SocketSet::new(vec![]),
			tcp: Vec::new(),
			udp: Vec::new(),
			allowed: params.allowed,
			mtu: params.mtu,
			next_port: 49_152,
			started: std::time::Instant::now(),
			outbound: Vec::new(),
			scratch: vec![0; 65_535],
			wake,
			counters: Counters::default(),
			next_summary: std::time::Instant::now() + SUMMARY,
		})
	}

	fn now(&self) -> Instant {
		Instant::from_micros(self.started.elapsed().as_micros() as i64)
	}

	fn reachable(&self, ip: Ipv4Addr) -> bool {
		self.allowed.iter().any(|cidr| cidr.contains(ip))
	}

	fn ephemeral_port(&mut self) -> u16 {
		let port = self.next_port;
		self.next_port = if port >= 65_000 { 49_152 } else { port + 1 };
		port
	}

	/// Handles one encrypted datagram from the endpoint. Replies and released packets are
	/// drained in the same bounded loop, as boringtun requires.
	pub(super) fn receive(&mut self, datagram: &[u8]) {
		let mut input = datagram;
		self.counters.received += 1;
		for _ in 0..64 {
			match self.tunn.decapsulate(None, input, &mut self.scratch) {
				TunnResult::WriteToNetwork(reply) => self.outbound.push(reply.to_vec()),
				TunnResult::WriteToTunnelV4(packet, _) => {
					if self.link.rx.len() < MAX_QUEUED {
						self.link.rx.push_back(packet.to_vec());
					} else {
						self.counters.inbound_full += 1;
					}
				}
				TunnResult::WriteToTunnelV6(..) => {}
				TunnResult::Done => break,
				TunnResult::Err(error) => {
					self.counters.decrypt_errors += 1;
					if trace::enabled() {
						self.counters.last_error = Some(format!("{error:?}"));
					}
					break;
				}
			}
			input = &[];
		}
	}

	fn timers(&mut self) {
		if let TunnResult::WriteToNetwork(datagram) = self.tunn.update_timers(&mut self.scratch) {
			self.outbound.push(datagram.to_vec());
		}
		self.summarize();
	}

	/// One ultra line every few seconds: what crossed the tunnel and what was lost or refused.
	fn summarize(&mut self) {
		if !trace::enabled() || std::time::Instant::now() < self.next_summary {
			return;
		}
		self.next_summary = std::time::Instant::now() + SUMMARY;
		let (handshake, tx_bytes, rx_bytes, loss, rtt) = self.tunn.stats();
		let counters = std::mem::take(&mut self.counters);
		trace::line(&format!(
			"tunnel: handshake_age={} tx_bytes={tx_bytes} rx_bytes={rx_bytes} loss={loss:.2} rtt_ms={} \r
			 datagrams sent={} send_failed={} received={} decrypt_errors={} last_error={} \r
			 inbound_queue_full={} downlink_dropped={} uplink_blocked={} tcp_aborted={} tcp={} udp={}",
			handshake.map_or("never".to_owned(), |age| format!("{}s", age.as_secs())),
			rtt.map_or("none".to_owned(), |rtt| rtt.to_string()),
			counters.sent,
			counters.send_failed,
			counters.received,
			counters.decrypt_errors,
			counters.last_error.as_deref().unwrap_or("none"),
			counters.inbound_full,
			counters.downlink_dropped,
			counters.uplink_blocked,
			counters.tcp_aborted,
			self.tcp.len(),
			self.udp.len()
		));
	}

	fn send_ip(&mut self, packet: &[u8]) {
		if let TunnResult::WriteToNetwork(datagram) =
			self.tunn.encapsulate(packet, &mut self.scratch)
		{
			self.outbound.push(datagram.to_vec());
		}
	}

	/// One pass: poll the stack, move bytes between sockets and bridges, poll again, and
	/// encrypt whatever the stack produced.
	pub(super) fn step(&mut self) {
		let now = self.now();
		self.iface.poll(now, &mut self.link, &mut self.sockets);
		self.bridge();
		let now = self.now();
		self.iface.poll(now, &mut self.link, &mut self.sockets);
		while let Some(packet) = self.link.tx.pop_front() {
			self.send_ip(&packet);
		}
	}

	fn idle(&mut self) -> Duration {
		let now = self.now();
		self.iface
			.poll_delay(now, &self.sockets)
			.map_or(MAX_IDLE, |delay| Duration::from(delay).min(MAX_IDLE))
			.max(Duration::from_micros(200))
	}

	fn flush(&mut self, socket: &UdpSocket) {
		for datagram in self.outbound.drain(..) {
			// Losing a datagram is what UDP does; WireGuard and TCP recover from it.
			if socket.try_send(&datagram).is_ok() {
				self.counters.sent += 1;
			} else {
				self.counters.send_failed += 1;
			}
		}
	}

	fn command(&mut self, command: Command) {
		match command {
			Command::Tcp { destination, reply } => self.open_tcp(destination, reply),
			Command::Udp { reply } => self.open_udp(reply),
		}
	}

	fn open_tcp(
		&mut self,
		destination: SocketAddrV4,
		reply: oneshot::Sender<Result<DuplexStream, &'static str>>,
	) {
		if self.tcp.len() >= MAX_TCP {
			let _ = reply.send(Err(SOCKET_LIMIT));
			return;
		}
		if !self.reachable(*destination.ip()) {
			let _ = reply.send(Err(OUTSIDE_ALLOWED));
			return;
		}
		let (client, server) = tokio::io::duplex(CHUNK * 4);
		let (to_app, from_socket) = mpsc::channel(BRIDGE_CHUNKS);
		let (to_socket, from_app) = mpsc::channel(BRIDGE_CHUNKS);
		tokio::spawn(pump(server, from_socket, to_socket, self.wake.clone()));
		let handle = self.sockets.add(tcp::Socket::new(
			tcp::SocketBuffer::new(vec![0; TCP_BUFFER]),
			tcp::SocketBuffer::new(vec![0; TCP_BUFFER]),
		));
		let local = self.ephemeral_port();
		let remote = (
			IpAddress::Ipv4(Ipv4Address::from(destination.ip().octets())),
			destination.port(),
		);
		let socket = self.sockets.get_mut::<tcp::Socket>(handle);
		if socket.connect(self.iface.context(), remote, local).is_err() {
			self.sockets.remove(handle);
			let _ = reply.send(Err("PROTECT TCP route could not start"));
			return;
		}
		self.tcp.push(TcpConn {
			handle,
			connect: Some(Connect {
				deadline: tokio::time::Instant::now() + DEADLINE,
				reply,
				client,
			}),
			to_app: Some(to_app),
			from_app,
			pending: Vec::new(),
			app_closed: false,
			fin: false,
		});
	}

	fn open_udp(&mut self, reply: oneshot::Sender<Result<UdpBridge, &'static str>>) {
		if self.udp.len() >= MAX_UDP {
			let _ = reply.send(Err(SOCKET_LIMIT));
			return;
		}
		let (app_tx, from_app) = mpsc::channel(BRIDGE_CHUNKS);
		let (to_app, app_rx) = mpsc::channel(BRIDGE_CHUNKS);
		let mut socket = udp::Socket::new(
			udp::PacketBuffer::new(
				vec![udp::PacketMetadata::EMPTY; UDP_SLOTS],
				vec![0; UDP_SLOTS * self.mtu],
			),
			udp::PacketBuffer::new(
				vec![udp::PacketMetadata::EMPTY; UDP_SLOTS],
				vec![0; UDP_SLOTS * self.mtu],
			),
		);
		let port = self.ephemeral_port();
		if socket.bind(IpListenEndpoint::from(port)).is_err() {
			let _ = reply.send(Err("PROTECT UDP route could not start"));
			return;
		}
		let handle = self.sockets.add(socket);
		self.udp.push(UdpConn {
			handle,
			to_app,
			from_app,
		});
		let _ = reply.send(Ok(UdpBridge {
			tx: app_tx,
			rx: app_rx,
		}));
	}

	fn bridge(&mut self) {
		let Self {
			tcp,
			udp,
			sockets,
			scratch,
			allowed,
			mtu,
			counters,
			..
		} = self;
		let mut finished_tcp = Vec::new();
		for (index, conn) in tcp.iter_mut().enumerate() {
			let socket = sockets.get_mut::<tcp::Socket>(conn.handle);
			if let Some(connect) = conn.connect.take() {
				match socket.state() {
					tcp::State::Established => {
						if connect.reply.send(Ok(connect.client)).is_err() {
							socket.abort();
							finished_tcp.push(index);
						}
						continue;
					}
					tcp::State::Closed => {
						let _ = connect.reply.send(Err("TCP connection refused"));
						finished_tcp.push(index);
						continue;
					}
					_ if tokio::time::Instant::now() >= connect.deadline => {
						let _ = connect.reply.send(Err("TCP connection timed out"));
						socket.abort();
						finished_tcp.push(index);
						continue;
					}
					_ => {
						conn.connect = Some(connect);
						continue;
					}
				}
			}
			// Stack → application, only while the bridge has room for another chunk.
			if socket.can_recv()
				&& conn
					.to_app
					.as_ref()
					.is_some_and(|to_app| to_app.capacity() > 0)
				&& let Ok(read) = socket.recv_slice(&mut scratch[..CHUNK])
				&& read > 0
			{
				let delivered = conn
					.to_app
					.as_ref()
					.is_some_and(|to_app| to_app.try_send(scratch[..read].to_vec()).is_ok());
				if !delivered {
					counters.tcp_aborted += 1;
					socket.abort();
					finished_tcp.push(index);
					continue;
				}
			}
			if !socket.may_recv() && !socket.can_recv() {
				// Remote side finished: the pump sees end-of-stream and closes the client.
				conn.to_app = None;
			}
			// Application → stack.
			if conn.pending.is_empty() && !conn.app_closed {
				match conn.from_app.try_recv() {
					Ok(chunk) => conn.pending = chunk,
					Err(mpsc::error::TryRecvError::Empty) => {}
					Err(mpsc::error::TryRecvError::Disconnected) => conn.app_closed = true,
				}
			}
			if !conn.pending.is_empty()
				&& socket.can_send()
				&& let Ok(written) = socket.send_slice(&conn.pending)
			{
				conn.pending.drain(..written);
			}
			if conn.app_closed && conn.pending.is_empty() && !conn.fin {
				socket.close();
				conn.fin = true;
			}
			if socket.state() == tcp::State::Closed {
				finished_tcp.push(index);
			}
		}
		finished_tcp.sort_unstable();
		finished_tcp.dedup();
		for index in finished_tcp.into_iter().rev() {
			let conn = tcp.swap_remove(index);
			sockets.remove(conn.handle);
		}

		let mut finished_udp = Vec::new();
		for (index, conn) in udp.iter_mut().enumerate() {
			let socket = sockets.get_mut::<udp::Socket>(conn.handle);
			while socket.can_recv() {
				let Ok((data, meta)) = socket.recv() else {
					break;
				};
				// Only IPv4 is enabled in smoltcp for this crate, so the address is always IPv4.
				let IpAddress::Ipv4(source) = meta.endpoint.addr;
				if conn.to_app.capacity() == 0 {
					counters.downlink_dropped += 1;
					continue;
				}
				let source = SocketAddrV4::new(Ipv4Addr::from(source.octets()), meta.endpoint.port);
				let _ = conn.to_app.try_send((source, data.to_vec()));
			}
			if conn.to_app.is_closed() {
				finished_udp.push(index);
				continue;
			}
			while socket.can_send() {
				match conn.from_app.try_recv() {
					Ok((destination, payload)) => {
						if payload.len() + IPV4_UDP_OVERHEAD > *mtu
							|| !allowed.iter().any(|cidr| cidr.contains(*destination.ip()))
						{
							counters.uplink_blocked += 1;
							continue;
						}
						let endpoint = IpEndpoint::new(
							IpAddress::Ipv4(Ipv4Address::from(destination.ip().octets())),
							destination.port(),
						);
						let _ = socket.send_slice(&payload, endpoint);
					}
					Err(mpsc::error::TryRecvError::Empty) => break,
					Err(mpsc::error::TryRecvError::Disconnected) => {
						finished_udp.push(index);
						break;
					}
				}
			}
		}
		finished_udp.sort_unstable();
		finished_udp.dedup();
		for index in finished_udp.into_iter().rev() {
			let conn = udp.swap_remove(index);
			sockets.remove(conn.handle);
		}
	}
}

/// Moves bytes between one client stream and the channels the stack uses. Ends on either
/// side's end-of-stream.
async fn pump(
	mut server: DuplexStream,
	mut from_socket: mpsc::Receiver<Vec<u8>>,
	to_socket: mpsc::Sender<Vec<u8>>,
	wake: Arc<Notify>,
) {
	let mut buffer = vec![0u8; CHUNK];
	loop {
		tokio::select! {
			read = tokio::io::AsyncReadExt::read(&mut server, &mut buffer) => match read {
				Ok(0) | Err(_) => break,
				Ok(count) => {
					if to_socket.send(buffer[..count].to_vec()).await.is_err() {
						break;
					}
					wake.notify_one();
				}
			},
			chunk = from_socket.recv() => match chunk {
				Some(data) => {
					if tokio::io::AsyncWriteExt::write_all(&mut server, &data).await.is_err() {
						break;
					}
				}
				None => break,
			},
		}
	}
	let _ = tokio::io::AsyncWriteExt::shutdown(&mut server).await;
}

/// Reads endpoint datagrams and hands them to the driver. Read errors (for example ICMP
/// refusals on a connected socket) are transient for UDP, so the reader keeps going.
async fn read(socket: Arc<UdpSocket>, inbound: mpsc::Sender<Vec<u8>>) {
	let mut buffer = vec![0u8; 65_535];
	loop {
		match socket.recv(&mut buffer).await {
			Ok(count) => {
				if inbound.send(buffer[..count].to_vec()).await.is_err() {
					return;
				}
			}
			Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
		}
	}
}

/// Owns the stack until its inbound channel closes. `hook` runs once per iteration after the
/// stack is driven; the engine passes a no-op and tests use it to serve as a remote peer.
pub(super) async fn run(
	mut stack: Stack,
	socket: Arc<UdpSocket>,
	mut commands: mpsc::Receiver<Command>,
	mut inbound: mpsc::Receiver<Vec<u8>>,
	mut hook: impl FnMut(&mut Stack),
) {
	let mut tick = tokio::time::interval(TICK);
	let mut commands_open = true;
	let wake = stack.wake.clone();
	stack.step();
	stack.flush(&socket);
	loop {
		let idle = stack.idle();
		tokio::select! {
			datagram = inbound.recv() => match datagram {
				Some(datagram) => stack.receive(&datagram),
				None => break,
			},
			command = commands.recv(), if commands_open => match command {
				Some(command) => stack.command(command),
				None => commands_open = false,
			},
			_ = tick.tick() => stack.timers(),
			_ = wake.notified() => {}
			_ = tokio::time::sleep(idle) => {}
		}
		hook(&mut stack);
		stack.step();
		stack.flush(&socket);
	}
	trace::line("tunnel stopped: the endpoint reader ended");
}

/// Handle used by the inlet and by the DNS resolver. Cloning it is cheap.
#[derive(Clone)]
pub(super) struct Handle {
	commands: mpsc::Sender<Command>,
}

impl Handle {
	pub(super) async fn tcp(
		&self,
		destination: SocketAddrV4,
	) -> Result<DuplexStream, &'static str> {
		let (reply, answer) = oneshot::channel();
		self.commands
			.send(Command::Tcp { destination, reply })
			.await
			.map_err(|_| ENGINE_STOPPED)?;
		match tokio::time::timeout(DEADLINE + Duration::from_secs(1), answer).await {
			Ok(Ok(result)) => result,
			Ok(Err(_)) => Err(ENGINE_STOPPED),
			Err(_) => Err("TCP connection timed out"),
		}
	}

	pub(super) async fn udp(&self) -> Result<UdpBridge, &'static str> {
		let (reply, answer) = oneshot::channel();
		self.commands
			.send(Command::Udp { reply })
			.await
			.map_err(|_| ENGINE_STOPPED)?;
		match tokio::time::timeout(DEADLINE, answer).await {
			Ok(Ok(result)) => result,
			Ok(Err(_)) => Err(ENGINE_STOPPED),
			Err(_) => Err("UDP route timed out"),
		}
	}

	/// Resolves an IPv4 address through the tunnel. Names never use the system resolver.
	pub(super) async fn resolve(&self, name: &str) -> Result<Ipv4Addr, &'static str> {
		let mut id = [0u8; 2];
		getrandom::fill(&mut id).map_err(|_| "Random source unavailable")?;
		let id = u16::from_be_bytes(id);
		let query = super::wg_dns::query(name, id).ok_or("Invalid host name")?;
		let mut bridge = self.udp().await?;
		bridge
			.tx
			.send((DNS_SERVER, query))
			.await
			.map_err(|_| ENGINE_STOPPED)?;
		tokio::time::timeout(DEADLINE, async {
			while let Some((from, reply)) = bridge.rx.recv().await {
				if from == DNS_SERVER
					&& let Some(ip) = super::wg_dns::answer(&reply, id)
				{
					return Ok(ip);
				}
			}
			Err(ENGINE_STOPPED)
		})
		.await
		.map_err(|_| "DNS lookup timed out")?
	}
}

/// Ultra diagnostics only: a name lookup through the tunnel every few seconds. When a call or a
/// stream dies this says whether the tunnel itself was still answering at that moment.
async fn canary(handle: Handle) {
	let mut every = tokio::time::interval(CANARY);
	every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
	loop {
		every.tick().await;
		let begin = std::time::Instant::now();
		let outcome = handle.resolve(CANARY_HOST).await;
		let ms = begin.elapsed().as_millis();
		match outcome {
			Ok(_) => trace::line(&format!("canary: dns ok in {ms} ms")),
			Err(reason) => trace::line(&format!("canary: dns failed ({reason}) after {ms} ms")),
		}
	}
}

/// A running tunnel. Dropping it aborts every task it owns, which closes every bridge.
pub(super) struct Engine {
	handle: Handle,
	tasks: Vec<JoinHandle<()>>,
}

impl Engine {
	pub(super) async fn start(params: Params, endpoint: SocketAddr) -> Result<Self, &'static str> {
		let bind = if endpoint.is_ipv4() {
			"0.0.0.0:0"
		} else {
			"[::]:0"
		};
		let socket = UdpSocket::bind(bind)
			.await
			.map_err(|_| "WireGuard UDP socket unavailable")?;
		socket
			.connect(endpoint)
			.await
			.map_err(|_| "WireGuard endpoint unreachable")?;
		Self::start_on(socket, params)
	}

	/// Starts the tunnel on a socket already connected to the endpoint.
	pub(super) fn start_on(socket: UdpSocket, params: Params) -> Result<Self, &'static str> {
		let socket = Arc::new(socket);
		let mut index = [0u8; 4];
		getrandom::fill(&mut index).map_err(|_| "Random source unavailable")?;
		let wake = Arc::new(Notify::new());
		let stack = Stack::new(params, u32::from_be_bytes(index), wake)?;
		let (commands, command_rx) = mpsc::channel(COMMANDS);
		let (inbound_tx, inbound_rx) = mpsc::channel(INBOUND);
		let reader = tokio::spawn(read(socket.clone(), inbound_tx));
		let driver = tokio::spawn(run(stack, socket, command_rx, inbound_rx, |_| {}));
		let handle = Handle { commands };
		let mut tasks = vec![reader, driver];
		if trace::enabled() {
			tasks.push(tokio::spawn(canary(handle.clone())));
		}
		Ok(Self { handle, tasks })
	}

	pub(super) fn handle(&self) -> Handle {
		self.handle.clone()
	}

	/// Adds a task whose lifetime must end with the engine, such as the local inlet.
	pub(super) fn own(&mut self, task: JoinHandle<()>) {
		self.tasks.push(task);
	}
}

impl Drop for Engine {
	fn drop(&mut self) {
		for task in &self.tasks {
			task.abort();
		}
	}
}

#[cfg(test)]
pub(crate) mod fixture {
	use super::*;
	use tokio::net::UdpSocket as TestUdp;

	pub(crate) const REMOTE_TCP: u16 = 9000;
	pub(crate) const REMOTE_UDP: u16 = 7;
	pub(crate) const CLIENT: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
	pub(crate) const REMOTE: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);

	fn secret(tag: u8) -> StaticSecret {
		let mut bytes = [tag; 32];
		bytes[0] = tag.wrapping_mul(7).max(1);
		StaticSecret::from(bytes)
	}

	/// Client secret, remote public key, remote secret, client public key.
	pub(crate) fn keys() -> (StaticSecret, PublicKey, StaticSecret, PublicKey) {
		let client = secret(11);
		let remote = secret(22);
		let client_public = PublicKey::from(&client);
		let remote_public = PublicKey::from(&remote);
		(client, remote_public, remote, client_public)
	}

	pub(crate) fn params(secret: StaticSecret, peer: PublicKey, address: Ipv4Addr) -> Params {
		Params {
			secret,
			peer,
			preshared: None,
			keepalive: Some(25),
			address,
			mtu: 1420,
			allowed: vec![Cidr::parse("0.0.0.0/0").expect("cidr")],
		}
	}

	/// Two loopback sockets connected to each other: (client side, remote side).
	pub(crate) async fn pair() -> (TestUdp, TestUdp) {
		let client = TestUdp::bind("127.0.0.1:0").await.expect("bind");
		let remote = TestUdp::bind("127.0.0.1:0").await.expect("bind");
		client
			.connect(remote.local_addr().expect("address"))
			.await
			.expect("connect");
		remote
			.connect(client.local_addr().expect("address"))
			.await
			.expect("connect");
		(client, remote)
	}

	/// Runs a remote peer on a connected socket. It serves TCP echo on `REMOTE_TCP` and UDP echo
	/// on `REMOTE_UDP`, inside its own stack.
	pub(crate) fn serve_remote(socket: TestUdp, secret: StaticSecret, peer: PublicKey) {
		let socket = Arc::new(socket);
		let mut stack =
			Stack::new(params(secret, peer, REMOTE), 2, Arc::new(Notify::new())).expect("stack");
		let listener = stack.sockets.add(tcp::Socket::new(
			tcp::SocketBuffer::new(vec![0; TCP_BUFFER]),
			tcp::SocketBuffer::new(vec![0; TCP_BUFFER]),
		));
		stack
			.sockets
			.get_mut::<tcp::Socket>(listener)
			.listen(REMOTE_TCP)
			.expect("listen");
		let echo = stack.sockets.add(udp::Socket::new(
			udp::PacketBuffer::new(vec![udp::PacketMetadata::EMPTY; 8], vec![0; 8 * 1500]),
			udp::PacketBuffer::new(vec![udp::PacketMetadata::EMPTY; 8], vec![0; 8 * 1500]),
		));
		stack
			.sockets
			.get_mut::<udp::Socket>(echo)
			.bind(REMOTE_UDP)
			.expect("bind echo");
		let mut pending = Vec::new();
		let (inbound_tx, inbound_rx) = mpsc::channel(INBOUND);
		let (_no_commands, commands) = mpsc::channel(1);
		tokio::spawn(read(socket.clone(), inbound_tx));
		tokio::spawn(run(stack, socket, commands, inbound_rx, move |stack| {
			let socket = stack.sockets.get_mut::<tcp::Socket>(listener);
			if socket.can_recv() {
				let mut buffer = [0u8; 4096];
				if let Ok(read) = socket.recv_slice(&mut buffer) {
					pending.extend_from_slice(&buffer[..read]);
				}
			}
			if !pending.is_empty()
				&& socket.can_send()
				&& let Ok(written) = socket.send_slice(&pending)
			{
				pending.drain(..written);
			}
			let udp = stack.sockets.get_mut::<udp::Socket>(echo);
			while udp.can_recv() {
				let Ok((data, meta)) = udp.recv() else {
					break;
				};
				let (data, endpoint) = (data.to_vec(), meta.endpoint);
				let _ = udp.send_slice(&data, endpoint);
			}
		}));
	}

	/// A client engine whose only peer is a remote over a direct socket pair.
	pub(crate) async fn direct(allowed: &str) -> Engine {
		let (client, remote) = pair().await;
		let (client_secret, remote_public, remote_secret, client_public) = keys();
		serve_remote(remote, remote_secret, client_public);
		let mut params = params(client_secret, remote_public, CLIENT);
		params.allowed = vec![Cidr::parse(allowed).expect("cidr")];
		Engine::start_on(client, params).expect("engine")
	}
}

#[cfg(test)]
mod tests {
	use super::fixture::*;
	use super::*;
	use std::time::Duration;
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::UdpSocket as TestUdp,
	};

	/// The ultra summary names the counters the written analysis reads, and counts what happened.
	#[test]
	fn the_ultra_summary_carries_the_counters_the_analysis_reads() {
		use std::sync::Mutex;
		static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
		fn sink(scope: &str, message: &str) {
			if scope == "protect" && message.starts_with("tunnel: ") {
				LINES
					.lock()
					.unwrap_or_else(|poison| poison.into_inner())
					.push(message.to_owned());
			}
		}
		trace::set_trace_sink(sink);
		trace::set_ultra(true);
		let (client, remote_public, ..) = keys();
		let mut stack = Stack::new(
			params(client, remote_public, REMOTE),
			9,
			Arc::new(Notify::new()),
		)
		.expect("stack");
		// Garbage is a decrypt error: it is counted and named, not silently dropped.
		stack.receive(&[0xde, 0xad, 0xbe, 0xef, 1, 2, 3, 4]);
		stack.counters.downlink_dropped = 5;
		stack.next_summary = std::time::Instant::now() - Duration::from_secs(1);
		stack.timers();
		let lines = LINES.lock().unwrap_or_else(|poison| poison.into_inner());
		let line = lines
			.iter()
			.find(|line| line.contains(" decrypt_errors=1 "))
			.expect("a summary that counted the bad datagram");
		for key in [
			" handshake_age=",
			" loss=",
			" sent=",
			" send_failed=",
			" received=1 ",
			" last_error=",
			" inbound_queue_full=",
			" downlink_dropped=5 ",
			" uplink_blocked=",
			" tcp_aborted=",
		] {
			assert!(line.contains(key), "{key:?} missing from {line}");
		}
		assert!(!line.contains("last_error=none"), "{line}");
	}

	/// Forwards between the client and the remote and drops every `every`-th datagram. Returns
	/// the address the client connects to and the address the remote connects to.
	async fn lossy(remote: SocketAddr, every: u64) -> (SocketAddr, SocketAddr) {
		let front = Arc::new(TestUdp::bind("127.0.0.1:0").await.expect("front"));
		let back = Arc::new(TestUdp::bind("127.0.0.1:0").await.expect("back"));
		let addresses = (
			front.local_addr().expect("front address"),
			back.local_addr().expect("back address"),
		);
		back.connect(remote).await.expect("connect");
		tokio::spawn(async move {
			let mut client = None;
			let mut count = 0u64;
			let (mut from_client, mut from_remote) = (vec![0u8; 65_535], vec![0u8; 65_535]);
			loop {
				tokio::select! {
					received = front.recv_from(&mut from_client) => {
						let Ok((length, from)) = received else { continue };
						client = Some(from);
						count += 1;
						if !count.is_multiple_of(every) {
							let _ = back.send(&from_client[..length]).await;
						}
					}
					received = back.recv(&mut from_remote) => {
						let Ok(length) = received else { continue };
						count += 1;
						if !count.is_multiple_of(every)
							&& let Some(client) = client
						{
							let _ = front.send_to(&from_remote[..length], client).await;
						}
					}
				}
			}
		});
		addresses
	}

	#[tokio::test]
	async fn tcp_echo_travels_through_a_real_udp_tunnel() {
		let engine = direct("0.0.0.0/0").await;
		let mut stream = engine
			.handle()
			.tcp(SocketAddrV4::new(REMOTE, REMOTE_TCP))
			.await
			.expect("connect");
		stream.write_all(b"hello").await.expect("write");
		let mut echoed = [0u8; 5];
		tokio::time::timeout(Duration::from_secs(20), stream.read_exact(&mut echoed))
			.await
			.expect("echo before deadline")
			.expect("read");
		assert_eq!(&echoed, b"hello");
	}

	#[tokio::test]
	async fn tcp_transfer_survives_datagram_loss() {
		let (client_secret, remote_public, remote_secret, client_public) = keys();
		let client = TestUdp::bind("127.0.0.1:0").await.expect("client");
		let remote = TestUdp::bind("127.0.0.1:0").await.expect("remote");
		let (front, back) = lossy(remote.local_addr().expect("remote address"), 5).await;
		client.connect(front).await.expect("connect");
		remote.connect(back).await.expect("connect");
		serve_remote(remote, remote_secret, client_public);
		let engine =
			Engine::start_on(client, params(client_secret, remote_public, CLIENT)).expect("engine");
		let stream = engine
			.handle()
			.tcp(SocketAddrV4::new(REMOTE, REMOTE_TCP))
			.await
			.expect("connect");
		let payload: Vec<u8> = (0..8 * 1024).map(|i| (i % 251) as u8).collect();
		let outgoing = payload.clone();
		let (mut reader, mut writer) = tokio::io::split(stream);
		let send = async {
			writer.write_all(&outgoing).await.expect("write");
		};
		let mut echoed = vec![0u8; payload.len()];
		let receive = async {
			reader.read_exact(&mut echoed).await.expect("read");
		};
		tokio::time::timeout(Duration::from_secs(60), async {
			tokio::join!(send, receive)
		})
		.await
		.expect("transfer before deadline");
		assert_eq!(echoed, payload);
	}

	#[tokio::test]
	async fn udp_round_trip_travels_through_the_tunnel() {
		let engine = direct("0.0.0.0/0").await;
		let mut bridge = engine.handle().udp().await.expect("udp");
		bridge
			.tx
			.send((SocketAddrV4::new(REMOTE, REMOTE_UDP), b"ping".to_vec()))
			.await
			.expect("send");
		let (from, payload) = tokio::time::timeout(Duration::from_secs(20), bridge.rx.recv())
			.await
			.expect("reply before deadline")
			.expect("open");
		assert_eq!(from, SocketAddrV4::new(REMOTE, REMOTE_UDP));
		assert_eq!(payload, b"ping");
	}

	#[tokio::test]
	async fn destinations_outside_allowed_ips_are_refused() {
		let engine = direct("192.168.0.0/16").await;
		let refused = engine
			.handle()
			.tcp(SocketAddrV4::new(REMOTE, REMOTE_TCP))
			.await;
		assert_eq!(refused.err(), Some(OUTSIDE_ALLOWED));
	}

	#[tokio::test]
	async fn dropping_the_engine_closes_open_bridges() {
		let engine = direct("0.0.0.0/0").await;
		let handle = engine.handle();
		let mut stream = handle
			.tcp(SocketAddrV4::new(REMOTE, REMOTE_TCP))
			.await
			.expect("connect");
		drop(engine);
		let mut buffer = [0u8; 1];
		let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer))
			.await
			.expect("closed before deadline");
		assert!(matches!(read, Ok(0) | Err(_)));
		assert!(handle.udp().await.is_err());
	}
}
