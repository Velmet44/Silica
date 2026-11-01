//! Silica DSL — material packs as data, compiled for the core.
//!
//! Implements the schema v1 load surface of `SPEC.md` §6: a `pack.toml`
//! manifest plus one TOML file per material (`empty` implicit at
//! id 0), parsed with strict unknown-key rejection, validated against
//! the loader notes finalized in D-009, and compiled into the [`Pack`]
//! registry `silica-core` dispatches on. Reactions and transitions are
//! data instead of code — hackability is pillar 4.
//!
//! The two compiles are [`load_pack`] (from a directory) and
//! [`compile_pack`] (from in-memory `(name, bytes)` pairs, the
//! WASM-friendly path). Every pack carries the byte-defined identity
//! hash of §6/D11 in [`Pack::hash`]; failures surface as
//! [`PackLoadError`] with the offending file named.
//!
//! No `unsafe` code is permitted in this crate (AGENTS.md rule 5).

#![forbid(unsafe_code)]

mod loader;

pub use loader::{PackLoadError, compile_pack, load_pack};
