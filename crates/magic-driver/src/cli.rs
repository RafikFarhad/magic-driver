use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use md_core::Mode;

#[derive(Debug, Parser)]
#[command(
    name = "magic-driver",
    version,
    about = "User-space driver for the Apple Magic Mouse 2",
    args_conflicts_with_subcommands = true,
    arg_required_else_help = true
)]
pub struct Cli {
    #[arg(
        short = 't',
        value_name = "PATH",
        num_args = 0..=1,
        help = "Test the configuration file, same as `config test`"
    )]
    #[allow(clippy::option_option)]
    pub test_config: Option<Option<PathBuf>>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(about = "Run the driver (service entry point)")]
    Run {
        #[arg(long, help = "Enable the decision trace")]
        debug: bool,
        #[arg(
            long,
            value_enum,
            help = "Diagnostic touch output mode instead of the scroll path"
        )]
        mode: Option<RunMode>,
    },
    #[command(about = "Print detected mice and session state")]
    Status,
    #[command(about = "Check the installation and environment")]
    Doctor,
    #[command(about = "Record raw reports to a file")]
    Record { file: PathBuf },
    #[command(about = "Feed a recording through the engine")]
    Replay {
        file: PathBuf,
        #[arg(long, help = "Drive real virtual devices")]
        emit: bool,
        #[arg(long, help = "Print outputs as JSON lines")]
        json: bool,
    },
    #[command(about = "Validate or print the configuration")]
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    #[command(about = "Print the live decision trace as JSON lines")]
    Trace,
    #[command(about = "Launch the optional diagnostic inspector")]
    Inspect {
        #[arg(long, value_name = "FILE", help = "Inspect a recording offline")]
        replay: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RunMode {
    RawForward,
    ScrollProbe,
}

impl RunMode {
    pub fn engine_mode(mode: Option<RunMode>) -> Mode {
        match mode {
            None => Mode::Normal,
            Some(RunMode::RawForward) => Mode::RawForward,
            Some(RunMode::ScrollProbe) => Mode::ScrollProbe,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    #[command(about = "Validate a config file and print the effective values")]
    Test { path: Option<PathBuf> },
    #[command(about = "Print a complete config file with all defaults")]
    Defaults,
}

impl Command {
    pub fn name(&self) -> &'static str {
        match self {
            Command::Run { .. } => "run",
            Command::Status => "status",
            Command::Doctor => "doctor",
            Command::Record { .. } => "record",
            Command::Replay { .. } => "replay",
            Command::Config { .. } => "config",
            Command::Trace => "trace",
            Command::Inspect { .. } => "inspect",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("magic-driver").chain(args.iter().copied()))
    }

    #[test]
    fn cli_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn short_test_flag_accepts_optional_path() {
        assert_eq!(parse(&["-t"]).unwrap().test_config, Some(None));
        assert_eq!(
            parse(&["-t", "/tmp/c.toml"]).unwrap().test_config,
            Some(Some(PathBuf::from("/tmp/c.toml")))
        );
    }

    #[test]
    fn short_test_flag_conflicts_with_subcommands() {
        assert!(parse(&["-t", "status"]).is_ok_and(|cli| cli.command.is_none()));
        assert!(parse(&["status", "-t"]).is_err());
    }

    #[test]
    fn all_architecture_subcommands_parse() {
        let cases: [&[&str]; 16] = [
            &["run"],
            &["run", "--debug"],
            &["run", "--mode", "raw-forward"],
            &["run", "--mode", "scroll-probe"],
            &["run", "--debug", "--mode", "raw-forward"],
            &["status"],
            &["doctor"],
            &["record", "out.rec"],
            &["replay", "in.rec"],
            &["replay", "in.rec", "--emit"],
            &["replay", "in.rec", "--json"],
            &["replay", "--json", "in.rec"],
            &["config", "test"],
            &["config", "defaults"],
            &["trace"],
            &["inspect", "--replay", "in.rec"],
        ];
        for args in cases {
            let cli = parse(args).unwrap_or_else(|error| panic!("{args:?}: {error}"));
            assert_eq!(cli.command.expect("command").name(), args[0]);
        }
    }

    #[test]
    fn config_test_takes_optional_path() {
        let cli = parse(&["config", "test", "x.toml"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Config {
                command: ConfigCommand::Test { path: Some(_) }
            })
        ));
    }

    #[test]
    fn replay_takes_a_file_and_flags() {
        let cli = parse(&["replay", "x.rec", "--json"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Replay {
                emit: false,
                json: true,
                ..
            })
        ));
        assert!(parse(&["replay"]).is_err());
    }

    #[test]
    fn run_mode_selects_the_engine_mode() {
        let mode = |args: &[&str]| match parse(args).unwrap().command {
            Some(Command::Run { mode, .. }) => RunMode::engine_mode(mode),
            other => panic!("{other:?}"),
        };
        assert_eq!(mode(&["run"]), Mode::Normal);
        assert_eq!(mode(&["run", "--debug"]), Mode::Normal);
        assert_eq!(mode(&["run", "--mode", "raw-forward"]), Mode::RawForward);
        assert_eq!(mode(&["run", "--mode", "scroll-probe"]), Mode::ScrollProbe);
        assert!(parse(&["run", "--mode", "pointer-only"]).is_err());
        assert!(parse(&["run", "--mode", "normal"]).is_err());
    }

    #[test]
    fn no_arguments_is_an_error() {
        assert!(parse(&[]).is_err());
    }
}
