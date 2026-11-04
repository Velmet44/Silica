//! `silica` — the 0.1 tooling CLI (SPEC.md §10/§11).
//!
//! Subcommands:
//! * `render` — step a canonical world and write M-spaced PNG frames.

use std::fs;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use image::{ImageBuffer, Rgba};
use silica_core::{Brush, World, WorldConfig};

#[derive(Parser, Debug)]
#[command(name = "silica", version, about = "Silica engine tooling")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Step a canonical world and write M-spaced PNG frames.
    Render(RenderArgs),
}

#[derive(Parser, Debug)]
struct RenderArgs {
    /// Which canonical world to render.
    #[arg(long, value_enum, default_value = "column_collapse")]
    world: WorldKind,
    /// Total ticks to simulate.
    #[arg(long, default_value_t = 600)]
    ticks: u32,
    /// Write a frame every N ticks.
    #[arg(long, default_value_t = 10)]
    every: u32,
    /// Output directory for the PNG frames.
    #[arg(long, default_value = "out")]
    out: PathBuf,
    /// Print per-pass wall-clock averages (diagnostics only).
    #[arg(long)]
    stats: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
#[value(rename_all = "snake_case")]
enum WorldKind {
    ColumnCollapse,
    LakeDrain,
    ForestFire,
    BoxBurn,
}

impl WorldKind {
    fn name(self) -> &'static str {
        match self {
            WorldKind::ColumnCollapse => "column_collapse",
            WorldKind::LakeDrain => "lake_drain",
            WorldKind::ForestFire => "forest_fire",
            WorldKind::BoxBurn => "box_burn",
        }
    }
}

fn starter_pack() -> silica_core::Pack {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("materials");
    silica_dsl::load_pack(&dir).expect("starter pack must load")
}

fn build_world(kind: WorldKind) -> (World, Vec<(u32, Vec<Brush>)>) {
    let pack = starter_pack();
    let (w, h) = (64u16, 48u16);
    let mut world = World::new(WorldConfig {
        width: w,
        height: h,
        seed: 0,
        walls: true,
        pack: &pack,
    });
    let mut scheduled: Vec<(u32, Vec<Brush>)> = Vec::new();
    match kind {
        WorldKind::ColumnCollapse => {
            world
                .paint(&Brush::rect(29, 4, 33, 23, 1))
                .expect("sand is defined");
        }
        WorldKind::LakeDrain => {
            world
                .paint(&Brush::rect(8, 24, 55, 32, 2))
                .expect("water is defined");
            world
                .paint(&Brush::rect(8, 16, 55, 23, 3))
                .expect("oil is defined");
        }
        WorldKind::ForestFire => {
            world
                .paint(&Brush::rect(20, 20, 44, 20, 7))
                .expect("wood is defined");
            world
                .paint(&Brush::rect(20, 35, 44, 35, 7))
                .expect("wood is defined");
            world
                .paint(&Brush::rect(20, 21, 20, 34, 7))
                .expect("wood is defined");
            world
                .paint(&Brush::rect(44, 21, 44, 34, 7))
                .expect("wood is defined");
            world
                .paint(&Brush::rect(32, 20, 32, 20, 4))
                .expect("fire is defined");
        }
        WorldKind::BoxBurn => {
            // Wooden box.
            world
                .paint(&Brush::rect(20, 20, 44, 20, 7))
                .expect("wood is defined");
            world
                .paint(&Brush::rect(20, 35, 44, 35, 7))
                .expect("wood is defined");
            world
                .paint(&Brush::rect(20, 21, 20, 34, 7))
                .expect("wood is defined");
            world
                .paint(&Brush::rect(44, 21, 44, 34, 7))
                .expect("wood is defined");
            // Sand dam on the left.
            world
                .paint(&Brush::rect(18, 33, 22, 35, 1))
                .expect("sand is defined");
            // Fire inside.
            world
                .paint(&Brush::rect(32, 27, 32, 27, 4))
                .expect("fire is defined");
            // Scheduled water pour at tick 50.
            scheduled.push((50, vec![Brush::rect(24, 18, 28, 18, 2)]));
        }
    }
    (world, scheduled)
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Render(args) => run_render(args),
    }
}

fn run_render(args: RenderArgs) {
    let (mut world, scheduled) = build_world(args.world);
    fs::create_dir_all(&args.out).expect("create output dir");

    let total = args.ticks;
    let every = args.every.max(1);
    let loop_start = std::time::Instant::now();
    let mut frame_idx = 0u32;

    // Frame at tick 0.
    write_frame(&world, &args.out, frame_idx);
    frame_idx += 1;

    let mut scheduled = scheduled;
    for tick in 1..=total {
        // Apply scheduled inputs for this tick.
        let mut inputs: Vec<Brush> = Vec::new();
        scheduled.retain(|(t, brushes)| {
            if *t == tick {
                inputs.extend(brushes.clone());
                false
            } else {
                true
            }
        });
        if let Err(e) = world.step(&inputs) {
            eprintln!("tick {tick}: step failed: {e}");
            break;
        }
        if tick % every == 0 {
            write_frame(&world, &args.out, frame_idx);
            frame_idx += 1;
        }
    }
    let elapsed = loop_start.elapsed();

    if args.stats {
        let ms_per_tick = elapsed.as_secs_f64() * 1000.0 / total.max(1) as f64;
        eprintln!(
            "rendered {} frames for {} ({} ticks, every {})",
            frame_idx,
            args.world.name(),
            total,
            every
        );
        eprintln!(
            "wall clock: {:.2} ms total, {:.3} ms/tick avg",
            elapsed.as_secs_f64() * 1000.0,
            ms_per_tick
        );
        eprintln!("§5 budget (native, 512x384, 60% active): total <= 13 ms/tick");
        eprintln!(
            "  apply_inputs 0.5 | rigid 1.5 | movement 5.0 | reaction 2.5 | heat 2.0 | pressure 1.0 | end 0.5"
        );
    }
}

fn write_frame(world: &World, out: &Path, idx: u32) {
    let w = world.width() as u32;
    let h = world.height() as u32;
    let mut buf = vec![0u8; (w * h * 4) as usize];
    world.render_rgba(&mut buf);
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(w, h, buf).expect("buffer size matches dimensions");
    let path = out.join(format!("frame_{idx:04}.png"));
    img.save(&path).expect("write png");
}
