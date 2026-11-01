//! The world: SoA cell storage, regions, stamps, RNG, the compiled
//! pack registry, the tick surface (SPEC.md §3/§4/§6/§9) and the
//! canonical state hash (appendix A).

use crate::brush;
use crate::brush::Brush;
use crate::error::SilicaError;
use crate::hash::Fnv1a64;
use crate::pack::{AuxRange, Pack};
use crate::region::RegionGrid;
use crate::rng::Rng;
use crate::temp::Temp;

/// Configuration for [`World::new`] (SPEC.md §9).
///
/// `pack` is the compiled material registry (SPEC.md §6): the tick
/// dispatches movement on its category table, and paint strokes are
/// only accepted for ids it defines. The world owns a `Clone` of the
/// pack, so one registry can be shared by many worlds (and its
/// content-hash joins saves and replays, D11, once formats land —
/// ROADMAP.md 0.3).
#[derive(Clone, Copy, Debug)]
pub struct WorldConfig<'pack> {
    /// Grid width in cells, 1..=u16::MAX.
    pub width: u16,
    /// Grid height in cells, 1..=u16::MAX.
    pub height: u16,
    /// Seed for the single RNG stream (SPEC.md §8 D4).
    pub seed: u64,
    /// Whether the map edges behave as walls — anchors per SPEC.md §4.
    /// Not defaulted: hosts state it. Whether SPEC.md §15 should
    /// eventually ratify `true` stays open until 0.2.
    /// Read only by [`World::is_anchor`], which the §7 rigid support
    /// analysis will call (ROADMAP.md 0.3).
    #[allow(dead_code)]
    pub walls: bool,
    /// The compiled pack (SPEC.md §6).
    pub pack: &'pack Pack,
}

/// A read-only snapshot of one cell (SPEC.md §9).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CellView {
    /// Material id; 0 is `empty` (SPEC.md §4).
    pub material: u16,
    /// Cell temperature in fixed-point 0.25 K units.
    pub temp: Temp,
    /// Category-defined aux word: fire life, plant growth, clone memory…
    pub aux: u16,
}

/// The simulation world: the grid plus RNG state, tick counter and
/// region grid (SPEC.md §3/§4).
///
/// Storage is struct-of-arrays, row-major — `material`, `temp`, `aux`,
/// `stamp` — preallocated at creation, so the steady-state tick never
/// allocates (SPEC.md §5). Every write stamps the cell with the current
/// tick (the bookkeeping behind §5's single-update rule) and wakes the
/// cell's region. Cells are addressed `(x, y)` with `x` the column.
///
/// The fields are crate-visible so the pass modules (`tick.rs`) can scan
/// the SoA arrays directly; the movement inner loop is hot-path code and
/// must not pay an accessor indirection. Public API stays the §9 sketch
/// surface; outside this crate the world is opaque.
pub struct World {
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) walls: bool,
    pub(crate) tick: u32,
    pub(crate) rng: Rng,
    pub(crate) regions: RegionGrid,
    /// The world's compiled pack: the registry the movement dispatch
    /// (and every other pass) consults, cloned from the config at
    /// construction and never mutated afterwards (SPEC.md §4/§6).
    pub(crate) pack: Pack,
    pub(crate) material: Vec<u16>,
    pub(crate) temp: Vec<i16>,
    pub(crate) aux: Vec<u16>,
    pub(crate) stamp: Vec<u32>,
    /// Pass audit trail; exists only under test so the §5 order is
    /// pinned by tests. Stripped from release builds.
    #[cfg(test)]
    pub(crate) pass_trace: Vec<&'static str>,
}

impl World {
    /// Creates a world: empty cells at ambient temperature (295 K), all
    /// regions asleep, the RNG seeded, tick 0.
    ///
    /// # Panics (programmer error, SPEC.md §8 D10)
    ///
    /// Panics if `width` or `height` is 0 — a zero-sized world is a
    /// degenerate configuration, not valid input. Absurdly large
    /// configurations may instead abort on allocation failure.
    pub fn new(config: WorldConfig<'_>) -> World {
        assert!(config.width != 0, "world width must be non-zero");
        assert!(config.height != 0, "world height must be non-zero");
        // u16 × u16 ≤ 65535², which fits u64 — and every supported usize.
        let cells = (config.width as u64 * config.height as u64) as usize;
        World {
            width: config.width,
            height: config.height,
            walls: config.walls,
            tick: 0,
            rng: Rng::new(config.seed),
            regions: RegionGrid::new(config.width, config.height),
            pack: config.pack.clone(),
            material: vec![0; cells],
            temp: vec![Temp::AMBIENT.0; cells],
            aux: vec![0; cells],
            stamp: vec![u32::MAX; cells],
            #[cfg(test)]
            pass_trace: Vec::new(),
        }
    }

    /// World width in cells.
    pub fn width(&self) -> u16 {
        self.width
    }

    /// World height in cells.
    pub fn height(&self) -> u16 {
        self.height
    }

    /// The current tick; 0 until the pipeline starts stepping.
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// The current RNG state — part of world state, so it is hashed,
    /// saved and replayed (SPEC.md §8 D4).
    pub fn rng_state(&self) -> [u32; 4] {
        self.rng.state()
    }

    /// Reads one cell, or `None` outside the grid (SPEC.md §9).
    pub fn cell(&self, x: u16, y: u16) -> Option<CellView> {
        let idx = self.cell_index(x, y)?;
        Some(CellView {
            material: self.material[idx],
            temp: Temp(self.temp[idx]),
            aux: self.aux[idx],
        })
    }

    /// Paints a brush at the current tick.
    ///
    /// Every covered in-bounds cell is a **fresh spawn** of the brush
    /// material: it takes the material's `default_temp` and a fresh
    /// aux word — 0, or one uniform draw in the material's `[lo, hi]`
    /// `aux_init` range from the D4 stream (SPEC.md §6 loader notes;
    /// the spawn is why those constants live in the registry).
    /// Painting `empty` (id 0) erases the covered cells to a fresh
    /// empty cell: vacuum, ambient temperature, aux 0.
    ///
    /// Cells painted this tick do not move again this tick: the §5
    /// single-update rule sees their fresh stamp (that placement behavior
    /// was ratified as D-010's paint rule — paint and state hash are
    /// independent of where `apply_inputs` sits in the pass order).
    /// Covered cells are enumerated row-major ascending, which fixes
    /// the aux draw order for the pack's RNG consumption contract.
    ///
    /// The brush material must be defined by the world's pack; an
    /// unknown id is reported per D10 and nothing is written.
    pub fn paint(&mut self, brush: &Brush) -> Result<(), SilicaError> {
        brush::paint(self, brush)
    }

    /// The canonical state hash (SPEC.md §9, appendix A): FNV-1a 64
    /// over `u32 tick` LE · `u32[4] rng_state` LE · then three separate
    /// row-major `u16` LE streams — `material`, then `temp`, then `aux`
    /// — mirroring the save format's "for material, then temp, then
    /// aux". Streams the bytes directly; nothing is allocated (§5).
    ///
    /// Canonical layout note: appendix A hashes three separate
    /// row-major streams — material, then temp, then aux — and *not* a
    /// per-cell interleaved `(material, temp, aux)` buffer; both are
    /// admissible readings of the appendix's one-liner. The choice was
    /// ratified as D-010 (1) (the interleaved reading would hash this
    /// 2×2 state to `0xe890_3a54_71cf_3701`; the known-answer tests
    /// encode the three-streams value on both sides of the boundary).
    ///
    /// The buffer does not include stamps or region activity: they are
    /// derived bookkeeping recomputable from the painted history, not
    /// simulation state.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv1a64::new();
        h.update(&self.tick.to_le_bytes());
        for word in self.rng.state() {
            h.update(&word.to_le_bytes());
        }
        for &m in &self.material {
            h.update(&m.to_le_bytes());
        }
        for &t in &self.temp {
            // Temperature is non-negative by construction (clamped on
            // store); hashing `i16::to_le_bytes` gives the canonical
            // little-endian two's-complement bytes either way.
            h.update(&t.to_le_bytes());
        }
        for &a in &self.aux {
            h.update(&a.to_le_bytes());
        }
        h.finish()
    }

    /// Writes a cell's material id.
    ///
    /// Registry validation happens at the paint boundary
    /// (`brush::paint`); the pass-facing primitives accept any id
    /// because they only ever recycle ids already in the grid. Stamps
    /// the cell and wakes its region.
    pub(crate) fn set_material(&mut self, x: u16, y: u16, id: u16) -> Result<(), SilicaError> {
        let idx = self
            .cell_index(x, y)
            .ok_or(SilicaError::OutOfBounds { x, y })?;
        self.material[idx] = id;
        self.touch(idx, x, y);
        Ok(())
    }

    /// Writes a cell temperature from `i32` units, clamped on store
    /// (SPEC.md §4). Stamps the cell and wakes its region.
    #[allow(dead_code)] // consumed by the §5 heat pass
    pub(crate) fn set_temp(&mut self, x: u16, y: u16, units: i32) -> Result<(), SilicaError> {
        let idx = self
            .cell_index(x, y)
            .ok_or(SilicaError::OutOfBounds { x, y })?;
        self.temp[idx] = Temp::from_units(units).0;
        self.touch(idx, x, y);
        Ok(())
    }

    /// Writes a cell's aux word as-is; aux arithmetic arrives with §5/§6
    /// and never wraps (TESTING.md pillar 3). Stamps + wakes.
    #[allow(dead_code)] // consumed by the §5/§6 aux-driven behaviours
    pub(crate) fn set_aux(&mut self, x: u16, y: u16, aux: u16) -> Result<(), SilicaError> {
        let idx = self
            .cell_index(x, y)
            .ok_or(SilicaError::OutOfBounds { x, y })?;
        self.aux[idx] = aux;
        self.touch(idx, x, y);
        Ok(())
    }

    /// Spawns one cell of material `id`: material, temperature and aux
    /// written together, stamped with the current tick, region woken —
    /// exactly one audit write. The paint path (and later the §7
    /// crumble and reaction spawns) go through here so every spawn has
    /// identical bookkeeping.
    pub(crate) fn spawn(
        &mut self,
        x: u16,
        y: u16,
        id: u16,
        temp: i16,
        aux: u16,
    ) -> Result<(), SilicaError> {
        let idx = self
            .cell_index(x, y)
            .ok_or(SilicaError::OutOfBounds { x, y })?;
        self.material[idx] = id;
        self.temp[idx] = temp;
        self.aux[idx] = aux;
        self.touch(idx, x, y);
        Ok(())
    }

    /// One uniform aux draw in the range's span from the single
    /// stream (SPEC.md §6 loader notes, D4 consumption point): the
    /// canonical mapping is `lo + word % span`, where `word` is one
    /// full xoshiro128** output — bias below 2^-15 for every
    /// u16-scale range and pinned against a mirror stream by the
    /// brush tests. Paint enumerates its covered cells row-major, so
    /// the cover order fixes the draw order.
    ///
    /// Always consumes exactly one draw per spawned cell of a
    /// material with `aux_init` — even a range with `lo == hi` draws
    /// (the schedule depends on the pack shape, never on values).
    pub(crate) fn draw_aux(&mut self, range: AuxRange) -> u16 {
        let word = self.rng.next_u32();
        let offset = word % range.span();
        // lo + offset ≤ hi ≤ 65535 — the sum fits u16 trivially
        // (documented non-capable, §8 D2).
        (u32::from(range.lo) + offset) as u16
    }

    /// The tick this cell was last written; `u32::MAX` means never.
    ///
    /// The §5 single-update rule compares stamps against the current
    /// tick. Initial stamps are `u32::MAX`, which no early tick equals;
    /// the guarantee degrades only if a run reaches tick `u32::MAX`.
    #[allow(dead_code)] // consumed by the §5 movement pass hot loop
    pub(crate) fn stamp_of(&self, x: u16, y: u16) -> Option<u32> {
        Some(self.stamp[self.cell_index(x, y)?])
    }

    /// Whether `(x, y)` is supported ground for rigid support analysis
    /// (SPEC.md §4, consumed by §7): the bottom row always; the map
    /// edges when `walls` is set.
    ///
    /// Coordinates are assumed in-bounds; out-of-bounds queries are
    /// panic-free but their result is unspecified.
    #[allow(dead_code)] // consumed by the §7 rigid support analysis
    pub(crate) fn is_anchor(&self, x: u16, y: u16) -> bool {
        if y == self.height - 1 {
            return true; // the bottom row is always supported ground
        }
        self.walls && (x == 0 || y == 0 || x == self.width - 1)
    }

    /// Row-major index of `(x, y)`, or `None` outside the grid.
    pub(crate) fn cell_index(&self, x: u16, y: u16) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        // Checked per SPEC.md §8 D2; the guard above makes `None`
        // unreachable on every supported target (u16 grid, ≥32-bit usize).
        (y as usize)
            .checked_mul(self.width as usize)
            .and_then(|row| row.checked_add(x as usize))
    }

    /// Records a write: stamp with the current tick, wake the region.
    pub(crate) fn touch(&mut self, idx: usize, x: u16, y: u16) {
        self.stamp[idx] = self.tick;
        self.regions.activate_cell(x, y);
    }
}

#[cfg(test)]
mod tests {
    use super::{World, WorldConfig};
    use crate::brush::Brush;
    use crate::error::SilicaError;
    use crate::pack::{Pack, test_packs};
    use crate::rng::Rng;
    use crate::temp::Temp;

    /// Appendix B, id 1: sand — the shared starter test-pack keeps this
    /// id for sand (the dsl tests pin the real pack's ids identically).
    const SAND_TEST_ID: u16 = 1;

    /// Test config: walls on, the shared starter pack (borrowed).
    /// Ids painted against it are checked at the paint boundary, so
    /// tests only paint ids the starter defines (or none at all).
    fn cfg(width: u16, height: u16, seed: u64, pack: &Pack) -> WorldConfig<'_> {
        WorldConfig {
            width,
            height,
            seed,
            walls: true,
            pack,
        }
    }

    #[test]
    fn new_initializes_every_cell_empty_at_ambient() {
        let pack = test_packs::starter();
        let world = World::new(cfg(512, 384, 42, &pack));
        let cells = 512 * 384usize;
        assert_eq!(world.material.len(), cells);
        assert_eq!(world.temp.len(), cells);
        assert_eq!(world.aux.len(), cells);
        assert_eq!(world.stamp.len(), cells);
        for y in 0..384u16 {
            for x in 0..512u16 {
                let cell = world.cell(x, y).unwrap();
                assert_eq!(cell.material, 0);
                assert_eq!(cell.temp, Temp::AMBIENT);
                assert_eq!(cell.aux, 0);
                assert_eq!(world.stamp_of(x, y), Some(u32::MAX));
            }
        }
    }

    /// `WorldConfig` carries no `Default` — hosts state every field,
    /// the pack included. The world holds its own `Clone` of it.
    #[test]
    fn config_carries_the_pack_and_the_world_clones_it() {
        let pack = test_packs::starter();
        let world = World::new(cfg(512, 384, 42, &pack));
        assert_eq!(world.width(), 512);
        assert_eq!(world.height(), 384);
        assert!(world.cell(511, 383).is_some());
        assert_eq!(world.pack.name(), "test-starter");
        assert_eq!(world.pack.len(), 3);
    }

    #[test]
    fn tick_starts_at_zero_and_seed_seeds_the_rng() {
        let pack = test_packs::starter();
        let world = World::new(cfg(64, 48, 42, &pack));
        assert_eq!(world.tick(), 0);
        assert_eq!(world.rng_state(), Rng::new(42).state());
        let other = World::new(cfg(64, 48, 43, &pack));
        assert_ne!(world.rng_state(), other.rng_state());
    }

    #[test]
    fn reads_outside_the_grid_are_none() {
        let pack = test_packs::starter();
        let world = World::new(cfg(64, 48, 42, &pack));
        assert!(world.cell(64, 0).is_none());
        assert!(world.cell(0, 48).is_none());
        assert!(world.cell(64, 48).is_none());
        assert!(world.cell(u16::MAX, u16::MAX).is_none());
    }

    #[test]
    fn writes_outside_the_grid_return_errors() {
        let pack = test_packs::starter();
        let mut world = World::new(cfg(64, 48, 42, &pack));
        assert_eq!(
            world.set_material(64, 0, 1),
            Err(SilicaError::OutOfBounds { x: 64, y: 0 })
        );
        assert_eq!(
            world.set_temp(0, 48, 1180),
            Err(SilicaError::OutOfBounds { x: 0, y: 48 })
        );
        assert_eq!(
            world.set_aux(u16::MAX, 0, 1),
            Err(SilicaError::OutOfBounds { x: u16::MAX, y: 0 })
        );
    }

    #[test]
    fn setters_store_values_and_preserve_neighbours() {
        let pack = test_packs::starter();
        let mut world = World::new(cfg(64, 48, 42, &pack));
        world.set_material(5, 5, 7).unwrap();
        world.set_temp(5, 5, 2800).unwrap();
        world.set_aux(5, 5, 444).unwrap();

        let cell = world.cell(5, 5).unwrap();
        assert_eq!(cell.material, 7);
        assert_eq!(cell.temp, Temp::from_kelvin(700));
        assert_eq!(cell.aux, 444);

        // A write touches exactly its own cell.
        let neighbour = world.cell(6, 5).unwrap();
        assert_eq!(neighbour.material, 0);
        assert_eq!(neighbour.temp, Temp::AMBIENT);
        assert_eq!(neighbour.aux, 0);

        // A material write alone leaves temp/aux alone.
        world.set_material(9, 9, u16::MAX).unwrap();
        let cell = world.cell(9, 9).unwrap();
        assert_eq!(cell.material, u16::MAX);
        assert_eq!(cell.temp, Temp::AMBIENT);
        assert_eq!(cell.aux, 0);
    }

    #[test]
    fn set_temp_clamps_on_store() {
        let pack = test_packs::starter();
        let mut world = World::new(cfg(64, 48, 42, &pack));
        world.set_temp(1, 1, 999_999).unwrap();
        assert_eq!(world.cell(1, 1).unwrap().temp, Temp::MAX);
        world.set_temp(2, 2, -100).unwrap();
        assert_eq!(world.cell(2, 2).unwrap().temp, Temp::ZERO);
        world.set_temp(3, 3, 1180).unwrap();
        assert_eq!(world.cell(3, 3).unwrap().temp, Temp::AMBIENT);
    }

    #[test]
    fn writes_stamp_the_current_tick() {
        let pack = test_packs::starter();
        let mut world = World::new(cfg(64, 48, 42, &pack));
        assert_eq!(world.tick(), 0);
        world.set_material(10, 10, 3).unwrap();
        assert_eq!(world.stamp_of(10, 10), Some(0)); // written this tick
        assert_eq!(world.stamp_of(11, 11), Some(u32::MAX)); // never written

        // The §5 end-of-tick increment does not exist yet; tests advance
        // the counter directly to exercise the stamp trick across ticks.
        world.tick = 5;
        world.set_aux(10, 10, 9).unwrap();
        assert_eq!(world.stamp_of(10, 10), Some(5));
        world.set_temp(10, 10, 900).unwrap();
        assert_eq!(world.stamp_of(10, 10), Some(5)); // same tick, same stamp
        world.tick = 6;
        world.set_material(10, 10, 4).unwrap();
        assert_eq!(world.stamp_of(10, 10), Some(6));
        assert_eq!(world.stamp_of(0, 0), Some(u32::MAX));
    }

    #[test]
    fn writes_wake_exactly_their_regions() {
        let pack = test_packs::starter();
        let mut world = World::new(cfg(512, 384, 0, &pack));
        assert_eq!(world.regions.active_regions().count(), 0);

        world.set_material(100, 100, 1).unwrap(); // region (3, 3)
        world.set_temp(200, 50, 900).unwrap(); // region (6, 1)
        world.set_aux(511, 383, 2).unwrap(); // region (15, 11)

        let active: Vec<(u16, u16)> = world.regions.active_regions().collect();
        assert_eq!(active, vec![(6, 1), (3, 3), (15, 11)]); // row-major
    }

    #[test]
    fn a_write_wakes_only_its_own_region() {
        let pack = test_packs::starter();
        let mut world = World::new(cfg(512, 384, 0, &pack));
        world.set_material(100, 100, 1).unwrap(); // region (3, 3)
        for ry in 0..12u16 {
            for rx in 0..16u16 {
                let expected = (rx, ry) == (3, 3);
                assert_eq!(
                    world.regions.is_active(rx, ry),
                    expected,
                    "region ({rx},{ry})"
                );
            }
        }
    }

    #[test]
    fn partial_edge_tiles_in_a_non_multiple_world() {
        let pack = test_packs::starter();
        let mut world = World::new(WorldConfig {
            width: 100,
            height: 70,
            seed: 1,
            walls: true,
            pack: &pack,
        });
        world.set_material(99, 69, 5).unwrap();
        assert!(world.regions.is_active(3, 2));
        assert!(!world.regions.is_active(2, 2));
        assert_eq!(world.cell(99, 69).unwrap().material, 5);
    }

    #[test]
    fn anchors_cover_bottom_row_and_walled_edges() {
        let pack = test_packs::starter();
        let walled = World::new(cfg(64, 48, 0, &pack));
        assert!(walled.is_anchor(0, 0)); // top-left corner: walled edge
        assert!(walled.is_anchor(63, 0)); // top edge
        assert!(walled.is_anchor(0, 24)); // left edge
        assert!(walled.is_anchor(63, 24)); // right edge
        assert!(walled.is_anchor(32, 47)); // bottom row: always ground
        assert!(walled.is_anchor(0, 47)); // bottom corner
        assert!(!walled.is_anchor(32, 24)); // interior

        let open = World::new(WorldConfig {
            width: 64,
            height: 48,
            seed: 0,
            walls: false,
            pack: &pack,
        });
        assert!(open.is_anchor(32, 47)); // bottom row is ground even without walls
        assert!(!open.is_anchor(0, 24));
        assert!(!open.is_anchor(63, 24));
        assert!(!open.is_anchor(32, 0));
    }

    #[test]
    #[should_panic(expected = "world width must be non-zero")]
    fn zero_width_is_a_documented_programmer_error() {
        let pack = test_packs::starter();
        let _ = World::new(WorldConfig {
            width: 0,
            height: 48,
            seed: 0,
            walls: true,
            pack: &pack,
        });
    }

    #[test]
    #[should_panic(expected = "world height must be non-zero")]
    fn zero_height_is_a_documented_programmer_error() {
        let pack = test_packs::starter();
        let _ = World::new(WorldConfig {
            width: 48,
            height: 0,
            seed: 0,
            walls: true,
            pack: &pack,
        });
    }

    // --- canonical state hash (appendix A) --------------------------------

    /// 1×1, seed 42, tick 0: 26 bytes — tick(4) + rng(16) + 3×u16 per
    /// cell. Known answer computed independently with a JS FNV-1a
    /// re-implementation before being locked here.
    #[test]
    fn state_hash_matches_the_known_answer_1_by_1_seed_42() {
        let pack = test_packs::starter();
        let world = World::new(cfg(1, 1, 42, &pack));
        assert_eq!(world.state_hash(), 0xfe8c_3b38_de4d_130c);
    }

    /// Same world shape with seed 0 — pins the rng_state field's
    /// participation in the hash.
    #[test]
    fn state_hash_matches_the_known_answer_1_by_1_seed_0() {
        let pack = test_packs::starter();
        let world = World::new(cfg(1, 1, 0, &pack));
        assert_eq!(world.state_hash(), 0xc59f_662a_8cb1_3780);
    }

    /// Heterogeneous multi-cell known answer, tick 9 (2×2, seed 42):
    /// sand at (0,0), 400 K at (1,0), aux 7 at (0,1), rest empty at
    /// ambient. This pins the three-streams reading of appendix A —
    /// a per-cell interleaved buffer would hash this state to
    /// `0xe890_3a54_71cf_3701` instead; the reading was ratified as
    /// D-010 (1), including that alternate value.
    #[test]
    fn state_hash_matches_the_known_answer_2_by_2_tick_9() {
        let pack = test_packs::starter();
        let mut world = World::new(cfg(2, 2, 42, &pack));
        world.paint(&Brush::rect(0, 0, 0, 0, SAND_TEST_ID)).unwrap();
        world.set_temp(1, 0, 1600).unwrap(); // 400 K
        world.set_aux(0, 1, 7).unwrap();
        world.tick = 9;
        assert_eq!(world.state_hash(), 0xd59b_7dfd_ddde_a099);
    }

    /// The same state one tick earlier — pins the tick field's
    /// participation byte for byte.
    #[test]
    fn state_hash_matches_the_known_answer_2_by_2_tick_8() {
        let pack = test_packs::starter();
        let mut world = World::new(cfg(2, 2, 42, &pack));
        world.paint(&Brush::rect(0, 0, 0, 0, SAND_TEST_ID)).unwrap();
        world.set_temp(1, 0, 1600).unwrap();
        world.set_aux(0, 1, 7).unwrap();
        world.tick = 8;
        assert_eq!(world.state_hash(), 0x4e9d_2198_5587_a784);
    }

    /// Every hashed state field must move the hash when it changes:
    /// tick, rng_state, material, temp and aux are each varied against
    /// an otherwise-identical twin world.
    #[test]
    fn state_hash_distinguishes_every_state_field() {
        let pack = test_packs::starter();
        let baseline = World::new(cfg(8, 8, 7, &pack));
        let hash_of = |w: &World| w.state_hash();

        // rng_state via the seed.
        let w = World::new(cfg(8, 8, 8, &pack));
        assert_ne!(hash_of(&w), hash_of(&baseline));

        // tick: identical worlds, one stepped.
        let mut w = World::new(cfg(8, 8, 7, &pack));
        w.step(&[]).unwrap();
        assert_ne!(hash_of(&w), hash_of(&baseline));

        // material.
        let mut w = World::new(cfg(8, 8, 7, &pack));
        w.paint(&Brush::rect(4, 4, 4, 4, SAND_TEST_ID)).unwrap();
        assert_ne!(hash_of(&w), hash_of(&baseline));

        // temp.
        let mut w = World::new(cfg(8, 8, 7, &pack));
        w.set_temp(4, 4, 1600).unwrap();
        assert_ne!(hash_of(&w), hash_of(&baseline));

        // aux.
        let mut w = World::new(cfg(8, 8, 7, &pack));
        w.set_aux(4, 4, 3).unwrap();
        assert_ne!(hash_of(&w), hash_of(&baseline));
    }

    /// A cell that moves changes the hash (movement is a material
    /// write, and the buffer pins exactly where material lives). The
    /// painted grain skips its placement tick, so it moves on the
    /// second step.
    #[test]
    fn state_hash_advances_when_a_cell_moves() {
        let pack = test_packs::starter();
        let mut world = World::new(WorldConfig {
            width: 16,
            height: 16,
            seed: 0,
            walls: true,
            pack: &pack,
        });
        world.paint(&Brush::rect(3, 3, 3, 3, SAND_TEST_ID)).unwrap();
        let before = world.state_hash();
        world.step(&[]).unwrap(); // placement tick: no movement
        assert_eq!(world.cell(3, 3).unwrap().material, SAND_TEST_ID);
        assert_ne!(world.state_hash(), before); // the tick advanced
        world.step(&[]).unwrap(); // now it falls
        assert_eq!(world.cell(3, 4).unwrap().material, SAND_TEST_ID);
        assert_eq!(world.cell(3, 3).unwrap().material, 0);
        assert_ne!(world.state_hash(), before);
    }
}
