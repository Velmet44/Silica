//! The core error type.
//!
//! `silica-core` is dependency-free (AGENTS.md rule 6), so the
//! `thiserror`-style impls are hand-rolled. The public API reports
//! failures through this enum and never panics on valid input
//! (`SPEC.md` §8 D10).

use std::fmt;

use crate::pack::PackProblem;

/// Errors returned by `silica-core`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SilicaError {
    /// A cell coordinate lies outside the world grid.
    ///
    /// Reported by the write primitives; reads outside the grid return
    /// `None` instead (TESTING.md pillar 3: API abuse returns errors,
    /// never panics).
    OutOfBounds { x: u16, y: u16 },
    /// A paint stroke named a material id the world's pack does not
    /// define (`SPEC.md` §6: ids originate in the registry, so an id
    /// the registry lacks is API misuse, not valid input).
    ///
    /// Only the paint boundary validates: internal pass writers only
    /// ever recycle ids already in the grid, which is why the
    /// crate-private write primitives accept any `u16`.
    UnknownMaterial { material: u16 },
    /// A pack definition violates the registry contract (`SPEC.md` §6
    /// load validation, enforced again here so the loader is never
    /// implicitly trusted).
    InvalidPack { id: u16, problem: PackProblem },
}

impl fmt::Display for SilicaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SilicaError::OutOfBounds { x, y } => {
                write!(
                    f,
                    "cell coordinate (x={x}, y={y}) is outside the world grid"
                )
            }
            SilicaError::UnknownMaterial { material } => {
                write!(
                    f,
                    "material id {material} is not defined by the world's pack"
                )
            }
            SilicaError::InvalidPack { id, problem } => {
                write!(f, "invalid pack material (id {id}): {problem}")
            }
        }
    }
}

impl std::error::Error for SilicaError {}

#[cfg(test)]
mod tests {
    use super::{PackProblem, SilicaError};

    #[test]
    fn display_names_the_out_of_bounds_coordinates() {
        let err = SilicaError::OutOfBounds { x: 512, y: 384 };
        assert_eq!(
            err.to_string(),
            "cell coordinate (x=512, y=384) is outside the world grid"
        );
    }

    #[test]
    fn display_names_the_unknown_material() {
        let err = SilicaError::UnknownMaterial { material: 17 };
        assert_eq!(
            err.to_string(),
            "material id 17 is not defined by the world's pack"
        );
    }

    #[test]
    fn display_names_the_pack_problem() {
        let err = SilicaError::InvalidPack {
            id: 1025,
            problem: PackProblem::IdAboveCap,
        };
        assert_eq!(
            err.to_string(),
            "invalid pack material (id 1025): id exceeds the registry cap (1024 allowed)"
        );
    }
}
