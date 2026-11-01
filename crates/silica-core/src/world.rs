//! The world: SoA cell storage, regions, stamps, RNG (SPEC.md §4).

use crate::error::SilicaError;
use crate::region::RegionGrid;
use crate::rng::Rng;
use crate::temp::Temp;

/// Configuration for [`World::new`] (SPEC.md §9).
///
/// The `pack` field of the §9 sketch joins when the compiled registry
/// type lands with the §6 loader (ROADMAP.md 0.1); until then material
/// ids are stored unvalidated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldConfig {
    /// Grid width in cells, 1..=u16::MAX.
    pub width: u16,
    /// Grid height in cells, 1..=u16::MAX.
    pub height: u16,
    /// Seed for the single RNG stream (SPEC.md §8 D4).
    pub seed: u64,
    /// Whether the map edges behave as walls — anchors per SPEC.md §4.
    /// The default value is an open question in SPEC.md §15, decided
    /// before 0.2.
    pub walls: bool,
}

impl Default for WorldConfig {
    /// The default world of SPEC.md §4: 512×384, seed 0, walls on.
    fn default() -> Self {
        WorldConfig {
            width: 512,
            height: 384,
            seed: 0,
            walls: true,
        }
    }
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
pub struct World {
    width: u16,
    height: u16,
    walls: bool,
    tick: u32,
    rng: Rng,
    regions: RegionGrid,
    material: Vec<u16>,
    temp: Vec<i16>,
    aux: Vec<u16>,
    stamp: Vec<u32>,
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
    pub fn new(config: WorldConfig) -> World {
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
            material: vec![0; cells],
            temp: vec![Temp::AMBIENT.0; cells],
            aux: vec![0; cells],
            stamp: vec![u32::MAX; cells],
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

    /// The current tick; 0 until the §5 pipeline starts stepping.
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

    /// Writes a cell's material id.
    ///
    /// Ids are not registry-validated until the compiled pack lands
    /// (§6). Stamps the cell and wakes its region.
    #[allow(dead_code)] // the §5/§6 passes write cells through this
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
    #[allow(dead_code)] // the §5 heat pass stores its i32 arithmetic here
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
    #[allow(dead_code)] // the §5/§6 aux-driven behaviours write through this
    pub(crate) fn set_aux(&mut self, x: u16, y: u16, aux: u16) -> Result<(), SilicaError> {
        let idx = self
            .cell_index(x, y)
            .ok_or(SilicaError::OutOfBounds { x, y })?;
        self.aux[idx] = aux;
        self.touch(idx, x, y);
        Ok(())
    }

    /// The tick this cell was last written; `u32::MAX` means never.
    ///
    /// The §5 single-update rule compares stamps against the current
    /// tick. Initial stamps are `u32::MAX`, which no early tick equals;
    /// the guarantee degrades only if a run reaches tick `u32::MAX`.
    #[allow(dead_code)] // consumed by the §5 single-update rule
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

    /// Records a write: stamp with the current tick, wake the region.
    fn touch(&mut self, idx: usize, x: u16, y: u16) {
        self.stamp[idx] = self.tick;
        self.regions.activate_cell(x, y);
    }

    /// Row-major index of `(x, y)`, or `None` outside the grid.
    fn cell_index(&self, x: u16, y: u16) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        // Checked per SPEC.md §8 D2; the guard above makes `None`
        // unreachable on every supported target (u16 grid, ≥32-bit usize).
        (y as usize)
            .checked_mul(self.width as usize)
            .and_then(|row| row.checked_add(x as usize))
    }
}

#[cfg(test)]
mod tests {
    use super::{World, WorldConfig};
    use crate::error::SilicaError;
    use crate::rng::Rng;
    use crate::temp::Temp;

    fn cfg(width: u16, height: u16, seed: u64) -> WorldConfig {
        WorldConfig {
            width,
            height,
            seed,
            walls: true,
        }
    }

    #[test]
    fn new_initializes_every_cell_empty_at_ambient() {
        let world = World::new(cfg(512, 384, 42));
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

    #[test]
    fn default_config_is_the_spec_world() {
        let config = WorldConfig::default();
        assert_eq!(config.width, 512);
        assert_eq!(config.height, 384);
        assert!(config.walls);
        let world = World::new(config);
        assert_eq!(world.width(), 512);
        assert_eq!(world.height(), 384);
        assert!(world.cell(511, 383).is_some());
    }

    #[test]
    fn tick_starts_at_zero_and_seed_seeds_the_rng() {
        let world = World::new(cfg(64, 48, 42));
        assert_eq!(world.tick(), 0);
        assert_eq!(world.rng_state(), Rng::new(42).state());
        let other = World::new(cfg(64, 48, 43));
        assert_ne!(world.rng_state(), other.rng_state());
    }

    #[test]
    fn reads_outside_the_grid_are_none() {
        let world = World::new(cfg(64, 48, 42));
        assert!(world.cell(64, 0).is_none());
        assert!(world.cell(0, 48).is_none());
        assert!(world.cell(64, 48).is_none());
        assert!(world.cell(u16::MAX, u16::MAX).is_none());
    }

    #[test]
    fn writes_outside_the_grid_return_errors() {
        let mut world = World::new(cfg(64, 48, 42));
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
        let mut world = World::new(cfg(64, 48, 42));
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
        let mut world = World::new(cfg(64, 48, 42));
        world.set_temp(1, 1, 999_999).unwrap();
        assert_eq!(world.cell(1, 1).unwrap().temp, Temp::MAX);
        world.set_temp(2, 2, -100).unwrap();
        assert_eq!(world.cell(2, 2).unwrap().temp, Temp::ZERO);
        world.set_temp(3, 3, 1180).unwrap();
        assert_eq!(world.cell(3, 3).unwrap().temp, Temp::AMBIENT);
    }

    #[test]
    fn writes_stamp_the_current_tick() {
        let mut world = World::new(cfg(64, 48, 42));
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
        let mut world = World::new(cfg(512, 384, 0));
        assert_eq!(world.regions.active_regions().count(), 0);

        world.set_material(100, 100, 1).unwrap(); // region (3, 3)
        world.set_temp(200, 50, 900).unwrap(); // region (6, 1)
        world.set_aux(511, 383, 2).unwrap(); // region (15, 11)

        let active: Vec<(u16, u16)> = world.regions.active_regions().collect();
        assert_eq!(active, vec![(6, 1), (3, 3), (15, 11)]); // row-major
    }

    #[test]
    fn a_write_wakes_only_its_own_region() {
        let mut world = World::new(cfg(512, 384, 0));
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
        let mut world = World::new(WorldConfig {
            width: 100,
            height: 70,
            seed: 1,
            walls: true,
        });
        world.set_material(99, 69, 5).unwrap();
        assert!(world.regions.is_active(3, 2));
        assert!(!world.regions.is_active(2, 2));
        assert_eq!(world.cell(99, 69).unwrap().material, 5);
    }

    #[test]
    fn anchors_cover_bottom_row_and_walled_edges() {
        let walled = World::new(cfg(64, 48, 0));
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
        });
        assert!(open.is_anchor(32, 47)); // bottom row is ground even without walls
        assert!(!open.is_anchor(0, 24));
        assert!(!open.is_anchor(63, 24));
        assert!(!open.is_anchor(32, 0));
    }

    #[test]
    #[should_panic(expected = "world width must be non-zero")]
    fn zero_width_is_a_documented_programmer_error() {
        let _ = World::new(WorldConfig {
            width: 0,
            height: 48,
            seed: 0,
            walls: true,
        });
    }

    #[test]
    #[should_panic(expected = "world height must be non-zero")]
    fn zero_height_is_a_documented_programmer_error() {
        let _ = World::new(WorldConfig {
            width: 48,
            height: 0,
            seed: 0,
            walls: true,
        });
    }
}
