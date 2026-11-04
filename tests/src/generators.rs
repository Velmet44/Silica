//! The golden world generators (TESTING.md pillar 1, SPEC.md §11).
//!
//! Each generator is a `(seed, script)` pair expressed in code: build
//! the world, paint starting material, and hand the result to the
//! suite. Nothing here reads the clock, the environment, or any second
//! RNG (`SPEC.md` §8 D4/D6) — identical calls must return identically
//! seeded, identically painted worlds.
//!
//! The material registry comes from the compiled starter pack
//! (`silica-dsl` over `materials/`); sand is id 1 there, reserved per
//! appendix B. The pack's identity is pinned so a pack change that
//! shifts the golden worlds is caught here, before any hash moves
//! (AGENTS.md rule 7).

use std::path::Path;

use silica_core::{Brush, Category, Pack, World, WorldConfig};
use silica_dsl;

/// The starter pack the golden worlds run against — loaded from disk
/// (deterministic content, no clock or environment) and identity-checked
/// before any world is built.
fn starter_pack() -> Pack {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("materials");
    let pack = silica_dsl::load_pack(&dir).expect("starter pack must load");
    assert_eq!(
        pack.hash(),
        0xc8ce_7d9f_920f_d109,
        "rule 7: the starter pack identity changed — investigate before regenerating goldens"
    );
    assert_eq!(
        pack.category_of(1),
        Category::Powder,
        "rule 7: sand must stay a powder for column_collapse to keep its meaning"
    );
    pack
}

/// Appendix B, id 1: sand — the starter pack's powder, loaded from
/// the compiled registry (§6) rather than a hard-coded literal.
const SAND: u16 = 1;
/// Appendix B, id 2: water.
const WATER: u16 = 2;
/// Appendix B, id 3: oil — floats on water (density 850 < 1000).
const OIL: u16 = 3;
/// Appendix B, id 4: fire.
const FIRE: u16 = 4;
/// Appendix B, id 7: wood.
const WOOD: u16 = 7;

/// World #1 — `column_collapse` (TESTING.md pillar 1: "powder piling,
/// angle of repose").
///
/// Script: a 5×20 sand column centered over the floor (cells
/// `x ∈ [29..=33]`, `y ∈ [4..=23]`, seed 0, walls on, 64×48). The
/// column drops, lands, and spreads into a pile; pile then stands
/// still, so every checkpoint hash from ~tick 100 on is the settled
/// shape.
///
/// Ratified choices (D-010 (2)/(3), D-009):
/// * **seed 0** — powder movement never draws from the RNG (D4
///   consumption points come with reactions/gas jitter), so the seed
///   participates in the hash only through the initial `rng_state`
///   words; zero is the canonical "nothing up my sleeve" value here.
///   Ratified in D-010's note as accepted detail.
/// * **64×48** — compact but wide enough that the pile never reaches
///   the walled side edges, so the golden exercises falling, landing,
///   piling, and rest without edge interactions.
/// * Sand painted at tick 0 (stamp 0) does not move during tick 0
///   itself — the §5 single-update rule (D-010 (3): paint and state
///   hash are independent of where `apply_inputs` sits in the pass
///   order).
pub fn column_collapse() -> World {
    let pack = starter_pack();
    let mut world = World::new(WorldConfig {
        width: 64,
        height: 48,
        seed: 0,
        walls: true,
        pack: &pack,
    });
    world
        .paint(&Brush::rect(29, 4, 33, 23, SAND))
        .expect("sand is defined by the starter pack");
    world
}

/// World #2 — `lake_drain` (TESTING.md pillar 1: liquids, dispersion,
/// density ordering).
///
/// Script: a layer of oil (density 850) over water (density 1000) in a
/// walled 64×48 world. Water, being heavier, sinks through the oil
/// (density swap, draft D-012) and pools at the bottom; oil floats to
/// the top and disperses horizontally (draft D-012 cap = 4). No RNG is
/// consumed at paint (neither water nor oil declares `aux_init`), so
/// the seed participates only through the initial `rng_state` words.
/// The world settles once the two liquids have separated.
pub fn lake_drain() -> World {
    let pack = starter_pack();
    let mut world = World::new(WorldConfig {
        width: 64,
        height: 48,
        seed: 0,
        walls: true,
        pack: &pack,
    });
    // Water basin first, then oil on top.
    world
        .paint(&Brush::rect(8, 24, 55, 32, WATER))
        .expect("water is defined by the starter pack");
    world
        .paint(&Brush::rect(8, 16, 55, 23, OIL))
        .expect("oil is defined by the starter pack");
    world
}

/// World #3 — `forest_fire` (TESTING.md pillar 1: combustion, fire life,
/// smoke decay).
///
/// Script: a wooden box (wood, id 7) in a walled 64×48 world with a
/// single ignition point (fire, id 4). Fire spreads to adjacent wood
/// via the `ignite` reaction (probability 0.05), burns out to smoke
/// when its aux (fire life, 30..90) reaches 0 (draft D-015 decay = 1
/// per tick), and smoke dissipates to empty (probability 0.001). Heat
/// diffusion (draft D-015, 4-neighbour average) carries temperature
/// between cells. RNG consumption happens at paint (fire's `aux_init`
/// draw per spawned fire cell) and at every reaction attempt (D4).
pub fn forest_fire() -> World {
    let pack = starter_pack();
    let mut world = World::new(WorldConfig {
        width: 64,
        height: 48,
        seed: 0,
        walls: true,
        pack: &pack,
    });
    // Wooden box.
    world
        .paint(&Brush::rect(20, 20, 44, 20, WOOD))
        .expect("wood is defined by the starter pack");
    world
        .paint(&Brush::rect(20, 35, 44, 35, WOOD))
        .expect("wood is defined by the starter pack");
    world
        .paint(&Brush::rect(20, 21, 20, 34, WOOD))
        .expect("wood is defined by the starter pack");
    world
        .paint(&Brush::rect(44, 21, 44, 34, WOOD))
        .expect("wood is defined by the starter pack");
    // Ignition point at the top of the box.
    world
        .paint(&Brush::rect(32, 20, 32, 20, FIRE))
        .expect("fire is defined by the starter pack");
    world
}
