# Silica — Testing Strategy

Determinism is what makes this codebase cheap to test: a 64-bit hash is a
complete assertion over the entire world state. Every pillar below exploits
that. `AGENTS.md` rule 7 governs golden regeneration; this file defines the
suites.

## Pillar 1 — Golden worlds (behavior lock)

**What:** ten committed worlds, each defined as (seed + scripted input) **code**,
never binary blobs. The suite runs each world for 10,000 ticks, hashing state
every 500 ticks, and compares against `tests/golden/manifest.txt`
(SPEC appendix A format: `world \t tick \t hash`).

**The worlds (each targets a subsystem):**

| # | Name | Exercises |
|---|------|-----------|
| 1 | `column_collapse` | powder piling, angle of repose |
| 2 | `lake_drain` | liquids, dispersion, density swap (oil on water) |
| 3 | `forest_fire` | combustion spread, fire life, smoke decay |
| 4 | `furnace` | heat diffusion, melt/freeze transitions, molten glass |
| 5 | `gunpowder_line` | explosions, pressure spikes |
| 6 | `ice_melt` | phase transitions, water pooling |
| 7 | `acid_quarry` | probabilistic dissolution |
| 8 | `ceiling_carve` | rigid support analysis, island spawn, crumble/weld |
| 9 | `pressure_chamber` | coarse pressure field, gas push |
| 10 | `random_soup` | cross-material chaos; generated from seed 0xC0FFEE |

**Rules:** the suite must pass on Linux, Windows, macOS (arm64) and wasm32 —
that *is* determinism rule D8. A hash change without a PR justification is a
build failure, not a diff to commit.

## Pillar 2 — Differential testing (correctness anchor)

- **Reference interpreter (`tests/ref/`):** a deliberately naive, obviously
  correct implementation of the same DSL and tick pipeline — no regions, no
  stamps, no batching, cell-by-cell in spec order. Randomized worlds (seeded)
  run through both engines; hashes must match at every 500-tick checkpoint.
  This is what makes later optimizations (SIMD, D12) safe.
- **Analytic cases:** a 1-D heat rod (two material columns, fixed boundary
  temps) compared against the closed-form fixed-point solution with ±1-unit
  tolerance after N ticks — documents where integer rounding may drift; a
  poured-sand pile whose final slope must land in 30–40° (statistical,
  non-blocking).
- **SUMO-style scenario outputs are N/A here** — that was the traffic engine's
  anchor; Silica's anchors are the reference interpreter and physics invariants.

## Pillar 3 — Property-based fuzzing (invariant hunting)

`proptest` suites asserting, for randomized seeds and input scripts:

1. **Determinism:** running the same (seed, inputs) twice yields identical
   hashes — the property the whole engine stands on.
2. **Conservation:** in a walled world with reactions disabled, per-material
   cell counts never change.
3. **Bounds:** temperatures stay in `[0, 32767]`; aux never wraps; material
   ids always exist in the registry; no cell is updated twice in one tick.
4. **Island bookkeeping:** island cells + grid cells are conserved across
   freeze/merge; islands never overlap occupied grid cells after merge.
5. **Roundtrips:** save→load preserves the hash; replaying an input log
   reproduces the live run's hash exactly.
6. **API abuse:** out-of-bounds coordinates, empty input logs, and corrupt
   save files return errors, never panic (D10).

## CI integration (see `ci.yml` sketch)

| Job | Runs on | Gates |
|---|---|---|
| `fmt-clippy` | ubuntu | format + `clippy -D warnings` |
| `test` | ubuntu, windows, macos (arm64), wasm32 (node) | unit + property suites |
| `golden` | all four platforms | 10 worlds × 10k ticks vs manifest |
| `differential` | ubuntu | reference interpreter vs optimized, 50 random worlds |
| `bench` | ubuntu (nightly, informational artifact) | criterion on 3 canonical worlds; fail > 5% regression |
| `soak` | weekly cron, ubuntu | 1M ticks on `random_soup`: hash stability + 250 MB memory ceiling |

**Coverage:** line coverage is a weak metric here — the golden suite is
full-state coverage. Target ≥ 85% lines in `silica-core` anyway, and treat any
golden-visible behavior without a named world as a gap to fill.

## Test-data policy

Worlds are generators (seeded scripts in Rust), not committed binaries. The
only committed artifacts are the manifest (text, ~200 lines) and three small
canonical benchmark worlds. If a test needs a crafted save file, generate it
in the test from a seed.
