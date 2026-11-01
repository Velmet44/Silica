# materials/ — starter pack (milestone 0.1)

This directory is the seed of the default pack that the spec package ships so
the first opencode session has real, loadable data files on day one.

**Contents:** `pack.toml` (manifest) + 8 materials — sand, water, oil, fire,
smoke, steam, wood, stone.

## Schema

The authority is `SPEC.md` §6 plus its *Loader notes* block. Summary:

- One file per material, `schema_version = 1` at the top of each file.
- `empty` (id 0) is implicit — never a file.
- **Ids are permanent** (SPEC appendix B). This subset uses 1, 2, 3, 4, 5, 6,
  7 and 12; everything it does not use stays free for the 0.3 pack.
- Transitions: `at_temp` (fires when temp ≥), `below_temp` (temp <),
  `aux_exhausted` (cell aux reached 0).
- Reactions: `name`, `other`, `self_becomes` required; `other_becomes`
  optional (default: other cell unchanged); `probability` optional (default
  1.0, rational with denominator 1,000,000); `spread` optional.
- `aux_init = [lo, hi]` sets the category's default aux (fire life) at spawn.
- `crumble_into` (rigid only) names the powder a hard-impact island
  re-enters as (SPEC §7.4).

## One documented deviation from ROADMAP 0.1

ROADMAP 0.1 lists 8 materials; this pack ships **steam** as the ninth because
water's `douse` reaction targets `steam`, and load validation rejects
references to materials that are not in the pack (SPEC §6). Dropping steam
instead would force `douse` to produce `empty`, which makes the milestone
GIF visibly worse.

## Adding a material

1. Add `materials/<name>.toml` with an unused id from SPEC appendix B.
2. Run loader validation (`cargo test -p silica-dsl` once 0.1 exists).
3. Bump `version` in `pack.toml`. The pack hash (FNV-1a 64 over the sorted
   file list + bytes, `u32 LE` name/content lengths, LF-normalized content)
   appears in saves and replays; the canonical *state* hash does not change
   unless the behavior of an already-present material changes — so additive
   materials never silently invalidate goldens (AGENTS.md rule 7 still
   applies to any behavioral change). Keep the files LF (`.gitattributes`
   pins it): the loader normalizes CRLF→LF before hashing, so an
   `core.autocrlf` checkout cannot fork the identity. The shipped starter
   pack identifies as `0xc8ce7d9f920fd109`.
