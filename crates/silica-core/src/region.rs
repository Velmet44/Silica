//! The 32×32 region grid with activity flags (SPEC.md §4).

/// Side length of a region in cells; regions tile the world from (0, 0).
pub(crate) const REGION_SIZE: u16 = 32;

/// A grid of 32×32 tracking tiles with one activity flag each
/// (SPEC.md §4).
///
/// Any cell write sets the containing region's flag; a scan pass clears
/// the flag when it produces no writes there. The clearing *policy* is a
/// §5 pass concern — this type only stores and reports flags.
///
/// Enumeration is row-major with `rx` fastest, a fixed order with no hash
/// structures anywhere (`SPEC.md` §8 D3); passes choose their own
/// traversal over it.
pub(crate) struct RegionGrid {
    width: u16,
    height: u16,
    cols: u16,
    rows: u16,
    active: Vec<bool>,
}

impl RegionGrid {
    /// Builds the grid for a `width × height` world; all regions start
    /// inactive. Allocated once at world creation; never grows.
    pub(crate) fn new(width: u16, height: u16) -> RegionGrid {
        let cols = width.div_ceil(REGION_SIZE);
        let rows = height.div_ceil(REGION_SIZE);
        // cols and rows are at most 2048, so the product fits u32 (and
        // every supported `usize`) — SPEC.md §8 D2.
        let len = (rows as u32 * cols as u32) as usize;
        RegionGrid {
            width,
            height,
            cols,
            rows,
            active: vec![false; len],
        }
    }

    /// The region containing cell `(x, y)`, or `None` outside the world.
    pub(crate) fn region_of(&self, x: u16, y: u16) -> Option<(u16, u16)> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some((x / REGION_SIZE, y / REGION_SIZE))
    }

    /// Marks the region containing cell `(x, y)` active; no-op outside
    /// the world (the `World` write primitives bounds-check first).
    pub(crate) fn activate_cell(&mut self, x: u16, y: u16) {
        if let Some((rx, ry)) = self.region_of(x, y) {
            self.activate(rx, ry);
        }
    }

    /// Marks region `(rx, ry)` active; no-op if the region does not exist.
    pub(crate) fn activate(&mut self, rx: u16, ry: u16) {
        if let Some(idx) = self.index(rx, ry) {
            self.active[idx] = true;
        }
    }

    /// Clears region `(rx, ry)`'s activity flag; no-op if absent.
    #[allow(dead_code)] // consumed by the §5 no-write scan clearing
    pub(crate) fn deactivate(&mut self, rx: u16, ry: u16) {
        if let Some(idx) = self.index(rx, ry) {
            self.active[idx] = false;
        }
    }

    /// Whether region `(rx, ry)` is active; `false` if it does not exist.
    pub(crate) fn is_active(&self, rx: u16, ry: u16) -> bool {
        self.index(rx, ry).is_some_and(|idx| self.active[idx])
    }

    /// Active region coordinates in row-major order (`rx` fastest).
    ///
    /// Zero allocation: the iterator only borrows the grid.
    #[allow(dead_code)] // consumed by the §5 movement/reaction/heat passes
    pub(crate) fn active_regions(&self) -> impl Iterator<Item = (u16, u16)> + '_ {
        (0..self.rows)
            .flat_map(move |ry| (0..self.cols).map(move |rx| (rx, ry)))
            .filter(move |&(rx, ry)| self.is_active(rx, ry))
    }

    /// Index of region `(rx, ry)` in the flag array, or `None` if absent.
    fn index(&self, rx: u16, ry: u16) -> Option<usize> {
        if rx >= self.cols || ry >= self.rows {
            return None;
        }
        // Checked per SPEC.md §8 D2; the guard above makes `None`
        // unreachable on every supported target.
        (ry as usize)
            .checked_mul(self.cols as usize)
            .and_then(|row| row.checked_add(rx as usize))
    }
}

#[cfg(test)]
mod tests {
    use super::RegionGrid;

    #[test]
    fn dims_round_up_to_whole_32_tiles() {
        let g = RegionGrid::new(512, 384);
        assert_eq!((g.cols, g.rows), (16, 12));
        let g = RegionGrid::new(32, 32);
        assert_eq!((g.cols, g.rows), (1, 1));
        let g = RegionGrid::new(1, 1);
        assert_eq!((g.cols, g.rows), (1, 1));
        let g = RegionGrid::new(33, 1);
        assert_eq!((g.cols, g.rows), (2, 1));
        let g = RegionGrid::new(100, 70);
        assert_eq!((g.cols, g.rows), (4, 3));
        let g = RegionGrid::new(65535, 65535);
        assert_eq!((g.cols, g.rows), (2048, 2048));
    }

    #[test]
    fn all_regions_start_inactive() {
        let g = RegionGrid::new(512, 384);
        for ry in 0..g.rows {
            for rx in 0..g.cols {
                assert!(!g.is_active(rx, ry));
            }
        }
    }

    #[test]
    fn region_of_maps_boundaries() {
        let g = RegionGrid::new(512, 384);
        assert_eq!(g.region_of(0, 0), Some((0, 0)));
        assert_eq!(g.region_of(31, 31), Some((0, 0)));
        assert_eq!(g.region_of(32, 0), Some((1, 0)));
        assert_eq!(g.region_of(0, 32), Some((0, 1)));
        assert_eq!(g.region_of(511, 383), Some((15, 11)));
        assert_eq!(g.region_of(512, 0), None);
        assert_eq!(g.region_of(0, 384), None);
        assert_eq!(g.region_of(u16::MAX, u16::MAX), None);

        let g = RegionGrid::new(100, 70); // partial edge tiles
        assert_eq!(g.region_of(99, 69), Some((3, 2)));
        assert_eq!(g.region_of(96, 64), Some((3, 2)));
        assert_eq!(g.region_of(100, 69), None);
    }

    #[test]
    fn activate_marks_only_the_target_region() {
        let mut g = RegionGrid::new(512, 384);
        g.activate(1, 2);
        assert!(g.is_active(1, 2));
        for ry in 0..g.rows {
            for rx in 0..g.cols {
                if (rx, ry) != (1, 2) {
                    assert!(!g.is_active(rx, ry), "region ({rx},{ry}) should be asleep");
                }
            }
        }
    }

    #[test]
    fn activate_is_idempotent_and_deactivate_clears() {
        let mut g = RegionGrid::new(512, 384);
        g.activate(1, 2);
        g.activate(1, 2); // setting is idempotent
        assert!(g.is_active(1, 2));
        g.deactivate(1, 2);
        assert!(!g.is_active(1, 2));
        g.deactivate(1, 2); // clearing is idempotent
        assert!(!g.is_active(1, 2));
        g.activate(1, 2);
        assert!(g.is_active(1, 2));
    }

    #[test]
    fn out_of_range_region_queries_are_tolerated() {
        let mut g = RegionGrid::new(512, 384);
        assert!(!g.is_active(16, 12)); // one past the edge
        assert!(!g.is_active(u16::MAX, u16::MAX));
        g.activate(16, 12); // no-op, must not panic (D10)
        g.deactivate(16, 12);
        assert!(!g.is_active(1, 2));
    }

    #[test]
    fn activate_cell_wakes_the_containing_region() {
        let mut g = RegionGrid::new(512, 384);
        g.activate_cell(0, 0);
        assert!(g.is_active(0, 0));
        g.activate_cell(40, 70);
        assert!(g.is_active(1, 2));
        g.activate_cell(511, 383);
        assert!(g.is_active(15, 11));
    }

    #[test]
    fn activate_cell_out_of_bounds_is_a_noop() {
        let mut g = RegionGrid::new(512, 384);
        g.activate_cell(512, 0);
        g.activate_cell(0, 384);
        g.activate_cell(u16::MAX, u16::MAX);
        for ry in 0..g.rows {
            for rx in 0..g.cols {
                assert!(!g.is_active(rx, ry));
            }
        }
    }

    #[test]
    fn active_regions_enumerates_row_major() {
        let mut g = RegionGrid::new(512, 384);
        g.activate(2, 0);
        g.activate(0, 1);
        g.activate(1, 0);
        let active: Vec<(u16, u16)> = g.active_regions().collect();
        assert_eq!(active, vec![(1, 0), (2, 0), (0, 1)]); // rx fastest
    }

    #[test]
    fn partial_edge_tiles_cover_their_cells() {
        // Region (3, 2) of a 100×70 world is only 4×6 cells.
        let mut g = RegionGrid::new(100, 70);
        g.activate_cell(99, 69);
        assert!(g.is_active(3, 2));
        assert!(!g.is_active(2, 2));
    }
}
