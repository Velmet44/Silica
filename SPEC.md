# Silica — Engine Specification

**Version:** 0.1 (implementation draft) · **Date:** 2026-10-31 · **License:** MIT
**Status:** Accepted for implementation. Changes require a matching entry in `DECISIONS.md`.

Silica is an embeddable falling-sand simulation engine: cellular automata over
hundreds of thousands of particles — sand, water, oil, fire, steam, acid — plus
rigid chunks that carve off ceilings and collapse, heat and pressure diffusion,
and a material DSL in which reactions are data instead of code. It is a library,
not a game: hosts link it (Rust, C ABI, or WASM), feed it brush inputs, render
the result, and nothing else. The one-line pitch: **your game can have fire
that actually spreads, in one dependency.**

---

## 1. Design pillars

Priority order — when two rules conflict, the higher pillar wins.

1. **Determinism.** Same seed + same input log + same material pack = bit-identical
   world on every CPU, OS and toolchain we ship. Replays, rollback netcode and
   golden-hash testing all fall out of this one property.
2. **Embeddability.** Library first. Zero dependencies in the core crate. A C ABI
   and a WASM build are release artifacts, not afterthoughts.
3. **Budgeted performance.** A fixed per-tick time budget with graceful,
   documented degradation — never unbounded frame spikes.
4. **Hackability.** Materials are data files. Worlds and replays are small,
   shareable artifacts. Users become authors.
5. **Phenomenological fidelity.** Looks right beats is right. No calibrated
   real-world data, ever.

## 2. Goals and non-goals

**Goals (v1.0)**

- Embeddable library: Rust crate (`silica-sim`), C ABI, and WASM package.
- Bit-exact cross-platform determinism with replay from a seed plus input log.
- Material DSL: TOML files define materials, reactions, phase transitions.
- Rigid islands: unsupported rigid material carves off, falls, crumbles or welds.
- Heat diffusion, combustion, phase changes, coarse pressure field.
- 512×384 world at 60 fps native on a 2020-class laptop; 33+ fps under WASM.
- Stable, versioned save and replay formats (appendix A).
- Golden-hash test infrastructure from the first commit.

**Non-goals (v1.0)**

- No renderer. The engine emits an RGBA buffer for convenience; the host draws.
- No editor application. The web playground is a thin demo, not a product.
- No GPU compute. Optional paid tier after 1.0.
- No networking. The replay format is the netcode *enabler*, not netcode.
- No calibrated physics. Relative density ordering, plausible speeds.
- No 3D. No SPH/particle fluids — grid cellular automata only.

## 3. Terminology

| Term | Meaning |
|---|---|
| **cell** | One grid site: material id + temperature + aux word. |
| **material** | A DSL-defined substance (`sand`, `water`, …). Id `0` is `empty`. |
| **category** | One of `powder`, `liquid`, `gas`, `solid`, `rigid`, `energy`. |
| **tick** | One full pass of the pipeline in §5. Fixed-order, fixed-effect. |
| **world** | The grid plus RNG state, tick counter, islands, pressure field. |
| **region** | 32×32 tracking tile; only *active* regions are scanned. |
| **rigid island** | A connected rigid region that lost support and is falling as one body. |
| **pack** | A directory of material TOML files + `pack.toml` manifest. |
| **golden world** | A committed (seed, script) pair whose tick hashes are locked in a manifest. |
| **replay** | Seed + pack hash + ordered input log; reproduces a run exactly. |

## 4. World model

- **MVP:** one fixed-size world, default **512×384** cells. Infinite chunked
  worlds are a post-1.0 extension; the region system is designed so the grid
  can later become a sparse map of regions without changing the tick pipeline.
- **Storage:** struct-of-arrays, row-major:

```rust
pub material: Vec<u16>,   // index into the compiled pack registry; 0 = empty
pub temp:     Vec<i16>,   // 0.25 K per unit; valid range [0, 32767] (≈ 8191 K)
pub aux:      Vec<u16>,   // category-defined: fire life, plant growth, clone memory, …
pub stamp:    Vec<u32>,   // last tick this cell was written; avoids clearing pass
```

- Temperature arithmetic happens in `i32` and is **clamped on store**. Ambient
  default 295 K → `1180` units. All limits are enforced, never assumed.
- **Regions:** 32×32 tiles with an activity flag, set by any write, cleared when
  a scan produces no writes. The movement/reaction/heat passes skip inactive
  regions (with a 1-region halo for heat).
- **Anchors:** the bottom row of the world and map-edge walls (config flag)
  count as supported ground for rigid support analysis (§7).

## 5. The tick

One call to `World::step(inputs)` executes exactly this pipeline, in this order:

| # | Pass | What it does | Budget (native) |
|---|------|--------------|------------------|
| 1 | `apply_inputs` | Apply queued brush strokes in submission order. | 0.5 ms |
| 2 | `rigid_pass` | Integrate islands, collide, crumble/weld, re-merge (§7). | 1.5 ms |
| 3 | `movement_pass` | CA movement, bottom-up rows, column direction alternates by tick parity. | 5.0 ms |
| 4 | `reaction_pass` | DSL reactions + combustion, top-down row-major fixed order. | 2.5 ms |
| 5 | `heat_pass` | Temperature diffusion (4-neighbour, `i32` intermediates) + phase transitions. | 2.0 ms |
| 6 | `pressure_pass` | Coarse 8×8 pressure grid: diffuse, push gases/liquids. | 1.0 ms |
| 7 | `end_of_tick` | Increment tick, expose hash hook. | 0.5 ms |

Total: **≤ 13 ms** on the reference machine (2020 4-core laptop, release build,
world 60% active) — 60 fps with margin. WASM budget is 1.5× (≤ 19.5 ms).

- **Scan order rationale:** a fixed bottom-up scan makes falling cheap; the
  alternating column direction removes the classic left/right pile bias while
  staying trivially deterministic. The reaction/heat scan runs top-down
  row-major, always the same — no parity, no ambiguity.
- **Single-update rule:** a cell moves at most once per tick (stamp check).
  Liquids with dispersion loop internally, following the same parity direction.
- **Zero allocation:** the steady-state tick performs no heap allocation. All
  buffers and the island arena are preallocated at world creation.
- **Degradation policy:** if the budget is exceeded, the engine reduces
  active-region scan detail for non-movement passes — it never reorders or
  drops passes, because that would break determinism. Degradation is
  documented, measured in benchmarks, and visible via a stats API.

## 6. Materials and the DSL

Materials live in **packs**: a directory of TOML files plus a `pack.toml`
manifest (`name`, `version`, semver). The default pack ships in-repo under
`materials/`. Schema version 1:

```toml
schema_version = 1

[material.sand]
id = 1
name = "Sand"
category = "powder"            # powder|liquid|gas|solid|rigid|energy
density = 1600                 # relative ordering only
default_temp = 295             # K
colors = ["#d9c07a", "#cfb56f", "#e2cc8b"]

[material.sand.transition]
melt = { at_temp = 1700, to = "molten_glass" }

[material.water]
id = 2
name = "Water"
category = "liquid"
density = 1000
default_temp = 295
colors = ["#3d6dd8", "#3562c6"]

[[material.water.reaction]]
name = "douse"
other = "fire"
self_becomes = "steam"
other_becomes = "empty"
probability = 1.0

[[material.gunpowder.reaction]]
name = "deflagrate"
other = "fire"
self_becomes = "fire"
other_becomes = "fire"
probability = 1.0
spread = "explosion"           # special kind: converts a radius, adds pressure
```

- **Color variants are chosen deterministically** from cell coordinates
  (`(x*7 + y*13) % len(colors)`), never from the RNG — otherwise every tick
  would churn the render and the state hash.
- **Probabilities** are rationals with denominator 1,000,000, drawn from the
  reaction RNG stream. At load, reactions are sorted by
  `(a_id, b_id or 0, declaration index)`; per cell, preconditions are checked
  in that order, one RNG draw per passing attempt.
- **Load validation** rejects: duplicate ids, unknown names in reactions or
  transitions, categories with illegal behaviour (e.g. `solid` with melt),
  and ids above the registry cap (1024).
- **Pack identity** = FNV-1a 64 over the sorted file list + file bytes. The
  hash is recorded in world saves and replays (D11).

**Loader notes (schema v1):**

- One file per material; `schema_version = 1` at the top of each file;
  `pack.toml` carries `name`, `version`, `license`. `empty` (id 0) is
  implicit and never has a file.
- Transition keys: `at_temp` (fires when temp ≥), `below_temp` (temp <), and
  `aux_exhausted` (the cell's aux counter reached 0). The rule name
  (`melt`, `burn_out`, …) is a descriptive label, not a keyword.
- Reaction fields: `name`, `other`, `self_becomes` required;
  `other_becomes` optional (default: other cell unchanged); `probability`
  optional (default 1.0); `spread` optional.
- `aux_init = [lo, hi]` optional: the category's default aux (fire life) is
  drawn once per spawned cell in `apply_inputs` from the main RNG stream
  (amends the documented consumption points in D4).
- `crumble_into` optional, `rigid` only: the powder a hard-impact island
  re-enters as (§7.4); must name a powder.
- A starter subset of the default pack for milestone 0.1 ships with this
  spec package under `materials/` (8 materials + steam, ids reserved per
  appendix B).

**Default pack, MVP (30 materials):** `empty, sand, water, oil, fire, smoke,
steam, wood, coal, gunpowder, acid, lava, stone, bedrock, glass, molten_glass,
ice, snow, plant, seed, spark, metal, wire, iron, rust, clone, void, torch,
gas, nitro` — full table with categories, densities and reactions in appendix B.

## 7. Rigid islands

Categories: `solid` never moves (bedrock, wire). `rigid` participates in
support analysis and can become an island (stone, wood, glass, ice, iron).

1. **Detect.** Support analysis runs only on dirty regions. A rigid cell is
   *supported* if a 4-connected path of rigid/solid cells reaches an anchor
   (§4). When a region loses its last support path, a new island is spawned.
   Island ids are assigned in detection scan order.
2. **Freeze.** The connected region is copied into a bitmap island (offset,
   bounds), the cells leave the grid, and the island enters ballistic flight.
3. **Fall (MVP).** Vertical velocity only: `v += g`, capped at terminal
   velocity; AABB sweep against grid solids and other islands, integrated in
   island-id order.
4. **Impact.** Impact speed above the material's crumble threshold → cells
   re-enter the grid as the material's `crumble_into` powder with lateral
   jitter (RNG-drawn, defined order); below threshold → cells weld back as
   rigid. Impact→liquid splash transfer arrives at 0.4.
5. **Rotation.** Deferred to 0.7: centroid + angle quantized to 1/256 turn,
   rotated-bitmap collision. The MVP's vertical-only rule is the price of an
   8-week MVP and is documented as such.

Island cap: 1024 live islands. Excess islands crumble instantly on spawn —
a documented, deterministic degradation, not a panic.

## 8. The determinism contract (frozen)

This section is law. Changing any rule below requires a `DECISIONS.md` entry.

- **D1** Integer-only arithmetic in `silica-core` (`#![deny(clippy::float_arithmetic)]`).
  Floats are allowed only in host-facing helpers outside the core crate.
- **D2** All overflow-capable arithmetic uses explicit `wrapping_*` / `checked_*` /
  `saturating_*`. `[profile.release] overflow-checks = true` so CI catches violations.
- **D3** Iteration order is fixed and documented per pass (§5). No `HashMap`/
  `HashSet` iteration anywhere in the simulation — use id-sorted `Vec` or `BTreeMap`.
- **D4** One PRNG: **xoshiro128\\*\\***, seeded from the world seed, consumed only
  at documented points (reaction attempts, gas jitter, crumble jitter, and
  `aux_init` draws at cell spawn in `apply_inputs`). RNG state
  is part of world state: hashed, saved, replayed.
- **D5** No threads in `silica-core` for 1.0. Future parallelism must be
  region-synchronous with a deterministic reduction order.
- **D6** No time, date, environment or address-dependent behaviour in core.
- **D7** Serialization is little-endian. Hashes run over canonical buffers
  defined byte-exactly in appendix A.
- **D8** Same seed + input log + pack content hash ⇒ identical state hash on
  x86-64 Linux/Windows, arm64 macOS, and wasm32. Enforced by the CI matrix.
- **D9** Toolchain pinned in `rust-toolchain.toml`. Upgrades are explicit PRs
  and may regenerate goldens only with written justification.
- **D10** The public API never panics on valid input; panics are programmer-error
  only and each one is documented.
- **D11** Packs are identified by content hash; saves and replays record it.
- **D12** SIMD or any optimized fast path must reproduce the scalar reference
  hash exactly — differential-tested in CI on every run.

## 9. Public API surface

```rust
// silica-core
let mut world = World::new(WorldConfig {
    width: 512, height: 384, seed: 42, walls: true, pack: &default_pack(),
});
world.step(&inputs)?;                    // one tick, §5 pipeline
world.paint(&Brush::circle(x, y, r, material, flow));
let cell = world.cell(x, y);             // Option<CellView>
let hash = world.state_hash();           // u64 FNV-1a, appendix A
world.render_rgba(&mut buf);             // convenience colors from the DSL
world.save(&mut writer)?;                // *.silica
let world = World::load(&mut reader, &pack)?;
let recorder = world.replay_recorder();  // records the input log
```

- **Input model:** everything a human can do is a brush stroke. A replay is
  `(seed, pack hash, ordered [(tick, BrushStroke)])` — nothing else exists.
- **C ABI (`silica-capi`):** opaque `silica_world*`, functions mirroring the
  Rust surface, `silica_error` enum, header generated by cbindgen. Ownership:
  the host owns the world handle; no callbacks.
- **WASM (`silica-wasm`):** wasm-bindgen surface, `render_rgba` into a JS
  `ImageData`; the playground uses only this.

## 10. Performance plan

- Region dirty tracking from 0.2 (the single biggest win: idle worlds cost ~0).
- Stamp trick instead of clearing arrays; SoA layout for cache-linear scans.
- Category dispatch through a `match` on the compiled category tag — no dyn
  calls in the movement inner loop.
- SIMD fast path (0.6): region-wide movement in 256-bit lanes; the scalar path
  remains the reference and both must hash identically (D12).
- Criterion benchmarks on three committed canonical worlds; CI fails on > 5%
  regression versus `main`.
- Memory ceiling: 250 MB for a 512×384 world with the default pack (CI soak).

## 11. Repository layout

```
silica/
├── Cargo.toml              # workspace
├── rust-toolchain.toml     # pinned (D9)
├── SPEC.md  AGENTS.md  TESTING.md  ROADMAP.md  DECISIONS.md
├── crates/
│   ├── silica-core/        # simulation only; zero deps; no unsafe
│   ├── silica-dsl/         # TOML → compiled material registry
│   ├── silica-formats/     # save / load / replay
│   ├── silica-capi/        # C ABI + cbindgen header
│   └── silica-wasm/        # wasm-bindgen surface
├── materials/              # default pack (TOML)
├── tools/                  # CLI: render, replay, hash, soak
├── tests/                  # golden worlds + generators (code, not binaries)
├── examples/               # raylib / SDL2 minimal embeds
└── playground/             # GitHub Pages demo (TypeScript)
```

## 12. Scope estimate

| Component | LOC (est.) |
|---|---|
| `silica-core` (passes, world, regions) | 6,000 |
| Rigid islands | 3,000 |
| `silica-dsl` (schema, loader, validation) | 2,500 |
| `silica-formats` (save/replay/manifest) | 2,000 |
| `silica-capi` + `silica-wasm` | 2,000 |
| Tools + CLI | 1,500 |
| Examples + playground glue | 2,000 |
| **Total (excluding tests)** | **≈ 19,000** |

## 13. MVP acceptance criteria (milestone 0.3)

1. Default pack: 30 materials, ≥ 12 reaction rules, ≥ 6 phase transitions.
2. Rigid islands: an unsupported stone slab carves off, falls, crumbles on hard
   impact, welds on soft impact. Island cap enforced.
3. 512×384 world, 60% active: ≥ 60 fps native, ≥ 33 fps wasm32 on the reference
   machine; zero heap allocation per steady-state tick (verified).
4. 10 golden worlds × 10,000 ticks green on Linux, Windows, macOS, wasm32.
5. Save/load roundtrip reproduces the state hash byte-exactly.
6. Any recorded replay reproduces the run's final hash exactly.
7. Playground deployed: 10 paintable materials, pause/step/run, temp view.
8. `cargo clippy -D warnings` clean; zero `unsafe` in `silica-core`;
   release `overflow-checks` clean on the full test suite.

## 14. Risks

| Risk | Mitigation |
|---|---|
| Noita-scale optimization sink | Fixed budgets (§5), dirty regions, SIMD as an optional later tier. |
| Rigid rotation complexity | Vertical-only MVP, rotation at 0.7, quantized angles spec'd now. |
| DSL schema churn breaking packs | `schema_version` + semver pack contract from day one. |
| Crate name taken (`silica`, dead 2018 baremetal stub) | Publish as **`silica-sim`**; npm likewise. Recorded in DECISIONS.md. |
| Agent-written code drift | Golden hashes + the AGENTS.md contract. |
| Replay format breakage | Version field, N−1 migration support policy. |
| Solo-dev burnout on the "boring" 80% | That 80% is exactly what opencode writes (see AGENTS.md split). |

## 15. Open questions (parking lot — decide by 0.2)

- Pressure model depth: full Powder-Toy-style velocity field vs diffusion-only.
- Default world walls flag.
- Liquid dispersion cap (cells/tick) — 4 vs 8.
- Playground framework: vanilla TypeScript vs a light React shell.

---

## Appendix A — Byte-level formats and the canonical hash

**World save `*.silica`** (little-endian):
`"SILW"` · `u16 format_version` · `u64 pack_hash` · `u64 seed` · `u32 tick` ·
`u16 width` · `u16 height` · `u32[4] rng_state` · RLE streams (`u32 run, u16 value`)
for `material`, then `temp`, then `aux`.

**Replay `*.silrep`:**
`"SILR"` · `u16 format_version` · `u64 pack_hash` · `u64 seed` · `u16 width` ·
`u16 height` · `u32 event_count` · events: `u32 tick, u16 kind, i16 x0, y0, x1,
y1, u16 material, u8 radius, u8 flow`.

**Canonical state hash** (FNV-1a 64 over): `u32 tick` LE · `u32[4] rng_state` LE ·
`material` row-major `u16` LE · `temp` · `aux`. Manifest lines:
`world_name \t tick \t hash-as-16-hex`.

## Appendix B — Default pack (MVP)

| id | material | category | notable behaviour |
|---|---|---|---|
| 0 | empty | — | vacuum |
| 1 | sand | powder | melts → molten_glass @ 1700 K |
| 2 | water | liquid | freezes @ 273 K, boils @ 373 K, douses fire |
| 3 | oil | liquid | flammable, floats on water |
| 4 | fire | energy | life in aux, decays → smoke, ignites flammables |
| 5 | smoke | gas | decays → empty |
| 6 | steam | gas | condenses → water below 373 K |
| 7 | wood | rigid | flammable, slow burn |
| 8 | coal | rigid | flammable, very slow burn |
| 9 | gunpowder | powder | deflagrates on fire contact (explosion spread) |
| 10 | acid | liquid | dissolves stone/wood (probabilistic) |
| 11 | lava | liquid | 1400 K, ignites, +water → stone + steam |
| 12 | stone | rigid | crumbles → sand; support-analysed |
| 13 | bedrock | solid | indestructible anchor |
| 14 | glass | rigid | shatters → sand on hard impact |
| 15 | molten_glass | liquid | freezes → glass below 1700 K |
| 16 | ice | rigid | melts → water @ 273 K |
| 17 | snow | powder | melts → water |
| 18 | plant | rigid | grows into adjacent water (aux), flammable |
| 19 | seed | powder | sprouts → plant on wet sand |
| 20 | spark | energy | travels along metal/wire |
| 21 | metal | solid | conducts spark, melts → lava @ 1500 K |
| 22 | wire | solid | conducts spark, cheap |
| 23 | iron | rigid | rusts: +water → rust (probabilistic) |
| 24 | rust | powder | inert |
| 25 | clone | solid | copies first contacted material into aux, emits it |
| 26 | void | solid | deletes anything entering |
| 27 | torch | solid | ignites neighbours, burns out via aux |
| 28 | gas | gas | flammable, rises |
| 29 | nitro | liquid | explodes on fire or hard impact |
