//! The compiled material pack registry (SPEC.md §6).
//!
//! A pack is a directory of TOML files plus a `pack.toml` manifest; the
//! `silica-dsl` loader parses and validates it and compiles it into the
//! [`Pack`] defined here — the type the world dispatches on. The
//! registry owns the canonical order: materials sit in a `Vec` sorted
//! by id, reactions per material in `(other, declaration index)` order
//! (§6's load order), and a category lookup is an O(1) table — no
//! `HashMap` anywhere (D3). Construction allocates; the steady-state
//! tick only reads.

use std::fmt;

use crate::error::SilicaError;
use crate::temp::Temp;

/// The denominator of reaction probabilities (SPEC.md §6: "rationals
/// with denominator 1,000,000"). A compiled [`Reaction`] stores the
/// integer numerator — `probability` over this denominator.
pub const PROBABILITY_DENOMINATOR: u32 = 1_000_000;

/// One of the six material categories (SPEC.md §3), plus the `None`
/// slot that the dispatch table keeps for the implicit `empty` (id 0)
/// and for any id the pack does not define.
///
/// `None` is never a real category: [`Pack::new`] rejects a
/// [`MaterialDef`] that claims it. Its only meaning is "no material
/// here — nothing moves, nothing reacts".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    /// The empty slot: id 0, and ids outside the pack. Never moves.
    None,
    /// Granular: falls, slides off piles, piles at rest (sand, snow).
    Powder,
    /// Flows, orders by relative density, enters dispersion (water, oil).
    Liquid,
    /// Rises, jitters, decays (smoke, steam).
    Gas,
    /// Never moves and is not part of islands (bedrock, clone).
    Solid,
    /// Participates in the §7 support analysis; can become an island
    /// (stone, wood, ice).
    Rigid,
    /// Category-defined aux lifetime/behavior (fire life, spark).
    Energy,
}

/// The `[lo, hi]` aux value a material's spawned cells draw (SPEC.md §6
/// loader notes: one uniform draw per spawned cell from the D4 stream).
///
/// `lo <= hi` is enforced by [`Pack::new`]; when `lo == hi` the range
/// is constant but still consumes one draw, so the consumption
/// schedule depends only on the pack shape, never on the values.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AuxRange {
    /// Smallest drawn aux value, inclusive.
    pub lo: u16,
    /// Largest drawn aux value, inclusive.
    pub hi: u16,
}

impl AuxRange {
    /// The range's span `hi - lo + 1` (at most 65536).
    pub(crate) fn span(self) -> u32 {
        // Both operands are ≤ 65535 and the difference fits u32
        // trivially; documented non-capable arithmetic (§8 D2).
        u32::from(self.hi) - u32::from(self.lo) + 1
    }
}

/// What condition fires a transition (schema v1 keys; SPEC.md §6
/// loader notes). Cell temperatures are stored as 0.25 K units ≥ 0, so
/// thresholds compile to `u16` units.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trigger {
    /// `at_temp`: fires when the cell temp is **≥** the threshold.
    AtTemp { units: u16 },
    /// `below_temp`: fires when the cell temp is **<** the threshold.
    BelowTemp { units: u16 },
    /// `aux_exhausted`: fires when the cell's aux word reached 0.
    AuxExhausted,
}

/// A named transition of one material — schema v1's
/// `[material.<key>.transition]` table entries (SPEC.md §6 loader
/// notes; e.g. fire's `burn_out`).
///
/// The rule name (`melt`, `burn_out`, …) is descriptive data, not a
/// keyword. The per-material transition order is the parse order —
/// BTreeMap label order, a deterministic order pinned for schema v1;
/// evaluation priority (heat pass, 0.1) will be a DECISIONS.md entry
/// when it matters.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Transition {
    /// The rule name from the pack (data, not a keyword).
    pub label: String,
    /// The condition that fires this transition.
    pub trigger: Trigger,
    /// The material the cell becomes, resolved to its id at load.
    pub becomes: u16,
}

/// One reaction rule of one material — schema v1's
/// `[[material.<key>.reaction]]` entries (SPEC.md §6 loader notes).
///
/// Precondition order is part of behavior: §6 sorts a material's
/// reactions by `(a_id, b_id or 0, declaration index)` at load. `a_id`
/// is constant per material and `b_id or 0` is the resolved `other`
/// id (0 = `empty`), so [`Pack::new`] canonicalizes each material's
/// list into exactly that `(other, declared index)` order, and the
/// reaction pass walks it in order, one RNG draw per passing attempt
/// (D4).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reaction {
    /// The rule name from the pack (data, not a keyword).
    pub label: String,
    /// The neighbour material the rule targets; 0 = the implicit
    /// `empty`.
    pub other: u16,
    /// What the reacting (self) cell becomes.
    pub self_becomes: u16,
    /// What the target (`other`) cell becomes after the rule fires;
    /// `None` leaves it unchanged (the loader-notes default).
    pub other_becomes: Option<u16>,
    /// The probability numerator over [`PROBABILITY_DENOMINATOR`],
    /// drawn against the reaction RNG stream (D4).
    pub probability: u32,
    /// Special propagation, if any (schema v1: `explosion` only).
    pub spread: Option<Spread>,
}

/// Special propagation modes for reactions (§6's `spread` key).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Spread {
    /// Gunpowder-style: convert a radius, add a pressure impulse.
    Explosion,
}

/// One compiled material definition.
///
/// Immutably held by [`Pack`]; hosts read these through
/// [`Pack::materials`] / [`Pack::get`]. Reference ids were resolved
/// from names by the loader; `empty` resolves to id 0.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MaterialDef {
    /// The permanent id from SPEC.md appendix B; `1..=Pack::CAP` per
    /// the §6 registry cap.
    pub id: u16,
    /// The pack's reference key (the `[material.<key>]` key), e.g.
    /// `sand` — the namespace the loader resolved names against.
    pub key: String,
    /// The display name from the pack, e.g. `Sand`.
    pub name: String,
    pub category: Category,
    /// Relative ordering only (§2, §6): no real-world unit.
    pub density: u32,
    /// The temperature a fresh cell of this material spawns with
    /// (§4: ambient 295 K for ordinary materials).
    pub default_temp: Temp,
    /// RGB color variants; render selects deterministically by
    /// `(x*7 + y*13) % len` (§6), never from the RNG.
    pub colors: Vec<[u8; 3]>,
    /// The `[lo, hi]` aux a spawned cell draws, if the material
    /// declares `aux_init`.
    pub aux_init: Option<AuxRange>,
    /// The powder a hard-impact island of this material re-enters as
    /// (§7.4); `rigid` materials only, validated at load.
    pub crumble_into: Option<u16>,
    /// The material's transitions, in parse (label-sorted) order.
    pub transitions: Vec<Transition>,
    /// The material's reactions in §6's canonical evaluation order.
    pub reactions: Vec<Reaction>,
}

/// Why a pack definition was rejected at the registry.
///
/// The DSL loader performs the §6 name-level validation with file
/// context; [`Pack::new`] re-derives the registry's own structural
/// guarantees so the core never trusts the loader implicitly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PackProblem {
    /// An explicit definition for id 0 — `empty` is implicit and never
    /// has a file (SPEC.md §6 loader notes).
    IdZero,
    /// An id above the §6 registry cap ([`Pack::CAP`] = 1024).
    IdAboveCap,
    /// Definitions not in strictly ascending id order — the `Vec`
    /// order is the registry order (D3), so anything else is invalid.
    /// Duplicate ids fail this check too.
    UnsortedIds,
    /// A definition claiming [`Category::None`], which is reserved for
    /// the implicit `empty` slot.
    NoneCategory,
    /// An `aux_init` range with `lo > hi`.
    AuxRangeInverted,
    /// A reaction/transition target id that no pack material defines
    /// (§6: unknown names are rejected; the loader has already
    /// resolved names to ids, so the registry re-checks the join).
    DanglingReference,
}

impl fmt::Display for PackProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackProblem::IdZero => write!(
                f,
                "id 0 is the implicit `empty` material and must not occur in a pack"
            ),
            PackProblem::IdAboveCap => {
                write!(f, "id exceeds the registry cap ({} allowed)", Pack::CAP)
            }
            PackProblem::UnsortedIds => {
                write!(f, "material definitions must be strictly ascending by id")
            }
            PackProblem::NoneCategory => {
                write!(f, "Category::None is reserved for the implicit empty slot")
            }
            PackProblem::AuxRangeInverted => write!(f, "aux_init range has lo > hi"),
            PackProblem::DanglingReference => write!(
                f,
                "reaction/transition names a material the pack does not define"
            ),
        }
    }
}

/// The compiled material pack: the registry a world's tick dispatches
/// on (SPEC.md §4: cell storage indexes "into the compiled pack
/// registry").
///
/// Built once via [`Pack::new`] (the DSL loader compiles it from TOML)
/// and immutable afterwards: the registry's content and canonical
/// order are part of world behavior — every pass reads them. The pack
/// content hash (D11) is carried verbatim and is recorded in saves and
/// replays once `silica-formats` lands.
///
/// Determinism contract: materials are stored strictly ascending by id
/// ([`Pack::new`] rejects anything else — D3, "iteration order is part
/// of behavior"), and the category table keeps lookups O(1) without
/// hash maps. Production uses allocate only here, never per tick.
#[derive(Clone, Debug, PartialEq)]
pub struct Pack {
    name: String,
    version: String,
    hash: u64,
    /// Compiled definitions, strictly ascending by id.
    defs: Vec<MaterialDef>,
    /// Dispatch slots: `CAP + 1` entries; [`Category::None`] wherever
    /// no material lives.
    categories: Vec<Category>,
}

impl Pack {
    /// The registry id cap (SPEC.md §6): ids above 1024 are rejected
    /// at load.
    pub const CAP: u16 = 1024;

    /// Compiles a registry from material definitions.
    ///
    /// # Arguments
    ///
    /// * `name`, `version` — the pack.toml identity, copied verbatim;
    /// * `hash` — the D11 pack content hash (in `silica-dsl`: FNV-1a 64
    ///   over the sorted file list with length framing and
    ///   LF-normalized file bytes);
    /// * `defs` — compiled definitions, **strictly ascending by id**
    ///   (the DSL's directory loader delivers that order already).
    ///
    /// # Errors
    ///
    /// Returns [`SilicaError::InvalidPack`] for ids outside `1..=CAP`,
    /// non-ascending (including duplicate) ids, a [`Category::None`]
    /// category, an inverted `aux_init` range, or a reaction/transition
    /// target id no pack material defines; see [`PackProblem`]. The
    /// deeper §6 rules (e.g. `crumble_into` naming a powder, category
    /// legality) are name-level loader validation and are not re-run
    /// here.
    ///
    /// Canonicalizes what the type must guarantee: each material's
    /// reactions are stored in §6's `(other, declaration index)`
    /// evaluation order.
    pub fn new(
        name: &str,
        version: &str,
        hash: u64,
        mut defs: Vec<MaterialDef>,
    ) -> Result<Pack, SilicaError> {
        // Structural validation, one pass in id order.
        let mut previous: Option<u16> = None;
        for def in &defs {
            if def.id == 0 {
                return Err(SilicaError::InvalidPack {
                    id: 0,
                    problem: PackProblem::IdZero,
                });
            }
            if def.id > Self::CAP {
                return Err(SilicaError::InvalidPack {
                    id: def.id,
                    problem: PackProblem::IdAboveCap,
                });
            }
            if let Some(id) = previous
                && def.id <= id
            {
                return Err(SilicaError::InvalidPack {
                    id: def.id,
                    problem: PackProblem::UnsortedIds,
                });
            }
            previous = Some(def.id);
            if def.category == Category::None {
                return Err(SilicaError::InvalidPack {
                    id: def.id,
                    problem: PackProblem::NoneCategory,
                });
            }
            if let Some(range) = def.aux_init
                && range.lo > range.hi
            {
                return Err(SilicaError::InvalidPack {
                    id: def.id,
                    problem: PackProblem::AuxRangeInverted,
                });
            }
        }

        // §6 canonicalization: reactions in `(other, declaration
        // index)` order. Annotate with the declaration index first so
        // the sort key is a total order — deterministic regardless of
        // the unstable sort's behavior.
        for def in &mut defs {
            let reactions = std::mem::take(&mut def.reactions);
            let mut annotated: Vec<(usize, Reaction)> = reactions.into_iter().enumerate().collect();
            annotated.sort_unstable_by_key(|(order, reaction)| (reaction.other, *order));
            def.reactions = annotated
                .into_iter()
                .map(|(_, reaction)| reaction)
                .collect();
        }

        // Defense against dangling targets after name resolution: id 0
        // (`empty`) is always a valid reference target.
        let defined = |id: u16| id == 0 || defs.iter().any(|d| d.id == id);
        for def in &defs {
            for reaction in &def.reactions {
                let core = [reaction.other, reaction.self_becomes];
                let mut targets = core.iter().copied().chain(reaction.other_becomes);
                if targets.any(|target| !defined(target)) {
                    return Err(SilicaError::InvalidPack {
                        id: def.id,
                        problem: PackProblem::DanglingReference,
                    });
                }
            }
            for transition in &def.transitions {
                if !defined(transition.becomes) {
                    return Err(SilicaError::InvalidPack {
                        id: def.id,
                        problem: PackProblem::DanglingReference,
                    });
                }
            }
            if def.crumble_into.is_some_and(|crumble| !defined(crumble)) {
                return Err(SilicaError::InvalidPack {
                    id: def.id,
                    problem: PackProblem::DanglingReference,
                });
            }
        }

        // Dispatch table: one slot per id 0..=CAP.
        let mut categories = vec![Category::None; usize::from(Self::CAP) + 1];
        for def in &defs {
            categories[usize::from(def.id)] = def.category;
        }

        Ok(Pack {
            name: name.to_owned(),
            version: version.to_owned(),
            hash,
            defs,
            categories,
        })
    }

    /// The pack's name (pack.toml), copied verbatim at compile.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The pack's semver string (pack.toml), copied verbatim.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The pack content hash (SPEC.md §8 D11): identifies the pack in
    /// saves and replays. The byte framing lives with the loader
    /// (`silica-dsl`).
    pub fn hash(&self) -> u64 {
        self.hash
    }

    /// Every compiled definition, in canonical ascending-id order.
    /// The implicit `empty` (id 0) is not among them.
    pub fn materials(&self) -> &[MaterialDef] {
        &self.defs
    }

    /// How many materials the pack defines (excluding `empty`).
    pub fn len(&self) -> usize {
        self.defs.len()
    }

    /// Whether the pack defines no materials at all — a manifest-only
    /// pack is valid input: in such a world nothing can ever move.
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// The definition behind an id, or `None` for `empty` (0) and for
    /// ids the pack does not define. O(log n) over the id-sorted
    /// `Vec` (D3).
    pub fn get(&self, id: u16) -> Option<&MaterialDef> {
        if id == 0 {
            return None;
        }
        self.defs
            .binary_search_by_key(&id, |def| def.id)
            .ok()
            .map(|index| &self.defs[index])
    }

    /// The dispatch category of an id.
    ///
    /// [`Category::None`] for the implicit `empty` (id 0) and for any
    /// id this pack does not define — both are never-moves slots.
    /// Real categories otherwise. O(1): a `CAP + 1` entry table built
    /// once at compile (no hash maps, D3).
    pub fn category_of(&self, id: u16) -> Category {
        if id > Self::CAP {
            return Category::None;
        }
        self.categories[usize::from(id)]
    }
}

/// Shared test pack builders used across the core unit tests
/// (cfg-gated: they never compile into production builds). Ids follow
/// SPEC.md appendix B.
#[cfg(test)]
pub(crate) mod test_packs {
    use super::*;

    /// A blank definition with the given identity — the shorthand
    /// every registry test builds from.
    pub(crate) fn def(id: u16, category: Category) -> MaterialDef {
        MaterialDef {
            id,
            key: format!("m{id}"),
            name: format!("M{id}"),
            category,
            density: 1000,
            default_temp: Temp::AMBIENT,
            colors: Vec::new(),
            aux_init: None,
            crumble_into: None,
            transitions: Vec::new(),
            reactions: Vec::new(),
        }
    }

    /// sand · powder · appendix-B id 1.
    pub(crate) fn sand() -> MaterialDef {
        def(1, Category::Powder)
    }

    /// water · liquid · appendix-B id 2.
    pub(crate) fn water() -> MaterialDef {
        def(2, Category::Liquid)
    }

    /// fire · energy · appendix-B id 4: hot at 1200 K, 30..90 aux.
    pub(crate) fn fire() -> MaterialDef {
        let mut fire = def(4, Category::Energy);
        fire.default_temp = Temp::from_kelvin(1200);
        fire.aux_init = Some(AuxRange { lo: 30, hi: 90 });
        fire
    }

    /// sand + water + fire — the "starter" unit-test pack.
    pub(crate) fn starter() -> Pack {
        Pack::new("test-starter", "0.1.0", 0, vec![sand(), water(), fire()]).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A blank definition with the given identity — the shorthand
    /// every registry test builds from.
    fn def(id: u16, category: Category) -> MaterialDef {
        MaterialDef {
            id,
            key: format!("m{id}"),
            name: format!("M{id}"),
            category,
            density: 1000,
            default_temp: Temp::AMBIENT,
            colors: Vec::new(),
            aux_init: None,
            crumble_into: None,
            transitions: Vec::new(),
            reactions: Vec::new(),
        }
    }

    #[test]
    fn cap_is_the_specified_1024() {
        assert_eq!(Pack::CAP, 1024);
    }

    #[test]
    fn builds_and_looks_up_by_id() {
        let pack = Pack::new(
            "t",
            "1",
            7,
            vec![def(1, Category::Powder), def(1024, Category::Solid)],
        )
        .expect("well-formed defs");
        assert_eq!(pack.name(), "t");
        assert_eq!(pack.version(), "1");
        assert_eq!(pack.hash(), 7);
        assert_eq!(pack.len(), 2);
        assert_eq!(pack.get(1).map(|def| def.category), Some(Category::Powder));
        assert_eq!(
            pack.get(1024).map(|def| def.category),
            Some(Category::Solid)
        );
        assert_eq!(pack.get(2), None);
        assert_eq!(pack.get(0), None); // empty is implicit
        assert_eq!(pack.get(9999), None);
        assert_ne!(pack.get(1), Some(&def(7, Category::Powder)));

        assert_eq!(pack.category_of(0), Category::None);
        assert_eq!(pack.category_of(1), Category::Powder);
        assert_eq!(pack.category_of(1024), Category::Solid);
        assert_eq!(pack.category_of(1025), Category::None);
        assert_eq!(pack.category_of(u16::MAX), Category::None);
    }

    #[test]
    fn count_is_1025_slots_fits_the_cap() {
        // The dispatch table must be addressable at the cap itself.
        let pack = Pack::new("t", "1", 0, vec![def(1024, Category::Gas)]).unwrap();
        assert_eq!(pack.category_of(1024), Category::Gas);
    }

    #[test]
    fn rejects_the_implicit_empty_id() {
        assert_eq!(
            Pack::new("t", "1", 0, vec![def(0, Category::Powder)]),
            Err(SilicaError::InvalidPack {
                id: 0,
                problem: PackProblem::IdZero
            })
        );
    }

    #[test]
    fn rejects_ids_above_the_cap() {
        assert_eq!(
            Pack::new("t", "1", 0, vec![def(Pack::CAP + 1, Category::Powder)]),
            Err(SilicaError::InvalidPack {
                id: Pack::CAP + 1,
                problem: PackProblem::IdAboveCap
            })
        );
    }

    #[test]
    fn rejects_non_ascending_including_duplicate_ids() {
        assert_eq!(
            Pack::new(
                "t",
                "1",
                0,
                vec![def(2, Category::Powder), def(1, Category::Powder)]
            ),
            Err(SilicaError::InvalidPack {
                id: 1,
                problem: PackProblem::UnsortedIds
            })
        );
        assert_eq!(
            Pack::new(
                "t",
                "1",
                0,
                vec![def(1, Category::Powder), def(1, Category::Powder)]
            ),
            Err(SilicaError::InvalidPack {
                id: 1,
                problem: PackProblem::UnsortedIds
            })
        );
    }

    #[test]
    fn rejects_the_reserved_none_category() {
        assert_eq!(
            Pack::new("t", "1", 0, vec![def(1, Category::None)]),
            Err(SilicaError::InvalidPack {
                id: 1,
                problem: PackProblem::NoneCategory
            })
        );
    }

    #[test]
    fn rejects_inverted_aux_ranges() {
        let mut first = def(1, Category::Energy);
        first.aux_init = Some(AuxRange { lo: 90, hi: 30 });
        assert_eq!(
            Pack::new("t", "1", 0, vec![first]),
            Err(SilicaError::InvalidPack {
                id: 1,
                problem: PackProblem::AuxRangeInverted
            })
        );
        // A valid range compiles; so does a degenerate one-point range.
        let mut second = def(1, Category::Energy);
        second.aux_init = Some(AuxRange { lo: 30, hi: 90 });
        Pack::new("t", "1", 0, vec![second]).unwrap();
    }

    #[test]
    fn accepts_empty_packs() {
        let pack = Pack::new("t", "1", 7, Vec::new()).expect("manifest-only pack");
        assert!(pack.is_empty());
        assert_eq!(pack.len(), 0);
        assert_eq!(pack.hash(), 7);
    }

    #[test]
    fn reactions_store_evaluation_order_from_6() {
        // Declared: other 5, then two more targetting `2`. The §6
        // evaluation order is `(other, declaration index)`:
        // [2-then-1] first [2-then-2], then [5-then-0].
        let dummy2 = def(2, Category::Powder);
        let dummy5 = def(5, Category::Powder);
        let mut def = def(1, Category::Powder);
        def.reactions = vec![
            Reaction {
                label: "a".into(),
                other: 5,
                self_becomes: 1,
                other_becomes: None,
                probability: 1,
                spread: None,
            },
            Reaction {
                label: "b".into(),
                other: 2,
                self_becomes: 1,
                other_becomes: None,
                probability: 1,
                spread: None,
            },
            Reaction {
                label: "c".into(),
                other: 2,
                self_becomes: 1,
                other_becomes: None,
                probability: 1,
                spread: None,
            },
        ];
        // Target ids 2 and 5 must exist in the pack; they are inert
        // here and only make the references resolve (§6 join).
        let pack = Pack::new("t", "1", 0, vec![def, dummy2, dummy5]).unwrap();
        let labels: Vec<&str> = pack
            .get(1)
            .unwrap()
            .reactions
            .iter()
            .map(|r| r.label.as_str())
            .collect();
        assert_eq!(labels, ["b", "c", "a"]);
    }

    #[test]
    fn rejects_dangling_reaction_and_transition_targets() {
        // References resolve to ids at load; the registry re-checks
        // the join so a mis-compiled def cannot slip through.
        let mut reaction_host = def(1, Category::Powder);
        reaction_host.reactions.push(Reaction {
            label: "x".into(),
            other: 0,
            self_becomes: 1,
            other_becomes: Some(9),
            probability: 1,
            spread: None,
        });
        assert_eq!(
            Pack::new("t", "1", 0, vec![reaction_host]),
            Err(SilicaError::InvalidPack {
                id: 1,
                problem: PackProblem::DanglingReference
            })
        );

        let mut transition_host = def(3, Category::Rigid);
        transition_host.transitions.push(Transition {
            label: "melt".into(),
            trigger: Trigger::AtTemp { units: 1 },
            becomes: 9,
        });
        assert_eq!(
            Pack::new("t", "1", 0, vec![transition_host]),
            Err(SilicaError::InvalidPack {
                id: 3,
                problem: PackProblem::DanglingReference
            })
        );

        let mut crumble_host = def(3, Category::Rigid);
        crumble_host.crumble_into = Some(9);
        assert_eq!(
            Pack::new("t", "1", 0, vec![crumble_host]),
            Err(SilicaError::InvalidPack {
                id: 3,
                problem: PackProblem::DanglingReference
            })
        );
    }

    #[test]
    fn zero_is_always_a_valid_reaction_target() {
        let mut def = def(1, Category::Powder);
        def.reactions.push(Reaction {
            label: "smoke".into(),
            other: 0,
            self_becomes: 0,
            other_becomes: None,
            probability: 1,
            spread: None,
        });
        Pack::new("t", "1", 0, vec![def]).unwrap();
    }
}
