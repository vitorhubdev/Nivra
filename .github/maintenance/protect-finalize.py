"""Additional PROTECT regressions found during the first native validation run."""
from pathlib import Path
import json

def once(s, a, b):
    n = s.count(a)
    if n != 1: raise RuntimeError(f'Unexpected match count {n}: {a[:120]!r}')
    return s.replace(a, b, 1)

p = Path('crates/ui/src/protect.rs')
s = p.read_text()
s = once(s, 'ui.label(crate::tr_ui!(ui, "Connection type"));', 'let connection_label = ui.label(crate::tr_ui!(ui, "Connection type"));')
s = once(s, '''                    for protocol in Protocol::ALL {
                        ui.selectable_value(&mut draft.protocol, protocol, protocol.label());
                    }
                });''', '''                    for protocol in Protocol::ALL {
                        ui.selectable_value(&mut draft.protocol, protocol, protocol.label());
                    }
                }).response.labelled_by(connection_label.id);''')
s = once(s, 'ui.label(crate::tr_ui!(ui, "What to protect"));', 'let scope_label = ui.label(crate::tr_ui!(ui, "What to protect"));')
s = once(s, 'egui::ComboBox::from_id_salt("protect_scope")', 'let scope_picker = egui::ComboBox::from_id_salt("protect_scope")')
s = once(s, '\t\t\t\t\t});\n\t\t\t});\n\t\t\tlet colors = crate::design::palette(ui);', '\t\t\t\t\t});\n                scope_picker.response.labelled_by(scope_label.id);\n\t\t\t});\n\t\t\tlet colors = crate::design::palette(ui);')
s = once(s, '''                ui.label(crate::tr_ui!(ui, "SOCKS4 has no password authentication; use SOCKS5"));''', '''                ui.label(crate::tr_ui!(ui, "SOCKS4 has no password authentication; use SOCKS5"));
                if ui.button(crate::tr_ui!(ui, "Clear password for SOCKS4")).clicked() {
                    draft.password = Draft::default().password;
                }''')
s = once(s, '        let probe = self.draft.probe_draft();', '        self.reserve.scope = self.draft.scope;\n        let probe = self.draft.probe_draft();')
s = once(s, '''\tfn request_enable(&mut self) {
\t\tif let Some(reason) = self.draft.missing() {''', '''\tfn request_enable(&mut self) {
        self.reserve.scope = self.draft.scope;
        let invalid = self.draft.missing().or_else(|| {
            (self.fallback && self.draft.protocol != Protocol::WireGuard)
                .then(|| self.reserve.missing()).flatten()
        });
        if let Some(reason) = invalid {''')
p.write_text(s)

p = Path('crates/model/src/protect.rs')
s = p.read_text()
s = once(s, 'for line in config.lines() {', "for line in config.trim_start_matches('\\u{feff}').lines() {")
p.write_text(s)

p = Path('apps/desktop/src/protect_profiles.rs')
s = p.read_text()
s += r'''
#[cfg(test)]
mod batch_delivery_tests {
    use super::*;
    #[test]
    fn a_full_profile_batch_cannot_drop_the_terminal_event() {
        let (send, receive) = mpsc::sync_channel(PROFILE_EVENTS);
        assert!(send.try_send(Event::Progress(0, MAX_PROFILES)).is_ok());
        for n in 0..MAX_PROFILES {
            let id = format!("synthetic-{n}");
            assert!(send.try_send(Event::Progress(n, MAX_PROFILES)).is_ok());
            assert!(send.try_send(Event::Location(id.clone(), String::new(), None)).is_ok());
            assert!(send.try_send(Event::Result(id.clone(), "synthetic result".into())).is_ok());
            assert!(send.try_send(Event::Located(id.clone())).is_ok());
            assert!(send.try_send(Event::Location(id, "synthetic location".into(), None)).is_ok());
        }
        assert!(send.try_send(Event::Progress(MAX_PROFILES, MAX_PROFILES)).is_ok());
        assert!(send.try_send(Event::Finished(Ok("completed"))).is_ok());
        assert!(matches!(receive.try_iter().last(), Some(Event::Finished(Ok("completed")))));
    }
}
'''
p.write_text(s)

p = Path('crates/protect/src/lib.rs')
s = p.read_text()
s += r'''
#[cfg(test)]
mod wireguard_ipv6_tests {
    use super::*;
    #[tokio::test]
    async fn ipv6_under_wireguard_fails_before_opening_a_udp_association() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let profile = Profile {
            protocol: Protocol::Socks5, scope: Scope::Streams,
            host: "127.0.0.1".into(), port: listener.local_addr().unwrap().port(),
            username: Zeroizing::new(String::new()), password: Zeroizing::new(String::new()),
            wireguard: Some(Zeroizing::new("synthetic config marker".into())), engine: None,
        };
        let result = tokio::time::timeout(Duration::from_secs(2),
            Datagram::connect_profile("[::1]:9".parse().unwrap(), Some(&profile)))
            .await.expect("IPv6 refusal is immediate");
        let error = match result { Err(error) => error, Ok(_) => panic!("unexpected IPv6 route") };
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(tokio::time::timeout(Duration::from_millis(30), listener.accept()).await.is_err());
    }
}
'''
p.write_text(s)

translations = {
    'Clear password for SOCKS4': ('Limpar senha para usar SOCKS4', 'Borrar contraseña para SOCKS4'),
    'Invalid WireGuard key': ('Chave WireGuard inválida. Importe o .conf original, não a cópia pública sem chaves.', 'Clave WireGuard inválida. Importe el .conf original, no la copia pública sin claves.'),
    'Too many WireGuard AllowedIPs': ('AllowedIPs excede o limite de 64 redes.', 'AllowedIPs supera el límite de 64 redes.'),
    'Too many WireGuard DNS servers': ('DNS excede o limite de 8 servidores.', 'DNS supera el límite de 8 servidores.'),
    'Too many WireGuard interface addresses': ('Address excede o limite de 8 endereços.', 'Address supera el límite de 8 direcciones.'),
}
for lang in ('en','pt-BR','es'):
    p = Path(f'crates/ui/locales/{lang}.ftl')
    s = p.read_text()
    existing = {line.split(' = ',1)[0] for line in s.splitlines() if ' = ' in line and not line.startswith('#')}
    for key, values in translations.items():
        if key not in existing:
            value = key if lang == 'en' else values[0 if lang == 'pt-BR' else 1]
            s += key+' = '+json.dumps(value, ensure_ascii=False)+'\n'
    p.write_text(s)
print('PROTECT accessibility labels, reserve scope and fail-closed regressions applied')
