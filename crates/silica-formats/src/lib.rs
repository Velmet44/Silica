//! Silica formats — saves and replays as small, shareable artifacts.
//!
//! Implements the byte-level contracts of `SPEC.md` appendix A: the
//! `*.silica` world save (`"SILW"` magic, little-endian, RLE streams over
//! material, then temp, then aux) and the `*.silrep` replay (seed, pack
//! hash and ordered brush events), plus the canonical FNV-1a 64 state
//! hash. All serialization is little-endian (D7); saves and replays record
//! the pack content hash (D11).
//!
//! No `unsafe` code is permitted in this crate (AGENTS.md rule 5).
//!
//! Scaffold only — the codecs land with milestone 0.3 (ROADMAP.md).

#![forbid(unsafe_code)]
