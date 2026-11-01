//! Silica DSL — material packs as data, compiled for the core.
//!
//! Loads the pack format of `SPEC.md` §6 (schema version 1: a `pack.toml`
//! manifest plus one TOML file per material, `empty` implicit at id 0) and
//! compiles it into the material registry types defined by `silica-core`.
//! Reactions are data instead of code — hackability is pillar 4.
//!
//! Load validation rejects duplicate ids, unknown names in reactions or
//! transitions, categories with illegal behavior (e.g. `solid` with melt)
//! and ids above the registry cap (1024). Invalid input returns errors,
//! never panics (`SPEC.md` §8 D10).
//!
//! No `unsafe` code is permitted in this crate (AGENTS.md rule 5).
//!
//! Scaffold only — the loader lands with milestone 0.1 (ROADMAP.md).

#![forbid(unsafe_code)]
