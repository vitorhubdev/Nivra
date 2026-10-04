# Synthetic Discord poll previews

These inspected images come from the native egui framebuffer via the existing
offline `profile_preview` example. They are **not OS-captured screenshots** and
do not establish native input, accessibility, or live Discord interoperability.

```powershell
cargo run --locked -p nivra --features demo --example profile_preview -- --demo --page=appearance --no-settings --timeline-text --poll-presence --output=docs/pr-evidence/polls/before.png
cargo run --locked -p nivra --features demo --example profile_preview -- --demo --page=appearance --no-settings --timeline-text --poll --output=docs/pr-evidence/polls/after.png
```

- `before.png` is the previous rendering for a message with a poll: the bounded
  `Poll` presence marker plus the external "Open in Discord" fallback.
- `after.png` is the native poll card: question, answers with emoji, the live
  tally, the total vote count and a timestamped row. In the offline example the
  vote buttons are clickable and move the synthetic tally in RAM only.

The fixtures are synthetic timeline rows and a locally generated poll; no service
credentials, network adapters or account data are used by this example.
