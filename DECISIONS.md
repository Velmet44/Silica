# Silica — Decision Record

Every decision that shapes the engine lives here, with rationale and status.
Changing an accepted decision requires a new entry — never a silent edit.

| ID | Date | Decision | Status |
|----|------|----------|--------|
| D-001 | 2026-10-31 | **Core language: Rust** | Accepted |
| D-002 | 2026-10-31 | **MVP scope: report path** — 30 materials, rigid-chunk collapse (vertical), 512×384 @ 60 fps, 6–8 weeks | Accepted |
| D-003 | 2026-10-31 | **Determinism: bit-exact on every platform** (SPEC §8) | Accepted |
| D-004 | 2026-10-31 | **Compute: CPU + SIMD**, WASM first-class; GPU is a post-1.0 optional tier | Accepted |
| D-005 | 2026-10-31 | **Material DSL: versioned TOML data files** | Accepted |
| D-006 | 2026-10-31 | **License: MIT** (user override of the Apache-2.0 recommendation) | Accepted |
| D-007 | 2026-10-31 | **Project name: Silica**; published crate and npm package: **`silica-sim`** | Accepted |
| D-008 | 2026-10-31 | **Spec kit: full package** — this directory — before any code | Accepted |
| D-009 | 2026-10-31 | **DSL loader notes finalized** (SPEC §6): one file per material; transition keys `at_temp`/`below_temp`/`aux_exhausted`; optional `other_becomes`, `probability`, `spread`, `aux_init`, `crumble_into`; starter pack ships 8 materials + steam | Accepted |

## Rationale

**D-001 Rust.** Memory safety catches a whole class of agent-written bugs at
compile time; cargo + crates.io is the distribution rail (rapier3d: ~2M
downloads); `wasm32-unknown-unknown` is a first-class target; and the C ABI
story (`cbindgen`) covers Unity/Godot/Unreal embedders. C++20 was the runner-up
(Powder Toy / Noita lineage) but memory-unsafe agent code is the one failure
mode a solo maintainer cannot afford.

**D-002 Report path.** The rigid carve-and-collapse is the headline demo —
"ceilings cave in" is what separates middleware from another sand toy. 6–8
weeks part-time with opencode writing the shell. The lean-core option (3–4
weeks, no rigid) was rejected as a weaker launch artifact; the go-big option
(infinite worlds, 10–12 weeks) as too much abandonment risk before first
publicity.

**D-003 Bit-exact everywhere.** One golden-hash suite instead of per-platform
testing; replays that work across machines (Minecraft seeds and Powder Toy
saves built content communities on exactly this); rollback netcode for
multiplayer hosts for free. Costs: integer-only core, pinned scan orders,
single-threaded MVP. Worth it — this is the moat.

**D-004 CPU + SIMD.** Noita itself is CPU-based; the WASM playground (the
product's front door) needs CPU; bit-exactness is tractable with integer SIMD,
and D12 (fast path must hash-identical to scalar) keeps it honest. GPU-first
would kill the playground and make determinism a research project.

**D-005 TOML DSL.** Reactions as diffable data: reviewable PRs, shareable
packs, generatable by AI, no runtime scripting sandbox. Compiled-in materials
were rejected because every new material would be a code PR; embedded scripting
(Lua/WASM hooks) because runtime scripting is where determinism goes to die.

**D-006 MIT.** User override of the Apache-2.0 recommendation. Rationale
accepted: MIT is the shortest, most familiar permissive license and removes
all friction for hobby and jam adoption, which is this project's growth path.
Trade-off acknowledged: no explicit patent grant. If patent concerns surface
later, a dual-license or relicense is cheap while there are few contributors —
the window closes as adoption grows, so decide by 0.5 if it ever matters.

**D-007 Name.** `silica` on crates.io is held by an abandoned bare-metal OS
stub (v0.0.1, 444 lines, last updated January 2018, ~2k downloads). The
project keeps the name **Silica** and publishes as **`silica-sim`** on crates.io
and npm. Alternatives (Grit, Sediment) were rejected on sound and fit. Revisit
only if a trademark issue appears.

**D-008 Full kit first.** SPEC, agent rules, testing strategy, roadmap and CI
sketch exist before the first line of engine code, because the entire method
(spec-first, golden-hash-gated, agent-executed) depends on the spec being
authoritative from commit zero.

**D-009 Loader notes.** Finalizing schema v1's underspecified corners before
implementation keeps the first opencode session from making contract decisions
by accident: per-file layout (as the kit's `materials/` shows), the two
temperature transition forms, aux-exhaustion transitions for fire/torch-style
lifetimes, the optional reaction fields, and `crumble_into` (already implied
by §7.4). The starter pack ships steam beyond ROADMAP's 8-material list
because water's `douse` targets steam and load validation rejects unknown
names — the alternative (douse → empty) visibly worsens the 0.1 demo GIF.
Pre-golden, so no hashes are affected; D4's consumption points gain the
`aux_init` spawn draw, still pre-first-golden.

## Still open

Parking-lot items live in SPEC §15 and get decided by milestone 0.2:
pressure model depth, default walls flag, liquid dispersion cap, playground
framework.
