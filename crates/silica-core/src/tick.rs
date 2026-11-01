//! The §5 tick pipeline: seven passes in the exact documented order.

use crate::brush::{self, Brush};
use crate::error::SilicaError;
use crate::pack::Category;
use crate::region::REGION_SIZE;
use crate::world::World;

impl World {
    /// One tick (`SPEC.md` §5): exactly this pipeline, in this order.
    ///
    /// 1. `apply_inputs`  — queued brush strokes, submission order,
    ///    each a fresh spawn complying with the pack registry's spawn
    ///    constants (§6)
    /// 2. `rigid_pass`    — island integrate/collide/crumble/weld (`§7`)
    /// 3. `movement_pass` — cells, bottom-up rows, column direction
    ///    alternating by tick parity
    /// 4. `reaction_pass` — DSL reactions + combustion, top-down
    ///    row-major, fixed order
    /// 5. `heat_pass` — temperature diffusion (4-neighbour, `i32`
    ///    intermediates, clamped on store) + phase transitions
    /// 6. `pressure_pass` — coarse 8×8 pressure grid: diffuse, push
    ///    gases/liquids
    /// 7. `end_of_tick` — tick increment + the state-hash hook
    ///
    /// Every stage is a separate function so a pass may be a no-op while
    /// its milestone has not landed — the order and the tick bookkeeping
    /// never move. The steady-state tick allocates nothing (§5): all
    /// buffers are preallocated at [`World::new`](crate::world::World::new).
    ///
    /// Current pass status (0.1):
    /// * `apply_inputs` — real: applies the queued strokes; `aux_init`
    ///   draws (D4, one per spawned cell) happen inside each stroke;
    /// * `movement_pass` — real but powder-only, dispatched through the
    ///   §6 pack registry's category table; liquids/gases land next in
    ///   ROADMAP.md 0.1 and stay no-op branches until then;
    /// * `reaction_pass`/`heat_pass` — no-op until reactions/heat;
    /// * `rigid_pass` — no-op until §7 islands (0.3);
    /// * `pressure_pass` — no-op until pressure v1 (0.3).
    ///
    /// Brush placement this tick participates from the next tick: a
    /// freshly painted cell is stamped with the current tick, and the
    /// §5 single-update rule forbids a second write in the same tick
    /// (the paint rule ratified in D-010 (3): paint + hash are
    /// independent of where `apply_inputs` sits in the pass order).
    ///
    /// # Errors
    ///
    /// A queued stroke whose material the world's pack does not define
    /// fails the tick before any other pass runs (D10: nothing is
    /// half-applied; the tick counter does not advance).
    pub fn step(&mut self, inputs: &[Brush]) -> Result<(), SilicaError> {
        self.apply_inputs(inputs)?;
        self.rigid_pass();
        self.movement_pass();
        self.reaction_pass();
        self.heat_pass();
        self.pressure_pass();
        self.end_of_tick();
        Ok(())
    }

    /// §5 pass 1: apply queued brush strokes in submission order
    /// (everything a host passes to [`World::step`]). Each stroke
    /// paints at the current tick — a fresh spawn per covered cell,
    /// with `aux_init` draws from the D4 stream — so freshly painted
    /// cells take part from the next tick (single-update rule).
    fn apply_inputs(&mut self, inputs: &[Brush]) -> Result<(), SilicaError> {
        // Validate every stroke's material first, so a failed input
        // brush aborts the tick before any cell is written (D10: nothing
        // is half-applied; the tick counter does not advance).
        for brush in inputs {
            brush::validate(self, brush)?;
        }
        #[cfg(test)]
        self.pass_trace.push("apply_inputs");
        for brush in inputs {
            brush::paint(self, brush)?;
        }
        Ok(())
    }

    /// §5 pass 2 — no-op in the 0.1 skeleton: islands arrive with the
    /// §7 support analysis (ROADMAP.md 0.3). Intentionally empty: the
    /// documented pipeline shape includes this stage even before it
    /// has behavior.
    fn rigid_pass(&mut self) {
        #[cfg(test)]
        self.pass_trace.push("rigid_pass");
    }

    /// §5 pass 3: cells move — bottom-up rows, column direction
    /// alternating by tick parity.
    ///
    /// Scan contract (`SPEC.md` §5, D3):
    /// * rows bottom-up (falling does not need a second pass backwards);
    /// * within a row, columns ascend on even ticks and descend on odd
    ///   ticks; the same parity bit drives the powder diagonal trial
    ///   order. This is the sweep parity ratified in D-010 (2): even
    ///   ticks scan ascending and prefer the down-left diagonal, odd
    ///   ticks scan descending and prefer down-right, falling back to
    ///   the other side — symmetric over any two consecutive ticks and
    ///   pile-bias-free, pinned by the tests below;
    /// * a cell is processed only when its material is non-empty, its
    ///   stamp is older than the current tick (single-update rule), and
    ///   its 32×32 region is active (§4).
    ///
    /// Movement writes go through `set_material`, so they stamp both
    /// the vacated and the filled cell and wake both regions. Region
    /// activity flags are only set by writes in this stage — the
    /// no-write clearing policy arrives with dirty-region degradation
    /// (ROADMAP.md 0.2).
    ///
    /// Movement dispatch reads the world's pack registry (§4's "index
    /// into the compiled pack registry"): powders fall, every other
    /// category's pass behavior is a future §5/§6 milestone. This pass
    /// and its inner loop are the human-owned hot path per AGENTS.md.
    fn movement_pass(&mut self) {
        #[cfg(test)]
        self.pass_trace.push("movement_pass");

        let even = self.tick.is_multiple_of(2);
        for row in (0..self.height).rev() {
            // Index base for the row: y ≤ 65534 and width ≤ 65535, so
            // the product is ≤ 4294836256 < u32::MAX — documented
            // non-capable arithmetic (§8 D2).
            let base = self.width as u32 * row as u32;
            // Column direction alternates by tick parity (§5).
            if even {
                for x in 0..self.width {
                    self.step_cell(base, x, row);
                }
            } else {
                for x in (0..self.width).rev() {
                    self.step_cell(base, x, row);
                }
            }
        }
    }

    /// Processes one cell of the movement scan.
    ///
    /// Fast paths: empty cells and cells written earlier this tick need
    /// nothing; cells in inactive regions are skipped (§4). Otherwise
    /// the id dispatches through the pack registry's category table
    /// (§4/§6) — powders fall; every other starter-pack category is
    /// static until its behavior lands (liquids/gases next in
    /// ROADMAP.md 0.1). Unknown ids (a static gap in the registry)
    /// also never move, which keeps grid-level writes forgiving.
    fn step_cell(&mut self, base: u32, x: u16, y: u16) {
        // base + x ≤ 4294836256 + 65534 < u32::MAX, and usize ≥ u32 on
        // every supported target (D8) — documented non-capable.
        let idx = base as usize + x as usize;
        let id = self.material[idx];
        if id == 0 {
            return;
        }
        if self.stamp[idx] == self.tick {
            return; // §5 single-update rule: already written this tick
        }
        if !self.regions.is_active(x / REGION_SIZE, y / REGION_SIZE) {
            return; // §4: passes skip inactive regions
        }
        match self.pack.category_of(id) {
            Category::Powder => self.powder_step(x, y, id),
            Category::None | Category::Solid | Category::Rigid => {}
            // TODO(0.1): liquid and gas movement branches with their
            // §5/§6 behavior (energy moves with its aux pass).
            Category::Liquid | Category::Gas | Category::Energy => {}
        }
    }

    /// Powder rules (the sand behavior of the first golden world):
    /// straight down when the below-cell is free; otherwise a diagonal
    /// slide, parity-preferred side first (D-010 sweep parity), only
    /// when not grounded on the bottom row.
    ///
    /// "Free" means the cell's material is 0. A cell vacated earlier
    /// this tick is free (its stamp is fresh but its material is
    /// empty); a cell filled earlier this tick is not (powders do not
    /// pass through or swap with other powders — deterministic, no
    /// escalation rules).
    fn powder_step(&mut self, x: u16, y: u16, id: u16) {
        let below = y.saturating_add(1);
        if self.is_free(x, below) {
            self.move_powder(x, y, x, below, id);
            return;
        }
        if below >= self.height {
            return; // bottom row: grounded on the world floor
        }
        // Diagonal trial order reuses the column-scan parity: even
        // ticks prefer down-left, odd ticks prefer down-right (the
        // sweep parity ratified in D-010 (2)).
        let left: Option<u16> = if x == 0 { None } else { Some(x - 1) };
        let candidates = if self.tick.is_multiple_of(2) {
            [left, Some(x + 1)]
        } else {
            [Some(x + 1), left]
        };
        for cx in candidates.into_iter().flatten() {
            if self.is_free(cx, below) {
                self.move_powder(x, y, cx, below, id);
                return;
            }
        }
        // Neither diagonal free: the grain stays put this tick.
    }

    /// A destination cell is free when its material is empty.
    /// Out-of-bounds coordinates are never free (the world edge and
    /// floor are solid).
    fn is_free(&self, x: u16, y: u16) -> bool {
        match self.cell_index(x, y) {
            Some(idx) => self.material[idx] == 0,
            None => false,
        }
    }

    /// Moves one powder cell: clears the source, fills the destination
    /// with the **same** material id it carried (the registry decides
    /// who moves, the cell keeps its own id). Both writes stamp the
    /// current tick and wake their regions, so neither cell can be
    /// processed a second time this tick.
    fn move_powder(&mut self, x0: u16, y0: u16, x1: u16, y1: u16, id: u16) {
        // Both destinations were bounds-checked by the caller's
        // is_free probes — the error arms are unreachable here.
        let _ = self.set_material(x0, y0, 0);
        let _ = self.set_material(x1, y1, id);
    }

    /// §5 pass 4 — no-op in the 0.1 skeleton: reactions arrive with the
    /// DSL registry (ROADMAP.md 0.1/0.4). Intentionally empty.
    fn reaction_pass(&mut self) {
        #[cfg(test)]
        self.pass_trace.push("reaction_pass");
    }

    /// §5 pass 5 — no-op in the 0.1 skeleton: temperature diffusion and
    /// phase transitions arrive with the heat pass (ROADMAP.md 0.1).
    /// Intentionally empty.
    fn heat_pass(&mut self) {
        #[cfg(test)]
        self.pass_trace.push("heat_pass");
    }

    /// §5 pass 6 — no-op in the 0.1 skeleton: the coarse pressure grid
    /// arrives with pressure v1 (ROADMAP.md 0.3). Intentionally empty.
    fn pressure_pass(&mut self) {
        #[cfg(test)]
        self.pass_trace.push("pressure_pass");
    }

    /// §5 pass 7: increment the tick and expose the hash hook
    /// ([`World::state_hash`]). The only writer of the tick counter —
    /// exactly once per step. `wrapping_add` per §8 D2: a run reaching
    /// tick `u32::MAX` wraps, degrading the stamp-trick guarantee at
    /// that exact boundary (documented in §4).
    fn end_of_tick(&mut self) {
        #[cfg(test)]
        self.pass_trace.push("end_of_tick");
        self.tick = self.tick.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use crate::brush::Brush;
    use crate::error::SilicaError;
    use crate::pack::{Category, Pack, test_packs};
    use crate::rng::Rng;
    use crate::world::{World, WorldConfig};

    /// Appendix B, id 1: sand — the shared starter test-pack keeps this
    /// id for sand (the dsl tests pin the real pack's ids identically).
    const SAND: u16 = 1;
    /// Appendix B, id 2: water — the starter pack's only liquid, static
    /// until liquid movement lands (ROADMAP.md 0.1).
    const WATER: u16 = 2;

    fn world(w: u16, h: u16) -> World {
        let pack = test_packs::starter();
        World::new(WorldConfig {
            width: w,
            height: h,
            seed: 0,
            walls: true,
            pack: &pack,
        })
    }

    #[test]
    fn step_runs_the_seven_passes_in_documented_order() {
        let mut w = world(8, 8);
        w.step(&[]).unwrap();
        assert_eq!(
            w.pass_trace,
            [
                "apply_inputs",
                "rigid_pass",
                "movement_pass",
                "reaction_pass",
                "heat_pass",
                "pressure_pass",
                "end_of_tick",
            ]
        );
        // A second step repeats the whole pipeline, in order.
        w.step(&[]).unwrap();
        assert_eq!(
            w.pass_trace,
            [
                "apply_inputs",
                "rigid_pass",
                "movement_pass",
                "reaction_pass",
                "heat_pass",
                "pressure_pass",
                "end_of_tick",
                "apply_inputs",
                "rigid_pass",
                "movement_pass",
                "reaction_pass",
                "heat_pass",
                "pressure_pass",
                "end_of_tick",
            ]
        );
    }

    #[test]
    fn step_advances_the_tick_exactly_once_and_returns_ok() {
        let mut w = world(4, 4);
        assert_eq!(w.tick(), 0);
        assert_eq!(w.step(&[]), Ok(()));
        assert_eq!(w.tick(), 1);
        assert_eq!(w.step(&[]), Ok(()));
        assert_eq!(w.tick(), 2);
    }

    #[test]
    fn input_brushes_apply_during_the_step_at_current_tick() {
        let mut w = world(16, 16);
        w.step(&[Brush::rect(5, 5, 5, 5, SAND)]).unwrap();
        // Painted during the step: material stored at the step's tick…
        assert_eq!(w.cell(5, 5).unwrap().material, SAND);
        assert_eq!(w.stamp_of(5, 5), Some(0));
        // …and the cell does not move again within that same tick
        // (single-update rule): the sand below is still empty.
        assert_eq!(w.cell(5, 6).unwrap().material, 0);
        assert_eq!(w.tick(), 1);
    }

    #[test]
    fn powder_falls_straight_down_one_cell_per_tick() {
        let mut w = world(8, 8);
        let _ = w.paint(&Brush::rect(3, 2, 3, 2, SAND));
        // Painted at tick 0 (stamp 0): it does not move inside tick 0.
        w.step(&[]).unwrap();
        assert_eq!(w.cell(3, 2).unwrap().material, SAND);
        // Then one cell per tick until it reaches the floor row (y=7).
        for step in 2..=6usize {
            w.step(&[]).unwrap();
            let y = (step + 1) as u16;
            assert_eq!(w.cell(3, y).unwrap().material, SAND, "step {step}");
        }
        // Grounded: below is out of bounds, both diagonals too — stays.
        for _ in 0..3 {
            w.step(&[]).unwrap();
        }
        assert_eq!(w.cell(3, 7).unwrap().material, SAND, "rests on the floor");
        assert_eq!(w.cell(3, 6).unwrap().material, 0);
    }

    #[test]
    fn powder_column_descends_together_and_never_moves_twice_per_tick() {
        let mut w = world(16, 16);
        let _ = w.paint(&Brush::rect(5, 2, 5, 4, SAND)); // three stacked grains
        w.step(&[]).unwrap(); // tick 0: freshly painted, no movement yet
        assert_eq!(w.cell(5, 2).unwrap().material, SAND);
        w.step(&[]).unwrap(); // tick 1: all three shift down one cell
        for y in 3..=5u16 {
            assert_eq!(w.cell(5, y).unwrap().material, SAND, "y == {y}");
        }
        assert_eq!(w.cell(5, 2).unwrap().material, 0);
    }

    #[test]
    fn powder_rests_on_powder_and_on_the_floor() {
        let mut w = world(8, 8);
        // A 3×1 shelf of sand along the bottom, and one grain on top of
        // the middle: the top grain's below-cell is not free and both
        // diagonals fall out of the bottom row, so nothing moves.
        let _ = w.paint(&Brush::rect(2, 7, 4, 7, SAND));
        let _ = w.paint(&Brush::rect(3, 6, 3, 6, SAND));
        for _ in 0..4 {
            w.step(&[]).unwrap();
        }
        // The exact painted shape, unchanged.
        for x in 2..=4u16 {
            assert_eq!(w.cell(x, 7).unwrap().material, SAND, "shelf ({x},7)");
        }
        assert_eq!(w.cell(3, 6).unwrap().material, SAND, "grain on top");
        for (x, y) in [
            (2u16, 6u16),
            (4u16, 6u16),
            (3u16, 5u16),
            (1u16, 7u16),
            (5u16, 7u16),
        ] {
            assert_eq!(w.cell(x, y).unwrap().material, 0, "({x},{y})");
        }
    }

    #[test]
    fn powder_resting_piles_slide_off_the_parity_preferred_side() {
        let mut w = world(16, 17); // bottom row y == 16
        // A stair: (8,16) base, (8,15) grain on top.
        let _ = w.paint(&Brush::rect(8, 15, 8, 16, SAND));
        w.step(&[]).unwrap(); // tick 0 skip
        w.step(&[]).unwrap(); // tick 1: odd tick → prefer the right diagonal
        assert_eq!(w.cell(9, 16).unwrap().material, SAND);
        assert_eq!(w.cell(7, 16).unwrap().material, 0);
        assert_eq!(w.cell(8, 15).unwrap().material, 0);
        assert_eq!(w.cell(8, 16).unwrap().material, SAND);
    }

    #[test]
    fn blocked_preferred_side_falls_back_to_the_other_diagonal() {
        let mut w = world(16, 17); // bottom row y == 16
        let _ = w.paint(&Brush::rect(8, 15, 8, 16, SAND));
        let _ = w.paint(&Brush::rect(9, 16, 9, 16, SAND)); // right side occupied
        w.step(&[]).unwrap(); // tick 0 skip
        w.step(&[]).unwrap(); // tick 1: right blocked → slide left
        assert_eq!(w.cell(7, 16).unwrap().material, SAND);
        assert_eq!(w.cell(9, 16).unwrap().material, SAND);
        assert_eq!(w.cell(8, 15).unwrap().material, 0);
    }

    #[test]
    fn diagonal_challenge_at_the_world_edge_stays_in_bounds() {
        // Left edge: the left diagonal is out of bounds (x-1 would
        // wrap); the grain must slide right, never wrap around.
        let mut w = world(16, 17);
        let _ = w.paint(&Brush::rect(0, 15, 0, 16, SAND));
        w.step(&[]).unwrap();
        w.step(&[]).unwrap();
        assert_eq!(w.cell(1, 16).unwrap().material, SAND);
        assert!(w.cell(0, 15).unwrap().material == 0);

        // Right edge: right diagonal out of bounds; slide left.
        let mut w = world(16, 17);
        let _ = w.paint(&Brush::rect(15, 15, 15, 15, SAND));
        let _ = w.paint(&Brush::rect(15, 16, 15, 16, SAND));
        w.step(&[]).unwrap();
        w.step(&[]).unwrap();
        assert_eq!(w.cell(14, 16).unwrap().material, SAND);
        // Nothing leaked to out-of-grid coordinates.
        assert!(w.cell(u16::MAX, 16).is_none());
    }

    #[test]
    fn steady_ticks_never_wake_regions_or_draw_rng() {
        // An empty world must tick with zero writes: no region wakes
        // (they would never sleep again) and the single RNG stream
        // stays at its seeded value (no documented consumption point
        // has been reached).
        let mut w = world(64, 48);
        for _ in 0..5 {
            w.step(&[]).unwrap();
        }
        assert_eq!(w.regions.active_regions().count(), 0);
        assert_eq!(w.rng_state(), Rng::new(0).state());
        assert_eq!(w.tick(), 5);
    }

    #[test]
    fn identical_worlds_stay_hash_identical_in_lockstep() {
        let paint = Brush::rect(29, 4, 33, 23, SAND);
        let mut a = World::new(WorldConfig {
            width: 64,
            height: 48,
            seed: 0,
            walls: true,
            pack: &test_packs::starter(),
        });
        let mut b = World::new(WorldConfig {
            width: 64,
            height: 48,
            seed: 0,
            walls: true,
            pack: &test_packs::starter(),
        });
        a.paint(&paint).unwrap();
        b.paint(&paint).unwrap();
        for step in 0..100 {
            a.step(&[]).unwrap();
            b.step(&[]).unwrap();
            assert_eq!(a.state_hash(), b.state_hash(), "step {step}");
        }
    }

    /// The category table decides who moves — not the appendix-B id:
    /// a powder at a non-appendix id falls exactly like sand does.
    #[test]
    fn registry_dispatches_powder_by_category_not_by_id() {
        // One powder at id 77 (an id the starter pack keeps free).
        let grain = test_packs::def(77, Category::Powder);
        let pack = Pack::new("custom-powder", "0.1.0", 0, vec![grain]).unwrap();
        let mut w = World::new(WorldConfig {
            width: 8,
            height: 8,
            seed: 0,
            walls: true,
            pack: &pack,
        });
        w.paint(&Brush::rect(3, 2, 3, 2, 77)).unwrap();
        w.step(&[]).unwrap(); // placement tick: no movement (D-010 paint rule)
        assert_eq!(w.cell(3, 2).unwrap().material, 77);
        w.step(&[]).unwrap();
        assert_eq!(w.cell(3, 3).unwrap().material, 77, "fell via the registry");
        assert_eq!(w.cell(3, 2).unwrap().material, 0);
    }

    /// Liquids (and gases) are compiled into the registry but remain
    /// static until their §5 movement branches land — the starter
    /// world's water does not teleport sand-style.
    #[test]
    fn liquids_stay_static_until_their_movement_lands() {
        let mut w = world(8, 8);
        w.paint(&Brush::rect(3, 3, 3, 3, WATER)).unwrap();
        for _ in 0..3 {
            w.step(&[]).unwrap();
        }
        assert_eq!(w.cell(3, 3).unwrap().material, WATER);
        assert_eq!(w.cell(3, 4).unwrap().material, 0);
    }

    /// A queued stroke naming an id outside the pack is rejected whole
    /// (D10): no pass runs, no cell is written, no RNG is consumed and
    /// the tick does not advance.
    #[test]
    fn a_failed_input_brush_aborts_the_tick_before_any_work() {
        let mut w = world(8, 8);
        let before = w.rng_state();
        let inputs = [
            Brush::rect(0, 0, 0, 0, SAND),     // valid…
            Brush::rect(4, 4, 4, 4, u16::MAX), // …second stroke is not
        ];
        assert_eq!(
            w.step(&inputs),
            Err(SilicaError::UnknownMaterial { material: u16::MAX })
        );
        assert_eq!(w.tick(), 0, "end_of_tick never ran");
        assert_eq!(w.regions.active_regions().count(), 0, "nothing woke");
        assert_eq!(w.cell(0, 0).unwrap().material, 0, "not even stroke 1");
        assert_eq!(w.rng_state(), before);
    }
}
