//! The golden world generators (TESTING.md pillar 1, SPEC.md §11).
//!
//! Each generator is a `(seed, script)` pair expressed in code: build
//! the world, paint starting material, and hand the result to the
//! suite. Nothing here reads the clock, the environment, or any second
//! RNG (`SPEC.md` §8 D4/D6) — identical calls must return identically
//! seeded, identically painted worlds.

use silica_core::{Brush, World, WorldConfig};

/// Appendix B, id 1: sand — the starter pack's only moving material in
/// the 0.1 skeleton (the §6 compiled registry will replace the literal
/// once it lands; the id is reserved and stable).
const SAND: u16 = 1;

/// World #1 — `column_collapse` (TESTING.md pillar 1: "powder piling,
/// angle of repose").
///
/// Script: a 5×20 sand column centered over the floor (cells
/// `x ∈ [29..=33]`, `y ∈ [4..=23]`, seed 0, walls on, 64×48). The
/// column drops, lands, and spreads into a pile; pile then stands
/// still, so every checkpoint hash from ~tick 100 on is the settled
/// shape.
///
/// Noted choices for review:
/// * **seed 0** — powder movement never draws from the RNG (D4
///   consumption points come with reactions/gas jitter), so the seed
///   participates in the hash only through the initial `rng_state`
///   words; zero is the canonical "nothing up my sleeve" value here.
/// * **64×48** — compact but wide enough that the pile never reaches
///   the walled side edges, so the golden exercises falling, landing,
///   piling, and rest without edge interactions.
/// * Sand painted at tick 0 (stamp 0) does not move during tick 0
///   itself — the §5 single-update rule — so effective falling starts
///   at tick 1.
pub fn column_collapse() -> World {
    let mut world = World::new(WorldConfig {
        width: 64,
        height: 48,
        seed: 0,
        walls: true,
    });
    world.paint(&Brush::rect(29, 4, 33, 23, SAND));
    world
}
