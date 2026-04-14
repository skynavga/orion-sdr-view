// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod app;
mod config;
mod decode;
mod source;
#[allow(unused)]
mod utils;

use clap::Parser;
use eframe::egui;

use app::{DECODE_BAR_H, ViewApp};
use config::ViewConfig;

#[derive(Parser)]
#[command(name = "orion-sdr-view", about = "SDR spectrum viewer")]
struct Cli {
    /// Path to a YAML configuration file
    #[arg(long, value_name = "FILE")]
    config: Option<std::path::PathBuf>,
}

fn main() -> eframe::Result<()> {
    let cli = Cli::parse();
    let cfg = ViewConfig::load(cli.config);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("orion-sdr-view")
            .with_inner_size([1200.0, 800.0 + DECODE_BAR_H]),
        ..Default::default()
    };
    eframe::run_native(
        "orion-sdr-view",
        options,
        Box::new(|cc| Ok(Box::new(ViewApp::new(cc, cfg)))),
    )
}
