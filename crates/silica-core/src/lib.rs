//! Silica core — an embeddable falling-sand simulation engine.
//!
//! Cellular automata over a fixed grid (`SPEC.md` §4): powders, liquids,
//! gases, solids and rigid islands, with heat and coarse pressure on top.
//! This crate is the simulation and nothing else — the material DSL
//! front-end, save/replay formats and language bindings live in sibling
//! crates. It is a library, not a game.
//!
//! # Determinism contract
//!
//! The rules of `SPEC.md` §8 are law for everything below this crate root:
//!
//! * **D1** integer-only arithmetic, enforced by the lint below; floats
//!   live only in host-facing helpers outside this crate. Temperature is
//!   fixed-point `i16`, 0.25 K per unit, clamped on store.
//! * **D2** overflow-capable arithmetic uses `wrapping_*`, `checked_*` or
//!   `saturating_*`; release builds keep overflow checks on.
//! * **D3** iteration order is fixed and documented per pass (`SPEC.md`
//!   §5); no `HashMap`/`HashSet` iteration in simulation code.
//! * **D4** one PRNG (xoshiro128\*\*) drawn from only at the documented
//!   consumption points; RNG state is part of world state.
//! * **D5/D6** no threads, no time/date/address-dependent behavior.
//! * **D7** serialization and hashes run over canonical little-endian
//!   buffers (`SPEC.md` appendix A).
//!
//! # Status
//!
//! The world model of `SPEC.md` §4 is implemented: SoA cell storage, the
//! fixed-point temperature type, the 32×32 region grid with activity
//! flags, the stamp trick, anchors, and the seeded RNG stream. The §5
//! tick skeleton is in place: seven passes run in the documented order
//! with `World::step`, the tick counter advances once per step, and the
//! canonical state hash (FNV-1a 64 over the appendix-A buffer) is
//! exposed — its layout, the sweep parity and the paint-skip rule were
//! ratified as D-010. Brush strokes (`Brush`, `World::paint`) enter the
//! world; powder movement is real, dispatched through the compiled pack
//! registry (§6): `WorldConfig` takes `pack: &Pack`, ids validate
//! against the registry at the paint boundary, and painting a cell
//! **spawns** the material — its `default_temp` and a fresh aux word
//! (one uniform D4 draw in `[lo, hi]` when the material declares
//! `aux_init`; the first RNG consumption point, pre-first-golden per
//! D-009). Liquids, gases, reactions, heat, islands and pressure are
//! still no-op stages pending their milestones; the DSL loader that
//! compiles packs lives in `silica-dsl`.

#![deny(clippy::float_arithmetic)]
#![forbid(unsafe_code)]

mod brush;
mod error;
mod hash;
mod pack;
mod region;
mod rng;
mod temp;
mod tick;
mod world;

pub use brush::Brush;
pub use error::SilicaError;
pub use hash::Fnv1a64;
pub use pack::{
    AuxRange, Category, MaterialDef, PROBABILITY_DENOMINATOR, Pack, PackProblem, Reaction, Spread,
    Transition, Trigger,
};
pub use temp::Temp;
pub use world::{CellView, World, WorldConfig};
