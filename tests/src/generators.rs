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
