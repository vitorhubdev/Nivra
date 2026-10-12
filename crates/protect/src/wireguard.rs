//! Optional user-selected sing-box engine. Configuration goes through stdin, never disk/argv.
use super::{DEADLINE, Profile};
use model::protect::{Draft, Protocol};
use std::{
	process::Stdio,
	sync::{
		Arc, Mutex, OnceLock,
		atomic::{AtomicU8, Ordering},
	},
};
use tokio::{
	io::AsyncWriteExt,
	process::{Child, Command},
	sync::mpsc::{Sender, channel},
};
use zeroize::Zeroizing;

static ENGINE_SLOTS: AtomicU8 = AtomicU8::new(0);
struct OwnedChild(Child);
impl Drop for OwnedChild {
	fn drop(&mut self) {
		ENGINE_SLOTS.fetch_sub(1, Ordering::AcqRel);
	}
}
fn reaper() -> &'static Sender<OwnedChild> {
	static REAPER: OnceLock<Sender<OwnedChild>> = OnceLock::new();
	REAPER.get_or_init(|| {
		let (tx, mut rx) = channel::<OwnedChild>(4);
		tokio::spawn(async move {
			while let Some(mut child) = rx.recv().await {
				let _ = child.0.start_kill();
				let _ = child.0.wait().await;
			}
		});
		tx
	})
}
pub(super) struct Engine(Mutex<Option<OwnedChild>>);
impl Drop for Engine {
	fn drop(&mut self) {
		if let Ok(child) = self.0.get_mut()
			&& let Some(mut child) = child.take()
		{
			let _ = child.0.start_kill(); // Terminate now; the bounded reaper waits outside rendering.
			// Four total engine slots include queued/being-reaped children, so this
			// four-slot queue cannot be full while submitting one of those slots.
			if let Err(error) = reaper().try_send(child) {
				let mut child = error.into_inner();
				let _ = child.0.start_kill();
			}
		}
	}
}
struct Config {
	private: Zeroizing<String>,
	public: String,
	psk: Zeroizing<String>,
	address: String,
	allowed: String,
	host: String,
	port: u16,
	keepalive: u16,
}
fn key(value: &str) -> bool {
	value.len() == 44
		&& value.ends_with('=')
		&& value[..43]
			.bytes()
			.all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
}
fn parse(text: &str) -> Result<Config, &'static str> {
	if text.len() > 4096 {
		return Err("WireGuard configuration exceeds 4 KiB");
	}
	let mut config = Config {
		private: Zeroizing::new(String::new()),
		public: String::new(),
		psk: Zeroizing::new(String::new()),
		address: String::new(),
		allowed: String::new(),
		host: String::new(),
		port: 0,
		keepalive: 0,
	};
	let mut section = "";
	let mut peers = 0;
	for raw in text.lines() {
		let line = raw.split('#').next().unwrap_or("").trim();
		if line.is_empty() {
			continue;
		}
		if line.starts_with('[') {
			section = line;
			match section {
				"[Interface]" => {}
				"[Peer]" => {
					peers += 1;
					if peers > 1 {
						return Err("Only one WireGuard peer is supported");
					}
				}
				_ => return Err("Unsupported WireGuard section"),
			};
			continue;
		}
		let (name, value) = line
			.split_once('=')
			.ok_or("Malformed WireGuard configuration")?;
		let value = value.trim();
		match (section, name.trim()) {
			("[Interface]", "PrivateKey") => *config.private = value.to_owned(),
			("[Interface]", "Address") => config.address = value.to_owned(),
			("[Peer]", "PublicKey") => config.public = value.to_owned(),
			("[Peer]", "PresharedKey") => *config.psk = value.to_owned(),
			("[Peer]", "AllowedIPs") => config.allowed = value.to_owned(),
			("[Peer]", "Endpoint") => {
				let endpoint = url::Url::parse(&format!("http://{value}"))
					.map_err(|_| "Invalid WireGuard endpoint")?;
				if !endpoint.username().is_empty()
					|| endpoint.password().is_some()
					|| endpoint.path() != "/"
					|| endpoint.query().is_some()
					|| endpoint.fragment().is_some()
				{
					return Err("Invalid WireGuard endpoint");
				}
				config.host = endpoint
					.host_str()
					.ok_or("Missing WireGuard host")?
					.trim_matches(['[', ']'])
					.to_owned();
				config.port = endpoint.port().ok_or("Missing WireGuard port")?;
			}
			("[Peer]", "PersistentKeepalive") => {
				config.keepalive = value.parse().map_err(|_| "Invalid keepalive")?
			}
			// Imported DNS does not modify the OS: engine DNS is explicit below.
			("[Interface]", "DNS") | ("[Interface]", "MTU") => {}
			_ => {
				return Err(
					"Unsupported WireGuard field; hooks and system routes are never executed",
				);
			}
		}
	}
	if peers != 1
		|| !key(&config.private)
		|| !key(&config.public)
		|| (!config.psk.is_empty() && !key(&config.psk))
		|| config.host.is_empty()
		|| config.port == 0
	{
		return Err("WireGuard needs valid keys, address and one peer endpoint");
	}
	for values in [&config.address, &config.allowed] {
		let mut count = 0;
		for value in values.split(',') {
			count += 1;
			let (ip, prefix) = value
				.trim()
				.split_once('/')
				.ok_or("WireGuard addresses require CIDR prefixes")?;
			let ip: std::net::IpAddr = ip.parse().map_err(|_| "Invalid WireGuard address")?;
			let prefix: u8 = prefix.parse().map_err(|_| "Invalid prefix")?;
			if prefix > if ip.is_ipv4() { 32 } else { 128 } || count > 8 {
				return Err("Invalid or excessive WireGuard addresses");
			}
		}
	}
	Ok(config)
}
#[derive(serde::Serialize)]
struct EngineConfig<'a> {
	log: Log,
	inbounds: [Inbound<'a>; 1],
	endpoints: [Endpoint<'a>; 1],
	dns: Dns,
	route: Route,
}
#[derive(serde::Serialize)]
struct Log {
	disabled: bool,
}
#[derive(serde::Serialize)]
struct Inbound<'a> {
	#[serde(rename = "type")]
	kind: &'static str,
	listen: &'static str,
	listen_port: u16,
	users: [User<'a>; 1],
}
#[derive(serde::Serialize)]
struct User<'a> {
	username: &'a str,
	password: &'a str,
}
#[derive(serde::Serialize)]
struct Endpoint<'a> {
	#[serde(rename = "type")]
	kind: &'static str,
	tag: &'static str,
	system: bool,
	address: Vec<&'a str>,
	private_key: &'a str,
	peers: [Peer<'a>; 1],
}
#[derive(serde::Serialize)]
struct Peer<'a> {
	address: &'a str,
	port: u16,
	public_key: &'a str,
	pre_shared_key: &'a str,
	allowed_ips: Vec<&'a str>,
	persistent_keepalive_interval: u16,
}
#[derive(serde::Serialize)]
struct Dns {
	servers: [DnsServer; 1],
	#[serde(rename = "final")]
	final_server: &'static str,
}
#[derive(serde::Serialize)]
struct DnsServer {
	#[serde(rename = "type")]
	kind: &'static str,
	tag: &'static str,
	server: &'static str,
	detour: &'static str,
}
#[derive(serde::Serialize)]
struct Route {
	#[serde(rename = "final")]
	outbound: &'static str,
	default_domain_resolver: &'static str,
}
fn serialize(
	config: &Config,
	port: u16,
	username: &str,
	password: &str,
) -> Result<Zeroizing<Vec<u8>>, &'static str> {
	let input = EngineConfig {
		log: Log { disabled: true },
		inbounds: [Inbound {
			kind: "socks",
			listen: "127.0.0.1",
			listen_port: port,
			users: [User { username, password }],
		}],
		endpoints: [Endpoint {
			kind: "wireguard",
			tag: "protect-wg",
			system: false,
			address: config.address.split(',').map(str::trim).collect(),
			private_key: &config.private,
			peers: [Peer {
				address: &config.host,
				port: config.port,
				public_key: &config.public,
				pre_shared_key: &config.psk,
				allowed_ips: config.allowed.split(',').map(str::trim).collect(),
				persistent_keepalive_interval: config.keepalive,
			}],
		}],
		dns: Dns {
			servers: [DnsServer {
				kind: "udp",
				tag: "protected-dns",
				server: "1.1.1.1",
				detour: "protect-wg",
			}],
			final_server: "protected-dns",
		},
		route: Route {
			outbound: "protect-wg",
			default_domain_resolver: "protected-dns",
		},
	};
	serde_json::to_vec(&input)
		.map(Zeroizing::new)
		.map_err(|_| "WireGuard config serialization failed")
}
pub async fn prepare(draft: &Draft) -> Result<Arc<Profile>, &'static str> {
	if draft.protocol != Protocol::WireGuard {
		return Profile::new(
			draft.protocol,
			draft.scope,
			&draft.host,
			draft.port,
			&draft.username,
			&draft.password,
		)
		.map(Arc::new);
	}
	let mut config = parse(&draft.wireguard)?;
	// Bootstrap only the peer's endpoint before the tunnel exists; all service
	// name resolution then goes through the WireGuard DNS detour.
	if config.host.parse::<std::net::IpAddr>().is_err() {
		let host = config.host.clone();
		let resolved = tokio::time::timeout(
			DEADLINE,
			tokio::net::lookup_host((host.as_str(), config.port)),
		)
		.await
		.map_err(|_| "WireGuard endpoint DNS timed out")?
		.map_err(|_| "WireGuard endpoint DNS failed")?
		.next()
		.ok_or("WireGuard endpoint DNS empty")?
		.ip()
		.to_string();
		config.host = resolved;
	}
	let path = std::path::Path::new(&draft.engine);
	if !path.is_absolute() || !path.is_file() {
		return Err("Select an installed sing-box 1.14 executable by absolute path");
	}
	let listener = std::net::TcpListener::bind("127.0.0.1:0")
		.map_err(|_| "Could not reserve local engine port")?;
	let port = listener
		.local_addr()
		.map_err(|_| "Could not reserve local engine port")?
		.port();
	let mut random = [0u8; 32];
	getrandom::fill(&mut random).map_err(|_| "Random source unavailable")?;
	let password = Zeroizing::new(
		random
			.iter()
			.map(|b| format!("{b:02x}"))
			.collect::<String>(),
	);
	zeroize::Zeroize::zeroize(&mut random);
	let serialized = serialize(&config, port, "nivra", &password)?;
	let mut command = Command::new(path);
	command
		.args(["run", "-c", "stdin"])
		.kill_on_drop(true)
		.stdin(Stdio::piped())
		.stdout(Stdio::null())
		.stderr(Stdio::null());
	#[cfg(windows)]
	{
		command.creation_flags(0x08000000);
	}
	drop(listener);
	ENGINE_SLOTS
		.fetch_update(Ordering::AcqRel, Ordering::Acquire, |slots| {
			(slots < 4).then_some(slots + 1)
		})
		.map_err(|_| "Wait for previous WireGuard engines to stop")?;
	let _ = reaper();
	let child = match command.spawn() {
		Ok(child) => child,
		Err(_) => {
			ENGINE_SLOTS.fetch_sub(1, Ordering::AcqRel);
			return Err("Could not start selected WireGuard engine");
		}
	};
	let engine = Engine(Mutex::new(Some(OwnedChild(child))));
	let mut stdin = {
		let mut child = engine.0.lock().map_err(|_| "Engine state unavailable")?;
		child
			.as_mut()
			.ok_or("Engine unavailable")?
			.0
			.stdin
			.take()
			.ok_or("Engine stdin unavailable")?
	};
	tokio::time::timeout(DEADLINE, stdin.write_all(&serialized))
		.await
		.map_err(|_| "Engine configuration handoff timed out")?
		.map_err(|_| "Engine rejected session configuration")?;
	drop(stdin);
	let profile = Arc::new(Profile {
		protocol: Protocol::Socks5,
		scope: draft.scope,
		host: "127.0.0.1".into(),
		port,
		username: Zeroizing::new("nivra".into()),
		password,
		engine: Some(engine),
	});
	let start = tokio::time::Instant::now();
	while start.elapsed() < DEADLINE {
		if profile
			.engine
			.as_ref()
			.ok_or("Engine unavailable")?
			.0
			.lock()
			.map_err(|_| "Engine state unavailable")?
			.as_mut()
			.ok_or("Engine unavailable")?
			.0
			.try_wait()
			.map_err(|_| "Engine state unavailable")?
			.is_some()
		{
			return Err(
				"WireGuard engine rejected configuration; use sing-box 1.14 with WireGuard support",
			);
		}
		if let Ok(mut socket) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
			use tokio::io::{AsyncReadExt, AsyncWriteExt};
			let probe = async {
				socket.write_all(&[5, 1, 2]).await?;
				let mut reply = [0; 2];
				socket.read_exact(&mut reply).await?;
				Ok::<bool, std::io::Error>(reply == [5, 2])
			};
			if matches!(
				tokio::time::timeout(std::time::Duration::from_millis(200), probe).await,
				Ok(Ok(true))
			) {
				return Ok(profile);
			}
		}
		tokio::time::sleep(std::time::Duration::from_millis(50)).await;
	}
	Err("WireGuard engine did not expose its authenticated local route")
}
#[cfg(test)]
mod tests {
	use super::*;
	fn fixture() -> String {
		format!(
			"[Interface]\nPrivateKey = {}\nAddress = 10.0.0.2/32\n[Peer]\nPublicKey = {}\nEndpoint = 192.0.2.1:51820\nAllowedIPs = 0.0.0.0/0\n",
			"A".repeat(43) + "=",
			"B".repeat(43) + "="
		)
	}
	#[test]
	fn import_is_bounded_and_never_accepts_hooks() {
		assert!(parse(&fixture()).is_ok());
		assert!(parse(&(fixture() + "PostUp = malicious\n")).is_err());
		assert!(parse(&"x".repeat(4097)).is_err());
	}
	#[test]
	fn engine_has_no_system_interface_or_direct_fallback() {
		let parsed = parse(&fixture()).unwrap();
		let bytes = serialize(&parsed, 1080, "local-user", "local-password").unwrap();
		let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
		assert_eq!(json["endpoints"][0]["system"], false);
		assert_eq!(json["route"]["final"], "protect-wg");
		assert_eq!(json["inbounds"][0]["listen"], "127.0.0.1");
		assert_eq!(
			json["inbounds"][0]["users"][0]["password"],
			"local-password"
		);
		assert!(json.get("outbounds").is_none());
	}
}
