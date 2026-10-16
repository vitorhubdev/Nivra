# PROTECT native routing

PROTECT is an integrated native Settings page and host transport module. It does
not grant network or credential imports to the Wasm extension runtime. Profiles
can be saved explicitly: at most 16 named copies, plus one active primary and at
most one reserve. Passwords and original WireGuard configurations use zeroizing
memory and the OS credential store only; no plaintext fallback is available.
The application-data `nivra/protect` folder contains bounded public metadata and
public-only `.conf` reference copies. These reference copies omit private keys,
preshared keys, unknown directives and comments; the complete original is restored
from the OS credential store. Losing that store means importing the original again.
Loading, removing or testing saved copies never applies or disables the active route.
Use Load, then Enable to change routes; Save a copy also provides duplication with
a new name. Profiles are local to this installation, shared across its accounts.

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

Test all runs the saved profiles sequentially, reports each route result and elapsed
probe time, continues after an individual unreadable profile or failed route, and
can be cancelled without applying any tested profile. Each capability test has a
30-second outer deadline. If PROTECT is active, batch tests skip WireGuard profiles:
a second tunnel with the same private key can make the provider move the active
peer endpoint and interrupt a call. Disable PROTECT before testing saved WireGuard
profiles; changing/applying routes while a batch runs requires cancelling it first.
Credential-store work has a single process-wide slot retained by the actual blocking
operation even if the outer task is cancelled, preventing abandoned OS calls from
creating an unbounded worker backlog.
Optional IP/country/flag lookups are off by default, require
an explicit checkbox each application session, and add at most 22 seconds per profile.
When selected, `https://ipwho.is/?fields=success,ip,country,country_code` observes the
exit IP through that tested profile; country is approximate. A fixed FlagCDN
`https://flagcdn.com/32x24/{iso2}.png` URL supplies its flag through the same profile.
Both third parties receive that route's exit IP; no account credentials are sent.
No failed query falls back to direct networking. JSON is bounded to 16 KiB; PNG
responses to 64 KiB with exactly 32x24 dimensions checked before decoding. Lookup
failure preserves the independent route-test result. Location/flags are session-only.

WireGuard runs inside Nivra. The tunnel is boringtun and the guest TCP/IP stack is smoltcp,
both Rust crates in the process: no external program, no sing-box, no GPL code, no system VPN,
interface, route, DNS or administrator change, and no automatic download. Import accepts one
bounded 4 KiB `.conf` peer with one IPv4 interface address and IPv4 `AllowedIPs`; `MTU` is
applied within 576-1500; hooks and system-route directives are rejected; `DNS` is displayed
from the imported file but is not applied. The tunnel resolver uses 1.1.1.1; system
DNS stays unchanged. Destinations outside `AllowedIPs` are refused. Names resolve through the tunnel
only. Traffic reaches the protected scopes through an authenticated loopback SOCKS5 inlet bound
to 127.0.0.1 with random per-session credentials; a failed tunnel never falls back to a direct
connection. Private keys are decoded into zeroizing buffers and the x25519 secret is zeroed on
drop. Bounds: 128 TCP and 32 UDP bridges, 64 KiB socket buffers, 512 queued packets, 16 KiB
chunks with 32-chunk bridges. Engine interoperability and real provider connectivity require a
separate owner test; offline fixtures (a UDP loopback peer, with a lossy forwarder in the tests)
do not establish those claims.

Changing a route retires existing account/media clients and reconnects signaling
with the current in-memory session. Active writes and transfers prevent Apply;
calls must finish before manual Apply. Protected media sockets reject obsolete
route generations. HTTP clients select the current route per request and disable
idle connection reuse, trading connection setup overhead for avoiding stale pools.

The implementation is original; no DiscGringo GPL source is incorporated.
