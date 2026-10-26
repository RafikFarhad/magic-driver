use std::ffi::OsString;
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use md_core::config::{Config, Diagnostic, ParsedConfig};
use md_core::profile::MAGIC_MOUSE_2_BT;

const CONFIG_RELATIVE_PATH: &str = "magic-driver/config.toml";

#[derive(Debug, PartialEq, Eq)]
pub enum ConfigLocation {
    Default(PathBuf),
    Explicit(PathBuf),
}

impl ConfigLocation {
    pub fn path(&self) -> &Path {
        match self {
            ConfigLocation::Default(path) | ConfigLocation::Explicit(path) => path,
        }
    }
}

pub fn default_config_path(
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    let xdg = xdg_config_home
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    let base = match xdg {
        Some(path) => path,
        None => PathBuf::from(home.filter(|home| !home.is_empty())?).join(".config"),
    };
    Some(base.join(CONFIG_RELATIVE_PATH))
}

#[derive(Debug)]
pub enum LoadOutcome {
    Loaded(ParsedConfig),
    MissingUsingDefaults(Config),
    Invalid {
        errors: Vec<Diagnostic>,
        warnings: Vec<Diagnostic>,
    },
    Unreadable(String),
}

pub fn load(location: &ConfigLocation) -> LoadOutcome {
    let path = location.path();
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return match location {
                ConfigLocation::Default(_) => LoadOutcome::MissingUsingDefaults(Config::default()),
                ConfigLocation::Explicit(_) => {
                    LoadOutcome::Unreadable("file does not exist".to_owned())
                }
            };
        }
        Err(error) => return LoadOutcome::Unreadable(error.to_string()),
    };
    if !metadata.is_file() {
        return LoadOutcome::Unreadable("not a regular file".to_owned());
    }
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => return LoadOutcome::Unreadable(error.to_string()),
    };
    match Config::parse(&source, &MAGIC_MOUSE_2_BT) {
        Ok(parsed) => LoadOutcome::Loaded(parsed),
        Err(errors) => LoadOutcome::Invalid {
            errors: errors.errors,
            warnings: errors.warnings,
        },
    }
}

pub fn format_diagnostic(severity: &str, path: &Path, diagnostic: &Diagnostic) -> String {
    let mut text = format!("{severity}: {}", path.display());
    if let Some(line) = diagnostic.line {
        write!(text, ":{line}").expect("writing to a String cannot fail");
    }
    text.push_str(": ");
    if let Some(key) = &diagnostic.key {
        write!(text, "{key}: ").expect("writing to a String cannot fail");
    }
    write!(text, "{}", diagnostic.kind).expect("writing to a String cannot fail");
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use md_core::config::DiagnosticKind;

    #[allow(clippy::unnecessary_wraps)]
    fn os(value: &str) -> Option<OsString> {
        Some(OsString::from(value))
    }

    #[test]
    fn xdg_config_home_takes_precedence() {
        assert_eq!(
            default_config_path(os("/xdg"), os("/home/u")),
            Some(PathBuf::from("/xdg/magic-driver/config.toml"))
        );
    }

    #[test]
    fn home_is_used_without_xdg_config_home() {
        assert_eq!(
            default_config_path(None, os("/home/u")),
            Some(PathBuf::from("/home/u/.config/magic-driver/config.toml"))
        );
    }

    #[test]
    fn relative_or_empty_xdg_config_home_is_ignored() {
        assert_eq!(
            default_config_path(os("relative/dir"), os("/home/u")),
            Some(PathBuf::from("/home/u/.config/magic-driver/config.toml"))
        );
        assert_eq!(
            default_config_path(os(""), os("/home/u")),
            Some(PathBuf::from("/home/u/.config/magic-driver/config.toml"))
        );
    }

    #[test]
    fn no_home_and_no_xdg_gives_no_path() {
        assert_eq!(default_config_path(None, None), None);
        assert_eq!(default_config_path(None, os("")), None);
    }

    #[test]
    fn diagnostics_format_with_path_line_and_key() {
        let diagnostic = Diagnostic {
            line: Some(4),
            key: Some("scroll.enabled".to_owned()),
            kind: DiagnosticKind::InvalidValue("expected a boolean".to_owned()),
        };
        assert_eq!(
            format_diagnostic("error", Path::new("/c.toml"), &diagnostic),
            "error: /c.toml:4: scroll.enabled: invalid value: expected a boolean"
        );
        let bare = Diagnostic {
            line: None,
            key: None,
            kind: DiagnosticKind::Syntax("bad".to_owned()),
        };
        assert_eq!(
            format_diagnostic("error", Path::new("/c.toml"), &bare),
            "error: /c.toml: invalid TOML: bad"
        );
    }
}
