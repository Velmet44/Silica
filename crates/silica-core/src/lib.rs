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
//! tick pipeline is not implemented — nothing draws from the RNG yet, and
//! the write primitives stay crate-private until the passes take shape.
//! The `pack` field of the §9 `WorldConfig` sketch arrives with the §6
//! compiled registry.

#![deny(clippy::float_arithmetic)]
#![forbid(unsafe_code)]

mod error;
mod region;
mod rng;
mod temp;
mod world;

pub use error::SilicaError;
pub use temp::Temp;
pub use world::{CellView, World, WorldConfig};
