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
	pub fn needs_udp(self) -> bool {
		self != Self::Videos
	}
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Protocol {
	Http,
	Socks4,
	#[default]
	Socks5,
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

pub struct Draft {
	pub protocol: Protocol,
	pub scope: Scope,
	pub host: String,
	pub port: u16,
	pub username: zeroize::Zeroizing<String>,
	pub password: zeroize::Zeroizing<String>,
	/// Explicit user-selected local sing-box executable; never discovered/downloaded.
	pub engine: String,
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
			engine: String::new(),
			wireguard: zeroize::Zeroizing::new(String::new()),
		}
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
			engine: self.engine.clone(),
			wireguard: self.wireguard.clone(),
		}
	}
}
pub enum Action {
	Test(Draft),
	Enable(Draft, Option<Draft>),
	Disable,
	ImportWireGuard,
}
