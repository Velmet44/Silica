//! The golden-manifest comparison test (TESTING.md pillar 1).
//!
//! Each checkpoint line of `golden/manifest.txt` is
//! `world \t tick \t hash-as-16-lowercase-hex` (SPEC.md appendix A).
//! The test runs every registered world once, stepping exactly to each
//! checkpoint tick, and asserts the canonical state hash. A manifest
//! hash may be regenerated **only** with a written justification in
//! the PR body ("what changed, why, which worlds affected" — AGENTS.md
//! rule 7); the generator helper below is deliberately #[ignore]-gated.

use silica_core::World;
use silica_tests::generators;

/// The registered golden worlds and their generators. Worlds appear
/// here in the order they land (TESTING.md pillar 1 table); the
/// manifest may only name names from this list.
fn generator_for(world: &str) -> World {
    match world {
        "column_collapse" => generators::column_collapse(),
        "lake_drain" => generators::lake_drain(),
        "forest_fire" => generators::forest_fire(),
        other => panic!(
            "manifest names unknown golden world '{other}' — register a generator in golden.rs"
        ),
    }
}

/// One parsed manifest line.
#[derive(Debug, PartialEq)]
struct Checkpoint {
    world: String,
    tick: u32,
    hash: u64,
}

/// Reads and validates `golden/manifest.txt`. Parsing is deliberately
/// strict (D10: malformed test data fails loudly, not quietly): every
/// data line must be exactly `name \t tick \t 16 hex digits`, sorted
/// by `(world, tick)` without duplicates.
fn load_manifest() -> Vec<Checkpoint> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/golden/manifest.txt");
    let text = std::fs::read_to_string(path).expect("golden manifest is missing");
    let mut checkpoints = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split('\t');
        let world = fields.next().unwrap_or_default().to_owned();
        let tick_text = fields.next().unwrap_or_default();
        let hash_text = fields.next().unwrap_or_default();
        if fields.next().is_some() {
            panic!("manifest line {}: too many tab fields: {line:?}", i + 1);
        }
        if world.is_empty() {
            panic!("manifest line {}: empty world name", i + 1);
        }
        let tick: u32 = tick_text
            .parse()
            .unwrap_or_else(|_| panic!("manifest line {}: bad tick '{tick_text}'", i + 1));
        if hash_text.len() != 16 || !hash_text.bytes().all(|b| b.is_ascii_hexdigit()) {
            panic!(
                "manifest line {}: hash must be 16 hex digits: '{hash_text}'",
                i + 1
            );
        }
        let hash = u64::from_str_radix(hash_text, 16)
            .unwrap_or_else(|_| panic!("manifest line {}: bad hash '{hash_text}'", i + 1));
        checkpoints.push(Checkpoint { world, tick, hash });
    }
    // Sorted by (world, tick), no duplicates — the suite relies on
    // running each world once, in lockstep with its checkpoints.
    let sorted = checkpoints.is_sorted_by(|a, b| match (a.world.as_str(), b.world.as_str()) {
        (w1, w2) if w1 == w2 => a.tick < b.tick,
        (w1, w2) => w1 < w2,
    });
    assert!(
        sorted,
        "manifest checkpoints must be sorted by (world, tick)"
    );
    checkpoints
}

/// Runs one world to each of its checkpoints (the manifest must
/// pre-group them), then hands the exhausted world back for extra
/// assertions.
fn verify(world: &mut World, checkpoints: &[&Checkpoint], world_name: &str) {
    for cp in checkpoints {
        while world.tick() < cp.tick {
            world.step(&[]).expect("golden world steps must not fail");
        }
        assert_eq!(world.tick(), cp.tick);
        let got = world.state_hash();
        assert_eq!(
            got,
            cp.hash,
            "golden '{world_name}' hash mismatch at tick {tick}: manifest {want:016x}, got {got:016x}",
            tick = cp.tick,
            want = cp.hash,
        );
    }
}

#[test]
fn golden_worlds_match_the_manifest() {
    let checkpoints = load_manifest();
    assert!(!checkpoints.is_empty(), "golden manifest is empty");

    // Group checkpoints by world (the manifest is validated sorted).
    let mut runs: Vec<(String, Vec<&Checkpoint>)> = Vec::new();
    for cp in &checkpoints {
        match runs.last_mut() {
            Some((name, group)) if *name == cp.world => group.push(cp),
            _ => runs.push((cp.world.clone(), vec![cp])),
        }
    }

    for (world_name, group) in &runs {
        let mut world = generator_for(world_name);
        // Sanity: two constructed instances must agree immediately —
        // a generator that is not deterministic is unfixable downstream.
        let twin = generator_for(world_name);
        assert_eq!(
            world.state_hash(),
            twin.state_hash(),
            "generator '{world_name}' is not deterministic at construction"
        );
        drop(twin);

        verify(&mut world, group, world_name);
        assert_settled(world_name, &mut world);
    }
}

/// Per-world settled-shape check: powder piles must come to rest.
///
/// The full state hash cannot be compared across ticks — appendix A
/// hashes the tick counter itself, so identical physics never yields
/// equal hashes at different ticks. The rest check therefore compares
/// the material grid (via the public read surface) after 1000 extra
/// ticks past the world's final checkpoint.
///
/// Worlds whose physics never rest (fire, steam, rain…) override this
/// with their own rule instead of inheriting the powder check.
fn assert_settled(world_name: &str, world: &mut World) {
    let material_grid = |w: &World| -> Vec<u16> {
        let mut mats = Vec::new();
        for y in 0..w.height() {
            for x in 0..w.width() {
                mats.push(w.cell(x, y).expect("grid cells are in-bounds").material);
            }
        }
        mats
    };
    // Worlds whose physics never rest (fire, steam, rain…) get their
    // own `else if` rule here instead of inheriting the powder check.
    if world_name == "column_collapse" {
        let before = material_grid(world);
        for _ in 0..1000 {
            world.step(&[]).expect("golden world steps must not fail");
        }
        assert_eq!(
            material_grid(world),
            before,
            "golden '{world_name}' is still rearranging 1000 ticks past its final checkpoint — perpetual motion?"
        );
    }
}

/// Manifest generation helper — **ignored by default**. Running it
/// prints appendix-A manifest lines for the registered worlds; those
/// lines are then committed as the locked manifest. This is the only
/// sanctioned path to new manifest hashes, and regenerating existing
/// ones requires a written PR justification (AGENTS.md rule 7).
#[test]
#[ignore = "prints manifest hashes — run only to generate or regenerate goldens, with AGENTS.md rule 7 justification"]
fn print_manifest_hashes() {
    for world_name in ["column_collapse", "lake_drain", "forest_fire"] {
        let mut world = generator_for(world_name);
        for tick in (500..=10_000).step_by(500) {
            while world.tick() < tick {
                world.step(&[]).expect("golden world steps must not fail");
            }
            println!("{world_name}\t{tick}\t{:016x}", world.state_hash());
        }
    }
}
