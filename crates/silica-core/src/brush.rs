//! Brush strokes — the only way new material enters the world
//! (SPEC.md §9: "everything a human can do is a brush stroke").

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

/// Applies one brush to the world: every covered in-bounds cell takes
/// the brush material; its temp and aux are untouched (the cell keeps
/// its temperature — v0 semantics, aux spawning arrives with the pack
/// registry, §6).
///
/// Determinism contract:
/// * covered cells are enumerated row-major ascending (`y`, then `x`)
///   so any future per-cell RNG draws (aux_init, §6) consume the stream
///   in a fixed order;
/// * out-of-bounds covered cells are skipped, not errors — a host brush
///   may straddle a world edge (D10: valid input, defined behavior,
///   never a panic);
/// * every written cell is stamped with the current tick and wakes its
///   region (the §5 single-update rule then applies).
pub(crate) fn paint(world: &mut World, brush: &Brush) {
    match brush.kind {
        BrushKind::Rect { x0, y0, x1, y1 } => {
            // Normalize swapped corners; every coordinate stays in
            // `[0, 65535]` so the u32 loop bounds cannot overflow
            // (documented non-capable arithmetic, §8 D2).
            let (min_x, max_x) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
            let (min_y, max_y) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
            for y in u32::from(min_y)..=u32::from(max_y) {
                for x in u32::from(min_x)..=u32::from(max_x) {
                    try_paint(world, x, y, brush.material);
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
                        try_paint(world, x, y, brush.material);
                    }
                }
            }
        }
    }
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
/// D10 — valid input, defined behavior, no panic).
fn try_paint(world: &mut World, x: u32, y: u32, material: u16) {
    if x < u32::from(world.width) && y < u32::from(world.height) {
        // Bounds checked above — the coordinates fit u16, so the error
        // arm of set_material is unreachable here.
        let _ = world.set_material(x as u16, y as u16, material);
    }
}

#[cfg(test)]
mod tests {
    use super::Brush;
    use crate::temp::Temp;
    use crate::world::{World, WorldConfig};

    /// Appendix B, id 1: sand. The compiled registry (§6) reuses this
    /// id; until then the starter pack value is used verbatim.
    const SAND: u16 = 1;

    fn world(w: u16, h: u16) -> World {
        World::new(WorldConfig {
            width: w,
            height: h,
            seed: 0,
            walls: true,
        })
    }

    #[test]
    fn rect_covers_the_closed_range() {
        let mut w = world(16, 16);
        w.paint(&Brush::rect(2, 3, 4, 5, SAND));
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
        w.paint(&Brush::rect(4, 5, 2, 3, SAND));
        for y in 3..=5u16 {
            for x in 2..=4u16 {
                assert_eq!(w.cell(x, y).unwrap().material, SAND, "({x},{y})");
            }
        }
        let mut reference = world(16, 16);
        reference.paint(&Brush::rect(2, 3, 4, 5, SAND));
        assert_eq!(w.state_hash(), reference.state_hash());
    }

    #[test]
    fn rect_out_of_bounds_is_clamped_not_an_error() {
        // A brush may straddle a world edge (D10: no panic), painting
        // exactly the in-bounds part.
        let mut w = world(8, 8);
        w.paint(&Brush::rect(6, 6, 10, 10, SAND));
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
        w.paint(&Brush::circle(10, 10, 0, SAND, 0));
        assert_eq!(w.cell(10, 10).unwrap().material, SAND);
        assert_eq!(w.cell(9, 10).unwrap().material, 0);
        assert_eq!(w.cell(10, 9).unwrap().material, 0);
    }

    #[test]
    fn circle_radius_2_covers_exactly_the_disk() {
        let mut w = world(16, 16);
        w.paint(&Brush::circle(8, 8, 2, SAND, 0));
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

    #[test]
    fn paint_touches_material_only_keeps_temp_and_aux() {
        let mut w = world(16, 16);
        w.paint(&Brush::rect(5, 5, 6, 6, SAND));
        let cell = w.cell(5, 5).unwrap();
        assert_eq!(cell.temp, Temp::AMBIENT);
        assert_eq!(cell.aux, 0);
        let empty = w.cell(0, 0).unwrap();
        assert_eq!(empty.temp, Temp::AMBIENT);
        assert_eq!(empty.aux, 0);
    }

    #[test]
    fn paint_stamps_and_wakes_the_covered_region() {
        let mut w = world(64, 64);
        w.paint(&Brush::rect(20, 20, 20, 20, SAND));
        assert_eq!(w.stamp_of(20, 20), Some(0));
        assert!(w.regions.is_active(0, 0)); // cell (20,20) → region (0,0)
        assert!(!w.regions.is_active(1, 0)); // untouched region stays asleep
    }
}
