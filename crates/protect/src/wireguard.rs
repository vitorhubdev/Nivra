//! WireGuard profiles. The configuration is validated here and run by the in-process engine
//! (`wg_engine`) behind a loopback SOCKS5 inlet (`wg_inlet`). No process is spawned, and no
//! system interface, route or DNS setting is changed.
use super::{
	DEADLINE, Profile,
	wg_engine::{Cidr, Engine, Params},
	wg_inlet::{self, Credentials},
};
use base64::Engine as _;
use model::protect::{Draft, Protocol};
use std::{
	net::{IpAddr, Ipv4Addr, SocketAddr},
	sync::Arc,
};
use tokio::net::{TcpListener, lookup_host};
use zeroize::Zeroizing;

const USERNAME: &str = "nivra";
const MTU_RANGE: std::ops::RangeInclusive<u16> = 576..=1500;
const MTU_DEFAULT: usize = 1420;
const MAX_ALLOWED: usize = 8;

struct Config {
	private: Zeroizing<String>,
	public: String,
	psk: Zeroizing<String>,
	address: String,
	allowed: String,
	host: String,
	port: u16,
	keepalive: u16,
	mtu: u16,
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
		mtu: MTU_DEFAULT as u16,
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
			("[Interface]", "MTU") => {
				config.mtu = value
					.parse::<u16>()
					.ok()
					.filter(|mtu| MTU_RANGE.contains(mtu))
					.ok_or("WireGuard MTU must be between 576 and 1500")?;
			}
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
			// DNS is not applied to the system: names resolve through the tunnel to a fixed resolver.
			("[Interface]", "DNS") => {}
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
	if config.address.contains(',') {
		return Err("WireGuard supports one IPv4 interface address");
	}
	let (ip, prefix) = config
		.address
		.trim()
		.split_once('/')
		.ok_or("WireGuard addresses require CIDR prefixes")?;
	if ip.parse::<Ipv4Addr>().is_err() || prefix.parse::<u8>().map_or(true, |p| p > 32) {
		return Err("WireGuard interface address must be IPv4");
	}
	if config.allowed.split(',').count() > MAX_ALLOWED {
		return Err("Too many WireGuard AllowedIPs");
	}
	for value in config.allowed.split(',') {
		if Cidr::parse(value).is_none() {
			return Err("WireGuard AllowedIPs must be IPv4 networks");
		}
	}
	Ok(config)
}

impl Config {
	fn params(&self) -> Result<Params, &'static str> {
		let (ip, _) = self
			.address
			.trim()
			.split_once('/')
			.ok_or("WireGuard addresses require CIDR prefixes")?;
		let address = ip
			.parse::<Ipv4Addr>()
			.map_err(|_| "WireGuard interface address must be IPv4")?;
		let allowed = self
			.allowed
			.split(',')
			.map(|value| Cidr::parse(value).ok_or("WireGuard AllowedIPs must be IPv4 networks"))
			.collect::<Result<Vec<_>, _>>()?;
		Ok(Params {
			secret: x25519_secret(&decode_key(&self.private)?),
			peer: x25519_public(&decode_key(&self.public)?),
			preshared: if self.psk.is_empty() {
				None
			} else {
				Some(decode_key(&self.psk)?)
			},
			keepalive: (self.keepalive != 0).then_some(self.keepalive),
			address,
			mtu: usize::from(self.mtu),
			allowed,
		})
	}
}

fn decode_key(value: &str) -> Result<[u8; 32], &'static str> {
	let bytes = Zeroizing::new(
		base64::engine::general_purpose::STANDARD
			.decode(value)
			.map_err(|_| "Invalid WireGuard key")?,
	);
	<[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| "Invalid WireGuard key")
}

fn x25519_secret(bytes: &[u8; 32]) -> boringtun::x25519::StaticSecret {
	boringtun::x25519::StaticSecret::from(*bytes)
}

fn x25519_public(bytes: &[u8; 32]) -> boringtun::x25519::PublicKey {
	boringtun::x25519::PublicKey::from(*bytes)
}

/// The WireGuard endpoint is the only name resolved outside the tunnel, because the tunnel
/// cannot exist before the endpoint does. Every service name resolves through the tunnel.
async fn endpoint(config: &Config) -> Result<SocketAddr, &'static str> {
	if let Ok(ip) = config.host.parse::<IpAddr>() {
		return Ok(SocketAddr::new(ip, config.port));
	}
	tokio::time::timeout(DEADLINE, lookup_host((config.host.as_str(), config.port)))
		.await
		.map_err(|_| "WireGuard endpoint DNS timed out")?
		.map_err(|_| "WireGuard endpoint DNS failed")?
		.next()
		.ok_or("WireGuard endpoint DNS empty")
}

fn session_password() -> Result<Zeroizing<String>, &'static str> {
	let mut random = [0u8; 32];
	getrandom::fill(&mut random).map_err(|_| "Random source unavailable")?;
	let password = Zeroizing::new(
		random
			.iter()
			.map(|b| format!("{b:02x}"))
			.collect::<String>(),
	);
	zeroize::Zeroize::zeroize(&mut random);
	Ok(password)
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
	let config = parse(&draft.wireguard)?;
	let endpoint = endpoint(&config).await?;
	let mut engine = Engine::start(config.params()?, endpoint).await?;
	let listener = TcpListener::bind("127.0.0.1:0")
		.await
		.map_err(|_| "Could not reserve local route port")?;
	let port = listener
		.local_addr()
		.map_err(|_| "Could not reserve local route port")?
		.port();
	let password = session_password()?;
	let handle = engine.handle();
	engine.own(tokio::spawn(wg_inlet::serve(
		listener,
		handle,
		Credentials::new(USERNAME, &password),
	)));
	Ok(Arc::new(Profile {
		protocol: Protocol::Socks5,
		scope: draft.scope,
		host: "127.0.0.1".into(),
		port,
		username: Zeroizing::new(USERNAME.into()),
		password,
		engine: Some(engine),
	}))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn fixture() -> String {
		format!(
			"[Interface]\nPrivateKey = {}\nAddress = 10.0.0.2/32\n[Peer]\nPublicKey = {}\nEndpoint = 192.0.2.1:51820\nAllowedIPs = 0.0.0.0/0\n",
			"A".repeat(43) + "=",
			"A".repeat(43) + "="
		)
	}

	#[test]
	fn import_is_bounded_and_never_accepts_hooks() {
		assert!(parse(&fixture()).is_ok());
		assert!(parse(&(fixture() + "PostUp = malicious\n")).is_err());
		assert!(parse(&"x".repeat(4097)).is_err());
	}

	#[test]
	fn ipv6_networks_and_addresses_are_refused() {
		let allowed_v6 = fixture().replace("0.0.0.0/0", "::/0");
		assert!(parse(&allowed_v6).is_err());
		let address_v6 = fixture().replace("10.0.0.2/32", "fd00::2/128");
		assert!(parse(&address_v6).is_err());
	}

	#[test]
	fn mtu_is_applied_within_bounds() {
		let with_mtu = |mtu: &str| fixture().replace("[Peer]", &format!("MTU = {mtu}\n[Peer]"));
		assert_eq!(parse(&with_mtu("1280")).unwrap().mtu, 1280);
		assert!(parse(&with_mtu("300")).is_err());
		assert!(parse(&with_mtu("9000")).is_err());
		assert_eq!(parse(&fixture()).unwrap().mtu, 1420);
	}

	#[test]
	fn engine_parameters_carry_the_validated_fields() {
		let params = parse(&fixture()).unwrap().params().unwrap();
		assert_eq!(params.address, Ipv4Addr::new(10, 0, 0, 2));
		assert_eq!(params.mtu, 1420);
		assert_eq!(params.allowed.len(), 1);
		assert!(params.preshared.is_none());
	}

	#[tokio::test]
	async fn prepare_opens_only_a_loopback_inlet() {
		let mut config = fixture();
		config = config.replace("192.0.2.1:51820", "127.0.0.1:9");
		let draft = Draft {
			protocol: Protocol::WireGuard,
			scope: model::protect::Scope::Complete,
			wireguard: Zeroizing::new(config),
			..Draft::default()
		};
		let profile = prepare(&draft).await.expect("prepare");
		assert_eq!(profile.host, "127.0.0.1");
		assert!(profile.port != 0);
		assert!(profile.engine.is_some());
		let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", profile.port))
			.await
			.expect("inlet");
		use tokio::io::{AsyncReadExt, AsyncWriteExt};
		stream.write_all(&[5, 1, 2]).await.expect("greet");
		let mut method = [0u8; 2];
		stream.read_exact(&mut method).await.expect("method");
		assert_eq!(method, [5, 2]);
	}
}
