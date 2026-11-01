//! The schema v1 pack loader (`SPEC.md` §6): parse, validate, compile.
//!
//! A pack is a directory of TOML files plus a `pack.toml` manifest
//! (which carries `name`, `version`, `license`): one file per material,
//! each declaring `schema_version = 1` and exactly one
//! `[material.<key>]` table. This module parses those files, applies
//! the schema's name-level validation and compiles the result into
//! [`Pack`] — the registry type `silica-core` dispatches on. Every
//! rejection is a [`PackLoadError`] naming the offending file in its
//! `Display`; invalid packs return errors, never panics (§8 D10).
//!
//! Schema-v1 rules implemented here (the loader notes ratified in
//! D-009):
//! * `schema_version` must be exactly 1. Ids start at 1 — `empty`
//!   (id 0) is implicit and never has a file — and must be unique.
//! * References (`other`, `self_becomes`, `other_becomes`, transition
//!   `to`, `crumble_into`) resolve against the pack's material keys;
//!   the name `empty` resolves to id 0 implicitly (D-009: smoke
//!   dissipation).
//! * A transition sets **exactly one** trigger — `at_temp` (fires when
//!   temp ≥), `below_temp` (temp <) or `aux_exhausted = true` (the
//!   cell's aux reached 0) — plus its `to` target. A material's
//!   transitions keep BTreeMap label order at compile: a deterministic
//!   order pinned for schema v1; evaluation priority becomes a
//!   DECISIONS.md entry when the heat pass needs one.
//! * Reactions need `name`, `other`, `self_becomes`; `other_becomes`
//!   is optional (the other cell is unchanged when absent) and
//!   `probability` defaults to 1.0. An explicit probability must be a
//!   finite number in [0, 1] and compiles to a numerator over
//!   [`PROBABILITY_DENOMINATOR`] by round-half-away — a load-time
//!   IEEE f64 conversion, allowed outside `silica-core` (§8 D1) and
//!   bit-identical on every supported target.
//! * `spread`, when set, is the sole schema-v1 token `explosion`.
//! * `crumble_into` is `rigid`-only and must name a **powder** (§7.4).
//!   This is the one category-legality rule encoded: §6's "solid with
//!   melt" example stays unencoded because appendix B's `metal` is
//!   `solid` and melts — the two documents contradict each other, so
//!   schema v1 checks only what both agree on.
//! * Temperatures are Kelvin and must land exactly in the storable
//!   fixed-point range: finite, 0..=8191.75 K, and a multiple of
//!   0.25 K. Out-of-range values are load errors rather than silent
//!   clamps (a pack that loads must be the pack that simulates).
//!
//! Pack identity (§6's "FNV-1a 64 over the sorted file list + file
//! bytes", matching [`Pack::hash`] for saves/replays, D11) frames each
//! file byte-exactly, in file-name byte-sorted order, `pack.toml`
//! included:
//!
//! ```text
//! u32 LE name length | name bytes | u32 LE content length | content bytes
//! ```
//!
//! where the content bytes are LF-normalized (CRLF pairs become LF), so
//! a `core.autocrlf` checkout cannot fork the identity. The shipped
//! starter pack identifies as `0xc8ce7d9f920fd109` — pinned by tests.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde::Deserialize;
use silica_core::Fnv1a64;
use silica_core::SilicaError;
use silica_core::Temp;
use silica_core::{
    AuxRange, Category, MaterialDef, PROBABILITY_DENOMINATOR, Pack, Reaction, Spread, Transition,
    Trigger,
};

/// Why a pack failed to load.
///
/// Every variant names the offending file in its
/// [`Display`](fmt::Display), so a caller can point at the exact TOML
/// the schema rejected (§8 D10: invalid input returns errors, never
/// panics).
#[derive(Debug, thiserror::Error)]
pub enum PackLoadError {
    /// A file on disk could not be read (or the pack directory itself
    /// was unreadable).
    #[error("could not read `{path}`: {source}")]
    Io {
        /// The path whose read failed.
        path: std::path::PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// A TOML file did not parse: invalid syntax, a key whose type
    /// mismatches schema v1, a missing required field, or a field the
    /// schema does not define (the loader is strict about unknown
    /// keys — a typo must fail the load, not vanish).
    #[error("{file}: {message}")]
    Toml {
        /// The file that failed to parse.
        file: String,
        /// The parser's explanation.
        message: String,
    },
    /// A file parsed but violates schema v1's rules (validation below
    /// the TOML syntax: unknown references, illegal combinations,
    /// out-of-range constants).
    #[error("{file}: {problem}")]
    Schema {
        /// The file the validation failed in.
        file: String,
        /// What the schema requires instead.
        problem: String,
    },
    /// The validated input was rejected by the core registry compiler
    /// — a belt-and-braces surface: the loader already checks the
    /// schema rules with file context, so reaching this variant means
    /// a pack slipped past validation structurally (e.g. an id above
    /// the §6 registry cap).
    #[error("invalid pack (core registry): {0}")]
    Registry(#[from] SilicaError),
}

/// Shorthand for a [`PackLoadError::Schema`] — most validations build
/// one of these.
fn schema_err(file: &str, problem: impl Into<String>) -> PackLoadError {
    PackLoadError::Schema {
        file: file.to_owned(),
        problem: problem.into(),
    }
}

/// Shorthand for a [`PackLoadError::Toml`] parse failure.
fn toml_err(file: &str, message: impl Into<String>) -> PackLoadError {
    PackLoadError::Toml {
        file: file.to_owned(),
        message: message.into(),
    }
}

/// The storable temperature ceiling in Kelvin: `i16::MAX` units × 0.25 K.
const MAX_KELVIN: f64 = i16::MAX as f64 / 4.0;

/// The Kelvin document type: schema v1's `default_temp` and transition
/// thresholds. Accepts either a TOML integer or float; the range and
/// quarter-multiple rules are validated at compile time.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Kelvin(f64);

impl<'de> Deserialize<'de> for Kelvin {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct KelvinVisitor;

        impl serde::de::Visitor<'_> for KelvinVisitor {
            type Value = Kelvin;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a temperature in Kelvin (0..=8191.75, a multiple of 0.25)")
            }

            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Ok(Kelvin(value))
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                // Small integers are exact in f64; absurdly large ones
                // fail the range check at compile time instead.
                Ok(Kelvin(value as f64))
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(Kelvin(value as f64))
            }
        }

        deserializer.deserialize_any(KelvinVisitor)
    }
}

/// The probability document type: schema v1's reaction `probability`,
/// integer or float, mapped to a numerator at compile time.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Probability(f64);

impl<'de> Deserialize<'de> for Probability {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ProbabilityVisitor;

        impl serde::de::Visitor<'_> for ProbabilityVisitor {
            type Value = Probability;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a probability as a number in [0, 1]")
            }

            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Ok(Probability(value))
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(Probability(value as f64))
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(Probability(value as f64))
            }
        }

        deserializer.deserialize_any(ProbabilityVisitor)
    }
}

/// The `pack.toml` manifest. `name`, `version`, `license` are required
/// (§6 loader notes); `description` is tolerated free-text data.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PackManifest {
    /// pack.toml `name`, copied verbatim into the compiled pack.
    name: String,
    /// pack.toml `version`, copied verbatim.
    version: String,
    /// pack.toml `license`, copied verbatim.
    license: String,
    /// Optional free-text description (the starter pack uses it).
    #[allow(dead_code)]
    description: Option<String>,
}

/// One material file's parse tree: `schema_version` at the top, then
/// exactly one `[material.<key>]` table.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MaterialFile {
    /// The schema version marker; must be 1.
    schema_version: u64,
    /// The `[material.<key>]` tables; validated to exactly one entry.
    material: BTreeMap<String, MaterialBody>,
}

/// The `[material.<key>]` body of one material file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MaterialBody {
    /// The permanent appendix-B id.
    id: u16,
    /// The display name (e.g. `Sand`).
    name: String,
    /// The category token; parsed and validated at compile.
    category: String,
    /// Relative ordering only (§2, §6).
    density: u32,
    /// The spawn temperature in Kelvin.
    default_temp: Kelvin,
    /// `#RRGGBB` variants; at least one required.
    colors: Vec<String>,
    /// The `[lo, hi]` aux a spawned cell draws, exactly two entries.
    aux_init: Option<[u16; 2]>,
    /// The powder a hard-impact island re-enters as; rigid-only,
    /// powder target (§7.4).
    crumble_into: Option<String>,
    /// The `[material.<key>.transition]` table: labels pointing at
    /// trigger + `to` bodies.
    transition: Option<BTreeMap<String, TransitionBody>>,
    /// The `[[material.<key>.reaction]]` entries, declaration order.
    reaction: Option<Vec<ReactionBody>>,
}

/// One transition entry inside `[material.<key>.transition]`: exactly
/// one trigger key plus the `to` target name.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransitionBody {
    /// `at_temp`: fires when the cell temp is ≥ this Kelvin threshold.
    at_temp: Option<Kelvin>,
    /// `below_temp`: fires when the cell temp is < this Kelvin threshold.
    below_temp: Option<Kelvin>,
    /// `aux_exhausted = true`: fires when the cell's aux reached 0.
    aux_exhausted: Option<bool>,
    /// The material the cell becomes; `empty` resolves to id 0.
    to: String,
}

/// One `[[material.<key>.reaction]]` entry.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReactionBody {
    /// The rule name (a descriptive label, not a keyword).
    name: String,
    /// The neighbour material the rule targets; `empty` → id 0.
    other: String,
    /// What the reacting cell becomes.
    self_becomes: String,
    /// What the other cell becomes; omitted leaves it unchanged.
    other_becomes: Option<String>,
    /// The chance the rule fires; omitted means 1.0.
    probability: Option<Probability>,
    /// Special propagation, if any (schema v1: `explosion` only).
    spread: Option<String>,
}

/// A material whose file has been parsed and structurally validated,
/// mid-compile: its references (strings) resolve against the pack's
/// key table in the second phase.
struct ParsedMaterial<'a> {
    /// The file the material came from (error messages).
    file: &'a str,
    /// The `[material.<key>]` key — the reference namespace.
    key: String,
    /// The category, parsed from the token in phase one.
    category: Category,
    /// The body, still holding unresolved reference names.
    body: MaterialBody,
}

/// Loads a material pack from a directory (SPEC.md §6).
///
/// Reads every `*.toml` file in `dir` (`pack.toml` required, one file
/// per material) and compiles them into the registry. Non-TOML files
/// (e.g. a README) are ignored. File names supply both the identity
/// hash and the error messages, so the directory listing order cannot
/// change either.
///
/// # Errors
///
/// [`PackLoadError`] for unreadable paths, schema violations (the
/// variant's `Display` names the file), and the core registry
/// compiler's own rejections.
pub fn load_pack(dir: &Path) -> Result<Pack, PackLoadError> {
    // Directory names in byte order: the identity hash and the
    // deterministic first-error rule both depend on them, not on the
    // OS listing order.
    let mut names: Vec<String> = Vec::new();
    let read = std::fs::read_dir(dir).map_err(|source| PackLoadError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in read {
        let entry = entry.map_err(|source| PackLoadError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let name = entry.file_name();
        let name = name.to_string_lossy().into_owned();
        if name.ends_with(".toml") {
            names.push(name);
        }
    }
    names.sort(); // String Ord: byte-by-byte.

    let mut owned: Vec<(String, Vec<u8>)> = Vec::with_capacity(names.len());
    for name in &names {
        let path = dir.join(name);
        let bytes = std::fs::read(&path).map_err(|source| PackLoadError::Io { path, source })?;
        owned.push((name.to_owned(), bytes));
    }
    let files: Vec<(&str, &[u8])> = owned
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    compile_pack(&files)
}

/// Compiles a pack from in-memory files (the WASM-friendly entry).
///
/// Each entry is `(file name with extension, exact file bytes)`; names
/// must be unique and exactly one entry is `pack.toml`, every other
/// entry a material file under schema v1. The bytes are hashed as
/// given (LF-normalized) for the pack identity, so callers hand them
/// over verbatim — no host-side rewriting.
///
/// # Sorting guarantee
///
/// The file list sorts by name bytes before anything else, so the
/// identity hash and every reported validation error are independent
/// of the caller's input order.
///
/// # Errors
///
/// [`PackLoadError`], as with [`load_pack`].
pub fn compile_pack(files: &[(&str, &[u8])]) -> Result<Pack, PackLoadError> {
    if files.is_empty() {
        return Err(schema_err(
            "pack.toml",
            "every pack needs exactly one pack.toml manifest (none found)",
        ));
    }

    // Phase zero: sort by name bytes and detect duplicate names.
    let mut files: Vec<(&str, &[u8])> = files.to_vec();
    files.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    for pair in files.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(schema_err(
                pair[1].0,
                "the pack input may not contain the same file name twice",
            ));
        }
    }

    // The identity hash runs over the whole pack (framing in the
    // module docs: SPEC.md §6 pack identity, D11).
    let hash = pack_identity(&files)?;

    // The manifest: exactly one `pack.toml` (duplicates rejected above).
    let manifest_bytes = files.iter().find_map(|&(name, bytes)| {
        if name == "pack.toml" {
            Some(bytes)
        } else {
            None
        }
    });
    let Some(manifest_bytes) = manifest_bytes else {
        return Err(schema_err(
            "pack.toml",
            "every pack needs exactly one pack.toml manifest (none found)",
        ));
    };
    let manifest: PackManifest = parse_toml(manifest_bytes, "pack.toml")?;
    validate_manifest(&manifest, "pack.toml")?;

    // Phase one: parse every material file, validate it structurally,
    // and build the reference namespace.
    let mut dir: BTreeMap<String, (u16, Category)> = BTreeMap::new();
    let mut id_files: BTreeMap<u16, String> = BTreeMap::new();
    let mut parsed: Vec<ParsedMaterial<'_>> = Vec::new();
    for &(name, bytes) in &files {
        if name == "pack.toml" {
            continue;
        }
        let def: MaterialFile = parse_toml(bytes, name)?;
        if def.schema_version != 1 {
            return Err(schema_err(
                name,
                format!(
                    "unsupported schema_version {} (schema v1 expects 1)",
                    def.schema_version
                ),
            ));
        }
        if def.material.len() != 1 {
            return Err(schema_err(
                name,
                format!(
                    "one material per file (found {} `[material.*]` tables)",
                    def.material.len()
                ),
            ));
        }
        let (key, body) = def.material.into_iter().next().expect("exactly one entry");
        if key == "empty" {
            return Err(schema_err(
                name,
                "`empty` (id 0) is implicit and never has a file",
            ));
        }
        let category = match body.category.as_str() {
            "powder" => Category::Powder,
            "liquid" => Category::Liquid,
            "gas" => Category::Gas,
            "solid" => Category::Solid,
            "rigid" => Category::Rigid,
            "energy" => Category::Energy,
            other => {
                return Err(schema_err(
                    name,
                    format!(
                        "unknown category `{other}` (expected powder|liquid|gas|solid|rigid|energy)"
                    ),
                ));
            }
        };
        if body.id == 0 {
            return Err(schema_err(
                name,
                "explicit id 0 — `empty` is implicit and never has a file",
            ));
        }

        // Cross-file uniqueness: one key per material, one id per key.
        // Two distinct registrations always disagree on the id; a
        // fully-identical duplicate is caught by the id table below.
        let occupied = dir.entry(key.clone()).or_insert((body.id, category));
        if occupied.0 != body.id {
            return Err(schema_err(
                name,
                format!(
                    "duplicate material key `{key}` (first defined in `{}`)",
                    id_files
                        .get(&occupied.0)
                        .map(String::as_str)
                        .unwrap_or("???")
                ),
            ));
        }
        if let Some(first) = id_files.get(&body.id) {
            return Err(schema_err(
                name,
                format!(
                    "duplicate material id {} (first defined in `{first}`)",
                    body.id
                ),
            ));
        }
        id_files.insert(body.id, name.to_owned());
        parsed.push(ParsedMaterial {
            file: name,
            key,
            category,
            body,
        });
    }

    // Phase two: resolve references and build the core definitions.
    // Materials compile in file-name order; the registry then orders
    // them by id.
    let mut defs: Vec<MaterialDef> = Vec::with_capacity(parsed.len());
    for material in &parsed {
        defs.push(build_def(material, &dir)?);
    }
    defs.sort_by_key(|def| def.id);

    Pack::new(&manifest.name, &manifest.version, hash, defs).map_err(PackLoadError::from)
}

/// Validates the manifest's presence rules beyond serde's required
/// fields (empty identity strings carry no information about a pack).
fn validate_manifest(manifest: &PackManifest, file: &str) -> Result<(), PackLoadError> {
    if manifest.name.is_empty() {
        return Err(schema_err(file, "manifest `name` must be non-empty"));
    }
    if manifest.version.is_empty() {
        return Err(schema_err(file, "manifest `version` must be non-empty"));
    }
    if manifest.license.is_empty() {
        return Err(schema_err(file, "manifest `license` must be non-empty"));
    }
    Ok(())
}

/// Parses a TOML document with schema v1's strictness: unknown keys are
/// parse errors, and the bytes must be valid UTF-8 (colors and the §/—
/// comments live in text).
fn parse_toml<T: for<'de> Deserialize<'de>>(bytes: &[u8], file: &str) -> Result<T, PackLoadError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| toml_err(file, format!("file is not valid UTF-8 ({error})")))?;
    toml::from_str(text).map_err(|error| toml_err(file, error.to_string()))
}

/// The pack identity hash (§6, matching [`Pack::hash`], D11): FNV-1a 64
/// over the name-byte-sorted file list, each file framed as
/// `u32 LE name length | name bytes | u32 LE content length |
/// LF-normalized content bytes`.
fn pack_identity(files: &[(&str, &[u8])]) -> Result<u64, PackLoadError> {
    let mut hash = Fnv1a64::new();
    for &(name, bytes) in files {
        let name_len = u32::try_from(name.len())
            .map_err(|_| schema_err(name, "file name exceeds the 4 GiB framing limit"))?;
        let content = normalize_lf(bytes);
        let content_len = u32::try_from(content.len())
            .map_err(|_| schema_err(name, "file exceeds the 4 GiB framing limit"))?;
        hash.update(&name_len.to_le_bytes());
        hash.update(name.as_bytes());
        hash.update(&content_len.to_le_bytes());
        hash.update(&content);
    }
    Ok(hash.finish())
}

/// LF-normalizes pack bytes: CRLF pairs become LF, lone CRs stay
/// verbatim. Never errors — it only feeds the identity hash.
fn normalize_lf(bytes: &[u8]) -> Vec<u8> {
    if !bytes.windows(2).any(|pair| pair == b"\r\n") {
        return bytes.to_vec();
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r'
            && let Some(&b'\n') = bytes.get(index + 1)
        {
            out.push(b'\n');
            index += 2;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    out
}

/// Compiles one parsed material into a core [`MaterialDef`]: resolves
/// every reference against the pack's key table, applies the remaining
/// per-material validation, and builds the fixed-point constants.
fn build_def<'a>(
    material: &ParsedMaterial<'a>,
    dir: &BTreeMap<String, (u16, Category)>,
) -> Result<MaterialDef, PackLoadError> {
    let file = material.file;
    let body = &material.body;

    if body.name.is_empty() {
        return Err(schema_err(file, "material `name` must be non-empty"));
    }

    // default_temp: finite, in the storable range, an exact multiple
    // of 0.25 K — see the module docs for why this is a load error and
    // not a silent clamp.
    let units = kelvin_to_units(body.default_temp.0, "default_temp", file)?;
    let default_temp = Temp::from_units(i32::from(units));

    let colors = compile_colors(&body.colors, file)?;

    let aux_init = body
        .aux_init
        .map(|values| {
            let [lo, hi] = values;
            if lo > hi {
                Err(schema_err(
                    file,
                    format!("aux_init range inverted ([{lo}, {hi}] — lo must be ≤ hi)"),
                ))
            } else {
                Ok(AuxRange { lo, hi })
            }
        })
        .transpose()?;

    let mut transitions = Vec::new();
    for (label, entry) in body.transition.iter().flat_map(|table| table.iter()) {
        transitions.push(compile_transition(label, entry, file, dir)?);
    }

    let mut reactions = Vec::new();
    for reaction in body.reaction.iter().flatten() {
        reactions.push(compile_reaction(reaction, file, dir)?);
    }

    // §7.4/§6 loader notes: rigid-only, and the target must be a
    // powder. `empty` is not a powder — D-009's implicit empty
    // resolution never applies to this key.
    let crumble_into = match &body.crumble_into {
        None => None,
        Some(target) => {
            if material.category != Category::Rigid {
                return Err(schema_err(
                    file,
                    format!(
                        "crumble_into is rigid-only (the material is {:?})",
                        material.category
                    ),
                ));
            }
            let target_id = resolve_ref(target, dir, file, "crumble_into")?;
            if target_id == 0 {
                return Err(schema_err(
                    file,
                    "`crumble_into` names `empty`, which is not a powder",
                ));
            }
            let (_, target_category) = dir.get(target).expect("resolved id is in the dir");
            if *target_category != Category::Powder {
                return Err(schema_err(
                    file,
                    format!(
                        "`crumble_into` must name a powder, and `{target}` is {:?}",
                        target_category
                    ),
                ));
            }
            Some(target_id)
        }
    };

    Ok(MaterialDef {
        id: body.id,
        key: material.key.clone(),
        name: body.name.clone(),
        category: material.category,
        density: body.density,
        default_temp,
        colors,
        aux_init,
        crumble_into,
        transitions,
        reactions,
    })
}

/// Builds one transition: exactly one trigger, threshold compiled into
/// fixed-point units, `to` resolved (the implicit `empty` is a legal
/// target).
///
/// `aux_exhausted = false` counts as unset — a rule carrying it alone
/// fails the exactly-one rule like any zero-trigger rule. Temperature
/// thresholds fail with their own Kelvin error before the trigger
/// selection runs.
fn compile_transition(
    label: &str,
    body: &TransitionBody,
    file: &str,
    dir: &BTreeMap<String, (u16, Category)>,
) -> Result<Transition, PackLoadError> {
    let at_temp = body
        .at_temp
        .as_ref()
        .map(|k| kelvin_to_units(k.0, "at_temp", file))
        .transpose()?;
    let below_temp = body
        .below_temp
        .as_ref()
        .map(|k| kelvin_to_units(k.0, "below_temp", file))
        .transpose()?;
    let triggered = body.aux_exhausted == Some(true);

    let trigger = match (at_temp, below_temp, triggered) {
        (Some(units), None, false) => Trigger::AtTemp { units },
        (None, Some(units), false) => Trigger::BelowTemp { units },
        (None, None, true) => Trigger::AuxExhausted,
        _ => {
            return Err(schema_err(
                file,
                format!(
                    "transition `{label}` must set exactly one of at_temp / below_temp / aux_exhausted = true"
                ),
            ));
        }
    };
    let becomes = resolve_ref(&body.to, dir, file, "to")?;
    Ok(Transition {
        label: label.to_owned(),
        trigger,
        becomes,
    })
}

/// Builds one reaction: references resolved, the probability compiled
/// to a numerator over [`PROBABILITY_DENOMINATOR`] (omitted = 1.0, the
/// loader-notes default), the spread token validated.
///
/// Reactions compile in declaration order; the core registry then
/// stores them in §6's `(other, declaration index)` evaluation order.
fn compile_reaction(
    body: &ReactionBody,
    file: &str,
    dir: &BTreeMap<String, (u16, Category)>,
) -> Result<Reaction, PackLoadError> {
    if body.name.is_empty() {
        return Err(schema_err(file, "reaction `name` must be non-empty"));
    }
    let other = resolve_ref(&body.other, dir, file, "other")?;
    let self_becomes = resolve_ref(&body.self_becomes, dir, file, "self_becomes")?;
    let other_becomes = body
        .other_becomes
        .as_ref()
        .map(|name| resolve_ref(name, dir, file, "other_becomes"))
        .transpose()?;
    let probability = body
        .probability
        .map(|probability| compile_probability(probability, file))
        .transpose()?
        .unwrap_or(PROBABILITY_DENOMINATOR);
    let spread = body
        .spread
        .as_deref()
        .map(|token| match token {
            "explosion" => Ok(Spread::Explosion),
            other => Err(schema_err(
                file,
                format!("unknown spread `{other}` (schema v1 supports only `explosion`)"),
            )),
        })
        .transpose()?;
    Ok(Reaction {
        label: body.name.clone(),
        other,
        self_becomes,
        other_becomes,
        probability,
        spread,
    })
}

/// Compiles the probability document type to the §6 numerator:
/// finite, in [0, 1], converted by round-half-away from zero
/// (`f64::round`). `p ∈ [0, 1]` keeps the product within
/// `[0, 1_000_000]`, so the u32 conversion cannot go out of range.
/// Load-time IEEE f64 conversions are bit-identical across the
/// supported targets (§8 D1's float ban stops at this crate).
fn compile_probability(probability: Probability, file: &str) -> Result<u32, PackLoadError> {
    let value = probability.0;
    if !value.is_finite() {
        return Err(schema_err(
            file,
            format!("reaction `probability` must be finite, got {value}"),
        ));
    }
    if !(0.0..=1.0).contains(&value) {
        return Err(schema_err(
            file,
            format!("reaction `probability` must be in [0, 1], got {value}"),
        ));
    }
    let numerator = (value * f64::from(PROBABILITY_DENOMINATOR)).round();
    Ok(numerator as u32)
}

/// Compiles the `colors` list: at least one variant, each an exact
/// `#RRGGBB` token. Colors are render data (§6: chosen by cell
/// coordinates, never from the RNG), so nothing here affects the
/// simulation — but a bit-exact byte format is still enforced so the
/// pack remains a byte-level contract.
fn compile_colors(tokens: &[String], file: &str) -> Result<Vec<[u8; 3]>, PackLoadError> {
    if tokens.is_empty() {
        return Err(schema_err(
            file,
            "`colors` must have at least one `#RRGGBB` entry",
        ));
    }
    let mut colors = Vec::with_capacity(tokens.len());
    for (index, token) in tokens.iter().enumerate() {
        colors.push(parse_color(token, file, index)?);
    }
    Ok(colors)
}

/// Parses one `#RRGGBB` token into its RGB triple.
fn parse_color(token: &str, file: &str, index: usize) -> Result<[u8; 3], PackLoadError> {
    let bytes = token.as_bytes();
    let invalid = |detail: String| schema_err(file, format!("colors[{index}] {detail}"));
    if bytes.len() != 7 || bytes[0] != b'#' {
        return Err(invalid(format!("`{token}` is not `#RRGGBB`")));
    }
    // Digits: u8 subtraction is bounded by the match arms, and
    // 15 × 16 + 15 ≤ 255 by construction — documented non-capable
    // arithmetic (AGENTS rule 2).
    fn nibble(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }
    let mut values = [0u8; 6];
    for (position, slot) in values.iter_mut().enumerate() {
        *slot = nibble(bytes[1 + position]).ok_or_else(|| {
            invalid(format!(
                "`{token}` is not `#RRGGBB` (byte {position} is not hex)"
            ))
        })?;
    }
    Ok([
        values[0] * 16 + values[1],
        values[2] * 16 + values[3],
        values[4] * 16 + values[5],
    ])
}

/// Resolves one reference name against the pack's key table. `empty`
/// resolves to id 0 implicitly (D-009): vacuum-facing rules like
/// smoke's dissipation compile without the pack naming a material.
fn resolve_ref(
    name: &str,
    dir: &BTreeMap<String, (u16, Category)>,
    file: &str,
    field: &str,
) -> Result<u16, PackLoadError> {
    if name == "empty" {
        return Ok(0);
    }
    dir.get(name).map(|(id, _)| *id).ok_or_else(|| {
        schema_err(
            file,
            format!("`{field}` names `{name}`, which no material in the pack defines"),
        )
    })
}

/// Validates a Kelvin document: finite, within the storable range
/// ([`MAX_KELVIN`]), and an exact multiple of 0.25 K. Tolerance
/// `1e-6` units absorbs binary-float representation around exact
/// quarters while rejecting wrong-scale values (e.g. 295.3 K →
/// 1181.2 units, off by 0.2).
fn kelvin_to_units(kelvin: f64, what: &str, file: &str) -> Result<u16, PackLoadError> {
    if !kelvin.is_finite() {
        return Err(schema_err(
            file,
            format!("`{what}` must be finite, got {kelvin}"),
        ));
    }
    if kelvin < 0.0 {
        return Err(schema_err(
            file,
            format!(
                "`{what}` must be >= 0 K (fixed-point temperatures are non-negative), got {kelvin} K"
            ),
        ));
    }
    if kelvin > MAX_KELVIN {
        return Err(schema_err(
            file,
            format!(
                "`{what}` exceeds the storable ceiling (8191.75 K in 0.25 K fixed point), got {kelvin} K"
            ),
        ));
    }
    let units = kelvin * 4.0;
    if (units - units.round()).abs() > 1e-6 {
        return Err(schema_err(
            file,
            format!("`{what}` must be a multiple of 0.25 K, got {kelvin} K"),
        ));
    }
    // Within `i16::MAX` units by the ceiling check above.
    Ok(units as u16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use silica_core::PackProblem;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The shipped starter pack's identity, computed independently
    /// (a Python FNV-1a reference over the framed, LF-normalized
    /// stream) before being pinned here.
    const PACK_ANCHOR: u64 = 0xc8ce_7d9f_920f_d109;

    /// Repo layout: `crates/silica-dsl` → `../../materials`.
    fn materials_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root present")
            .join("materials")
    }

    /// A fresh temporary pack directory per call; error-matrix packs
    /// are built on disk so the directory entry ([`load_pack`]) is
    /// exercised, not just the in-memory one.
    fn temp_pack_dir(tag: &str) -> PathBuf {
        static VISIT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "silica-dsl-load-{}-{}-{}",
            std::process::id(),
            tag,
            VISIT.fetch_add(1, Ordering::Relaxed),
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const MANIFEST: &str = "name = \"t\"\nversion = \"1.0.0\"\nlicense = \"MIT\"\n";

    /// A minimal valid material file; `extra` lands inside the
    /// `[material.<key>]` table (plain keys like `crumble_into`, or
    /// `[material.<key>.transition]` / `[[material.<key>.reaction]]`
    /// sub-tables after them).
    fn material_file(key: &str, id: u16, category: &str, extra: &str) -> String {
        format!(
            "schema_version = 1\n[material.{key}]\nid = {id}\nname = \"{key}\"\ncategory = \"{category}\"\ndensity = 1\ndefault_temp = 295\ncolors = [\"#101010\"]\n{extra}"
        )
    }

    /// The shared minimal pack: manifest + `key.toml`.
    fn pack_with_sand(sand: String) -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("pack.toml", MANIFEST.as_bytes().to_vec()),
            ("sand.toml", sand.into_bytes()),
        ]
    }

    /// Asserts the compile fails exactly where the test aims: a
    /// [`PackLoadError::Schema`] naming `want_file` with `fragment`
    /// in the problem text.
    fn expect_schema_error(files: &[(&str, Vec<u8>)], want_file: &str, fragment: &str) {
        let borrowed: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(name, bytes)| (*name, bytes.as_slice()))
            .collect();
        match compile_pack(&borrowed) {
            Err(PackLoadError::Schema { file, problem }) => {
                assert_eq!(file, want_file, "problem was: {problem}");
                assert!(
                    problem.contains(fragment),
                    "problem `{problem}` lacks `{fragment}`"
                );
            }
            other => panic!(
                "expected a schema error naming `{want_file}` containing `{fragment}`, got {other:?}"
            ),
        }
    }

    /// Same as [`expect_schema_error`] for [`PackLoadError::Toml`] —
    /// parse-level rejections (syntax, unknown keys, missing fields).
    fn expect_toml_error(files: &[(&str, Vec<u8>)], want_file: &str, fragment: &str) {
        let borrowed: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(name, bytes)| (*name, bytes.as_slice()))
            .collect();
        match compile_pack(&borrowed) {
            Err(PackLoadError::Toml { file, message }) => {
                assert_eq!(file, want_file, "message was: {message}");
                assert!(
                    message.contains(fragment),
                    "message `{message}` lacks `{fragment}`"
                );
            }
            other => panic!(
                "expected a parse error naming `{want_file}` containing `{fragment}`, got {other:?}"
            ),
        }
    }

    /// The in-memory compile: borrows the owned files and, on a
    /// successful pack, unwraps — the valid-case combinations only.
    /// Invalid-input tests use the destructive helpers above instead.
    fn compile(files: &[(&str, Vec<u8>)]) -> Result<Pack, PackLoadError> {
        let borrowed: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(name, bytes)| (*name, bytes.as_slice()))
            .collect();
        compile_pack(&borrowed)
    }

    #[test]
    fn starter_pack_loads_with_the_pinned_identity() {
        let pack = load_pack(&materials_dir()).unwrap();
        assert_eq!(pack.name(), "silica-default");
        assert_eq!(pack.version(), "0.1.0");
        assert_eq!(
            pack.hash(),
            PACK_ANCHOR,
            "rule 7: stop before regenerating anything and investigate the mismatch"
        );
        let ids: Vec<u16> = pack.materials().iter().map(|def| def.id).collect();
        assert_eq!(ids, [1, 2, 3, 4, 5, 6, 7, 12]);
        let categories: Vec<Category> = pack.materials().iter().map(|def| def.category).collect();
        assert_eq!(
            categories,
            [
                Category::Powder, // sand
                Category::Liquid, // water
                Category::Liquid, // oil
                Category::Energy, // fire
                Category::Gas,    // smoke
                Category::Gas,    // steam
                Category::Rigid,  // wood
                Category::Rigid,  // stone
            ]
        );
    }

    #[test]
    fn starter_pack_definitions_reach_the_registry_faithfully() {
        let pack = load_pack(&materials_dir()).unwrap();

        let sand = pack.get(1).unwrap();
        assert_eq!(sand.key, "sand");
        assert_eq!(sand.density, 1600);
        assert_eq!(sand.default_temp, Temp::AMBIENT); // 295 K
        assert_eq!(
            sand.colors,
            vec![[0xd9, 0xc0, 0x7a], [0xcf, 0xb5, 0x6f], [0xe2, 0xcc, 0x8b]]
        );
        assert_eq!(sand.aux_init, None);
        assert!(sand.reactions.is_empty());

        let water = pack.get(2).unwrap();
        assert_eq!(water.reactions.len(), 1);
        let douse = &water.reactions[0];
        assert_eq!(douse.label, "douse");
        assert_eq!(douse.other, 4); // fire
        assert_eq!(douse.self_becomes, 6); // steam
        assert_eq!(douse.other_becomes, Some(0)); // the fire is gone
        assert_eq!(douse.probability, PROBABILITY_DENOMINATOR); // 1.0
        assert_eq!(douse.spread, None);

        let oil = pack.get(3).unwrap();
        assert_eq!(oil.density, 850); // < water → floats
        assert_eq!(oil.reactions.len(), 1);
        let ignite = &oil.reactions[0];
        assert_eq!(ignite.label, "ignite");
        assert_eq!(ignite.other, 4);
        assert_eq!(ignite.self_becomes, 4); // fire
        assert_eq!(ignite.other_becomes, None); // the fire cell is unchanged
        assert_eq!(ignite.probability, 900_000); // 0.9

        let fire = pack.get(4).unwrap();
        assert_eq!(fire.default_temp.units(), 4800); // 1200 K
        assert_eq!(fire.aux_init, Some(AuxRange { lo: 30, hi: 90 }));
        assert_eq!(fire.transitions.len(), 1);
        assert_eq!(fire.transitions[0].label, "burn_out");
        assert_eq!(fire.transitions[0].trigger, Trigger::AuxExhausted);
        assert_eq!(fire.transitions[0].becomes, 5); // smoke

        let smoke = pack.get(5).unwrap();
        assert_eq!(smoke.default_temp.units(), 1400); // 350 K
        let dissipate = &smoke.reactions[0];
        assert_eq!(dissipate.label, "dissipate");
        assert_eq!(dissipate.other, 0); // the implicit empty
        assert_eq!(dissipate.self_becomes, 0);
        assert_eq!(dissipate.probability, 1_000); // 0.001

        let steam = pack.get(6).unwrap();
        assert_eq!(steam.default_temp.units(), 1600); // 400 K
        assert_eq!(steam.transitions.len(), 1);
        let condense = &steam.transitions[0];
        assert_eq!(condense.label, "condense");
        assert_eq!(condense.trigger, Trigger::BelowTemp { units: 1492 }); // 373 K
        assert_eq!(condense.becomes, 2); // water

        let wood = pack.get(7).unwrap();
        assert_eq!(wood.crumble_into, None);
        assert_eq!(wood.reactions.len(), 1);
        assert_eq!(wood.reactions[0].label, "ignite");
        assert_eq!(wood.reactions[0].probability, 50_000); // 0.05

        let stone = pack.get(12).unwrap();
        assert_eq!(stone.density, 2500);
        assert_eq!(stone.crumble_into, Some(1)); // sand
    }

    /// Reactions compile in declaration order, then the core registry
    /// canonicalizes to §6's `(other, declaration index)` order.
    #[test]
    fn reactions_canonicalize_to_the_6_evaluation_order() {
        let sand = material_file(
            "sand",
            1,
            "powder",
            "[[material.sand.reaction]]\nname = \"faceted\"\nother = \"gravel\"\nself_becomes = \"empty\"\n[[material.sand.reaction]]\nname = \"first\"\nother = \"empty\"\nself_becomes = \"empty\"\n",
        );
        let mut files = pack_with_sand(sand);
        files.push((
            "gravel.toml",
            material_file("gravel", 2, "powder", "").into_bytes(),
        ));
        let pack = compile(&files).unwrap();
        let labels: Vec<&str> = pack
            .get(1)
            .unwrap()
            .reactions
            .iter()
            .map(|reaction| reaction.label.as_str())
            .collect();
        assert_eq!(labels, ["first", "faceted"]); // empty (0) before gravel
    }

    /// The 0.1 load surface accepts a reaction against the implicit
    /// `empty` without the pack carrying an empty material file.
    #[test]
    fn the_implicit_empty_is_a_reference_target() {
        let sand = material_file(
            "sand",
            1,
            "powder",
            "[[material.sand.reaction]]\nname = \"vanish\"\nother = \"empty\"\nself_becomes = \"empty\"\nprobability = 0\n",
        );
        let pack = compile(&pack_with_sand(sand)).unwrap();
        let reaction = &pack.get(1).unwrap().reactions[0];
        assert_eq!(reaction.other, 0);
        assert_eq!(reaction.probability, 0, "a probability of 0.0 is legal");
    }

    /// An omitted probability is the loader-notes default 1.0.
    #[test]
    fn reaction_probability_defaults_to_one() {
        let sand = material_file(
            "sand",
            1,
            "powder",
            "[[material.sand.reaction]]\nname = \"always\"\nother = \"empty\"\nself_becomes = \"empty\"\n",
        );
        let pack = compile(&pack_with_sand(sand)).unwrap();
        assert_eq!(
            pack.get(1).unwrap().reactions[0].probability,
            PROBABILITY_DENOMINATOR
        );
    }

    /// The integer form of a probability compiles like 1.0 does.
    #[test]
    fn reaction_probability_accepts_whole_numbers() {
        let sand = material_file(
            "sand",
            1,
            "powder",
            "[[material.sand.reaction]]\nname = \"ones\"\nother = \"empty\"\nself_becomes = \"empty\"\nprobability = 1\n",
        );
        let pack = compile(&pack_with_sand(sand)).unwrap();
        assert_eq!(
            pack.get(1).unwrap().reactions[0].probability,
            PROBABILITY_DENOMINATOR
        );
    }

    /// `at_temp` is the schema's third trigger form (appendix B's
    /// melt transitions use it in later milestones).
    #[test]
    fn transitions_accept_the_at_temp_form() {
        let sand = material_file(
            "sand",
            1,
            "powder",
            "[material.sand.transition]\nmelt = { at_temp = 100, to = \"empty\" }\n",
        );
        let pack = compile(&pack_with_sand(sand)).unwrap();
        assert_eq!(
            pack.get(1).unwrap().transitions[0].trigger,
            Trigger::AtTemp { units: 400 }
        );
    }

    #[test]
    fn the_explosion_spread_compiles() {
        let sand = material_file(
            "sand",
            1,
            "powder",
            "[[material.sand.reaction]]\nname = \"pop\"\nother = \"empty\"\nself_becomes = \"empty\"\nspread = \"explosion\"\n",
        );
        let pack = compile(&pack_with_sand(sand)).unwrap();
        assert_eq!(
            pack.get(1).unwrap().reactions[0].spread,
            Some(Spread::Explosion)
        );
    }

    // --- validation matrix --------------------------------------------------
    //
    // Each case builds the smallest pack the rule can load; the
    // error names the file and states the broken rule.

    #[test]
    fn a_pack_without_the_manifest_is_rejected() {
        let dir = temp_pack_dir("no-manifest");
        std::fs::write(
            dir.join("sand.toml"),
            material_file("sand", 1, "powder", ""),
        )
        .unwrap();
        match load_pack(&dir) {
            Err(PackLoadError::Schema { file, problem }) => {
                assert_eq!(file, "pack.toml");
                assert!(problem.contains("exactly one pack.toml"), "{problem}");
            }
            other => panic!("expected a missing-manifest schema error, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_pack_input_needs_the_manifest_too() {
        let borrowed: [(&str, &[u8]); 0] = [];
        match compile_pack(&borrowed) {
            Err(PackLoadError::Schema { file, problem }) => {
                assert_eq!(file, "pack.toml");
                assert!(problem.contains("exactly one pack.toml"), "{problem}");
            }
            other => panic!("expected a missing-manifest schema error, got {other:?}"),
        }
    }

    #[test]
    fn a_manifest_missing_a_required_field_is_rejected() {
        let files = vec![
            ("pack.toml", b"name = \"t\"\nversion = \"1.0.0\"\n".to_vec()),
            (
                "sand.toml",
                material_file("sand", 1, "powder", "").into_bytes(),
            ),
        ];
        expect_toml_error(&files, "pack.toml", "missing field `license`");
    }

    #[test]
    fn unknown_manifest_keys_are_parse_errors() {
        let manifest = "name = \"t\"\nversion = \"1.0.0\"\nlicense = \"MIT\"\nlicence = \"MIT\"\n";
        let files = vec![
            ("pack.toml", manifest.as_bytes().to_vec()),
            (
                "sand.toml",
                material_file("sand", 1, "powder", "").into_bytes(),
            ),
        ];
        expect_toml_error(&files, "pack.toml", "unknown field");
    }

    #[test]
    fn unknown_material_keys_are_parse_errors() {
        let files = pack_with_sand(material_file("sand", 1, "powder", "friction = 3\n"));
        expect_toml_error(&files, "sand.toml", "unknown field");
    }

    #[test]
    fn a_wrong_schema_version_is_rejected() {
        let sand = material_file("sand", 1, "powder", "")
            .replace("schema_version = 1", "schema_version = 2");
        expect_schema_error(&pack_with_sand(sand), "sand.toml", "schema_version");
    }

    #[test]
    fn duplicate_ids_across_files_are_rejected() {
        // File-name order is fixed: gravel.toml parses first, so
        // sand.toml is the duplicate that gets reported.
        let files = vec![
            ("pack.toml", MANIFEST.as_bytes().to_vec()),
            (
                "gravel.toml",
                material_file("gravel", 1, "powder", "").into_bytes(),
            ),
            (
                "sand.toml",
                material_file("sand", 1, "powder", "").into_bytes(),
            ),
        ];
        expect_schema_error(&files, "sand.toml", "duplicate material id 1");
    }

    #[test]
    fn duplicate_material_keys_across_files_are_rejected() {
        let files = vec![
            ("pack.toml", MANIFEST.as_bytes().to_vec()),
            (
                "a.toml",
                material_file("sand", 1, "powder", "").into_bytes(),
            ),
            (
                "b.toml",
                material_file("sand", 2, "powder", "").into_bytes(),
            ),
        ];
        expect_schema_error(&files, "b.toml", "duplicate material key `sand`");
    }

    #[test]
    fn a_material_key_named_empty_is_rejected() {
        let files = vec![
            ("pack.toml", MANIFEST.as_bytes().to_vec()),
            (
                "wood.toml",
                material_file("empty", 8, "solid", "").into_bytes(),
            ),
        ];
        expect_schema_error(&files, "wood.toml", "`empty` (id 0) is implicit");
    }

    #[test]
    fn an_explicit_id_zero_is_rejected() {
        expect_schema_error(
            &pack_with_sand(material_file("sand", 0, "powder", "")),
            "sand.toml",
            "explicit id 0",
        );
    }

    #[test]
    fn an_id_above_the_registry_cap_is_a_core_registry_rejection() {
        let owned = pack_with_sand(material_file("sand", 2000, "powder", ""));
        let borrowed: Vec<(&str, &[u8])> = owned
            .iter()
            .map(|(name, bytes)| (*name, bytes.as_slice()))
            .collect();
        match compile_pack(&borrowed) {
            Err(PackLoadError::Registry(SilicaError::InvalidPack {
                id: 2000,
                problem: PackProblem::IdAboveCap,
            })) => {}
            other => panic!("expected the registry cap rejection, got {other:?}"),
        }
    }

    #[test]
    fn unknown_reference_names_are_rejected() {
        let sand = material_file(
            "sand",
            1,
            "powder",
            "[[material.sand.reaction]]\nname = \"douse\"\nother = \"steem\"\nself_becomes = \"empty\"\n",
        );
        expect_schema_error(&pack_with_sand(sand), "sand.toml", "`other` names `steem`");
    }

    #[test]
    fn an_unknown_transition_target_is_rejected() {
        let sand = material_file(
            "sand",
            1,
            "powder",
            "[material.sand.transition]\nmelt = { at_temp = 100, to = \"grvel\" }\n",
        );
        expect_schema_error(&pack_with_sand(sand), "sand.toml", "`to` names `grvel`");
    }

    #[test]
    fn an_unknown_category_token_is_rejected() {
        expect_schema_error(
            &pack_with_sand(material_file("sand", 1, "plasma", "")),
            "sand.toml",
            "unknown category `plasma`",
        );
    }

    #[test]
    fn an_empty_color_list_is_rejected() {
        let sand =
            material_file("sand", 1, "powder", "").replace("colors = [\"#101010\"]", "colors = []");
        expect_schema_error(&pack_with_sand(sand), "sand.toml", "at least one");
    }

    #[test]
    fn a_non_hex_color_token_is_rejected() {
        let sand = material_file("sand", 1, "powder", "")
            .replace("colors = [\"#101010\"]", "colors = [\"red\"]");
        expect_schema_error(&pack_with_sand(sand), "sand.toml", "colors[0]");
    }

    #[test]
    fn an_inverted_aux_range_is_rejected() {
        expect_schema_error(
            &pack_with_sand(material_file("sand", 1, "powder", "aux_init = [90, 30]\n")),
            "sand.toml",
            "aux_init range inverted ([90, 30]",
        );
    }

    #[test]
    fn probabilities_out_of_range_are_rejected() {
        for probability in ["1.5", "-0.001"] {
            let sand = material_file(
                "sand",
                1,
                "powder",
                &format!(
                    "[[material.sand.reaction]]\nname = \"t\"\nother = \"empty\"\nself_becomes = \"empty\"\nprobability = {probability}\n"
                ),
            );
            expect_schema_error(&pack_with_sand(sand), "sand.toml", "must be in [0, 1]");
        }
    }

    #[test]
    fn infinite_probability_literals_are_rejected() {
        let sand = material_file(
            "sand",
            1,
            "powder",
            "[[material.sand.reaction]]\nname = \"t\"\nother = \"empty\"\nself_becomes = \"empty\"\nprobability = inf\n",
        );
        expect_schema_error(&pack_with_sand(sand), "sand.toml", "must be finite");
    }

    #[test]
    fn an_unknown_spread_token_is_rejected() {
        let sand = material_file(
            "sand",
            1,
            "powder",
            "[[material.sand.reaction]]\nname = \"t\"\nother = \"empty\"\nself_becomes = \"empty\"\nspread = \"laser\"\n",
        );
        expect_schema_error(&pack_with_sand(sand), "sand.toml", "unknown spread `laser`");
    }

    #[test]
    fn crumble_into_is_rigid_only_and_names_a_powder() {
        // A powder may not declare it at all.
        expect_schema_error(
            &pack_with_sand(material_file(
                "sand",
                1,
                "powder",
                "crumble_into = \"sand\"\n",
            )),
            "sand.toml",
            "crumble_into is rigid-only",
        );

        // A rigid declaring a liquid target is rejected too.
        let files = vec![
            ("pack.toml", MANIFEST.as_bytes().to_vec()),
            (
                "stone.toml",
                material_file("stone", 2, "rigid", "crumble_into = \"water\"\n").into_bytes(),
            ),
            (
                "water.toml",
                material_file("water", 3, "liquid", "").into_bytes(),
            ),
        ];
        expect_schema_error(&files, "stone.toml", "`crumble_into` must name a powder");
    }

    #[test]
    fn crumble_into_never_targets_the_implicit_empty() {
        expect_schema_error(
            &pack_with_sand(material_file(
                "sand",
                1,
                "rigid",
                "crumble_into = \"empty\"\n",
            )),
            "sand.toml",
            "`crumble_into` names `empty`, which is not a powder",
        );
    }

    #[test]
    fn an_unknown_crumble_target_is_rejected() {
        expect_schema_error(
            &pack_with_sand(material_file(
                "sand",
                1,
                "rigid",
                "crumble_into = \"silt\"\n",
            )),
            "sand.toml",
            "`crumble_into` names `silt`",
        );
    }

    #[test]
    fn transitions_with_zero_or_two_triggers_are_rejected() {
        for trigger in [
            "melt = { to = \"empty\" }",
            "melt = { aux_exhausted = false, to = \"empty\" }",
            "melt = { at_temp = 100, below_temp = 10, to = \"empty\" }",
            "melt = { at_temp = 100, aux_exhausted = true, to = \"empty\" }",
        ] {
            let sand = material_file(
                "sand",
                1,
                "powder",
                &format!("[material.sand.transition]\n{trigger}\n"),
            );
            expect_schema_error(
                &pack_with_sand(sand),
                "sand.toml",
                "must set exactly one of at_temp / below_temp / aux_exhausted",
            );
        }
    }

    #[test]
    fn temperatures_failing_the_fixed_point_rules_are_rejected() {
        // Not a multiple of 0.25 K, on both scales.
        for value in ["295.3", "0.3"] {
            let sand = material_file("sand", 1, "powder", "")
                .replace("default_temp = 295", &format!("default_temp = {value}"));
            expect_schema_error(
                &pack_with_sand(sand),
                "sand.toml",
                "must be a multiple of 0.25 K",
            );
        }
        // Above the storable ceiling.
        let sand = material_file("sand", 1, "powder", "")
            .replace("default_temp = 295", "default_temp = 9000");
        expect_schema_error(&pack_with_sand(sand), "sand.toml", "storable ceiling");
        // Below zero.
        let sand = material_file("sand", 1, "powder", "")
            .replace("default_temp = 295", "default_temp = -1");
        expect_schema_error(&pack_with_sand(sand), "sand.toml", ">= 0 K");
        // Not finite.
        let sand = material_file("sand", 1, "powder", "")
            .replace("default_temp = 295", "default_temp = nan");
        expect_schema_error(&pack_with_sand(sand), "sand.toml", "must be finite");
    }

    #[test]
    fn a_threshold_failing_the_fixed_point_rules_names_itself() {
        let sand = material_file(
            "sand",
            1,
            "powder",
            "[material.sand.transition]\nmelt = { at_temp = 100.1, to = \"empty\" }\n",
        );
        expect_schema_error(
            &pack_with_sand(sand),
            "sand.toml",
            "`at_temp` must be a multiple",
        );
    }

    #[test]
    fn a_file_with_two_materials_is_rejected() {
        let sand = material_file("sand", 1, "powder", "")
            + "[material.gravel]\nid = 2\nname = \"Gravel\"\ncategory = \"powder\"\ndensity = 1\ndefault_temp = 295\ncolors = [\"#202020\"]\n";
        expect_schema_error(
            &pack_with_sand(sand),
            "sand.toml",
            "one material per file (found 2",
        );
    }

    #[test]
    fn a_missing_required_field_is_a_parse_error() {
        let sand = "schema_version = 1\n[material.sand]\nname = \"Sand\"\ncategory = \"powder\"\ndensity = 1\ndefault_temp = 295\ncolors = [\"#101010\"]\n";
        expect_toml_error(
            &pack_with_sand(sand.to_owned()),
            "sand.toml",
            "missing field `id`",
        );
    }

    #[test]
    fn an_aux_init_of_the_wrong_length_is_a_parse_error() {
        expect_toml_error(
            &pack_with_sand(material_file("sand", 1, "powder", "aux_init = [30]\n")),
            "sand.toml",
            "invalid length",
        );
    }

    /// The LF-normalization guarantee: the same pack written with
    /// CRLF line endings hashes identically to the LF one — a
    /// `core.autocrlf` checkout cannot fork the pack identity.
    #[test]
    fn crlf_contents_hash_identically_to_lf_contents() {
        let lf_dir = temp_pack_dir("crlf-lf");
        std::fs::write(lf_dir.join("pack.toml"), MANIFEST.as_bytes()).unwrap();
        std::fs::write(
            lf_dir.join("sand.toml"),
            material_file("sand", 1, "powder", ""),
        )
        .unwrap();

        let crlf_dir = temp_pack_dir("crlf-crlf");
        std::fs::write(crlf_dir.join("pack.toml"), MANIFEST.replace('\n', "\r\n")).unwrap();
        std::fs::write(
            crlf_dir.join("sand.toml"),
            material_file("sand", 1, "powder", "").replace('\n', "\r\n"),
        )
        .unwrap();

        let lf = load_pack(&lf_dir).unwrap();
        let crlf = load_pack(&crlf_dir).unwrap();
        assert_eq!(
            lf.hash(),
            crlf.hash(),
            "CRLF checkouts must not fork the pack identity"
        );
        // The two-file mini pack's identity itself, computed
        // independently with a Python FNV-1a reference.
        assert_eq!(lf.hash(), 0x7764_db50_3ce2_cf0d);
    }
}
