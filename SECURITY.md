# Security

Do not post tokens, passwords, QR images, MFA codes, private histories, signed URLs or raw captures in issues. Report vulnerabilities privately to a maintainer using a contact listed on their repository profile; no dedicated reporting address has been configured yet.

Use synthetic reproductions. The unofficial normal-user adapter is experimental and carries Discord account risk. No bot substitute, credential extraction from other applications, challenge bypass, telemetry or hidden messaging backend is supported. See docs/threat-model.md and docs/storage-policy.md.

The project does not yet promise production security support or full protocol compatibility. Dependency audit and platform release gates must pass before publishing a supported release.

## Release audit — October 28, 2026

The baseline audit in Actions run `40647144485` failed with five vulnerabilities
and two denied warnings. The media/PROTECT work does not waive these checks and
must not be described as a security-cleared 1.0.21 release.

| Locked dependency | Advisory / finding | Published patched line |
| --- | --- | --- |
| crypto-bigint 0.5.5 | [RUSTSEC-2026-0387](https://rustsec.org/advisories/RUSTSEC-2026-0387.html) | >=0.7.6 |
| ctutils 0.4.2 | [RUSTSEC-2026-0385](https://rustsec.org/advisories/RUSTSEC-2026-0385.html) | >=0.4.4 |
| rustls-webpki 0.103.15 | [RUSTSEC-2026-0382](https://rustsec.org/advisories/RUSTSEC-2026-0382.html) | >=0.103.16 on the 0.103 line |
| webpki-root-certs 1.0.9 | [RUSTSEC-2026-0383](https://rustsec.org/advisories/RUSTSEC-2026-0383.html) | >=1.0.10 |
| webpki-roots 1.0.9 | [RUSTSEC-2026-0381](https://rustsec.org/advisories/RUSTSEC-2026-0381.html) | >=1.0.10 |
| tokio 1.53.1 | [RUSTSEC-2026-0386](https://rustsec.org/advisories/RUSTSEC-2026-0386.html), unsound API warning | >=1.53.3 |
| clipboard-win 5.4.1 | Yanked registry version | 5.4.3 (included in the tested candidate) |

The locked crypto-bigint 0.5 line arrives through elliptic-curve 0.13 and the
p256/p384/k256 graph used by davey/OpenMLS/HPKE. Adding an unrelated crypto-bigint
0.7 dependency does not replace those vulnerable edges. Do not change the version
string, disable DAVE, or add an advisory ignore to make this gate pass. Update or
qualify the actual dependency graph, run both lockfile audits and native regression
checks, and preserve the encrypted protocol behavior. No exploitability test or
claim of credential compromise was made by this audit.

The revised Windows audit [40649493147](https://github.com/vitorhubdev/Nivra/actions/runs/40649493147)
updated ctutils to 0.4.4, rustls-webpki to 0.103.16, both current root-certificate
packages to 1.0.10, Tokio to 1.53.3 and clipboard-win to 5.4.3. Formatting, strict
Clippy, 1,885 workspace tests, doctests, policy, offline media checks and both
license checks passed with that dependency graph. The root strict audit still
fails on **crypto-bigint 0.5.5 / RUSTSEC-2026-0387**; the fuzz lockfile audit passes.
The source can be reviewed, but 1.0.21 remains unpublished. The pre-existing
exceptions in `.cargo/audit.toml` were not expanded; a passing filtered audit
would not imply that those historical risks disappeared.
