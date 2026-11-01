//! Silica golden worlds — generators, scripts, and the manifest lock.
//!
//! Worlds here are **seeded scripts in Rust** (TESTING.md pillar 1),
//! never binary blobs: a generator returns a fully constructed
//! `World` and the suite runs it under the deterministic pipeline,
//! comparing state hashes against `golden/manifest.txt` at the manifest's
//! checkpoint ticks (`SPEC.md` appendix A format:
//! `world \t tick \t hash`).
//!
//! Golden hashes are locked artifacts. They change only when sim
//! behavior *should* change, and only with a written justification in
//! the PR body (AGENTS.md rule 7) — the regeneration helper in
//! `golden.rs` is deliberately `#[ignore]`-gated.

/// The golden world generators: worlds as seeded scripts in Rust.
pub mod generators;

// Regeneration policy is documented in `generators`; the manifest
// comparison test is the package's `golden` test target (declared in
// Cargo.toml, which also pins the flat SPEC §11 paths).
