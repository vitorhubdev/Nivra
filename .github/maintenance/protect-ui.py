"""Task-scoped PROTECT UI, persistence and diagnostic edits. No live network access."""
from pathlib import Path
import json
changed = []
def edit(path, transform):
    p = Path(path)
    text = p.read_text()
    result = transform(text)
    if result == text: raise RuntimeError(f'No change for {path}')
    p.write_text(result)
    changed.append(path)
def once(s, old, new):
    n = s.count(old)
    if n != 1: raise RuntimeError(f'Expected one match, got {n}: {old[:160]!r}')
    return s.replace(old, new, 1)

PROFILE_FORM = r'''fn profile(ui: &mut egui::Ui, draft: &mut Draft, id: &str, reveal: &mut bool) {
    ui.push_id(id, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(crate::tr_ui!(ui, "Connection type"));
            egui::ComboBox::from_id_salt("protocol")
                .selected_text(draft.protocol.label())
                .show_ui(ui, |ui| {
                    for protocol in Protocol::ALL {
                        ui.selectable_value(&mut draft.protocol, protocol, protocol.label());
                    }
                });
        });
        if draft.protocol == Protocol::WireGuard {
            ui.label(crate::tr_ui!(ui, "WireGuard carries IPv4 inside Nivra; valid IPv6 entries are not routed and never fall back to direct."));
            ui.label(crate::tr_ui!(ui, "DNS uses the first allowed IPv4 resolver from the configuration; without DNS it uses 1.1.1.1. System DNS is unchanged."));
            let public = model::protect::public_wireguard(&draft.wireguard);
            if !public.is_empty() {
                egui::CollapsingHeader::new(crate::tr_ui!(ui, "Configuration details"))
                    .id_salt("public-wireguard").show(ui, |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(public).monospace()).wrap());
                    });
            }
            ui.checkbox(reveal, crate::tr_ui!(ui, "Reveal and edit configuration, including private keys"));
            if *reveal {
                let label = ui.label(crate::tr_ui!(ui, "WireGuard configuration"));
                ui.add(egui::TextEdit::multiline(&mut *draft.wireguard)
                    .char_limit(4096).desired_rows(7).desired_width(f32::INFINITY).code_editor())
                    .labelled_by(label.id);
            }
        } else {
            let server = ui.label(crate::tr_ui!(ui, "Server"));
            ui.add(egui::TextEdit::singleline(&mut draft.host)
                .hint_text("proxy.example.com").char_limit(253))
                .labelled_by(server.id);
            ui.horizontal(|ui| {
                let port = ui.label(crate::tr_ui!(ui, "Port"));
                ui.add(egui::DragValue::new(&mut draft.port).range(1..=65535))
                    .labelled_by(port.id);
            });
            let username = ui.label(crate::tr_ui!(ui, "Username (optional)"));
            ui.add(egui::TextEdit::singleline(&mut *draft.username).char_limit(255))
                .labelled_by(username.id);
            if draft.protocol != Protocol::Socks4 {
                let password = ui.label(crate::tr_ui!(ui, "Password (optional)"));
                ui.add(egui::TextEdit::singleline(&mut *draft.password).password(true).char_limit(255))
                    .labelled_by(password.id);
            } else if !draft.password.is_empty() {
                // Changing the selector must not silently erase saved credentials.
                ui.label(crate::tr_ui!(ui, "SOCKS4 has no password authentication; use SOCKS5"));
            }
        }
    });
}
'''
CONTROLS = r'''    fn connection_controls(&mut self, ui: &mut egui::Ui, demo: bool) {
        let idle = !demo && !self.busy && !self.profiles_busy;
        ui.add_enabled_ui(idle, |ui| {
            profile(ui, &mut self.draft, "primary", &mut self.reveal);
        });
        let compatible = self.draft.protocol.supports_scope(self.draft.scope);
        if !compatible {
            crate::design::notice(ui, crate::design::Level::Warning,
                crate::tr_ui!(ui, "HTTP and SOCKS4 can protect files, not live voice or screen streams. Test TCP or explicitly switch the scope."));
            if ui.add_enabled(idle, egui::Button::new(crate::tr_ui!(ui, "Use Videos and files"))
                .min_size(egui::vec2(0.0,36.0))).clicked() {
                self.draft.scope = Scope::Videos;
                self.status = "Changes not applied yet".into();
            }
        }
        let probe = self.draft.probe_draft();
        let test_problem = probe.missing();
        let reserve_problem = (self.fallback && self.draft.protocol != Protocol::WireGuard)
            .then(|| self.reserve.missing()).flatten();
        let enable_problem = self.draft.missing().or(reserve_problem);
        if let Some(reason) = enable_problem {
            ui.add(egui::Label::new(crate::tr_str!(ui, reason)).wrap());
        }
        ui.horizontal_wrapped(|ui| {
            let test = ui.add_enabled(idle && test_problem.is_none(),
                egui::Button::new(crate::tr_ui!(ui, "Test connection"))
                    .min_size(egui::vec2(148.0,36.0)))
                .on_disabled_hover_text(crate::tr_str!(ui, test_problem.unwrap_or("Operation in progress")));
            if test.clicked() {
                self.action = Some(Action::Test(probe));
                self.busy = true;
                self.status = "Testing…".into();
            }
            let enable = ui.add_enabled(idle && enable_problem.is_none(),
                egui::Button::new(crate::tr_ui!(ui, "Enable"))
                    .min_size(egui::vec2(108.0,36.0)))
                .on_disabled_hover_text(crate::tr_str!(ui, enable_problem.unwrap_or("Operation in progress")));
            if enable.clicked() { self.request_enable(); }
        });
        ui.label(egui::RichText::new(crate::tr_ui!(ui,
            "Testing does not activate PROTECT. A TCP success does not prove UDP or a working call."))
            .small().color(crate::design::palette(ui).muted));
        ui.add_space(6.0);
        ui.add_enabled_ui(idle && enable_problem.is_none(), |ui| {
            ui.horizontal_wrapped(|ui| {
                let label = ui.label(crate::tr_ui!(ui, "Profile name"));
                ui.add(egui::TextEdit::singleline(&mut self.save_name)
                    .hint_text(crate::tr_ui!(ui,"Profile name")).desired_width(180.0).char_limit(64))
                    .labelled_by(label.id);
                if ui.add_enabled(!self.save_name.trim().is_empty(),
                    egui::Button::new(crate::tr_ui!(ui,"Save profile"))
                        .min_size(egui::vec2(0.0,36.0))).clicked() {
                    self.action = Some(Action::Profiles(ProfileAction::Save {
                        name: self.save_name.clone(), draft: self.draft.clone(),
                    }));
                }
            });
        });
    }

    fn advanced(&mut self, ui: &mut egui::Ui, demo: bool) {
        ui.add_enabled_ui(!demo && !self.busy && !self.profiles_busy, |ui| {
            if self.draft.protocol != Protocol::WireGuard {
                ui.checkbox(&mut self.fallback, crate::tr_ui!(ui,
                    "Automatically try one reserve profile when reconnecting"));
                if self.fallback {
                    self.reserve.scope = self.draft.scope;
                    profile(ui, &mut self.reserve, "reserve", &mut self.reveal);
                }
            }
            if !self.flags_removed && ui.button(crate::tr_ui!(ui,
                "Remove flags and exit country")).clicked() {
                self.flags_removed = true;
                self.flags.clear();
            }
        });
        ui.label(crate::tr_ui!(ui,
            "A failed protected route never silently switches to a direct connection. Only media covered by the changed route is affected."));
    }
'''
UI_TESTS = r'''
    #[test]
    fn http_stream_draft_offers_a_tcp_probe_without_activating_media() {
        use egui_kittest::kittest::Queryable as _;
        let mut harness = egui_kittest::HarnessBuilder::default().allow_missing_glyphs()
            .build_ui_state(|ui, settings: &mut super::Settings| {
                settings.connection_controls(ui, false);
            }, super::Settings {
                draft: model::protect::Draft { protocol: model::protect::Protocol::Http,
                    host: "192.0.2.20".into(), port:80, scope:model::protect::Scope::Streams,
                    ..Default::default() }, ..Default::default()
            });
        harness.run();
        harness.get_by_role_and_label(egui::Role::Button,"Test connection").click();
        harness.run();
        assert!(matches!(&harness.state().action,
            Some(model::protect::Action::Test(d)) if d.scope == model::protect::Scope::Videos));
        assert_eq!(harness.state().draft.scope, model::protect::Scope::Streams);
        assert!(!harness.state().enabled);
    }

    #[test]
    fn explicit_scope_switch_makes_http_eligible_without_changing_live_route() {
        use egui_kittest::kittest::Queryable as _;
        let mut harness = egui_kittest::HarnessBuilder::default().allow_missing_glyphs()
            .build_ui_state(|ui, settings: &mut super::Settings| {
                settings.connection_controls(ui, false);
            }, super::Settings {
                enabled:true, active:Some(model::protect::Scope::Streams),
                draft:model::protect::Draft { protocol:model::protect::Protocol::Http,
                    host:"192.0.2.20".into(), port:80, scope:model::protect::Scope::Streams,
                    ..Default::default() }, ..Default::default()
            });
        harness.run();
        harness.get_by_role_and_label(egui::Role::Button,"Use Videos and files").click();
        harness.run();
        assert_eq!(harness.state().draft.scope, model::protect::Scope::Videos);
        assert_eq!(harness.state().active, Some(model::protect::Scope::Streams));
        harness.get_by_role_and_label(egui::Role::Button,"Enable").click();
        harness.run();
        assert!(matches!(&harness.state().action,
            Some(model::protect::Action::Enable(d,_)) if d.scope == model::protect::Scope::Videos));
    }
'''

def ui(s):
    s = once(s, '\tdraft: Draft,\n', '\tdraft: Draft,\n\tsave_name: String,\n')
    a, b = s.index('\t/// What most people never need,'), s.index('\tpub(crate) fn show(', s.index('\t/// What most people never need,'))
    s = s[:a] + CONTROLS + s[b:]
    s = once(s, '\t\tself.scope_row(ui);\n\t\tui.add_space(12.0);', '\t\tself.scope_row(ui);\n\t\tui.add_space(12.0);\n        self.connection_controls(ui, demo);\n        ui.add_space(16.0);')
    a, b = s.index('fn profile('), s.index('impl crate::MessagingUi {')
    s = s[:a] + PROFILE_FORM + s[b:]
    s = once(s, 'parts.push(format!("{} · {elapsed}", crate::tr_str!(ui, result)));', 'parts.push(format!("{} · {}: {elapsed}", crate::tr_str!(ui, result), crate::tr_ui!(ui,"Test duration")));')
    s = once(s, 'ui.label(egui::RichText::new(detail).small().color(colors.muted));', '''let color = if row.result.is_empty() { colors.muted }
                            else if row.result.starts_with("TCP/TLS: ok · UDP DNS: ok")
                                || row.result.starts_with("TCP/TLS: ok · UDP: not needed") {
                                colors.positive
                            } else { colors.danger };
                            ui.add(egui::Label::new(egui::RichText::new(detail).small().color(color)).wrap());''')
    return s.rstrip()[:-1] + UI_TESTS + '\n}\n'
edit('crates/ui/src/protect.rs', ui)

def profiles(s):
    s = once(s, 'const MAX_PROFILES: usize = 16;', '''const MAX_PROFILES: usize = 16;
// At most five per-profile events plus batch control messages can be produced
// before the UI gets another frame. Do not lose Finished and stay busy forever.
const PROFILE_EVENTS: usize = MAX_PROFILES * 6 + 8;''')
    s = once(s, 'mpsc::sync_channel(64)', 'mpsc::sync_channel(PROFILE_EVENTS)')
    s = once(s, '''\t\t\t\t\t\tlet total = profiles.len();
\t\t\t\t\t\tpublish(Event::Progress(0, total));''', '''                        let total = profiles.len();
                        let mut failures = false;
                        publish(Event::Progress(0, total));''')
    s = once(s, '''\t\t\t\t\t\t\t\tErr(error) => {
\t\t\t\t\t\t\t\t\tpublish(Event::Result(id, error.into()));
\t\t\t\t\t\t\t\t\tcontinue;
\t\t\t\t\t\t\t\t}''', '''                                Err(error) => {
                                    failures = true;
                                    publish(Event::Result(id, error.into()));
                                    continue;
                                }''')
    s = once(s, '''\t\t\t\t\t\t\tif draft.protocol == Protocol::WireGuard && protect::enabled() {''', '''                            // A rejected configuration has no network duration or ping.
                            let invalid = draft.missing().or_else(|| {
                                (draft.protocol == Protocol::WireGuard)
                                    .then(|| protect::validate_wireguard(&draft.wireguard).err()).flatten()
                            });
                            if let Some(reason) = invalid {
                                failures = true;
                                publish(Event::Result(id, reason.into()));
                                continue;
                            }
                            if draft.protocol == Protocol::WireGuard && protect::enabled() {''')
    s = once(s, '\t\t\t\t\t\t\t\tpublish(Event::Result(id, WG_TEST_BLOCKED.into()));', '\t\t\t\t\t\t\t\tfailures = true;\n                                publish(Event::Result(id, WG_TEST_BLOCKED.into()));')
    s = once(s, '\t\t\t\t\t\t\t\tErr(error) => (error, None),', '\t\t\t\t\t\t\t\tErr(error) => { failures = true; (error, None) },')
    s = once(s, '''\t\t\t\t\t\tOk(
\t\t\t\t\t\t\t"All saved profiles tested; active route unchanged. Probes do not prove a working call.",
\t\t\t\t\t\t)''', '''                        Ok(if failures {
                            "Some profile tests failed; inspect each result. The active route is unchanged."
                        } else {
                            "All saved profiles tested; active route unchanged. Probes do not prove a working call."
                        })''')
    s = once(s, 'fn remove_named(name: &str) -> Result<(), &\'static str> {', 'fn remove_named_except(name: &str, keep: &str) -> Result<(), &\'static str> {')
    s = once(s, 'for row in rows()?.into_iter().filter(|row| row.name == name) {', 'for row in rows()?.into_iter().filter(|row| row.name == name && row.id != keep) {')
    s = once(s, 'fn save(name: String, draft: Draft) -> Result<(), &\'static str> {', 'fn save(name: String, draft: Draft) -> Result<String, &\'static str> {')
    s = once(s, '\tresult\n}\n\nfn snapshot_row', '\tresult.map(|()| id)\n}\n\nfn snapshot_row')
    s = once(s, '''\t\t\t\t\t\tSome(ProfileAction::Save { name, draft }) => {
\t\t\t\t\t\t\tremove_named(&name)?;
\t\t\t\t\t\t\tsave(name, draft.clone())?;''', '''                        Some(ProfileAction::Save { name, draft }) => {
                            if let Some(reason) = draft.missing() { return Err(reason); }
                            if draft.protocol == Protocol::WireGuard {
                                protect::validate_wireguard(&draft.wireguard)?;
                            }
                            let id = save(name.clone(), draft.clone())?;
                            remove_named_except(&name, &id)?;''')
    return s
edit('apps/desktop/src/protect_profiles.rs', profiles)

def docs(s):
    old = '''bounded 4 KiB `.conf` peer with one IPv4 interface address and IPv4 `AllowedIPs`; `MTU` is
applied within 576-1500; hooks and system-route directives are rejected; `DNS` is displayed
from the imported file but is not applied. The tunnel resolver uses 1.1.1.1; system
DNS stays unchanged.'''
    new = '''bounded 4 KiB `.conf` with one peer and one IPv4 interface address. Valid IPv6
addresses and AllowedIPs may coexist in provider profiles, but only IPv4 entries are
installed in the guest stack. IPv6-only profiles are rejected, and IPv6 destinations
never bypass the protected route. Repeated Address, AllowedIPs and DNS fields are
combined; malformed networks, duplicate singleton fields and unsupported hooks are
rejected. MTU stays bounded to 576-1500. The resolver and UDP capability probe use
the first configured IPv4 DNS inside AllowedIPs, or 1.1.1.1 only when DNS is absent.
System DNS is unchanged. A resolver outside AllowedIPs produces an explicit error.'''
    s = once(s, old, new)
    return s + '''

## PROTECT validation and recovery audit

The protocol, requested scope and active route are separate states. HTTP/SOCKS4
can be tested as TCP-only without changing the selected or live scope. The user
must explicitly select **Videos and files** before enabling those proxies; live
voice/camera/screens require SOCKS5 UDP or WireGuard. The main page exposes these
controls and a named action to resolve incompatible selections instead of hiding
all manual fields under Advanced. Connectivity tests never activate a profile.

Imported WireGuard configs are validated before saving. A replacement is saved
before deleting its predecessor, so a failed vault write cannot destroy the prior
copy. At the 16-profile cap a replacement may require removing an unused profile
first; the existing copy is retained if saving cannot proceed.

Configuration failures have no fabricated zero-millisecond ping. Network test
elapsed time is labeled **Test duration**, not streaming latency. Batch failures
remain visible and the bounded result channel fits the complete 16-profile batch,
including the terminal event. Tests use documentation-only IPs and synthetic keys;
passing local tunnel/proxy tests does not validate a commercial provider, a user's
private key, their network reachability or a real Discord call.
'''
edit('docs/protect.md', docs)

TRANSLATIONS = {
    "Connection type": ("Tipo de conexão", "Tipo de conexión"),
    "Configuration details": ("Detalhes da configuração", "Detalles de configuración"),
    "WireGuard configuration": ("Configuração WireGuard", "Configuración WireGuard"),
    "WireGuard carries IPv4 inside Nivra; valid IPv6 entries are not routed and never fall back to direct.": (
        "O WireGuard transporta IPv4 dentro do Nivra. Entradas IPv6 válidas são aceitas no arquivo, mas não são roteadas nem desviadas para a conexão direta.",
        "WireGuard transporta IPv4 dentro de Nivra. Las entradas IPv6 válidas no se enrutan ni usan una conexión directa."),
    "DNS uses the first allowed IPv4 resolver from the configuration; without DNS it uses 1.1.1.1. System DNS is unchanged.": (
        "O DNS usa o primeiro servidor IPv4 permitido do arquivo; sem DNS configurado, usa 1.1.1.1. O DNS do sistema não é alterado.",
        "DNS usa el primer servidor IPv4 permitido del archivo; sin DNS configurado usa 1.1.1.1. El DNS del sistema no cambia."),
    "HTTP and SOCKS4 can protect files, not live voice or screen streams. Test TCP or explicitly switch the scope.": (
        "HTTP e SOCKS4 podem proteger arquivos, mas não voz ou tela ao vivo. Teste a conexão TCP ou altere explicitamente o que será protegido.",
        "HTTP y SOCKS4 protegen archivos, no voz ni pantalla en vivo. Pruebe TCP o cambie explícitamente el alcance."),
    "Use Videos and files": ("Usar Vídeos e arquivos", "Usar Vídeos y archivos"),
    "Testing does not activate PROTECT. A TCP success does not prove UDP or a working call.": (
        "Testar não ativa o PROTECT. Sucesso em TCP não comprova UDP nem uma chamada funcional.",
        "Probar no activa PROTECT. El éxito de TCP no demuestra UDP ni una llamada funcional."),
    "Profile name": ("Nome do perfil", "Nombre del perfil"),
    "Save profile": ("Salvar perfil", "Guardar perfil"),
    "Test duration": ("Duração do teste", "Duración de la prueba"),
    "Operation in progress": ("Operação em andamento", "Operación en curso"),
    "Some profile tests failed; inspect each result. The active route is unchanged.": (
        "Alguns testes falharam. Confira o resultado de cada perfil; a rota ativa foi preservada.",
        "Algunas pruebas fallaron. Revise cada resultado; la ruta activa no cambió."),
    "WireGuard validated; saving securely. Select the saved profile to connect.": (
        "WireGuard validado; salvando com segurança. Selecione o perfil salvo para conectar.",
        "WireGuard validado; guardando de forma segura. Seleccione el perfil guardado para conectar."),
    "WireGuard requires an IPv4 interface address; IPv6-only is unsupported": (
        "É necessário um endereço IPv4 em Address. Perfis somente IPv6 não são suportados.",
        "Se requiere una dirección IPv4 en Address. Los perfiles solo IPv6 no se admiten."),
    "WireGuard requires an IPv4 AllowedIPs route; IPv6-only is unsupported": (
        "É necessária uma rota IPv4 em AllowedIPs. Perfis somente IPv6 não são suportados.",
        "Se requiere una ruta IPv4 en AllowedIPs. Los perfiles solo IPv6 no se admiten."),
    "WireGuard DNS resolver is outside IPv4 AllowedIPs": (
        "O DNS não está incluído nas rotas IPv4 de AllowedIPs.",
        "El servidor DNS no está incluido en las rutas IPv4 de AllowedIPs."),
    "WireGuard requires an IPv4 DNS resolver; IPv6-only DNS is unsupported": (
        "Configure um DNS IPv4; DNS somente IPv6 não é suportado.",
        "Configure un DNS IPv4; DNS solo IPv6 no se admite."),
    "WireGuard DNS must contain IP addresses": (
        "DNS deve conter endereços IP válidos separados por vírgula.",
        "DNS debe contener direcciones IP válidas separadas por comas."),
    "Invalid WireGuard AllowedIPs network": ("Rede inválida em AllowedIPs.", "Red inválida en AllowedIPs."),
    "Invalid WireGuard AllowedIPs address or prefix": ("Endereço ou prefixo inválido em AllowedIPs.", "Dirección o prefijo inválido en AllowedIPs."),
    "Invalid WireGuard AllowedIPs prefix": ("Prefixo inválido em AllowedIPs.", "Prefijo inválido en AllowedIPs."),
    "Invalid WireGuard interface IP address or prefix": ("Endereço ou prefixo inválido em Address.", "Dirección o prefijo inválido en Address."),
    "Invalid WireGuard interface IPv4 address": ("Endereço IPv4 inválido em Address.", "Dirección IPv4 inválida en Address."),
    "Invalid WireGuard DNS IPv4 address": ("Endereço IPv4 do DNS inválido.", "Dirección IPv4 de DNS inválida."),
    "Duplicate WireGuard singleton field": ("Campo único repetido na configuração WireGuard.", "Campo único duplicado en la configuración WireGuard."),
    "WireGuard needs one Interface before one Peer": ("O arquivo deve conter uma seção Interface seguida de uma seção Peer.", "El archivo debe contener una sección Interface seguida de una sección Peer."),
}
for lang in ('en', 'pt-BR', 'es'):
    def translate(s, lang=lang):
        existing = {line.split(' = ',1)[0] for line in s.splitlines() if ' = ' in line and not line.startswith('#')}
        s = s.rstrip()+'\n\n# PROTECT validation and TCP/UDP recovery guidance.\n'
        for key, values in TRANSLATIONS.items():
            if key not in existing:
                value = key if lang == 'en' else values[0 if lang == 'pt-BR' else 1]
                s += key+' = '+json.dumps(value, ensure_ascii=False)+'\n'
        return s
    edit(f'crates/ui/locales/{lang}.ftl', translate)
print('PROTECT UI changes:', ', '.join(changed))
