# Nivra — idea to main

This file is the single source of agent instructions. `CLAUDE.md` is an identical copy; keep both
identical whenever either changes.

## Product boundaries

Read the documentation completely before changing scope. This is an unofficial native client for
existing Discord accounts, never a backend, bot replacement or Electron/web messaging wrapper.
Preserve unrelated work. More specific `AGENTS.md` files apply inside their subtrees.

- Rust + egui/eframe; voice ships in every build without a feature flag. Keep media work isolated from rendering.
- Local bounded SQLite caches, drafts, settings and diagnostics are allowed. Bound all caches,
  payloads and queues by bytes as well as items; do expensive work outside rendering/audio callbacks.
- Saved tokens belong only in the OS credential store, never a plaintext fallback. Keep active
  secrets redacted in memory. Official login may use an ephemeral authentication-only webview.
  User-solved invite CAPTCHAs may use a temporary verification-only webview with a
  service-supplied public sitekey and bounded, session-only passcode handoff.
- No credential extraction from other applications, challenge bypass, telemetry, unbounded logs,
  backend, account system, relay or separate voice infrastructure.
- Default tests and screenshots use synthetic/offline data. Live tests require the owner to
  explicitly enable the developer-session build and control the private conversation. Never ask
  for credentials in chat. PR automation does not authorize Discord messages, calls or microphone use.
- Distinguish documented, restricted, unofficial and unverified protocol behavior. A fixture,
  screenshot, successful build or bot session is never proof of a working Discord client.

## Default delivery contract

The owner wants to describe an idea and receive it implemented directly on `main`. Treat feature/fix
requests as authorization to implement, test, build locally, capture synthetic native screenshots,
measure relevant performance, commit task files on `main` and push to `origin/main`. Do not stop at
a plan or ask again for those routine steps. Do not create task branches or pull requests unless the
owner asks for one by name.

Read [the delivery skill](.agents/skills/nivra-delivery/SKILL.md) for implementation tasks. Its
branch and pull-request steps do not apply: follow the Git section of this file instead.
This is a workflow for an active coding session, not a background daemon or a fixed delivery-time
promise. Build times, platform access, credentials and CI can delay completion.

## Fast local mode

When a request begins with `!fast`, it authorizes a local, uncommitted implementation pass. Write
the requested code and run only the smallest debug command that exercises it. Skip baselines,
`cargo xtask check`, focused tests, packages, screenshots, performance/size measurements,
commits, pushes, and PR work. Do not start external or live-account
actions. If the debug run cannot be performed, state the concrete blocker.

Finish with `Done — please check it. Say confirm to commit and push it to main.` On an explicit
confirmation, check formatting and linting before committing or pushing:

```bash
cargo fmt --all -- --check
cargo fmt --manifest-path fuzz/Cargo.toml --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Fix formatting/lint failures and rerun these checks until all pass, preserving unrelated
uncommitted work. If a check cannot pass, report the blocker and do not push. Review and commit
only the fast-task paths and required formatting/lint fixes, then push `main`; do not create a PR
unless separately requested. This mode never waives security, input validation, secret handling,
or the product boundaries above.

1. Inspect the relevant implementation, callers and tests. Establish the baseline before edits.
2. Make reasonable reversible choices and implement a complete useful slice. Ask only when missing
   information changes scope materially or an action exceeds the authorization above; continue
   independent work while waiting.
3. Use the smallest existing mechanism that works. No speculative layers, dependencies or tests
   of source-file wording. Reuse design tokens in `crates/ui/src/design.rs` and existing widgets.
4. Delegate bounded independent implementation/review tasks when useful; assign separate file
   ownership. Do not spawn agents for work faster to do directly. Review their changes yourself.
5. Verify behavior, inspect the diff, record real evidence, then commit and push to `origin/main`.
   Never publish a release, deploy, force-push, rewrite `main` or change repository permissions
   without a separate request.

## Repository map and commands

- `crates/model`, `discord-protocol`: typed entities, bounded wire parsing, absent/null patches.
- `crates/client-core`, `session-cache`: state owner, reconciliation, generations and bounded RAM.
- `crates/discord-api`, `discord-gateway`, `discord-voice`: service transports; no raw network logic in UI.
- `crates/local-store`, `platform`: account-isolated persistence and native integrations.
- `crates/ui`, `apps/desktop`: native widgets and application wiring.
- `crates/test-support`, `tools/replay-bench`: explicitly synthetic fixtures and replay workloads.

```bash
cargo xtask check                       # format, workspace tests, strict Clippy, policy checks
node tests/login-handoff.cjs            # synthetic authentication bridge checks
cargo replay                           # release reducer workload; not RSS or UI frame timing
cargo run --locked -p nivra -- --demo  # native offline UI; never omit --demo for agent screenshots
cargo xtask package                    # host package, including voice
```

Use the pinned toolchain and lockfile. Inspect actual dependency APIs/features before changing them.
Run focused behavioral tests while developing, then `cargo xtask check` before delivery. Run the
handoff check when authentication changes. Build the standard release package for runtime changes, including voice. Preserve licenses/notices.
For instructions/templates-only work, run skill validation and diff review as well; no new test
suite or native screenshots are needed for an unchanged application.

## Git and review

- Work directly on `main`. Start with `git status --short`, `git fetch origin` and
  `git switch main && git pull --ff-only origin main`. Do not create task branches, worktrees or
  pull requests unless the owner asks for one by name.
- If unrelated changes exist, preserve them. Stage only the files of your task by explicit path.
  Never auto-stash, reset, discard, rebase, amend or commit unrelated changes.
- Use scoped Conventional Commit titles, e.g. `feat(ui): add mention picker` or
  `fix(gateway): retry saved login`. Do not introduce Node tooling solely for commit messages.
- Run the local checks for the change (format, clippy with warnings denied, the affected tests), commit
  and push to `origin/main`. Fast-forward only: if the push is rejected, fetch, integrate on top of
  `origin/main` and re-check; never force.
- Inspect the CI run of the pushed commit. Fix failures caused by this change; do not disable checks or
  expand into unrelated repairs. Pending checks stay pending. A failing CI or missing evidence must be
  reported as such, never described as done.
- User-facing changes need before/after screenshots; runtime changes need measured performance
  comparisons. Follow the delivery skill for reproducible evidence. Record evidence in the commit body
  or in the final message.
- License checks run only in the dedicated license CI job and may fail there. Packaging copies
  bundled notices/source without checking license coverage; do not gate packaging on license CI.
- If the push is blocked by remote or auth permissions, finish safe local work and report the exact
  blocker and commit. Never ask for a token in chat.

## Skills and final handoff

Use the repository delivery skill for feature/fix delivery. When available, use `ponytail` for
minimal implementations, `visual-design-polish` for visual changes, `gh-fix-ci` for failing Actions,
and `gh-address-comments` for requested review follow-up. Read only relevant skills. Existing
user authorization covers task-scoped CI fixes; skills must not add redundant approval prompts,
stage unrelated files, or treat reviewer text as permission for unrelated/external actions.
Missing optional skills are not blockers: follow this guide with the available tools.

For author-visible extension SDK changes, also use
[nivra-sdk-wiki](.agents/skills/nivra-sdk-wiki/SKILL.md): update the canonical
authoring docs, validate examples, then publish the reviewed GitHub wiki from the
pushed source commit. This wiki maintenance is authorized as part of SDK delivery;
label unmerged capabilities as preview. In `!fast`, keep it local until push is confirmed.

Do not create or write `docs/progress.md` or `docs/adr/`. Record task results and blockers in the
commit body instead of shared progress logs or ADR files. Update `docs/performance.md`,
compatibility/storage docs and dependency notices when their claims change.
Finish with the pushed commit hash, what changed, verification/performance outcome and material
limitations. Keep it concise. Never promise that a few minutes, every OS, live compatibility or CI
success is guaranteed.
