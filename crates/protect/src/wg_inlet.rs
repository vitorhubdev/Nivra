//! Loopback SOCKS5 inlet for the in-process WireGuard engine. It listens on 127.0.0.1 only,
//! requires the session's random username and password, and serves CONNECT and UDP ASSOCIATE.
//! Targets are IPv4 addresses or names; names resolve through the tunnel. Nothing is
//! forwarded directly, so a refused or failed tunnel never falls back to the local network path.
use super::trace;
use super::wg_engine::{Handle, UdpBridge};
use std::{
	io,
	net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4},
	sync::Arc,
	time::Duration,
};
use tokio::{
	io::{AsyncReadExt, AsyncWriteExt},
	net::{TcpListener, TcpStream, UdpSocket},
	sync::Semaphore,
};
use zeroize::Zeroizing;

const MAX_CLIENTS: usize = 64;
const HANDSHAKE: Duration = Duration::from_secs(10);
const UNSPECIFIED: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);

/// The one username and password accepted by a session.
pub(super) struct Credentials {
	username: Zeroizing<Vec<u8>>,
	password: Zeroizing<Vec<u8>>,
}

impl Credentials {
	pub(super) fn new(username: &str, password: &str) -> Arc<Self> {
		Arc::new(Self {
			username: Zeroizing::new(username.as_bytes().to_vec()),
			password: Zeroizing::new(password.as_bytes().to_vec()),
		})
	}

	fn accepts(&self, username: &[u8], password: &[u8]) -> bool {
		// Both comparisons run to completion so the reply time does not reveal a prefix match.
		same(&self.username, username) & same(&self.password, password)
	}
}

fn same(expected: &[u8], given: &[u8]) -> bool {
	expected.len() == given.len()
		&& expected
			.iter()
			.zip(given)
			.fold(0u8, |acc, (a, b)| acc | (a ^ b))
			== 0
}

fn invalid(message: &'static str) -> io::Error {
	io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Accepts sessions until the listener is dropped with its task. Each session holds one slot.
pub(super) async fn serve(listener: TcpListener, handle: Handle, credentials: Arc<Credentials>) {
	let slots = Arc::new(Semaphore::new(MAX_CLIENTS));
	let mut refused = 0u64;
	loop {
		let Ok((client, peer)) = listener.accept().await else {
			tokio::time::sleep(Duration::from_millis(10)).await;
			continue;
		};
		if !peer.ip().is_loopback() {
			continue;
		}
		let Ok(permit) = slots.clone().try_acquire_owned() else {
			refused += 1;
			if refused.is_power_of_two() {
				trace::line(&format!(
					"inlet refused a client: all {MAX_CLIENTS} slots are busy ({refused} refused so far)"
				));
			}
			continue;
		};
		let handle = handle.clone();
		let credentials = credentials.clone();
		tokio::spawn(async move {
			let _permit = permit;
			if let Err(error) = session(client, handle, credentials).await {
				note_session_error(&error);
			}
		});
	}
}

/// Ultra diagnostics for a session that ended in an error. Normal closes are errors too, so only
/// the first few are written.
fn note_session_error(error: &io::Error) {
	static SEEN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
	if super::trace::enabled() && SEEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 100 {
		trace::line(&format!("inlet session ended: kind={:?}", error.kind()));
	}
}

/// Ultra diagnostics for a relayed TCP connection: long ones and failures only, with how long it
/// lived and how many bytes crossed. A short, clean connection is routine and stays out.
fn note_relay(port: u16, lived: Duration, result: &io::Result<(u64, u64)>) {
	if !super::trace::enabled() {
		return;
	}
	match result {
		Ok((up, down)) if lived >= Duration::from_secs(5) => trace::line(&format!(
			"relay to port {port} closed after {} s: up_bytes={up} down_bytes={down}",
			lived.as_secs()
		)),
		Ok(_) => {}
		Err(error) => trace::line(&format!(
			"relay to port {port} failed after {} s: kind={:?}",
			lived.as_secs(),
			error.kind()
		)),
	}
}

enum Target {
	Ip(Ipv4Addr),
	Name(String),
}

async fn session(
	mut client: TcpStream,
	handle: Handle,
	credentials: Arc<Credentials>,
) -> io::Result<()> {
	let request = tokio::time::timeout(HANDSHAKE, negotiate(&mut client, &credentials))
		.await
		.map_err(|_| invalid("SOCKS5 handshake timed out"))??;
	let Some((command, target, port)) = request else {
		return Ok(());
	};
	match command {
		1 => connect(client, handle, target, port).await,
		3 => associate(client, handle).await,
		_ => reply(&mut client, 7, UNSPECIFIED).await,
	}
}

/// Greeting, RFC 1929 authentication and the request head. Returns `None` after answering an
/// unsupported address type, so the caller only has to close.
async fn negotiate(
	client: &mut TcpStream,
	credentials: &Credentials,
) -> io::Result<Option<(u8, Target, u16)>> {
	if client.read_u8().await? != 5 {
		return Err(invalid("not SOCKS5"));
	}
	let methods = client.read_u8().await?;
	let mut offered = vec![0u8; methods as usize];
	client.read_exact(&mut offered).await?;
	if !offered.contains(&2) {
		client.write_all(&[5, 0xff]).await?;
		return Err(invalid("username/password authentication required"));
	}
	client.write_all(&[5, 2]).await?;

	if client.read_u8().await? != 1 {
		return Err(invalid("unsupported authentication version"));
	}
	let username = read_bytes(client).await?;
	let password = read_bytes(client).await?;
	if !credentials.accepts(&username, &password) {
		client.write_all(&[1, 1]).await?;
		return Err(invalid("wrong credentials"));
	}
	client.write_all(&[1, 0]).await?;

	let mut head = [0u8; 3];
	client.read_exact(&mut head).await?;
	if head[0] != 5 {
		return Err(invalid("not SOCKS5 request"));
	}
	let target = match client.read_u8().await? {
		1 => {
			let mut octets = [0u8; 4];
			client.read_exact(&mut octets).await?;
			Target::Ip(Ipv4Addr::from(octets))
		}
		3 => {
			let length = client.read_u8().await?;
			let mut name = vec![0u8; length as usize];
			client.read_exact(&mut name).await?;
			let name = String::from_utf8(name).map_err(|_| invalid("invalid host name"))?;
			Target::Name(name)
		}
		_ => {
			reply(client, 8, UNSPECIFIED).await?;
			return Ok(None);
		}
	};
	let port = client.read_u16().await?;
	Ok(Some((head[1], target, port)))
}

async fn read_bytes(client: &mut TcpStream) -> io::Result<Zeroizing<Vec<u8>>> {
	let length = client.read_u8().await?;
	let mut bytes = Zeroizing::new(vec![0u8; length as usize]);
	client.read_exact(&mut bytes).await?;
	Ok(bytes)
}

async fn reply(client: &mut TcpStream, code: u8, bound: SocketAddr) -> io::Result<()> {
	let mut packet = vec![5, code, 0];
	match bound.ip() {
		IpAddr::V4(ip) => {
			packet.push(1);
			packet.extend_from_slice(&ip.octets());
		}
		IpAddr::V6(ip) => {
			packet.push(4);
			packet.extend_from_slice(&ip.octets());
		}
	}
	packet.extend_from_slice(&bound.port().to_be_bytes());
	client.write_all(&packet).await
}

async fn connect(
	mut client: TcpStream,
	handle: Handle,
	target: Target,
	port: u16,
) -> io::Result<()> {
	let ip = match target {
		Target::Ip(ip) => ip,
		Target::Name(name) => match handle.resolve(&name).await {
			Ok(ip) => ip,
			Err(reason) => {
				trace::line(&format!("name lookup failed: {reason}"));
				return reply(&mut client, 4, UNSPECIFIED).await;
			}
		},
	};
	match handle.tcp(SocketAddrV4::new(ip, port)).await {
		Ok(mut stream) => {
			reply(&mut client, 0, UNSPECIFIED).await?;
			let began = std::time::Instant::now();
			let relayed = tokio::io::copy_bidirectional(&mut client, &mut stream).await;
			note_relay(port, began.elapsed(), &relayed);
			relayed?;
			Ok(())
		}
		Err(reason) => {
			trace::line(&format!("connect to port {port} failed: {reason}"));
			reply(&mut client, 1, UNSPECIFIED).await
		}
	}
}

/// UDP ASSOCIATE for IPv4 destinations. The association lives as long as the TCP control
/// connection; the relay socket is loopback-only and accepts one client address.
async fn associate(mut client: TcpStream, handle: Handle) -> io::Result<()> {
	let UdpBridge { tx, mut rx } = match handle.udp().await {
		Ok(bridge) => bridge,
		Err(_) => return reply(&mut client, 1, UNSPECIFIED).await,
	};
	let relay = UdpSocket::bind("127.0.0.1:0").await?;
	reply(&mut client, 0, relay.local_addr()?).await?;

	let mut peer: Option<SocketAddr> = None;
	let mut buffer = vec![0u8; 65_535];
	let mut control = [0u8; 1];
	let mut uplink_dropped = 0u64;
	let ended;
	loop {
		tokio::select! {
			received = relay.recv_from(&mut buffer) => {
				let Ok((count, from)) = received else {
					continue;
				};
				match peer {
					None => peer = Some(from),
					Some(known) if known != from => continue,
					Some(_) => {}
				}
				if let Some((destination, payload)) = parse_datagram(&buffer[..count])
					&& tx.try_send((destination, payload.to_vec())).is_err()
				{
					uplink_dropped += 1;
				}
			}
			incoming = rx.recv() => {
				let Some((source, payload)) = incoming else {
					ended = "the tunnel closed it";
					break;
				};
				if let Some(peer) = peer {
					let mut packet = header(source);
					packet.extend_from_slice(&payload);
					let _ = relay.send_to(&packet, peer).await;
				}
			}
			read = client.read(&mut control) => match read {
				Ok(0) | Err(_) => {
					ended = "the app closed it";
					break;
				}
				Ok(_) => {}
			},
		}
	}
	trace::line(&format!(
		"udp association ended: {ended}; uplink_dropped={uplink_dropped}"
	));
	Ok(())
}

/// Parses `RSV RSV FRAG ATYP=1 ADDR PORT DATA`. Fragments and non-IPv4 targets are dropped.
fn parse_datagram(packet: &[u8]) -> Option<(SocketAddrV4, &[u8])> {
	if packet.len() < 10 || packet[..3] != [0, 0, 0] || packet[3] != 1 {
		return None;
	}
	let ip = Ipv4Addr::new(packet[4], packet[5], packet[6], packet[7]);
	let port = u16::from_be_bytes([packet[8], packet[9]]);
	Some((SocketAddrV4::new(ip, port), &packet[10..]))
}

fn header(source: SocketAddrV4) -> Vec<u8> {
	let mut packet = vec![0, 0, 0, 1];
	packet.extend_from_slice(&source.ip().octets());
	packet.extend_from_slice(&source.port().to_be_bytes());
	packet
}

#[cfg(test)]
mod tests {
	use super::super::wg_engine::fixture::*;
	use super::*;
	use tokio::net::TcpStream as Client;

	async fn inlet(password: &str) -> (super::super::wg_engine::Engine, SocketAddr) {
		let mut engine = direct("0.0.0.0/0").await;
		let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
		let address = listener.local_addr().expect("address");
		let task = tokio::spawn(serve(
			listener,
			engine.handle(),
			Credentials::new("nivra", password),
		));
		engine.own(task);
		(engine, address)
	}

	async fn authenticate(stream: &mut Client, username: &str, password: &str) -> bool {
		stream.write_all(&[5, 1, 2]).await.expect("greet");
		let mut method = [0u8; 2];
		stream.read_exact(&mut method).await.expect("method");
		if method != [5, 2] {
			return false;
		}
		let mut packet = vec![1, username.len() as u8];
		packet.extend_from_slice(username.as_bytes());
		packet.push(password.len() as u8);
		packet.extend_from_slice(password.as_bytes());
		stream.write_all(&packet).await.expect("auth");
		let mut status = [0u8; 2];
		stream.read_exact(&mut status).await.expect("status");
		status == [1, 0]
	}

	#[tokio::test]
	async fn connect_requires_the_session_credentials() {
		let (_engine, address) = inlet("secret-session").await;
		let mut wrong = Client::connect(address).await.expect("connect");
		assert!(!authenticate(&mut wrong, "nivra", "guess").await);

		let mut right = Client::connect(address).await.expect("connect");
		assert!(authenticate(&mut right, "nivra", "secret-session").await);
		let mut request = vec![5, 1, 0, 1, 10, 0, 0, 2];
		request.extend_from_slice(&REMOTE_TCP.to_be_bytes());
		right.write_all(&request).await.expect("request");
		let mut head = [0u8; 10];
		right.read_exact(&mut head).await.expect("reply");
		assert_eq!(&head[..2], &[5, 0]);
		right.write_all(b"ok").await.expect("payload");
		let mut echoed = [0u8; 2];
		tokio::time::timeout(Duration::from_secs(20), right.read_exact(&mut echoed))
			.await
			.expect("echo before deadline")
			.expect("read");
		assert_eq!(&echoed, b"ok");
	}

	#[tokio::test]
	async fn clients_without_password_auth_are_refused() {
		let (_engine, address) = inlet("secret-session").await;
		let mut client = Client::connect(address).await.expect("connect");
		client.write_all(&[5, 1, 0]).await.expect("greet");
		let mut method = [0u8; 2];
		client.read_exact(&mut method).await.expect("method");
		assert_eq!(method, [5, 0xff]);
	}

	#[tokio::test]
	async fn udp_associate_relays_a_datagram_through_the_tunnel() {
		let (_engine, address) = inlet("secret-session").await;
		let mut control = Client::connect(address).await.expect("connect");
		assert!(authenticate(&mut control, "nivra", "secret-session").await);
		control
			.write_all(&[5, 3, 0, 1, 0, 0, 0, 0, 0, 0])
			.await
			.expect("associate");
		let mut head = [0u8; 10];
		control.read_exact(&mut head).await.expect("reply");
		assert_eq!(&head[..2], &[5, 0]);
		let relay = SocketAddr::new(
			IpAddr::V4(Ipv4Addr::new(head[4], head[5], head[6], head[7])),
			u16::from_be_bytes([head[8], head[9]]),
		);

		let client = tokio::net::UdpSocket::bind("127.0.0.1:0")
			.await
			.expect("client");
		let mut packet = header(SocketAddrV4::new(REMOTE, REMOTE_UDP));
		packet.extend_from_slice(b"ping");
		client.send_to(&packet, relay).await.expect("send");
		let mut buffer = [0u8; 64];
		let (length, _) =
			tokio::time::timeout(Duration::from_secs(20), client.recv_from(&mut buffer))
				.await
				.expect("reply before deadline")
				.expect("receive");
		assert_eq!(&buffer[..length], &packet[..]);
	}

	#[test]
	fn udp_headers_round_trip() {
		let destination = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 2), 7000);
		let mut packet = header(destination);
		packet.extend_from_slice(b"payload");
		assert_eq!(
			parse_datagram(&packet),
			Some((destination, &b"payload"[..]))
		);
		assert!(parse_datagram(&[0, 0, 1, 1, 10, 0, 0, 2, 0, 7]).is_none());
		assert!(parse_datagram(&[0, 0, 0, 4, 0, 0, 0, 0, 0, 0]).is_none());
	}
}
