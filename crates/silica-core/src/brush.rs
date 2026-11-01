//! Brush strokes — the only way new material enters the world
//! (SPEC.md §9: "everything a human can do is a brush stroke"). Paint
//! is the spawn boundary of the pack registry (§6).

use crate::error::SilicaError;
use crate::pack::AuxRange;
use crate::temp::Temp;
use crate::world::World;

/// A paint stroke: one covering set plus one material id.
///
/// `flow` exists in the §9 sketch for liquids; its semantics arrive with
/// the liquid rules (ROADMAP.md 0.1). Until then a brush paints every
/// covered cell deterministically and hosts may pass `0`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Brush {
    kind: BrushKind,
    material: u16,
    flow: u8,
}

/// The covering geometry of a brush.
///
/// Private on purpose: hosts build strokes through the `Brush::…`
/// constructors so the enum never becomes API, and any future brush
/// kinds stay behind constructors.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BrushKind {
    /// Filled disc centred at `(cx, cy)`; covers cells whose squared
    /// distance to the centre is `<= radius²`.
    Circle { cx: u16, cy: u16, radius: u16 },
    /// Filled axis-aligned rectangle over the closed range
    /// `[x0..=x1] × [y0..=y1]`; swapped corners are normalized.
    Rect { x0: u16, y0: u16, x1: u16, y1: u16 },
}

impl Brush {
    /// A filled disc centred at `(x, y)` with radius `radius`.
    ///
    /// A radius of 0 paints exactly the centre cell.
    pub fn circle(x: u16, y: u16, radius: u16, material: u16, flow: u8) -> Brush {
        Brush {
            kind: BrushKind::Circle {
                cx: x,
                cy: y,
                radius,
            },
            material,
            flow,
        }
    }

    /// A filled axis-aligned rectangle over the closed range
    /// `[x0..=x1] × [y0..=y1]` (flow 0 — rects pour dense).
    pub fn rect(x0: u16, y0: u16, x1: u16, y1: u16, material: u16) -> Brush {
        Brush {
            kind: BrushKind::Rect { x0, y0, x1, y1 },
            material,
            flow: 0,
        }
    }
}

/// Validates a brush's material against the world's pack without
/// writing anything (D10: a rejected paint never half-applies, and a
/// tick with a failed input brush writes no cell at all).
pub(crate) fn validate(world: &World, brush: &Brush) -> Result<(), SilicaError> {
    spawn_spec(world, brush.material)?;
    Ok(())
}

/// Applies one brush to the world: every covered in-bounds cell is a
/// **fresh spawn** of the brush material — the registry's spawn
/// semantics (§6): the material, its `default_temp`, and a fresh aux
/// word (0, or one uniform draw in the `aux_init` range from the D4
/// stream). Painting `empty` spawns a fresh empty cell instead
/// (vacuum, ambient temp, aux 0).
///
/// Determinism contract:
/// * covered cells are enumerated row-major ascending (`y`, then `x`)
///   so any per-cell RNG draws (`aux_init`, D4) consume the stream in
///   a fixed order — covered-cell order;
/// * out-of-bounds covered cells are skipped, not errors — a host brush
///   may straddle a world edge (D10: valid input, defined behavior,
///   never a panic) and draw nothing;
/// * every written cell is stamped with the current tick and wakes its
///   region (the §5 single-update rule then applies);
/// * the brush material resolves once, before any cell is written — a
///   rejected paint never half-applies.
///
/// # Errors
///
/// [`SilicaError::UnknownMaterial`] when the brush material is not
/// defined by the world's pack (id 0 is always accepted: it erases).
pub(crate) fn paint(world: &mut World, brush: &Brush) -> Result<(), SilicaError> {
    let spec = spawn_spec(world, brush.material)?;
    match brush.kind {
        BrushKind::Rect { x0, y0, x1, y1 } => {
            // Normalize swapped corners; every coordinate stays in
            // `[0, 65535]` so the u32 loop bounds cannot overflow
            // (documented non-capable arithmetic, §8 D2).
            let (min_x, max_x) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
            let (min_y, max_y) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
            for y in u32::from(min_y)..=u32::from(max_y) {
                for x in u32::from(min_x)..=u32::from(max_x) {
                    try_paint(world, x, y, spec);
                }
            }
        }
        BrushKind::Circle { cx, cy, radius } => {
            // Square scan bounds; saturating keeps them inside u32 even
            // when cx ± radius underflows above 0 or overflows past
            // u16 range only conceptually — cx, cy, radius are ≤ 65535,
            // so bounds stay ≤ 131070, which cannot overflow u32
            // (documented non-capable).
            let min_x = u32::from(cx).saturating_sub(u32::from(radius));
            let max_x = u32::from(cx).saturating_add(u32::from(radius));
            let min_y = u32::from(cy).saturating_sub(u32::from(radius));
            let max_y = u32::from(cy).saturating_add(u32::from(radius));
            let r2 = u64::from(radius) * u64::from(radius);
            for y in min_y..=max_y {
                let dy2 = square_abs_delta(y, cy);
                for x in min_x..=max_x {
                    let dx2 = square_abs_delta(x, cx);
                    if dx2.saturating_add(dy2) <= r2 {
                        try_paint(world, x, y, spec);
                    }
                }
            }
        }
    }
    Ok(())
}

/// The per-material constants of one painted spawn, copied out of the
/// registry up front (per-cell loops need `&mut World`, so per-cell
/// registry lookups are amortized to one).
///
/// * `id` 0 is the eraser: a fresh empty cell (ambient, aux 0);
/// * any other id must exist in the registry — checked here, per D10.
fn spawn_spec(world: &World, material: u16) -> Result<SpawnSpec, SilicaError> {
    if material == 0 {
        return Ok(SpawnSpec {
            id: 0,
            temp: Temp::AMBIENT.0,
            aux: None,
        });
    }
    let def = world
        .pack
        .get(material)
        .ok_or(SilicaError::UnknownMaterial { material })?;
    Ok(SpawnSpec {
        id: material,
        temp: def.default_temp.0,
        aux: def.aux_init,
    })
}

/// The spawn constants of one paint stroke (the spec struct of
/// [`spawn_spec`]; `Copy` by design — no per-cell allocation, §5).
#[derive(Clone, Copy)]
struct SpawnSpec {
    id: u16,
    temp: i16,
    aux: Option<AuxRange>,
}

/// Squared absolute distance between two coordinates; the caller's loop
/// bounds guarantee both within 65535 of each other, so the square fits
/// `u64` trivially (documented non-capable, §8 D2).
fn square_abs_delta(a: u32, b: u16) -> u64 {
    let delta = i64::from(a) - i64::from(b);
    let d = delta.unsigned_abs(); // |delta| ≤ 65535², far below u64::MAX
    d * d
}

/// Paints one covered cell if it is inside the world; out-of-bounds
/// covered cells are skipped (a host brush may straddle a world edge,
/// D10 — valid input, defined behavior, no panic) and draw no aux.
fn try_paint(world: &mut World, x: u32, y: u32, spec: SpawnSpec) {
    if x < u32::from(world.width) && y < u32::from(world.height) {
        // Bounds checked above — the coordinates fit u16, so the error
        // arm of spawn is unreachable here.
        let aux = match spec.aux {
            Some(range) => world.draw_aux(range),
            None => 0,
        };
        let _ = world.spawn(x as u16, y as u16, spec.id, spec.temp, aux);
    }
}

#[cfg(test)]
mod tests {
    use super::Brush;
    use crate::error::SilicaError;
    use crate::pack::test_packs;
    use crate::rng::Rng;
    use crate::temp::Temp;
    use crate::world::{World, WorldConfig};

    /// Appendix-B ids the shared starter test-pack pins: sand (its
    /// powder) and fire (its `aux_init` energy material).
    const SAND: u16 = 1;
    const FIRE: u16 = 4;

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
    fn rect_covers_the_closed_range() {
        let mut w = world(16, 16);
        w.paint(&Brush::rect(2, 3, 4, 5, SAND)).unwrap();
        for y in 3..=5u16 {
            for x in 2..=4u16 {
                assert_eq!(w.cell(x, y).unwrap().material, SAND, "({x},{y})");
            }
        }
        for y in 0..16u16 {
            for x in 0..16u16 {
                if !(2..=4).contains(&x) || !(3..=5).contains(&y) {
                    assert_eq!(w.cell(x, y).unwrap().material, 0, "({x},{y})");
                }
            }
        }
    }

    #[test]
    fn rect_normalizes_swapped_corners() {
        let mut w = world(16, 16);
        w.paint(&Brush::rect(4, 5, 2, 3, SAND)).unwrap();
        for y in 3..=5u16 {
            for x in 2..=4u16 {
                assert_eq!(w.cell(x, y).unwrap().material, SAND, "({x},{y})");
            }
        }
        let mut reference = world(16, 16);
        reference.paint(&Brush::rect(2, 3, 4, 5, SAND)).unwrap();
        assert_eq!(w.state_hash(), reference.state_hash());
    }

    #[test]
    fn rect_out_of_bounds_is_clamped_not_an_error() {
        // A brush may straddle a world edge (D10: no panic), painting
        // exactly the in-bounds part.
        let mut w = world(8, 8);
        w.paint(&Brush::rect(6, 6, 10, 10, SAND)).unwrap();
        for y in 0..8u16 {
            for x in 0..8u16 {
                let expected = if (6..=7).contains(&x) && (6..=7).contains(&y) {
                    SAND
                } else {
                    0
                };
                assert_eq!(w.cell(x, y).unwrap().material, expected, "({x},{y})");
            }
        }
    }

    #[test]
    fn circle_radius_0_paints_its_centre_only() {
        let mut w = world(16, 16);
        w.paint(&Brush::circle(10, 10, 0, SAND, 0)).unwrap();
        assert_eq!(w.cell(10, 10).unwrap().material, SAND);
        assert_eq!(w.cell(9, 10).unwrap().material, 0);
        assert_eq!(w.cell(10, 9).unwrap().material, 0);
    }

    #[test]
    fn circle_radius_2_covers_exactly_the_disk() {
        let mut w = world(16, 16);
        w.paint(&Brush::circle(8, 8, 2, SAND, 0)).unwrap();
        // Offsets with dx² + dy² <= 4: 13 cells.
        let disk = [
            (0, 0),
            (-1, 0),
            (1, 0),
            (0, -1),
            (0, 1),
            (-1, -1),
            (1, 1),
            (-1, 1),
            (1, -1),
            (-2, 0),
            (2, 0),
            (0, -2),
            (0, 2),
        ];
        for (dx, dy) in disk {
            assert_eq!(
                w.cell((8 + dx) as u16, (8 + dy) as u16).unwrap().material,
                SAND,
                "offset {dx},{dy}"
            );
        }
        // Disc corners: |dx|=1,|dy|=2 and |dx|=2,|dy|=1 must stay empty.
        for (dx, dy) in [(-1, -2), (1, 2), (-2, -1), (2, 1)] {
            assert_eq!(
                w.cell((8 + dx) as u16, (8 + dy) as u16).unwrap().material,
                0,
                "offset {dx},{dy}"
            );
        }
    }

    /// §6 spawn semantics: a painted cell takes the material's
    /// `default_temp` and a fresh aux — even when it is overpainting a
    /// cell that sat at a different temperature.
    #[test]
    fn paint_spawns_the_material_with_default_temp_and_fresh_aux() {
        let mut w = world(16, 16);
        // Preheat and pre-mark the cell so the spawn has to overwrite.
        w.set_temp(5, 5, 800).unwrap(); // 200 K
        w.set_aux(5, 5, 444).unwrap();
        w.paint(&Brush::rect(5, 5, 5, 5, FIRE)).unwrap();

        let cell = w.cell(5, 5).unwrap();
        assert_eq!(cell.material, FIRE);
        assert_eq!(cell.temp.0, 4800); // fire defaults to 1200 K
        assert_ne!(cell.aux, 444); // fresh aux, not inherited
        assert!((30..=90).contains(&cell.aux), "aux in the fire life range");

        // A material without `aux_init` spawns aux 0 — the fresh
        // instance does not leak aux from whatever sat there before.
        w.paint(&Brush::rect(6, 6, 6, 6, SAND)).unwrap();
        assert_eq!(w.cell(6, 6).unwrap().aux, 0);
        // ...and its spawn temp is the material's default (ambient).
        assert_eq!(w.cell(6, 6).unwrap().temp, Temp::AMBIENT);
    }

    /// Painting `empty` erases: the covered cells become fresh empty
    /// cells — vacuum, ambient temperature, aux 0.
    #[test]
    fn painting_empty_resets_cells_to_fresh_empty() {
        let mut w = world(16, 16);
        w.paint(&Brush::rect(3, 3, 3, 3, FIRE)).unwrap();
        w.paint(&Brush::rect(3, 3, 3, 3, 0)).unwrap();
        let cell = w.cell(3, 3).unwrap();
        assert_eq!(cell.material, 0);
        assert_eq!(cell.temp, Temp::AMBIENT);
        assert_eq!(cell.aux, 0);
    }

    /// An out-of-pack brush id is API misuse (D10): rejected whole,
    /// nothing half-written, and no RNG consumed (the exit happens
    /// before any per-cell draw).
    #[test]
    fn unknown_material_paint_is_rejected_whole() {
        let mut w = world(16, 16);
        let before = w.rng_state();
        let err = w.paint(&Brush::rect(5, 5, 6, 6, u16::MAX)).unwrap_err();
        assert_eq!(err, SilicaError::UnknownMaterial { material: u16::MAX });
        for x in [5u16, 6] {
            assert_eq!(w.cell(x, 5).unwrap().material, 0, "(x,5) untouched");
        }
        assert_eq!(w.rng_state(), before);
    }

    /// The aux draw order: one draw per covered cell, in the cover
    /// enumeration order (row-major ascending), equal to
    /// `lo + word % span` off a mirror stream.
    #[test]
    fn aux_draws_consume_the_stream_in_cover_order() {
        let mut w = world(16, 16); // seed 0
        // 2×2 fire rect: four covered cells in row-major order.
        w.paint(&Brush::rect(2, 1, 3, 2, FIRE)).unwrap();
        let order = [(2u16, 1u16), (3, 1), (2, 2), (3, 2)];
        let mut mirror = Rng::new(0);
        for (x, y) in order {
            let word = mirror.next_u32();
            // §6 loader-note draw: uniform in [30, 90] = lo + word % span.
            let expected = 30 + (word % 61);
            assert_eq!(
                w.cell(x, y).unwrap().aux as u32,
                expected,
                "aux at ({x},{y})"
            );
        }
        assert_eq!(w.rng_state(), mirror.state(), "exactly four draws");
    }

    /// Out-of-bounds covered cells spawn nothing and therefore draw
    /// nothing: the RNG consumption schedule counts in-bounds cells
    /// only (the cover order still governs the in-bounds cells).
    #[test]
    fn straddling_brushes_draw_only_for_in_bounds_cells() {
        // Rect straddling the right/bottom edges: 5×5 stroke, 2×2
        // in-bounds — exactly two spawns per covered row × column.
        let mut w = world(8, 8);
        w.paint(&Brush::rect(6, 6, 10, 10, FIRE)).unwrap();
        let mut mirror = Rng::new(0);
        for _ in 0..4 {
            mirror.next_u32();
        }
        assert_eq!(w.rng_state(), mirror.state(), "four in-bounds spawns");
        for y in [6u16, 7] {
            for x in [6u16, 7] {
                assert_eq!(w.cell(x, y).unwrap().material, FIRE, "({x},{y})");
            }
        }
        assert!(w.cell(8, 7).is_none(), "out of bounds, skipped");

        // Circle centred on the left edge: the fully out-of-bounds
        // column is not enumerated at all, so 4 cells spawn.
        let mut w = world(8, 8);
        w.paint(&Brush::circle(0, 4, 1, FIRE, 0)).unwrap();
        let mut mirror = Rng::new(0);
        for _ in 0..4 {
            mirror.next_u32();
        }
        assert_eq!(w.rng_state(), mirror.state(), "four in-bounds spawns");
        for (x, y) in [(0u16, 3u16), (0, 4), (0, 5), (1, 4)] {
            assert_eq!(w.cell(x, y).unwrap().material, FIRE, "({x},{y})");
        }
    }

    #[test]
    fn paint_stamps_and_wakes_the_covered_region() {
        let mut w = world(64, 64);
        w.paint(&Brush::rect(20, 20, 20, 20, SAND)).unwrap();
        assert_eq!(w.stamp_of(20, 20), Some(0));
        assert!(w.regions.is_active(0, 0)); // cell (20,20) → region (0,0)
        assert!(!w.regions.is_active(1, 0)); // untouched region stays asleep
    }
}
