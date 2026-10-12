# Nivra patch: egui `context.rs` accesskit focus guard

Upstream egui (pinned rev `99df44a801749aee958295ed96fccad8dfecb289`, crate `egui` 0.36.2)
builds the AccessKit `TreeUpdate` with `focus` set to `memory.focused()` even when that id has
no node in the same update. AccessKit then panics with
`Focused ID #<id> is not in the node list` on the UI thread, which closes Nivra (seen on
1.0.14 and 1.0.15 on Windows, 2026-10-09 and 2026-10-11).

Change: `focus` falls back to the root when the focused id is not among the update's nodes.

Not verified: no test reproduces the panic path on this rev. A focused widget that disappears is
already cleared by egui, so the node-less focus must come from another path. The guard only
removes the panic condition; the root cause is still open.
Upstream master has the equivalent fix (`.filter(|id| state.nodes.contains_key(id))`).

Remove this vendored crate when the pinned egui rev includes that fix and the rev is bumped.
The manifest was generated from the pinned egui workspace: workspace-inherited versions were
replaced with the workspace values, and sibling crates (`emath`, `epaint`) point at the same git rev.
