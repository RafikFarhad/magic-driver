use std::ffi::OsStr;
use std::path::PathBuf;

use clap::Parser;

use crate::live::socket_path;

#[derive(Debug, Clone, PartialEq, Eq, Parser)]
#[command(
    name = "magic-driver-inspector",
    version,
    about = "Read-only diagnostic window for magic-driver"
)]
pub struct Args {
    #[arg(long, value_name = "FILE", help = "Inspect a recording offline")]
    pub replay: Option<PathBuf>,
    #[arg(
        long,
        value_name = "FILE",
        requires = "replay",
        help = "Config file for the offline engine (default: the user config, else defaults)"
    )]
    pub config: Option<PathBuf>,
    #[arg(
        long,
        value_name = "PATH",
        conflicts_with = "replay",
        help = "Trace socket of the running daemon (default: $XDG_RUNTIME_DIR/magic-driver/trace.sock)"
    )]
    pub socket: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Replay {
        recording: PathBuf,
        config: Option<PathBuf>,
    },
    Live {
        socket: PathBuf,
    },
}

impl Args {
    pub fn source(&self, runtime_dir: Option<&OsStr>) -> Result<Source, String> {
        if let Some(recording) = &self.replay {
            return Ok(Source::Replay {
                recording: recording.clone(),
                config: self.config.clone(),
            });
        }
        let socket = match &self.socket {
            Some(socket) => socket.clone(),
            None => socket_path(runtime_dir).ok_or_else(|| {
                "cannot locate the trace socket: XDG_RUNTIME_DIR is not set; use --socket or --replay"
                    .to_owned()
            })?,
        };
        Ok(Source::Live { socket })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, clap::Error> {
        Args::try_parse_from(std::iter::once("magic-driver-inspector").chain(args.iter().copied()))
    }

    #[test]
    fn cli_definition_is_consistent() {
        use clap::CommandFactory;
        Args::command().debug_assert();
    }

    #[test]
    fn no_arguments_selects_live_mode_on_the_runtime_socket() {
        let args = parse(&[]).unwrap();
        assert_eq!(
            args.source(Some(OsStr::new("/run/user/1000"))),
            Ok(Source::Live {
                socket: PathBuf::from("/run/user/1000/magic-driver/trace.sock")
            })
        );
        assert!(args.source(None).is_err());
        assert!(args.source(Some(OsStr::new("relative"))).is_err());
    }

    #[test]
    fn replay_selects_offline_mode_with_an_optional_config() {
        let args = parse(&["--replay", "in.rec"]).unwrap();
        assert_eq!(
            args.source(None),
            Ok(Source::Replay {
                recording: PathBuf::from("in.rec"),
                config: None
            })
        );
        let args = parse(&["--replay", "in.rec", "--config", "c.toml"]).unwrap();
        assert_eq!(
            args.source(None),
            Ok(Source::Replay {
                recording: PathBuf::from("in.rec"),
                config: Some(PathBuf::from("c.toml"))
            })
        );
    }

    #[test]
    fn socket_overrides_the_runtime_dir_and_conflicts_with_replay() {
        let args = parse(&["--socket", "/tmp/x/trace.sock"]).unwrap();
        assert_eq!(
            args.source(None),
            Ok(Source::Live {
                socket: PathBuf::from("/tmp/x/trace.sock")
            })
        );
        assert!(parse(&["--socket", "/tmp/s", "--replay", "in.rec"]).is_err());
        assert!(parse(&["--config", "c.toml"]).is_err());
    }
}
