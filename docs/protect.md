# PROTECT native routing

PROTECT is an integrated native Settings page and host transport module. It does
not grant network or credential imports to the Wasm extension runtime. Profiles
are session-only: one primary and at most one reserve; passwords and imported
WireGuard private configuration use zeroizing memory and are never saved to disk.

| Scope | Routed traffic | Supported protocols |
| --- | --- | --- |
| Videos and files | Attachment uploads, downloads, audio and video files | HTTP, SOCKS4a, SOCKS5, WireGuard |
| Voice and camera | Discord voice signaling and shared voice/camera UDP | SOCKS5 with UDP, WireGuard |
| Screen streams | Screen signaling and screen/audio UDP | SOCKS5 with UDP, WireGuard |
| Complete Discord connection | Discord REST, Gateway, Discord application metadata, avatars, files and live media | SOCKS5 with UDP, WireGuard |

Voice and camera share Discord's media transport and cannot have independent
routes. Official login's authentication webview, the public updater/extension
catalog, external browser links and Spotify local integration remain separate.
"Complete" means the Discord client connection, not every Windows application.

Test sends an unauthenticated Discord gateway request through the selected proxy.
Media scopes additionally require a UDP DNS query/response through SOCKS5 to
1.1.1.1:53. Success proves these capabilities, not a Discord call, camera or
screenshare. No account token, microphone or live conversation is used by Test.
Each network probe has a ten-second deadline; host operations have a sixty-second
outer deadline. HTTP/SOCKS4 are rejected for media scopes rather than silently
sending UDP directly. A failed protected connection never falls back to direct.
Only a configured reserve profile is eligible. HTTP writes are not replayed.

WireGuard requires a user-selected installed sing-box 1.14 executable with
WireGuard support. Import accepts one bounded 4 KiB `.conf` peer; shell hooks and
unsupported system-route directives are rejected. The engine runs in user space,
with an authenticated loopback SOCKS5 inlet and WireGuard-only routing. Private
configuration is serialized into zeroizing memory and passed through asynchronous
stdin, with logs/stdout/stderr disabled. No temporary credential file, system VPN,
privileged interface or automatic binary download is created. At most four active
or retiring engine processes are admitted; termination is reaped outside drawing.
Engine interoperability and real provider connectivity require a separate owner
test; offline fixtures do not establish those claims.

Changing a route retires existing account/media clients and reconnects signaling
with the current in-memory session. Active writes and transfers prevent Apply;
calls must finish before manual Apply. Protected media sockets reject obsolete
route generations. HTTP clients select the current route per request and disable
idle connection reuse, trading connection setup overhead for avoiding stale pools.

The implementation is original; no DiscGringo GPL source is incorporated.
