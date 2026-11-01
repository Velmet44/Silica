# AGENTS.md — Rules for opencode

You are working on **Silica**, an embeddable falling-sand simulation engine in
Rust (MIT). It is a library, not a game. `SPEC.md` is the authority: if code
and spec disagree, the spec wins — fix the code and say so in the PR.

## Hard rules (never violate, never "fix" silently)

1. **No floating-point arithmetic in `silica-core`.** The crate root carries
   `#![deny(clippy::float_arithmetic)]`. Temperature is fixed-point (`i16`,
   0.25 K units). If you need fractions, use integer math with defined rounding.
2. **All overflow-capable arithmetic uses `wrapping_*`, `checked_*` or
   `saturating_*`.** Release builds run with `overflow-checks = true`; any
   plain `+`/`-`/`*` that can overflow is a CI failure, not a style nit.
3. **Iteration order is part of the behavior.** Follow the scan orders in
   SPEC §5 exactly. Never iterate a `HashMap`/`HashSet` in simulation code —
   use `Vec` sorted by id or `BTreeMap`.
4. **One RNG stream** (xoshiro128\*\*, SPEC §8/D4). Draw from it only at the
   documented consumption points. Never seed a second generator, never use
   time, addresses, or thread ids.
5. **Zero `unsafe` in `silica-core` and `silica-dsl`.** `unsafe` is allowed
   only in `silica-capi`/`silica-wasm` boundary shims, each with a `// SAFETY:`
   comment explaining the invariant.
6. **Zero new dependencies in `silica-core`.** No deps at all — not even
   `serde`. Other crates may use vetted deps (`serde`, `toml`, `cbindgen`,
   `wasm-bindgen`, `proptest`, `criterion`).
7. **Never regenerate golden files to make CI pass.** Golden hashes change
   only when behavior *should* change, and only with a written justification
   in the PR ("what changed, why, which worlds affected"). If a golden fails
   and you don't know why, stop and report — do not regenerate.
8. **Public API changes ship with a SPEC.md update in the same PR.** SPEC §9
   is the surface contract; formats in appendix A are byte-level contracts.
9. **No panics on valid input** in library code (SPEC §8/D10). Return
   `Result` with the crate error enum; `unwrap`/`expect` are test-only.
10. **The steady-state tick allocates nothing.** If you need a buffer, take it
    from the preallocated world state.

## Division of labor

**The human writes** (taste-critical, subtle-bug-expensive):
- movement-pass inner loop and update ordering
- rigid-island integration and collision
- the canonical hash and serialization byte layout
- performance budgets and benchmark interpretation
- anything under SPEC §8 (the determinism contract)

**You write** (specifiable, testable, boilerplate-shaped):
- tests of every kind: unit, golden generators, differential, property
- `silica-dsl` loader + validation
- `silica-formats` save/load/replay
- `silica-capi` and `silica-wasm` surfaces
- tools/CLI (`render`, `replay`, `hash`, `soak`)
- examples, playground glue, CI config, docs, and material TOML files
- benches scaffolding and failure reports

## Workflow per feature

1. Read the SPEC section that governs the feature. Quote the rule you are
   implementing in the PR description.
2. **Write the failing tests first** — unit test for the logic, plus a golden
   generator if behavior is visible in world state.
3. Implement. Keep diffs minimal; no drive-by refactors of the hot loops.
4. `cargo fmt`, `cargo clippy -D warnings`, `cargo test` — all green.
5. If you touched movement, reactions, heat, or rigid: run the golden suite
   and the differential suite. If a golden hash changes, stop (rule 7).
6. If you touched anything in a tick pass: run `cargo bench` before/after and
   report the delta. Regressions > 5% block the PR.
7. PR description template: *What rule from SPEC §… · What tests prove it ·
   Bench delta · Golden impact (must be "none")*.

## Conventions

- Module names and API follow SPEC §9/§11. New modules go where the layout
  says, not in a new top-level directory.
- Errors: `thiserror` enum per crate, `pub enum SilicaError` in core.
- Commit style: `core(movement): alternate column scan by tick parity` —
  area(scope): imperative summary, one logical change per commit.
- Doc comments on every public item: one line of what, one line of the
  determinism/performance contract if relevant.
- Directory-scoped rule files may layer stricter rules (e.g. a future
  `crates/silica-core/AGENTS.md`); they never weaken this file.
