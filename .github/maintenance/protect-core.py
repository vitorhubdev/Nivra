"""Apply exact reviewed PROTECT core edits. No network or credential access."""
from pathlib import Path
changed = []
def edit(path, transform):
    p = Path(path)
    text = p.read_text()
    result = transform(text)
    if result == text:
        raise RuntimeError(f'No change for {path}')
    p.write_text(result)
    changed.append(path)
def once(s, old, new):
    n = s.count(old)
    if n != 1:
        raise RuntimeError(f'Expected one match, got {n}: {old[:160]!r}')
    return s.replace(old, new, 1)

PARSER = r'''fn append_list(target: &mut String, value: &str) {
    if !target.is_empty() { target.push(','); }
    target.push_str(value);
}

fn interface_ipv4(list: &str) -> Result<Ipv4Addr, &'static str> {
    if list.split(',').count() > 8 { return Err("Too many WireGuard interface addresses"); }
    let mut ipv4 = None;
    for entry in list.split(',') {
        let entry = entry.trim();
        let (ip, prefix) = entry.split_once('/').map_or((entry, None), |(ip, p)| (ip, Some(p)));
        let ip: IpAddr = ip.parse().map_err(|_| "Invalid WireGuard interface IP address or prefix")?;
        let max = if ip.is_ipv4() { 32 } else { 128 };
        let prefix = prefix.map(str::parse::<u8>).transpose()
            .map_err(|_| "Invalid WireGuard interface IP address or prefix")?.unwrap_or(max);
        if prefix > max { return Err("Invalid WireGuard interface IP address or prefix"); }
        if let IpAddr::V4(ip) = ip {
            if ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() {
                return Err("Invalid WireGuard interface IPv4 address");
            }
            if ipv4.replace(ip).is_some() { return Err("WireGuard supports one IPv4 interface address"); }
        }
    }
    ipv4.ok_or("WireGuard requires an IPv4 interface address; IPv6-only is unsupported")
}

fn allowed_ipv4(list: &str) -> Result<Vec<Cidr>, &'static str> {
    if list.split(',').count() > MAX_ALLOWED { return Err("Too many WireGuard AllowedIPs"); }
    let mut allowed = Vec::new();
    for entry in list.split(',') {
        let entry = entry.trim();
        if let Some(cidr) = Cidr::parse(entry) { allowed.push(cidr); continue; }
        let (ip, prefix) = entry.split_once('/').ok_or("Invalid WireGuard AllowedIPs network")?;
        let prefix: u8 = prefix.parse().map_err(|_| "Invalid WireGuard AllowedIPs prefix")?;
        match ip.parse::<IpAddr>() {
            Ok(IpAddr::V6(_)) if prefix <= 128 => {},
            _ => return Err("Invalid WireGuard AllowedIPs address or prefix"),
        }
    }
    if allowed.is_empty() { return Err("WireGuard requires an IPv4 AllowedIPs route; IPv6-only is unsupported"); }
    Ok(allowed)
}

/// Choose the first configured IPv4 DNS that the tunnel actually permits.
/// No resolver is contacted here. Explicit IPv6-only DNS never falls back to host DNS.
fn tunnel_dns(list: &str, allowed: &[Cidr]) -> Result<Ipv4Addr, &'static str> {
    let fallback = "1.1.1.1";
    let list = if list.is_empty() { fallback } else { list };
    if list.split(',').count() > 8 { return Err("Too many WireGuard DNS servers"); }
    let mut candidates = Vec::new();
    for entry in list.split(',') {
        let ip: IpAddr = entry.trim().parse().map_err(|_| "WireGuard DNS must contain IP addresses")?;
        if let IpAddr::V4(ip) = ip {
            if ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() {
                return Err("Invalid WireGuard DNS IPv4 address");
            }
            candidates.push(ip);
        }
    }
    if candidates.is_empty() { return Err("WireGuard requires an IPv4 DNS resolver; IPv6-only DNS is unsupported"); }
    candidates.into_iter().find(|ip| allowed.iter().any(|cidr| cidr.contains(*ip)))
        .ok_or("WireGuard DNS resolver is outside IPv4 AllowedIPs")
}

/// Offline validation, also used before importing a profile into the OS vault.
pub fn validate(text: &str) -> Result<(), &'static str> { parse(text).map(|_| ()) }

fn parse(text: &str) -> Result<Config, &'static str> {
    if text.len() > 4096 { return Err("WireGuard configuration exceeds 4 KiB"); }
    let mut config = Config {
        private: Zeroizing::new(String::new()), public: String::new(),
        psk: Zeroizing::new(String::new()), address: String::new(),
        allowed: String::new(), dns: String::new(), host: String::new(),
        port: 0, keepalive: 0, mtu: MTU_DEFAULT as u16,
    };
    let mut section = "";
    let mut peers = 0;
    let mut interfaces = 0;
    let mut seen = std::collections::BTreeSet::new();
    for raw in text.trim_start_matches('\u{feff}').lines() {
        let line = raw.split(['#', ';']).next().unwrap_or("").trim();
        if line.is_empty() { continue; }
        if line.starts_with('[') {
            section = line;
            match section {
                "[Interface]" => {
                    interfaces += 1;
                    if interfaces > 1 || peers != 0 { return Err("WireGuard needs one Interface before one Peer"); }
                }
                "[Peer]" => {
                    peers += 1;
                    if peers > 1 { return Err("Only one WireGuard peer is supported"); }
                    if interfaces != 1 { return Err("WireGuard needs one Interface before one Peer"); }
                }
                _ => return Err("Unsupported WireGuard section"),
            }
            continue;
        }
        let (name, value) = line.split_once('=').ok_or("Malformed WireGuard configuration")?;
        let (name, value) = (name.trim(), value.trim());
        if !matches!(name, "Address" | "AllowedIPs" | "DNS") && !seen.insert((section, name)) {
            return Err("Duplicate WireGuard singleton field");
        }
        match (section, name) {
            ("[Interface]", "PrivateKey") => *config.private = value.to_owned(),
            ("[Interface]", "Address") => append_list(&mut config.address, value),
            ("[Interface]", "DNS") => append_list(&mut config.dns, value),
            ("[Interface]", "MTU") => {
                config.mtu = value.parse::<u16>().ok().filter(|m| MTU_RANGE.contains(m))
                    .ok_or("WireGuard MTU must be between 576 and 1500")?;
            }
            ("[Peer]", "PublicKey") => config.public = value.to_owned(),
            ("[Peer]", "PresharedKey") => *config.psk = value.to_owned(),
            ("[Peer]", "AllowedIPs") => append_list(&mut config.allowed, value),
            ("[Peer]", "PersistentKeepalive") => config.keepalive = value.parse().map_err(|_| "Invalid keepalive")?,
            ("[Peer]", "Endpoint") => {
                // URL normalizes :80 away. Read an explicitly supplied UDP port
                // from the authority, never infer an HTTP default for WireGuard.
                let (_, port) = value.rsplit_once(':').ok_or("Missing WireGuard port")?;
                if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
                    return Err("Invalid WireGuard endpoint");
                }
                config.port = port.parse::<u16>().ok().filter(|p| *p != 0)
                    .ok_or("Invalid WireGuard endpoint")?;
                if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
                    return Err("Invalid WireGuard endpoint");
                }
                let url = url::Url::parse(&format!("http://{value}"))
                    .map_err(|_| "Invalid WireGuard endpoint")?;
                if !url.username().is_empty() || url.password().is_some() || url.path() != "/"
                    || url.query().is_some() || url.fragment().is_some() {
                    return Err("Invalid WireGuard endpoint");
                }
                config.host = url.host_str().ok_or("Missing WireGuard host")?
                    .trim_matches(['[', ']']).to_owned();
            }
            _ => return Err("Unsupported WireGuard field; hooks and system routes are never executed"),
        }
    }
    if interfaces != 1 || peers != 1 || config.host.is_empty() || config.port == 0 {
        return Err("WireGuard needs valid keys, address and one peer endpoint");
    }
    // Canonical base64 validation happens before any endpoint DNS or network work.
    let _private = Zeroizing::new(decode_key(&config.private)?);
    let _ = decode_key(&config.public)?;
    if !config.psk.is_empty() { let _psk = Zeroizing::new(decode_key(&config.psk)?); }
    let _ = interface_ipv4(&config.address)?;
    let allowed = allowed_ipv4(&config.allowed)?;
    let _ = tunnel_dns(&config.dns, &allowed)?;
    Ok(config)
}

impl Config {
    fn params(&self) -> Result<Params, &'static str> {
        let address = interface_ipv4(&self.address)?;
        let allowed = allowed_ipv4(&self.allowed)?;
        let dns = tunnel_dns(&self.dns, &allowed)?;
        Ok(Params {
            secret: x25519_secret(&decode_key(&self.private)?),
            peer: x25519_public(&decode_key(&self.public)?),
            preshared: if self.psk.is_empty() { None } else { Some(decode_key(&self.psk)?) },
            keepalive: (self.keepalive != 0).then_some(self.keepalive),
            address, dns, mtu: usize::from(self.mtu), allowed,
        })
    }
}

'''

PARSER_TESTS = r'''
    #[test]
    fn screenshot_dual_stack_profile_keeps_only_ipv4_without_direct_ipv6() {
        let mixed = fixture().replace("0.0.0.0/0", "0.0.0.0/0, ::/0")
            .replace("10.0.0.2/32", "10.0.0.2/32, fd00::2/128")
            .replace("[Peer]", "DNS = 1.1.1.1, 8.8.8.8\n[Peer]");
        let p = parse(&mixed).unwrap().params().unwrap();
        assert_eq!(p.address, Ipv4Addr::new(10,0,0,2));
        assert_eq!(p.dns, Ipv4Addr::new(1,1,1,1));
        assert_eq!(p.allowed.len(), 1);
        assert!(p.allowed[0].contains(Ipv4Addr::new(8,8,8,8)));
    }

    #[test]
    fn repeated_lists_bom_and_comments_preserve_provider_values() {
        let text = fixture()
            .replace("Address = 10.0.0.2/32", "Address = 10.0.0.2\nAddress = fd00::2/128 ; IPv6")
            .replace("AllowedIPs = 0.0.0.0/0", "AllowedIPs = 0.0.0.0/1\nAllowedIPs = 128.0.0.0/1, ::/0")
            .replace("[Peer]", "DNS = 9.9.9.9\nDNS = 1.1.1.1\n[Peer]");
        let p = parse(&format!("\u{feff}{text}")).unwrap().params().unwrap();
        assert_eq!(p.allowed.len(), 2);
        assert_eq!(p.dns, Ipv4Addr::new(9,9,9,9));
    }

    #[test]
    fn invalid_routes_dns_and_duplicate_keys_fail_before_network() {
        for route in ["::/129", "::/-1", "bad/64", "::", ""] {
            assert!(validate(&fixture().replace("0.0.0.0/0", &format!("0.0.0.0/0, {route}"))).is_err());
        }
        for dns in ["::1", "999.1.1.1", "1.1.1.1, typo"] {
            assert!(validate(&fixture().replace("[Peer]", &format!("DNS = {dns}\n[Peer]"))).is_err());
        }
        let limited = fixture().replace("0.0.0.0/0", "10.0.0.0/8");
        assert_eq!(validate(&limited), Err("WireGuard DNS resolver is outside IPv4 AllowedIPs"));
        let limited = limited.replace("[Peer]", "DNS = 9.9.9.9, 10.0.0.53\n[Peer]");
        assert_eq!(parse(&limited).unwrap().params().unwrap().dns, Ipv4Addr::new(10,0,0,53));
        assert_eq!(validate(&(fixture()+"Endpoint = 192.0.2.2:443\n")), Err("Duplicate WireGuard singleton field"));
        for key in ["A".repeat(42)+"B=", "é".repeat(22), "x".repeat(43)+"="] {
            let text = fixture().replacen(&("A".repeat(43)+"="), &key, 1);
            assert!(validate(&text).is_err());
        }
    }

    #[test]
    fn explicit_wireguard_port_80_is_not_discarded_as_http_default() {
        for endpoint in ["192.0.2.1:80", "example.invalid:80", "[2001:db8::1]:80"] {
            let c = fixture().replace("192.0.2.1:51820", endpoint);
            assert_eq!(parse(&c).unwrap().port, 80);
        }
        for endpoint in ["192.0.2.1", "example.invalid", "example.invalid:0", "example.invalid:65536", "user@host:80", "host/path:80"] {
            assert!(validate(&fixture().replace("192.0.2.1:51820", endpoint)).is_err());
        }
    }

    #[tokio::test]
    async fn dual_stack_config_drives_encrypted_ipv4_tcp_and_udp_echo() {
        use super::super::wg_engine::fixture::*;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (secret, peer, remote_secret, remote_peer) = keys();
        let private = base64::engine::general_purpose::STANDARD.encode(secret.to_bytes());
        let public = base64::engine::general_purpose::STANDARD.encode(peer.as_bytes());
        let conf = format!("[Interface]\nPrivateKey = {private}\nAddress = {CLIENT}/32, fd00::1/128\nDNS = 1.1.1.1\n[Peer]\nPublicKey = {public}\nAllowedIPs = 0.0.0.0/0, ::/0\nEndpoint = 127.0.0.1:443\n");
        let (client, remote) = pair().await;
        serve_remote(remote, remote_secret, remote_peer);
        let engine = Engine::start_on(client, parse(&conf).unwrap().params().unwrap()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(25), async {
            let handle = engine.handle();
            let mut stream = handle.tcp(std::net::SocketAddrV4::new(REMOTE, REMOTE_TCP)).await.unwrap();
            stream.write_all(b"protect").await.unwrap();
            let mut echo = [0u8;7];
            stream.read_exact(&mut echo).await.unwrap();
            assert_eq!(&echo, b"protect");
            let mut udp = handle.udp().await.unwrap();
            let addr = std::net::SocketAddrV4::new(REMOTE, REMOTE_UDP);
            udp.tx.send((addr, b"stream".to_vec())).await.unwrap();
            let (from, data) = udp.rx.recv().await.unwrap();
            assert_eq!(from, addr);
            assert_eq!(&data, b"stream");
        }).await.expect("bounded local WireGuard TCP/UDP roundtrip");
    }
'''

def wireguard(s):
    s = once(s, 'const MAX_ALLOWED: usize = 8;', 'const MAX_ALLOWED: usize = 64;')
    s = once(s, '\tallowed: String,\n', '\tallowed: String,\n\tdns: String,\n')
    a, b = s.index('fn key('), s.index('fn decode_key(')
    s = s[:a] + PARSER + s[b:]
    s = once(s, '\tlet config = parse(&draft.wireguard)?;\n\tlet endpoint = endpoint(&config).await?;\n\tlet mut engine = Engine::start(config.params()?, endpoint).await?;', '\tlet config = parse(&draft.wireguard)?;\n\tlet params = config.params()?;\n\tlet endpoint = endpoint(&config).await?;\n\tlet mut engine = Engine::start(params, endpoint).await?;')
    return s.rstrip()[:-1] + PARSER_TESTS + '\n}\n'
edit('crates/protect/src/wireguard.rs', wireguard)

def engine(s):
    s = once(s, 'const DNS_SERVER: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(1, 1, 1, 1), 53);\n', '')
    s = once(s, '\tfn contains(self, ip: Ipv4Addr) -> bool {', '\tpub(super) fn contains(self, ip: Ipv4Addr) -> bool {')
    s = once(s, '\tpub address: Ipv4Addr,\n', '\tpub address: Ipv4Addr,\n\tpub dns: Ipv4Addr,\n')
    s = once(s, 'pub(super) struct Handle {\n\tcommands: mpsc::Sender<Command>,\n}', 'pub(super) struct Handle {\n\tcommands: mpsc::Sender<Command>,\n\tdns: SocketAddrV4,\n}')
    s = once(s, '.send((DNS_SERVER, query))', '.send((self.dns, query))')
    s = once(s, 'if from == DNS_SERVER', 'if from == self.dns')
    s = once(s, '\t\tlet stack = Stack::new(params, u32::from_be_bytes(index), wake)?;', '\t\tlet dns = SocketAddrV4::new(params.dns, 53);\n\t\tlet stack = Stack::new(params, u32::from_be_bytes(index), wake)?;')
    s = once(s, 'let handle = Handle { commands };', 'let handle = Handle { commands, dns };')
    s = once(s, '\tpub(super) fn handle(&self) -> Handle {', '\tpub(super) fn dns_server(&self) -> SocketAddrV4 { self.handle.dns }\n\n\tpub(super) fn handle(&self) -> Handle {')
    s = once(s, '\t\t\taddress,\n\t\t\tmtu: 1420,', '\t\t\taddress,\n\t\t\tdns: Ipv4Addr::new(1, 1, 1, 1),\n\t\t\tmtu: 1420,')
    return s
edit('crates/protect/src/wg_engine.rs', engine)

def network(s):
    s = once(s, 'pub use wireguard::prepare;', 'pub use wireguard::{prepare, validate as validate_wireguard};')
    s = once(s, 'let target: SocketAddr = "1.1.1.1:53".parse().map_err(|_| UDP_ASSOCIATION)?;', '''let target: SocketAddr = match profile.engine.as_ref() {
        Some(engine) => engine.dns_server().into(),
        None => "1.1.1.1:53".parse().map_err(|_| UDP_ASSOCIATION)?,
    };''')
    old = '\t\tif p.protocol != Protocol::Socks5 {\n\t\t\treturn Err(invalid());\n\t\t}\n\t\tlet setup = async {'
    new = '''\t\tif p.protocol != Protocol::Socks5 {
            return Err(invalid());
        }
        if p.wireguard.is_some() && destination.is_ipv6() {
            return Err(io::Error::new(io::ErrorKind::Unsupported,
                "WireGuard routes IPv4 only; IPv6 never falls back to direct"));
        }
        let setup = async {'''
    return once(s, old, new)
edit('crates/protect/src/lib.rs', network)

def model(s):
    s = once(s, '\tpub fn scheme(self) -> Option<&\'static str> {', '''    pub fn supports_scope(self, scope: Scope) -> bool {
        !scope.needs_udp() || matches!(self, Self::Socks5 | Self::WireGuard)
    }
    pub fn scheme(self) -> Option<&'static str> {''')
    s = once(s, 'impl Draft {\n', '''impl Draft {
    /// A connectivity probe may check a TCP-only proxy without changing the
    /// selected or active scope. It must not imply support for live media.
    pub fn probe_draft(&self) -> Self {
        let mut probe = self.clone();
        if !probe.protocol.supports_scope(probe.scope) { probe.scope = Scope::Videos; }
        probe
    }
''')
    old = '''\t\tif self.scope.needs_udp()
\t\t\t&& !matches!(self.protocol, Protocol::Socks5 | Protocol::WireGuard)
\t\t{'''
    s = once(s, old, '\t\tif !self.protocol.supports_scope(self.scope) {')
    return s + '''
#[cfg(test)]
mod protect_probe_tests {
    use super::*;
    #[test]
    fn http_can_be_probed_without_silently_changing_stream_protection() {
        let draft = Draft { protocol: Protocol::Http, scope: Scope::Streams,
            host: "192.0.2.10".into(), port: 80, ..Draft::default() };
        assert!(draft.missing().is_some());
        let probe = draft.probe_draft();
        assert_eq!(probe.scope, Scope::Videos);
        assert!(probe.missing().is_none());
        assert_eq!(draft.scope, Scope::Streams);
        assert!(!Protocol::Http.supports_scope(Scope::Complete));
        for scope in Scope::ALL {
            assert!(Protocol::Socks5.supports_scope(scope));
            assert!(Protocol::WireGuard.supports_scope(scope));
        }
    }
}
'''
edit('crates/model/src/protect.rs', model)

def desktop(s):
    s = once(s, '''\t\t\t\t\t\t.await
\t\t\t\t\t\t.map_err(|_| "WireGuard import worker stopped")??;
\t\t\t\t\t\tOk(ProtectChange::Imported(config, name))''', '''\t\t\t\t\t\t.await
                        .map_err(|_| "WireGuard import worker stopped")??;
                        protect::validate_wireguard(&config)?;
                        Ok(ProtectChange::Imported(config, name))''')
    s = once(s, '"WireGuard imported and saved. Click it in the list to connect."', '"WireGuard validated; saving securely. Select the saved profile to connect."')
    return s
edit('apps/desktop/src/main.rs', desktop)
print('PROTECT core changes:', ', '.join(changed))
