//! Native route choices. Credentials never belong in serialized preferences.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
	#[default]
	Videos,
	Voice,
	Streams,
	Complete,
}
impl Scope {
	pub const ALL: [Self; 4] = [Self::Videos, Self::Voice, Self::Streams, Self::Complete];
	pub fn label(self) -> &'static str {
		match self {
			Self::Videos => "Videos and files",
			Self::Voice => "Voice and camera",
			Self::Streams => "Screen streams",
			Self::Complete => "Complete Nivra connection",
		}
	}
	pub fn contains(self, traffic: Self) -> bool {
		self == Self::Complete || self == traffic
	}
	/// Only the complete route carries the account gateway and must precede login.
	pub fn protects_login(self) -> bool {
		self == Self::Complete
	}
	pub fn needs_udp(self) -> bool {
		self != Self::Videos
	}
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Protocol {
	Http,
	Socks4,
	Socks5,
	#[default]
	WireGuard,
}
impl Protocol {
	pub const ALL: [Self; 4] = [Self::Http, Self::Socks4, Self::Socks5, Self::WireGuard];
	pub fn label(self) -> &'static str {
		match self {
			Self::Http => "HTTP",
			Self::Socks4 => "SOCKS4",
			Self::Socks5 => "SOCKS5",
			Self::WireGuard => "WireGuard",
		}
	}
	pub fn scheme(self) -> Option<&'static str> {
		match self {
			Self::Http => Some("http"),
			Self::Socks4 => Some("socks4a"),
			Self::Socks5 => Some("socks5h"),
			Self::WireGuard => None,
		}
	}
}

/// Shared bounded proxy-field validation. URL parsing and connection probes remain in the host.
pub fn proxy_problem(
	protocol: Protocol,
	host: &str,
	port: u16,
	username: &str,
	password: &str,
) -> Option<&'static str> {
	if host.trim().is_empty() {
		return Some("Enter the proxy server");
	}
	if host.len() > 253
		|| host
			.chars()
			.any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '@' | '?' | '#'))
	{
		return Some("Enter a hostname or IP address without a URL or spaces");
	}
	if port == 0 {
		return Some("Port must be between 1 and 65535");
	}
	if username.len() > 255
		|| password.len() > 255
		|| username.contains('\0')
		|| password.contains('\0')
	{
		return Some("Proxy credentials must fit in 255 bytes and contain no null characters");
	}
	if protocol == Protocol::Socks4 && !password.is_empty() {
		return Some("SOCKS4 has no password authentication; use SOCKS5");
	}
	None
}

pub struct Draft {
	pub protocol: Protocol,
	pub scope: Scope,
	pub host: String,
	pub port: u16,
	pub username: zeroize::Zeroizing<String>,
	pub password: zeroize::Zeroizing<String>,
	pub wireguard: zeroize::Zeroizing<String>,
}
impl Default for Draft {
	fn default() -> Self {
		Self {
			protocol: Protocol::default(),
			scope: Scope::default(),
			host: String::new(),
			port: 1080,
			username: zeroize::Zeroizing::new(String::new()),
			password: zeroize::Zeroizing::new(String::new()),
			wireguard: zeroize::Zeroizing::new(String::new()),
		}
	}
}
impl Draft {
	/// Why this draft cannot be tested or enabled yet, if anything is still missing.
	pub fn missing(&self) -> Option<&'static str> {
		if self.protocol == Protocol::WireGuard {
			if self.wireguard.len() > 4096 {
				return Some("WireGuard configuration exceeds 4 KiB");
			}
			if self.wireguard.trim().is_empty() {
				return Some("Import a WireGuard configuration first");
			}
		} else if let Some(reason) = proxy_problem(
			self.protocol,
			&self.host,
			self.port,
			&self.username,
			&self.password,
		) {
			return Some(reason);
		}
		if self.scope.needs_udp()
			&& !matches!(self.protocol, Protocol::Socks5 | Protocol::WireGuard)
		{
			return Some("Live media needs SOCKS5 with UDP or WireGuard");
		}
		None
	}
}
impl Clone for Draft {
	fn clone(&self) -> Self {
		Self {
			protocol: self.protocol,
			scope: self.scope,
			host: self.host.clone(),
			port: self.port,
			username: self.username.clone(),
			password: self.password.clone(),
			wireguard: self.wireguard.clone(),
		}
	}
}
pub enum Action {
	Test(Draft),
	Enable(Draft, Option<Draft>),
	Disable,
	ImportWireGuard,
	Profiles(ProfileAction),
}

/// Saved rows contain no credentials. The host owns bounded persistence and tests.
#[derive(Clone)]
pub struct SavedProfile {
	pub id: String,
	pub name: String,
	pub protocol: Protocol,
	pub scope: Scope,
	pub result: String,
	pub location: String,
}
pub enum ProfileAction {
	Remember(Draft),
	/// Host-only committed route snapshot; editing and testing cannot replace it.
	RememberActive {
		activation: String,
		draft: Draft,
	},
	Save {
		name: String,
		draft: Draft,
	},
	Load(String),
	Delete(String),
	TestAll {
		location: bool,
	},
	CancelTests,
}

/// Only recognized public fields are displayed or written to the local copy.
/// Unknown directives and comments may contain secrets and are omitted.
pub fn public_wireguard(config: &str) -> String {
	let mut public = String::new();
	for line in config.lines() {
		let line = line.split(['#', ';']).next().unwrap_or_default().trim();
		if matches!(line, "[Interface]" | "[Peer]") {
			public.push_str(line);
			public.push('\n');
		} else if let Some((key, value)) = line.split_once('=') {
			let key = key.trim();
			if matches!(
				key,
				"Address"
					| "DNS" | "MTU" | "PublicKey"
					| "Endpoint" | "AllowedIPs"
					| "PersistentKeepalive"
			) {
				public.push_str(key);
				public.push_str(" = ");
				public.push_str(value.trim());
				public.push('\n');
			}
		}
	}
	public
}

#[cfg(test)]
mod missing_tests {
	use super::{Draft, Protocol, Scope};

	#[test]
	fn public_configuration_excludes_private_keys_comments_and_unknown_directives() {
		let config = "[Interface]\nPrivateKey = hidden\nAddress = 10.0.0.1/32 # secret comment\nDNS = 1.1.1.1\n[Peer]\nPresharedKey = more-hidden\nPublicKey = public\nEndpoint = example.invalid:51820\nSecret = unknown\n";
		let public = super::public_wireguard(config);
		assert!(public.contains("Address = 10.0.0.1/32\n"));
		assert!(public.contains("DNS = 1.1.1.1"));
		assert!(public.contains("PublicKey = public"));
		for hidden in ["hidden", "comment", "unknown", "PrivateKey", "PresharedKey"] {
			assert!(!public.contains(hidden));
		}
	}

	#[test]
	fn an_unconfigured_route_is_never_reported_ready() {
		let mut draft = Draft {
			protocol: Protocol::Socks5,
			..Draft::default()
		};
		assert_eq!(draft.missing(), Some("Enter the proxy server"));
		draft.host = "proxy.example".into();
		assert_eq!(draft.missing(), None);
		draft.protocol = Protocol::WireGuard;
		assert_eq!(
			draft.missing(),
			Some("Import a WireGuard configuration first")
		);
		draft.wireguard = zeroize::Zeroizing::new("[Interface]".into());
		assert_eq!(draft.missing(), None);
	}

	#[test]
	fn live_media_needs_udp_capable_protocols() {
		let mut draft = Draft {
			host: "proxy.example".into(),
			protocol: Protocol::Http,
			scope: Scope::Voice,
			..Draft::default()
		};
		assert!(draft.missing().is_some());
		draft.scope = Scope::Videos;
		assert_eq!(draft.missing(), None);
	}
}

#[cfg(test)]
mod route_validation_tests {
	use super::*;
	#[test]
	fn only_complete_routes_gate_discord_login() {
		for scope in Scope::ALL {
			assert_eq!(scope.protects_login(), scope == Scope::Complete);
		}
	}
	#[test]
	fn proxy_validation_rejects_invalid_fields_before_connecting() {
		let mut draft = Draft {
			protocol: Protocol::Socks5,
			host: "127.0.0.1".into(),
			..Draft::default()
		};
		assert!(draft.missing().is_none());
		draft.port = 0;
		assert_eq!(draft.missing(), Some("Port must be between 1 and 65535"));
		draft.port = 1080;
		for host in [
			"https://proxy.invalid",
			"proxy.invalid/path",
			"proxy.invalid ",
			"proxy@invalid",
			"proxy\0invalid",
		] {
			draft.host = host.into();
			assert!(draft.missing().is_some(), "{host:?}");
		}
		draft.host = "::1".into();
		assert!(draft.missing().is_none());
		draft.password.push_str(&"é".repeat(128));
		assert!(
			draft.missing().is_some(),
			"limits are bytes, not Unicode character counts"
		);
		draft.password.clear();
		draft.protocol = Protocol::Socks4;
		draft.password.push_str("secret");
		assert!(draft.missing().is_some());
	}
}
