//! Session-only native routing. No discovery, implicit direct fallback, persistence or account access.
use model::protect::{Protocol, Scope};
use std::{
	io,
	net::SocketAddr,
	sync::{Arc, OnceLock, RwLock},
	time::Duration,
};
use tokio::{
	io::{AsyncReadExt, AsyncWriteExt},
	net::{TcpStream, UdpSocket},
};
use zeroize::Zeroizing;
#[cfg(test)]
mod wg_core;
mod wg_stack;
mod wireguard;
pub use wireguard::prepare;

const DEADLINE: Duration = Duration::from_secs(10);
pub struct Profile {
	pub protocol: Protocol,
	pub scope: Scope,
	pub host: String,
	pub port: u16,
	username: Zeroizing<String>,
	password: Zeroizing<String>,
	engine: Option<wireguard::Engine>,
}
impl std::fmt::Debug for Profile {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ProtectProfile")
			.field("protocol", &self.protocol)
			.field("scope", &self.scope)
			.field("credentials", &"[redacted]")
			.finish()
	}
}
impl Profile {
	pub fn new(
		protocol: Protocol,
		scope: Scope,
		host: &str,
		port: u16,
		username: &str,
		password: &str,
	) -> Result<Self, &'static str> {
		if protocol == Protocol::WireGuard {
			return Err(
				"WireGuard requires a configured local tunnel engine; this build cannot isolate its routes",
			);
		}
		if scope.needs_udp() && protocol != Protocol::Socks5 {
			return Err(
				"Live media requires SOCKS5 UDP; HTTP and SOCKS4 support videos and files only",
			);
		}
		if host.is_empty()
			|| host.len() > 253
			|| port == 0
			|| host
				.chars()
				.any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '@' | '?' | '#'))
			|| username.len() > 255
			|| password.len() > 255
			|| username.contains('\0')
			|| password.contains('\0')
		{
			return Err("Check server, port and credential lengths");
		}
		if protocol == Protocol::Socks4 && !password.is_empty() {
			return Err("SOCKS4 has no password authentication; use SOCKS5");
		}
		let profile = Self {
			protocol,
			scope,
			host: host.to_owned(),
			port,
			username: Zeroizing::new(username.to_owned()),
			password: Zeroizing::new(password.to_owned()),
			engine: None,
		};
		profile.url()?;
		Ok(profile)
	}
	fn url(&self) -> Result<url::Url, &'static str> {
		let mut url = url::Url::parse(&format!(
			"{}://localhost",
			self.protocol.scheme().ok_or("Unsupported route")?
		))
		.map_err(|_| "Invalid route")?;
		url.set_host(Some(&self.host))
			.map_err(|_| "Invalid server")?;
		url.set_port(Some(self.port)).map_err(|_| "Invalid port")?;
		url.set_username(&self.username)
			.map_err(|_| "Invalid username")?;
		url.set_password((!self.password.is_empty()).then_some(&*self.password))
			.map_err(|_| "Invalid password")?;
		Ok(url)
	}
}
#[derive(Default)]
struct Routes {
	generation: u64,
	primary: Option<Arc<Profile>>,
	reserve: Option<Arc<Profile>>,
	failures: u8,
}
fn routes() -> &'static RwLock<Routes> {
	static ROUTES: OnceLock<RwLock<Routes>> = OnceLock::new();
	ROUTES.get_or_init(|| RwLock::new(Routes::default()))
}
pub fn generation() -> u64 {
	routes().read().map(|r| r.generation).unwrap_or(0)
}
pub fn enabled() -> bool {
	routes().read().is_ok_and(|r| r.primary.is_some())
}
/// Scope of the live primary route, if one is applied. The UI shows PROTECT markers only for it.
pub fn active_scope() -> Option<Scope> {
	routes()
		.read()
		.ok()
		.and_then(|r| r.primary.as_ref().map(|p| p.scope))
}
pub fn has_fallback() -> bool {
	routes().read().is_ok_and(|r| r.reserve.is_some())
}
/// Only called after explicit Apply. Profiles are bounded to primary + one reserve.
pub fn apply(
	primary: Option<Arc<Profile>>,
	reserve: Option<Arc<Profile>>,
) -> Result<(), &'static str> {
	if reserve
		.as_ref()
		.is_some_and(|p| primary.as_ref().is_none_or(|a| a.scope != p.scope))
	{
		return Err("Fallback must use the same scope");
	}
	let mut state = routes().write().map_err(|_| "Route state unavailable")?;
	state.generation = state.generation.wrapping_add(1);
	state.primary = primary;
	state.reserve = reserve;
	state.failures = 0;
	Ok(())
}
pub fn selected(scope: Scope) -> Result<Option<Arc<Profile>>, &'static str> {
	let r = routes().read().map_err(|_| "Route state unavailable")?;
	Ok(r.primary
		.as_ref()
		.filter(|p| p.scope.contains(scope))
		.cloned())
}
pub fn ensure_current(expected: u64) -> Result<(), &'static str> {
	if generation() != expected {
		return Err("PROTECT route changed; reconnect before continuing");
	};
	routes()
		.read()
		.map_err(|_| "Route state unavailable")
		.map(|_| ())
}
/// Dynamic route selection prevents long-lived readers from keeping an old direct
/// connection after Apply. Idle pooling is disabled; in-flight sends are retired by the host.
pub fn http(builder: reqwest::ClientBuilder, scope: Scope) -> reqwest::ClientBuilder {
	builder
		.no_proxy()
		.pool_max_idle_per_host(0)
		.proxy(proxy(scope))
}
pub fn proxy(scope: Scope) -> reqwest::Proxy {
	reqwest::Proxy::custom(move |_| match selected(scope) {
		Ok(Some(p)) => Some(
			p.url()
				.unwrap_or_else(|_| url::Url::parse("http://127.0.0.1:1").expect("constant URL")),
		),
		Ok(None) => None,
		Err(_) => Some(url::Url::parse("http://127.0.0.1:1").expect("constant URL")),
	})
}
fn invalid() -> io::Error {
	io::Error::other("PROTECT proxy handshake failed")
}
async fn read_address(stream: &mut TcpStream) -> io::Result<SocketAddr> {
	let kind = stream.read_u8().await?;
	let ip = match kind {
		1 => std::net::IpAddr::V4(std::net::Ipv4Addr::from(stream.read_u32().await?)),
		4 => {
			let mut bytes = [0; 16];
			stream.read_exact(&mut bytes).await?;
			std::net::IpAddr::V6(bytes.into())
		}
		3 => {
			let len = stream.read_u8().await? as usize;
			let mut bytes = vec![0; len];
			stream.read_exact(&mut bytes).await?;
			let host = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
			let port = stream.read_u16().await?;
			return tokio::net::lookup_host((host, port))
				.await?
				.next()
				.ok_or_else(invalid);
		}
		_ => return Err(invalid()),
	};
	Ok(SocketAddr::new(ip, stream.read_u16().await?))
}
async fn socks5(
	stream: &mut TcpStream,
	p: &Profile,
	command: u8,
	host: &str,
	port: u16,
) -> io::Result<SocketAddr> {
	let auth = !p.username.is_empty() || !p.password.is_empty();
	stream
		.write_all(if auth { &[5, 1, 2] } else { &[5, 1, 0] })
		.await?;
	let mut reply = [0; 2];
	stream.read_exact(&mut reply).await?;
	if reply != [5, if auth { 2 } else { 0 }] {
		return Err(invalid());
	}
	if auth {
		let mut packet = Zeroizing::new(vec![1, p.username.len() as u8]);
		packet.extend_from_slice(p.username.as_bytes());
		packet.push(p.password.len() as u8);
		packet.extend_from_slice(p.password.as_bytes());
		stream.write_all(&packet).await?;
		stream.read_exact(&mut reply).await?;
		if reply != [1, 0] {
			return Err(invalid());
		}
	}
	let mut packet = vec![5, command, 0];
	match host.parse::<std::net::IpAddr>() {
		Ok(std::net::IpAddr::V4(ip)) => {
			packet.push(1);
			packet.extend_from_slice(&ip.octets());
		}
		Ok(std::net::IpAddr::V6(ip)) => {
			packet.push(4);
			packet.extend_from_slice(&ip.octets());
		}
		Err(_) => {
			if host.len() > 255 {
				return Err(invalid());
			}
			packet.extend_from_slice(&[3, host.len() as u8]);
			packet.extend_from_slice(host.as_bytes());
		}
	}
	packet.extend_from_slice(&port.to_be_bytes());
	stream.write_all(&packet).await?;
	let mut head = [0; 3];
	stream.read_exact(&mut head).await?;
	if head != [5, 0, 0] {
		return Err(invalid());
	}
	read_address(stream).await
}
async fn tunnel(p: &Profile, host: &str, port: u16) -> io::Result<TcpStream> {
	let mut stream = TcpStream::connect((p.host.as_str(), p.port)).await?;
	match p.protocol {
		Protocol::Socks5 => {
			socks5(&mut stream, p, 1, host, port).await?;
		}
		Protocol::Socks4 => {
			let mut packet = vec![4, 1];
			packet.extend_from_slice(&port.to_be_bytes());
			if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
				packet.extend_from_slice(&ip.octets());
			} else {
				packet.extend_from_slice(&[0, 0, 0, 1]);
			}
			packet.extend_from_slice(p.username.as_bytes());
			packet.push(0);
			if host.parse::<std::net::Ipv4Addr>().is_err() {
				packet.extend_from_slice(host.as_bytes());
				packet.push(0);
			}
			stream.write_all(&packet).await?;
			let mut reply = [0; 8];
			stream.read_exact(&mut reply).await?;
			if reply[0] != 0 || reply[1] != 90 {
				return Err(invalid());
			}
		}
		Protocol::Http => {
			if !p.username.is_empty() || !p.password.is_empty() {
				return Err(io::Error::other(
					"HTTP proxy credentials currently supported for file traffic only",
				));
			}
			let target = if host.contains(':') {
				format!("[{host}]:{port}")
			} else {
				format!("{host}:{port}")
			};
			stream
				.write_all(
					format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes(),
				)
				.await?;
			let mut response = Vec::with_capacity(128);
			while !response.ends_with(b"\r\n\r\n") {
				if response.len() >= 8192 {
					return Err(invalid());
				}
				response.push(stream.read_u8().await?);
			}
			let first = std::str::from_utf8(&response)
				.map_err(|_| invalid())?
				.lines()
				.next()
				.ok_or_else(invalid)?;
			if !first.split_whitespace().nth(1).is_some_and(|s| s == "200") {
				return Err(invalid());
			}
		}
		Protocol::WireGuard => return Err(invalid()),
	}
	Ok(stream)
}
async fn dial(scope: Scope, host: &str, port: u16) -> io::Result<TcpStream> {
	let epoch = generation();
	let profile = selected(scope).map_err(|_| invalid())?;
	let stream = match profile {
		Some(profile) => tokio::time::timeout(DEADLINE, tunnel(&profile, host, port))
			.await
			.map_err(|_| invalid())??,
		None => TcpStream::connect((host, port)).await?,
	};
	ensure_current(epoch).map_err(|_| invalid())?;
	Ok(stream)
}
pub async fn websocket(
	url: &str,
	config: Option<tokio_tungstenite::tungstenite::protocol::WebSocketConfig>,
	scope: Scope,
) -> Result<
	(
		tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
		tokio_tungstenite::tungstenite::handshake::client::Response,
	),
	tokio_tungstenite::tungstenite::Error,
> {
	let epoch = generation();
	let parsed =
		url::Url::parse(url).map_err(|_| tokio_tungstenite::tungstenite::Error::Io(invalid()))?;
	let host = parsed
		.host_str()
		.ok_or_else(|| tokio_tungstenite::tungstenite::Error::Io(invalid()))?;
	let port = parsed
		.port_or_known_default()
		.ok_or_else(|| tokio_tungstenite::tungstenite::Error::Io(invalid()))?;
	let stream = dial(scope, host, port).await?;
	let connected =
		tokio_tungstenite::client_async_tls_with_config(url, stream, config, None).await?;
	ensure_current(epoch).map_err(|_| tokio_tungstenite::tungstenite::Error::Io(invalid()))?;
	Ok(connected)
}

/// One UDP socket and a retained TCP association; SOCKS5 never sends media directly.
pub struct Datagram {
	socket: UdpSocket,
	association: Option<TcpStream>,
	destination: SocketAddr,
	epoch: Option<u64>,
}
impl Datagram {
	fn current(&self) -> io::Result<()> {
		if self.epoch.is_some_and(|e| ensure_current(e).is_err()) {
			return Err(io::Error::new(
				io::ErrorKind::ConnectionAborted,
				"PROTECT route changed; reconnect live media",
			));
		}
		Ok(())
	}
	pub async fn connect(destination: SocketAddr, scope: Scope) -> io::Result<Self> {
		let epoch = generation();
		let profile = selected(scope).map_err(|_| invalid())?;
		let mut socket = Self::connect_profile(destination, profile.as_deref()).await?;
		ensure_current(epoch).map_err(|_| invalid())?;
		socket.epoch = Some(epoch);
		Ok(socket)
	}
	async fn connect_profile(
		destination: SocketAddr,
		profile: Option<&Profile>,
	) -> io::Result<Self> {
		let Some(p) = profile else {
			let socket = UdpSocket::bind(if destination.is_ipv4() {
				"0.0.0.0:0"
			} else {
				"[::]:0"
			})
			.await?;
			socket.connect(destination).await?;
			return Ok(Self {
				socket,
				association: None,
				destination,
				epoch: None,
			});
		};
		if p.protocol != Protocol::Socks5 {
			return Err(invalid());
		}
		let setup = async {
			let mut association = TcpStream::connect((p.host.as_str(), p.port)).await?;
			let relay = socks5(&mut association, p, 3, "0.0.0.0", 0).await?;
			let relay = if relay.ip().is_unspecified() {
				SocketAddr::new(association.peer_addr()?.ip(), relay.port())
			} else {
				relay
			};
			if relay.port() == 0 {
				return Err(invalid());
			}
			let socket = UdpSocket::bind(if relay.is_ipv4() {
				"0.0.0.0:0"
			} else {
				"[::]:0"
			})
			.await?;
			socket.connect(relay).await?;
			Ok(Self {
				socket,
				association: Some(association),
				destination,
				epoch: None,
			})
		};
		tokio::time::timeout(DEADLINE, setup)
			.await
			.map_err(|_| invalid())?
	}
	fn packet(&self, data: &[u8]) -> io::Result<Vec<u8>> {
		if data.len() > 4096 {
			return Err(invalid());
		}
		let mut packet = Vec::with_capacity(data.len() + 22);
		packet.extend_from_slice(&[0, 0, 0]);
		match self.destination.ip() {
			std::net::IpAddr::V4(ip) => {
				packet.push(1);
				packet.extend_from_slice(&ip.octets());
			}
			std::net::IpAddr::V6(ip) => {
				packet.push(4);
				packet.extend_from_slice(&ip.octets());
			}
		}
		packet.extend_from_slice(&self.destination.port().to_be_bytes());
		packet.extend_from_slice(data);
		Ok(packet)
	}
	pub fn try_send(&self, data: &[u8]) -> io::Result<usize> {
		self.current()?;
		if self.association.is_none() {
			return self.socket.try_send(data);
		}
		let packet = self.packet(data)?;
		self.socket.try_send(&packet).map(|_| data.len())
	}
	pub async fn send(&self, data: &[u8]) -> io::Result<usize> {
		self.current()?;
		if self.association.is_none() {
			return self.socket.send(data).await;
		}
		let packet = self.packet(data)?;
		self.socket.send(&packet).await.map(|_| data.len())
	}
	pub async fn recv(&self, data: &mut [u8]) -> io::Result<usize> {
		self.current()?;
		if self.association.is_none() {
			let count = self.socket.recv(data).await?;
			self.current()?;
			return Ok(count);
		}
		let mut packet = [0; 4118];
		let count = self.socket.recv(&mut packet).await?;
		self.current()?;
		if count < 4 || packet[..3] != [0, 0, 0] {
			return Err(invalid());
		}
		let (ip, offset) = match packet[3] {
			1 if count >= 10 => (
				std::net::IpAddr::V4(std::net::Ipv4Addr::new(
					packet[4], packet[5], packet[6], packet[7],
				)),
				8,
			),
			4 if count >= 22 => {
				let mut ip = [0; 16];
				ip.copy_from_slice(&packet[4..20]);
				(std::net::IpAddr::V6(ip.into()), 20)
			}
			_ => return Err(invalid()),
		};
		let port = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
		if SocketAddr::new(ip, port) != self.destination || count - offset - 2 > data.len() {
			return Err(invalid());
		}
		let payload = &packet[offset + 2..count];
		data[..payload.len()].copy_from_slice(payload);
		Ok(payload.len())
	}
}

/// Unauthenticated route test; caller runs on its own bounded worker, never rendering.
// Each result states both transports, so a user sees which one failed.
const TCP_FAILED: &str = "TCP/TLS: failed · UDP: not tested";
const TCP_REJECTED: &str = "TCP/TLS: ok · Discord rejected the probe · UDP: not tested";
const UDP_NOT_NEEDED: &str =
	"TCP/TLS: ok · UDP: not needed for videos and files · No account credentials were sent";
const UDP_OK: &str = "TCP/TLS: ok · UDP: ok · Voice, camera and streams can use this route";
const UDP_ASSOCIATION: &str = "TCP/TLS: ok · UDP: SOCKS5 association failed";
const UDP_SEND: &str = "TCP/TLS: ok · UDP: send failed";
const UDP_TIMEOUT: &str = "TCP/TLS: ok · UDP: timed out";
const UDP_REJECTED_REPLY: &str = "TCP/TLS: ok · UDP: response rejected";
const UDP_INVALID_REPLY: &str = "TCP/TLS: ok · UDP: invalid response";

pub async fn test(profile: Arc<Profile>) -> Result<&'static str, &'static str> {
	let _ = rustls::crypto::ring::default_provider().install_default();
	let client = reqwest::Client::builder()
		.no_proxy()
		.proxy(reqwest::Proxy::all(profile.url()?).map_err(|_| "Proxy configuration failed")?)
		.redirect(reqwest::redirect::Policy::none())
		.retry(reqwest::retry::never())
		.timeout(DEADLINE)
		.build()
		.map_err(|_| "Proxy client failed")?;
	let response = client
		.get("https://discord.com/api/v10/gateway")
		.send()
		.await
		.map_err(|_| TCP_FAILED)?;
	if !response.status().is_success() {
		return Err(TCP_REJECTED);
	}
	if !profile.scope.needs_udp() {
		return Ok(UDP_NOT_NEEDED);
	}
	// A fixed public DNS endpoint proves a UDP payload round trip without
	// resolving a test hostname on the unprotected system DNS path.
	let target: SocketAddr = "1.1.1.1:53".parse().map_err(|_| UDP_ASSOCIATION)?;
	let socket = Datagram::connect_profile(target, Some(&profile))
		.await
		.map_err(|_| UDP_ASSOCIATION)?;
	let mut request = vec![
		0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c',
		b'o', b'm', 0, 0, 1, 0, 1,
	];
	getrandom::fill(&mut request[..2]).map_err(|_| UDP_SEND)?;
	socket.send(&request).await.map_err(|_| UDP_SEND)?;
	let mut reply = [0; 512];
	let n = tokio::time::timeout(DEADLINE, socket.recv(&mut reply))
		.await
		.map_err(|_| UDP_TIMEOUT)?
		.map_err(|_| UDP_REJECTED_REPLY)?;
	if n < 12 || reply[..2] != request[..2] || reply[2] & 0x80 == 0 || reply[3] & 0x0f != 0 {
		return Err(UDP_INVALID_REPLY);
	}
	Ok(UDP_OK)
}

/// Host calls on one periodic worker. Two consecutive failed probes are required;
/// reserve must pass its own capability test before replacing the active route.
/// Writes are never replayed. Live media must reconnect after a successful swap.
pub enum Health {
	Healthy(&'static str),
	Swap(Arc<Profile>, Option<Arc<Profile>>),
}
pub async fn check_health() -> Result<Health, &'static str> {
	let (primary, reserve, epoch) = {
		let r = routes().read().map_err(|_| "Route state unavailable")?;
		(
			r.primary.clone().ok_or("PROTECT is disabled")?,
			r.reserve.clone(),
			r.generation,
		)
	};
	if test(primary.clone()).await.is_ok() {
		let mut r = routes().write().map_err(|_| "Route state unavailable")?;
		if r.generation == epoch {
			r.failures = 0
		}
		return Ok(Health::Healthy("Protected route healthy"));
	}
	{
		let mut r = routes().write().map_err(|_| "Route state unavailable")?;
		if r.generation != epoch {
			return Err("Route changed during health test");
		}
		r.failures = r.failures.saturating_add(1);
		if r.failures < 2 {
			return Err("Protected route degraded; retrying health test");
		}
	}
	let reserve = reserve.ok_or("Protected route unavailable; direct fallback is blocked")?;
	test(reserve.clone())
		.await
		.map_err(|_| "Both protected profiles failed; direct fallback is blocked")?;
	let r = routes().read().map_err(|_| "Route state unavailable")?;
	if r.generation != epoch {
		return Err("Route changed during fallback test");
	}
	Ok(Health::Swap(reserve, Some(primary)))
}

#[cfg(test)]
mod tests {
	use super::*;
	use tokio::net::TcpListener;
	#[test]
	fn profiles_reject_invalid_and_incompatible_inputs() {
		assert!(Profile::new(Protocol::Http, Scope::Voice, "localhost", 8080, "", "").is_err());
		assert!(Profile::new(Protocol::Socks5, Scope::Videos, "user@host", 1080, "", "").is_err());
		assert!(Profile::new(Protocol::Socks5, Scope::Videos, "[", 1080, "", "").is_err());
		let profile = Profile::new(
			Protocol::Socks5,
			Scope::Voice,
			"localhost",
			1080,
			"private-user",
			"private-pass",
		)
		.unwrap();
		assert!(!format!("{profile:?}").contains("private"));
	}
	#[tokio::test]
	async fn socks4_tcp_reaches_destination_through_proxy() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let port = listener.local_addr().unwrap().port();
		let task = tokio::spawn(async move {
			let (mut socket, _) = listener.accept().await.unwrap();
			let mut head = [0; 8];
			socket.read_exact(&mut head).await.unwrap();
			assert_eq!(head, [4, 1, 1, 187, 0, 0, 0, 1]);
			let mut user = [0; 6];
			socket.read_exact(&mut user).await.unwrap();
			assert_eq!(&user, b"alice\0");
			let mut host = [0; 16];
			socket.read_exact(&mut host).await.unwrap();
			assert_eq!(&host, b"example.invalid\0");
			socket.write_all(&[0, 90, 0, 0, 0, 0, 0, 0]).await.unwrap();
			socket.write_all(b"relayed").await.unwrap();
		});
		let p = Profile::new(
			Protocol::Socks4,
			Scope::Videos,
			"127.0.0.1",
			port,
			"alice",
			"",
		)
		.unwrap();
		let mut socket = tunnel(&p, "example.invalid", 443).await.unwrap();
		let mut reply = [0; 7];
		socket.read_exact(&mut reply).await.unwrap();
		assert_eq!(&reply, b"relayed");
		task.await.unwrap();
	}
	#[tokio::test]
	async fn http_connect_is_bounded_and_checks_status() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let port = listener.local_addr().unwrap().port();
		let task = tokio::spawn(async move {
			let (mut socket, _) = listener.accept().await.unwrap();
			let mut request = Vec::new();
			while !request.ends_with(b"\r\n\r\n") {
				request.push(socket.read_u8().await.unwrap());
				assert!(request.len() < 512)
			}
			assert!(request.starts_with(b"CONNECT example.invalid:443 HTTP/1.1\r\n"));
			socket
				.write_all(b"HTTP/1.1 200 OK\r\n\r\nrelayed")
				.await
				.unwrap();
		});
		let p = Profile::new(Protocol::Http, Scope::Videos, "127.0.0.1", port, "", "").unwrap();
		let mut socket = tunnel(&p, "example.invalid", 443).await.unwrap();
		let mut reply = [0; 7];
		socket.read_exact(&mut reply).await.unwrap();
		assert_eq!(&reply, b"relayed");
		task.await.unwrap();
	}
	async fn handshake(socket: &mut TcpStream, command: u8, relay: SocketAddr) {
		let mut greeting = [0; 3];
		socket.read_exact(&mut greeting).await.unwrap();
		assert_eq!(greeting, [5, 1, 0]);
		socket.write_all(&[5, 0]).await.unwrap();
		let mut header = [0; 4];
		socket.read_exact(&mut header).await.unwrap();
		assert_eq!(&header[..3], &[5, command, 0]);
		match header[3] {
			1 => {
				let mut tail = [0; 6];
				socket.read_exact(&mut tail).await.unwrap();
			}
			3 => {
				let len = socket.read_u8().await.unwrap();
				let mut tail = vec![0; len as usize + 2];
				socket.read_exact(&mut tail).await.unwrap();
			}
			_ => panic!("unexpected target"),
		};
		let mut response = vec![5, 0, 0, 1];
		let std::net::IpAddr::V4(ip) = relay.ip() else {
			panic!()
		};
		response.extend_from_slice(&ip.octets());
		response.extend_from_slice(&relay.port().to_be_bytes());
		socket.write_all(&response).await.unwrap();
	}
	#[tokio::test]
	async fn socks5_udp_round_trip_preserves_datagram_and_association() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let port = listener.local_addr().unwrap().port();
		let relay = UdpSocket::bind("127.0.0.1:0").await.unwrap();
		let relay_address = relay.local_addr().unwrap();
		let destination: SocketAddr = "192.0.2.12:50000".parse().unwrap();
		let task = tokio::spawn(async move {
			let (mut control, _) = listener.accept().await.unwrap();
			handshake(&mut control, 3, relay_address).await;
			let mut packet = [0; 64];
			let (count, from) = relay.recv_from(&mut packet).await.unwrap();
			assert_eq!(&packet[..10], &[0, 0, 0, 1, 192, 0, 2, 12, 195, 80]);
			assert_eq!(&packet[10..count], b"synthetic");
			relay.send_to(&packet[..count], from).await.unwrap();
			let mut byte = [0];
			assert_eq!(control.read(&mut byte).await.unwrap(), 0);
		});
		let p = Profile::new(Protocol::Socks5, Scope::Voice, "127.0.0.1", port, "", "").unwrap();
		let socket = Datagram::connect_profile(destination, Some(&p))
			.await
			.unwrap();
		socket.send(b"synthetic").await.unwrap();
		let mut reply = [0; 64];
		let n = socket.recv(&mut reply).await.unwrap();
		assert_eq!(&reply[..n], b"synthetic");
		drop(socket);
		task.await.unwrap();
	}
	#[tokio::test]
	async fn socks5_tcp_and_reserve_never_dial_direct() {
		let _ = rustls::crypto::ring::default_provider().install_default();
		let udp_receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
		let old_direct = Datagram::connect(udp_receiver.local_addr().unwrap(), Scope::Complete)
			.await
			.unwrap();
		let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let direct = destination.local_addr().unwrap();
		let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let bad = closed.local_addr().unwrap().port();
		drop(closed);
		let reserve = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let port = reserve.local_addr().unwrap().port();
		let task = tokio::spawn(async move {
			let (mut socket, _) = reserve.accept().await.unwrap();
			handshake(&mut socket, 1, direct).await;
			socket.write_all(b"reserve").await.unwrap();
		});
		let primary = Arc::new(
			Profile::new(Protocol::Socks5, Scope::Complete, "127.0.0.1", bad, "", "").unwrap(),
		);
		let reserve = Arc::new(
			Profile::new(Protocol::Socks5, Scope::Complete, "127.0.0.1", port, "", "").unwrap(),
		);
		apply(Some(primary), Some(reserve.clone())).unwrap();
		assert!(old_direct.send(b"must not leak").await.is_err());
		assert!(
			dial(Scope::Complete, "127.0.0.1", direct.port())
				.await
				.is_err()
		);
		// Only the host promotes a tested reserve after retiring old transports.
		let before = generation();
		apply(Some(reserve), None).unwrap();
		assert_ne!(generation(), before);
		let mut socket = dial(Scope::Complete, "127.0.0.1", direct.port())
			.await
			.unwrap();
		let mut reply = [0; 7];
		socket.read_exact(&mut reply).await.unwrap();
		assert_eq!(&reply, b"reserve");
		task.await.unwrap();
		assert!(
			tokio::time::timeout(Duration::from_millis(50), destination.accept())
				.await
				.is_err()
		);
		apply(
			Some(Arc::new(
				Profile::new(Protocol::Socks5, Scope::Complete, "127.0.0.1", bad, "", "").unwrap(),
			)),
			None,
		)
		.unwrap();
		assert!(
			dial(Scope::Complete, "127.0.0.1", direct.port())
				.await
				.is_err()
		);
		assert!(
			tokio::time::timeout(Duration::from_millis(50), destination.accept())
				.await
				.is_err()
		);
		let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let proxy_port = proxy.local_addr().unwrap().port();
		let server = tokio::spawn(async move {
			let (mut socket, _) = proxy.accept().await.unwrap();
			let mut request = Vec::new();
			while !request.ends_with(b"\r\n\r\n") {
				request.push(socket.read_u8().await.unwrap());
				assert!(request.len() < 2048)
			}
			let text = String::from_utf8(request).unwrap().to_ascii_lowercase();
			assert!(
				text.contains(
					"proxy-authorization: basic dxnlcjpwYXNz"
						.to_ascii_lowercase()
						.as_str()
				)
			);
			socket
				.write_all(
					b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nproxy!",
				)
				.await
				.unwrap();
		});
		apply(
			Some(Arc::new(
				Profile::new(
					Protocol::Http,
					Scope::Videos,
					"127.0.0.1",
					proxy_port,
					"user",
					"pass",
				)
				.unwrap(),
			)),
			None,
		)
		.unwrap();
		let client = http(reqwest::Client::builder(), Scope::Videos)
			.build()
			.unwrap();
		let result = client
			.get(format!("http://127.0.0.1:{}/fixture", direct.port()))
			.send()
			.await
			.unwrap()
			.text()
			.await
			.unwrap();
		assert_eq!(result, "proxy!");
		server.await.unwrap();
		assert!(
			tokio::time::timeout(Duration::from_millis(50), destination.accept())
				.await
				.is_err()
		);
		apply(None, None).unwrap();
	}
}
