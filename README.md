# Silica — Spec Package

Everything needed to start building **Silica**, the embeddable falling-sand
simulation engine, with opencode driving implementation in copilot mode.
Derived from *Ten Engines Nobody Built — The Addition Edition* (project #1,
score 24/25) and locked by the decisions in `DECISIONS.md`.

## Contents

| File | Purpose |
|---|---|
| `SPEC.md` | The engine specification — architecture, tick pipeline, DSL, rigid islands, determinism contract, formats, acceptance criteria. **The authority.** |
| `AGENTS.md` | Rules opencode reads before touching anything: hard rules, division of labor, per-feature workflow, conventions. Copy to the repo root unchanged. |
| `TESTING.md` | Three test pillars: golden worlds, differential testing, property fuzzing + CI matrix and the golden-regeneration policy. |
| `ROADMAP.md` | Demo-driven milestone ladder 0.1 → 1.0, each with scope, artifact, acceptance and distribution beat. |
| `DECISIONS.md` | The locked decision record (language, MVP scope, determinism, compute, DSL, MIT license, name, kit) with rationale. |
| `ci.yml` | GitHub Actions sketch — place at `.github/workflows/ci.yml`. |
| `LICENSE` | MIT license text (D-006) — copy to the repo root and every crate. |
| `materials/` | Starter pack for milestone 0.1: `pack.toml` + 8 loadable materials (sand, water, oil, fire, smoke, steam, wood, stone) with ids reserved per SPEC appendix B. |

## Assembling the repository (one-time)

```bash
mkdir silica && cd silica && git init
# copy SPEC.md AGENTS.md TESTING.md ROADMAP.md DECISIONS.md LICENSE to the root
cp -r /path/to/materials .          # starter pack, SPEC §6 schema
mkdir -p .github/workflows && cp /path/to/ci.yml .github/workflows/ci.yml
cargo new --lib crates/silica-core
# create the remaining workspace members per SPEC §11, then:
cargo init --workspace  # or hand-write the root Cargo.toml
```

Pin the toolchain (`rust-toolchain.toml`), set
`[profile.release] overflow-checks = true` in the root `Cargo.toml`, add the
`#![deny(clippy::float_arithmetic)]` attribute to `crates/silica-core/src/lib.rs`
on day one, and commit the first three golden worlds before feature work starts.

## opencode session 1 — exact first prompts

Copy `AGENTS.md` to the repo root, start opencode, and open with:

> Read AGENTS.md and SPEC.md fully. Then implement SPEC §4 (world model):
> the SoA cell storage, the fixed-point temperature type with clamped store,
> the region grid with activity flags, and the stamp trick. Write the unit
> tests first — including bounds, clamping, and the region set/clear
> invariants from TESTING.md pillar 3. Do not implement the tick pipeline yet.

Then, in the same or next session:

> Read SPEC §5 and §8. Implement the tick skeleton: the seven passes as
> no-op-able stages in the exact documented order, the tick counter, and the
> canonical state hash from appendix A (FNV-1a 64 over the defined buffer).
> Add the first golden world (a single column of sand falling into a pile) and
> the manifest comparison test. Report the manifest hash for my review.

From there, follow ROADMAP 0.1 ("The GIF") — the movement pass, 8 materials
as a hard-coded registry, and the `silica render` CLI.

## Reminders

- The steady-state tick allocates nothing; the golden manifest is law
  (AGENTS.md rule 7); the MIT text ships as `LICENSE` — repo root and every
  crate.
- `materials/` is loadable from day one (schema: SPEC §6 + its loader
  notes); ids are permanent, and additive materials never change existing
  golden hashes (only the pack hash moves).
- Publish targets: crates.io **`silica-sim`** (the `silica` name is a dead
  2018 stub), npm `silica-sim`, C header via cbindgen.
- Every milestone ships a demo artifact — see ROADMAP.md.
