// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Thin entry point: parse arguments, then either open a window or run the
//! headless replay driver.  Everything else lives in the library, so `tests/`
//! can reach it.

use clap::Parser;
use eframe::egui;

use orion_sdr_view::app::{DECODE_BAR_H, ViewApp};
use orion_sdr_view::config::ViewConfig;
use orion_sdr_view::replay;

#[derive(Parser)]
#[command(name = "orion-sdr-view", about = "SDR spectrum viewer")]
struct Cli {
    /// Path to a YAML configuration file
    #[arg(long, value_name = "FILE")]
    config: Option<std::path::PathBuf>,

    /// Run with no window, renderer or GPU, driven by --script and/or --duration
    #[arg(long)]
    headless: bool,

    /// Timed key script to replay.  `assert` directives are parsed and ignored
    #[arg(long, value_name = "FILE")]
    script: Option<std::path::PathBuf>,

    /// Write the measurement stream to FILE as JSON Lines; overrides the
    /// script's own `dump`
    ///
    /// Deliberately does *not* imply --headless: dumping from an interactive
    /// session is a reasonable future want, but it would not be reproducible and
    /// should not be silently conflated with a run that is.
    #[arg(long, value_name = "FILE")]
    dump: Option<std::path::PathBuf>,

    /// Bound a headless run to this many scripted seconds; overrides the
    /// script's own `duration`
    #[arg(long, value_name = "SECS")]
    duration: Option<f32>,
}

fn main() -> eframe::Result<()> {
    let cli = Cli::parse();
    let cfg = ViewConfig::load(cli.config.clone());

    if cli.headless {
        return run_headless(cfg, &cli);
    }
    if cli.script.is_some() || cli.dump.is_some() || cli.duration.is_some() {
        eprintln!("orion-sdr-view: --script, --dump and --duration require --headless");
        std::process::exit(2);
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("orion-sdr-view")
            .with_inner_size([1200.0, 800.0 + DECODE_BAR_H]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "orion-sdr-view",
        options,
        Box::new(|cc| Ok(Box::new(ViewApp::new(&cc.egui_ctx, cfg)))),
    )
}

/// A headless run **fails loudly** — a non-zero exit and a message on stderr for
/// an unparsable script, an unwritable dump or a dropped decode chunk.  Nobody
/// is watching it, so a quiet failure would look exactly like a clean run.
fn run_headless(cfg: ViewConfig, cli: &Cli) -> eframe::Result<()> {
    match replay::run_file(
        cfg,
        cli.script.as_deref(),
        cli.dump.as_deref(),
        cli.duration,
    ) {
        Ok(summary) => {
            eprintln!(
                "orion-sdr-view: {} frames, {} samples, {} records",
                summary.frames, summary.samples, summary.records
            );
            Ok(())
        }
        Err(e) => {
            eprintln!("orion-sdr-view: {e}");
            std::process::exit(1);
        }
    }
}
