use std::env;
use std::process::ExitCode;

use clap::Parser;
use eframe::egui;
use magic_driver_inspector::app::InspectorApp;
use magic_driver_inspector::args::{Args, Source};
use magic_driver_inspector::replay::{OfflineSource, default_config_path};

fn main() -> ExitCode {
    let args = Args::parse();
    let source = match args.source(env::var_os("XDG_RUNTIME_DIR").as_deref()) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    let result = match source {
        Source::Replay { recording, config } => {
            let default_config =
                default_config_path(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME"));
            let offline = match OfflineSource::open(&recording, config.as_deref(), default_config) {
                Ok(offline) => offline,
                Err(error) => {
                    eprintln!("error: {error}");
                    return ExitCode::FAILURE;
                }
            };
            for warning in &offline.chosen.warnings {
                eprintln!("warning: {warning}");
            }
            let title = format!("magic-driver inspector: {}", recording.display());
            open_window(
                &title,
                Box::new(move |cc| InspectorApp::offline(cc, offline)),
            )
        }
        Source::Live { socket } => {
            let title = format!("magic-driver inspector: live {}", socket.display());
            open_window(&title, Box::new(move |cc| InspectorApp::live(cc, socket)))
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: cannot open the inspector window: {error}");
            ExitCode::FAILURE
        }
    }
}

type Builder = Box<dyn FnOnce(&eframe::CreationContext<'_>) -> InspectorApp>;

fn open_window(title: &str, build: Builder) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_app_id("magic-driver-inspector")
            .with_inner_size([1400.0, 900.0]),
        ..Default::default()
    };
    eframe::run_native(
        "magic-driver-inspector",
        options,
        Box::new(move |cc| Ok(Box::new(build(cc)))),
    )
}
