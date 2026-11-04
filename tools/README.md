# tools/ — Silica CLI (SPEC.md §10/§11)

Build with `cargo build -p silica-tools`. Binary: `silica`.

## `silica render`

Step a canonical world and write M-spaced PNG frames.

```
silica render --world <column_collapse|lake_drain|forest_fire|box_burn> \
              --ticks N --every M --out DIR [--stats]
```

- `--world` — which scene (default `column_collapse`).
- `--ticks` — total ticks to simulate (default 600).
- `--every` — write a frame every N ticks (default 10).
- `--out` — output directory (created if missing).
- `--stats` — print wall-clock averages and the §5 budget table (diagnostics only).

### box_burn (the 0.1 artifact scene)

```
silica render --world box_burn --ticks 600 --every 10 --out frames/box_burn --stats
```

Assemble the GIF externally (ROADMAP: "assemble the GIF externally"):

```
ffmpeg -framerate 6 -pattern_type glob -i 'frames/box_burn/frame_*.png' \
       -vf "scale=640:-1:flags=neighbor" -loop 0 box_burn.gif
```

(6 fps = 10 ticks/frame at the engine's 60 fps; adjust `-framerate` to taste.)

## Empty-cell background color

`World::render_rgba` paints `empty` (id 0) with a dark slate
`(24, 24, 28, 255)` — drafted here because SPEC §6 is silent on the
background color (flagged for review → D-016 draft).
