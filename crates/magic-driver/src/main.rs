mod cli;
mod config_file;

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command, ConfigCommand};
use crate::config_file::{ConfigLocation, LoadOutcome, default_config_path, format_diagnostic};

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Some(path) = cli.test_config {
        return config_test(path);
    }
    match cli.command {
        Some(Command::Config {
            command: ConfigCommand::Test { path },
        }) => config_test(path),
        Some(Command::Config {
            command: ConfigCommand::Defaults,
        }) => config_defaults(),
        Some(command) => not_implemented(command.name()),
        None => ExitCode::FAILURE,
    }
}

fn not_implemented(name: &str) -> ExitCode {
    eprintln!("magic-driver {name}: not implemented");
    ExitCode::FAILURE
}

fn config_defaults() -> ExitCode {
    print!("{}", md_core::config::Config::default().render());
    ExitCode::SUCCESS
}

fn config_test(path: Option<PathBuf>) -> ExitCode {
    let location = match path {
        Some(path) => ConfigLocation::Explicit(path),
        None => {
            if let Some(path) =
                default_config_path(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME"))
            {
                ConfigLocation::Default(path)
            } else {
                eprintln!(
                    "error: cannot locate the config file: neither XDG_CONFIG_HOME nor HOME is set"
                );
                return ExitCode::FAILURE;
            }
        }
    };
    let path = location.path();
    match config_file::load(&location) {
        LoadOutcome::Loaded(parsed) => {
            for warning in &parsed.warnings {
                eprintln!("{}", format_diagnostic("warning", path, warning));
            }
            print!("{}", parsed.config.render());
            eprintln!("magic-driver: config {} test is successful", path.display());
            ExitCode::SUCCESS
        }
        LoadOutcome::MissingUsingDefaults(config) => {
            print!("{}", config.render());
            eprintln!(
                "magic-driver: config {} does not exist, using defaults",
                path.display()
            );
            ExitCode::SUCCESS
        }
        LoadOutcome::Invalid { errors, warnings } => {
            for warning in &warnings {
                eprintln!("{}", format_diagnostic("warning", path, warning));
            }
            for error in &errors {
                eprintln!("{}", format_diagnostic("error", path, error));
            }
            eprintln!("magic-driver: config {} test failed", path.display());
            ExitCode::FAILURE
        }
        LoadOutcome::Unreadable(reason) => {
            eprintln!("error: {}: {reason}", path.display());
            eprintln!("magic-driver: config {} test failed", path.display());
            ExitCode::FAILURE
        }
    }
}
