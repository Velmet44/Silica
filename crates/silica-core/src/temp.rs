//! Fixed-point temperature (SPEC.md §4).

/// A cell temperature in fixed-point units of 0.25 K.
///
/// The storable range is `[Temp::ZERO, Temp::MAX]` = `[0, 32767]` units,
/// i.e. 0 K to 8191.75 K in 0.25 K steps. Every construction path clamps
/// into that range, so the `[0, 32767]` bound of TESTING.md pillar 3
/// holds by construction — the world can never store an out-of-range
/// temperature.
///
/// Per SPEC.md §4, arithmetic on temperatures happens in `i32` units and
/// is clamped on store; this type performs no arithmetic beyond the
/// documented saturating conversions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Temp(pub(crate) i16);

impl Temp {
    /// 0 K.
    pub const ZERO: Temp = Temp(0);
    /// The hottest storable temperature: 32767 units = 8191.75 K.
    pub const MAX: Temp = Temp(i16::MAX);
    /// Ambient default: 295 K = 1180 units (SPEC.md §4).
    pub const AMBIENT: Temp = Temp(1180);

    /// Builds a temperature from raw 0.25 K units, clamped into range.
    pub fn from_units(units: i32) -> Temp {
        Temp(clamp_units(units))
    }

    /// Builds a temperature from whole kelvin, clamped into range.
    ///
    /// Determinism: the `k * 4` scaling is a `saturating_mul`, so extreme
    /// inputs clamp to `Temp::MAX`/`Temp::ZERO` instead of overflowing
    /// (`SPEC.md` §8 D2).
    pub fn from_kelvin(k: i32) -> Temp {
        Temp(clamp_units(k.saturating_mul(4)))
    }

    /// The temperature in raw 0.25 K units (`i32` for cheap arithmetic).
    pub const fn units(self) -> i32 {
        self.0 as i32
    }

    /// The temperature in whole kelvin, rounded half up.
    ///
    /// e.g. 2 units (0.5 K) reads as 1 K; 32767 units (8191.75 K) as
    /// 8192 K. The addition saturates as belt-and-braces: `units` is
    /// already at most 32767, so it cannot overflow `i32`.
    pub fn kelvin(self) -> i32 {
        self.units().saturating_add(2) / 4
    }
}

/// Clamps raw units into the storable `i16` range — SPEC.md §4's valid
/// range `[0, 32767]`, enforced on every store.
fn clamp_units(units: i32) -> i16 {
    units.clamp(0, i16::MAX as i32) as i16
}

#[cfg(test)]
mod tests {
    use super::Temp;

    #[test]
    fn ambient_is_295_kelvin() {
        assert_eq!(Temp::AMBIENT.units(), 1180);
        assert_eq!(Temp::AMBIENT.kelvin(), 295);
        assert_eq!(Temp::from_kelvin(295), Temp::AMBIENT);
    }

    #[test]
    fn kelvin_conversions_are_exact_quarters() {
        assert_eq!(Temp::from_kelvin(0), Temp::ZERO);
        assert_eq!(Temp::from_kelvin(273).units(), 1092);
        assert_eq!(Temp::from_kelvin(373).units(), 1492);
        assert_eq!(Temp::from_kelvin(1700).units(), 6800);
        assert_eq!(Temp::from_kelvin(8191).units(), 32764);
        assert_eq!(Temp::from_kelvin(8192), Temp::MAX); // 32768 clamps
    }

    #[test]
    fn from_units_clamps_into_the_storable_range() {
        assert_eq!(Temp::from_units(0), Temp::ZERO);
        assert_eq!(Temp::from_units(1180), Temp::AMBIENT);
        assert_eq!(Temp::from_units(32767), Temp::MAX);
        assert_eq!(Temp::from_units(32768), Temp::MAX);
        assert_eq!(Temp::from_units(i32::MAX), Temp::MAX);
        assert_eq!(Temp::from_units(-1), Temp::ZERO);
        assert_eq!(Temp::from_units(i32::MIN), Temp::ZERO);
    }

    #[test]
    fn from_kelvin_saturates_without_overflow() {
        // SPEC.md §8 D2: a plain `k * 4` would overflow-panic on these in
        // checked builds; the saturating form must clamp instead.
        assert_eq!(Temp::from_kelvin(i32::MAX), Temp::MAX);
        assert_eq!(Temp::from_kelvin(i32::MIN), Temp::ZERO);
        assert_eq!(Temp::from_kelvin(-273), Temp::ZERO);
    }

    #[test]
    fn kelvin_rounds_half_up() {
        assert_eq!(Temp::from_units(1).kelvin(), 0);
        assert_eq!(Temp::from_units(2).kelvin(), 1); // 0.5 K rounds up
        assert_eq!(Temp::from_units(3).kelvin(), 1);
        assert_eq!(Temp::from_units(4).kelvin(), 1);
        assert_eq!(Temp::from_units(5).kelvin(), 1);
        assert_eq!(Temp::from_units(6).kelvin(), 2);
        assert_eq!(Temp::from_units(7).kelvin(), 2);
        assert_eq!(Temp::ZERO.kelvin(), 0);
        assert_eq!(Temp::MAX.kelvin(), 8192); // 8191.75 K
    }

    #[test]
    fn max_is_the_documented_range_end() {
        assert_eq!(Temp::MAX.units(), i16::MAX as i32);
        assert_eq!(Temp::MAX.units(), 32767);
    }
}
