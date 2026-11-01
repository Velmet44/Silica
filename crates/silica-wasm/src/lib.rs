//! Silica WASM — the browser surface that powers the playground.
//!
//! A wasm-bindgen wrapper over the Rust API of `SPEC.md` §9, with
//! `render_rgba` targeting a JS `ImageData`. The playground (ROADMAP.md
//! 0.2) uses only this surface — it is a thin demo, not a product.
//!
//! `unsafe` is allowed in this crate only inside JS boundary shims, each
//! carrying a `// SAFETY:` comment stating the invariant (AGENTS.md rule
//! 5). None exists yet.
//!
//! Scaffold only — the wasm-bindgen surface lands with milestone 0.2
//! (ROADMAP.md).
