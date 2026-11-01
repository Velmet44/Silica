# Silica — Roadmap and Milestone Ladder

Rule of the ladder: **every milestone ships a shareable artifact** — a GIF, a
link, a video. A milestone without a demo is not done. Weeks are part-time
(solo dev + opencode in copilot mode).

## 0.1 — "The GIF" · weeks 1–2

**Scope:** 8 materials (sand, water, oil, fire, smoke, wood, stone, empty),
movement + reaction + heat passes, no rigid, no pressure. CLI `silica render`
writes PNG frames from a scripted world; assemble the GIF externally.
**Artifact:** a 10-second loop — pour water on a fire in a wooden box, watch
the box burn and the sand dam redirect the flow.
**Acceptance:** golden suite infrastructure exists from day one (3 worlds,
5k ticks); budgets instrumented; repo public with SPEC/AGENTS/TESTING in place.
**Distribution:** none yet — or a quiet build-in-public thread if desired.

## 0.2 — "The Playground" · weeks 3–5

**Scope:** `silica-wasm` + a canvas playground on GitHub Pages. 20 materials,
brushes with radius/flow, pause/step/run, temperature view toggle. Region
dirty tracking lands here (idle worlds must cost ~nothing).
**Artifact:** the zero-install playground link — the product's front door.
**Acceptance:** ≥ 33 fps on a mid-range laptop in-browser; playground works
on a phone (touch); first 10 seconds need no instructions ("click to pour
sand").
**Distribution:** r/Simulated, r/rust "Show" thread, personal socials. Link
goes to the playground, never the repo.

## 0.3 — "The MVP" · weeks 6–8 → SPEC §13 acceptance checklist

**Scope:** rigid islands (vertical carve/fall/crumble/weld), full 30-material
default pack, save/load, replay recording, pressure pass v1.
**Artifact:** the collapse GIF (stone ceiling undermined by acid, carving,
falling, crumbling) + playground update with replays shareable as URLs.
**Distribution:** the main launch — Show HN (playground link), r/gamedev,
r/noita ("I built the middleware the genre never had"), crosspost to
r/PowderToy. Mail the replay-URL demo to 5 simulation YouTubers with a
two-line pitch.

## 0.4 — "The DSL" · weeks 9–11

**Scope:** material packs as a public, documented contract; community material
PRs enabled; `pack.toml` semver; docs site (the SPEC rendered, plus a
material-author guide); aux-driven behaviors (clone, torch life).
**Artifact:** "materials as pull requests" — a demo pack authored in a PR,
plus a time-lapse of an ecosystem of community materials.
**Distribution:** r/rust writeup on the DSL design; the material-author guide
as a linkable asset; start the subreddit's monthly showcase habit.

## 0.5 — "The Library" · weeks 12–15

**Scope:** C ABI + cbindgen header; Godot and Unity example projects; npm
package `silica-sim` for the wasm build; crates.io 0.5 with semver contract.
**Artifact:** a 60-second video: "add fire that spreads to your Unity game in
three lines" — same video cut for Godot.
**Distribution:** Unity/Godot forums and subreddits; asset-store teaser pages
(paid tiers later); devlog YouTubers round two, this time with the embed story.

## 0.6–0.9 — quarterly hardening (post-MVP, order by demand)

- SIMD fast path, differential-gated (D12).
- Rigid rotation (quantized 1/256-turn angles, SPEC §7.5).
- Infinite chunked worlds (regions become a sparse map).
- Pressure v2 (velocity field), island↔liquid splash, rollback-netcode docs.
- Console/paid tiers only if commercial pull is real (see report ch. 4).

## 1.0 — API freeze · ~week 24

API frozen and semver'd; docs complete; 100-material default pack; three
language bindings (Rust, C, TS); sponsorship page; the playground promoted
to "try it" front and center on the repo landing page.

## What slips first (pre-decided)

When time runs short, cut in this order: rotation → pressure v2 → infinite
worlds → SIMD. **Never cut:** determinism, budgets, golden infrastructure,
or a milestone's demo artifact. Revisit DECISIONS.md whenever the ladder
changes shape.
