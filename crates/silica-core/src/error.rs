//! The core error type.
//!
//! `silica-core` is dependency-free (AGENTS.md rule 6), so the
//! `thiserror`-style impls are hand-rolled. The public API reports
//! failures through this enum and never panics on valid input
//! (`SPEC.md` §8 D10).

use std::fmt;

/// Errors returned by `silica-core`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SilicaError {
    /// A cell coordinate lies outside the world grid.
    ///
    /// Reported by the write primitives; reads outside the grid return
    /// `None` instead (TESTING.md pillar 3: API abuse returns errors,
    /// never panics).
    OutOfBounds { x: u16, y: u16 },
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
        }
    }
}

impl std::error::Error for SilicaError {}

#[cfg(test)]
mod tests {
    use super::SilicaError;

    #[test]
    fn display_names_the_out_of_bounds_coordinates() {
        let err = SilicaError::OutOfBounds { x: 512, y: 384 };
        assert_eq!(
            err.to_string(),
            "cell coordinate (x=512, y=384) is outside the world grid"
        );
    }
}
