#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

use md_core::config::{Config, Diagnostic};

use crate::config_file::{self, ConfigLocation, LoadOutcome, format_diagnostic};

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigSource {
    path: PathBuf,
    logged_warnings: Option<Vec<Diagnostic>>,
}

pub fn initial_config(outcome: &LoadOutcome) -> Config {
    match outcome {
        LoadOutcome::Loaded(parsed) => parsed.config.clone(),
        LoadOutcome::MissingUsingDefaults(config) => config.clone(),
        LoadOutcome::Invalid { .. } | LoadOutcome::Unreadable(_) => Config::default(),
    }
}

impl ConfigSource {
    pub fn new(path: PathBuf) -> ConfigSource {
        ConfigSource {
            path,
            logged_warnings: None,
        }
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> LoadOutcome {
        config_file::load(&ConfigLocation::Default(self.path.clone()))
    }

    pub fn report_initial(&mut self, outcome: &LoadOutcome) {
        let path = self.path.display().to_string();
        match outcome {
            LoadOutcome::Loaded(parsed) => {
                self.log_warnings(&parsed.warnings);
                tracing::info!("config loaded from {path}");
            }
            LoadOutcome::MissingUsingDefaults(_) => {
                self.log_warnings(&[]);
                tracing::info!("config {path} does not exist, using defaults");
            }
            LoadOutcome::Invalid { errors, warnings } => {
                self.log_warnings(warnings);
                self.log_errors(errors);
                tracing::error!(
                    "config {path} is invalid, using defaults until it is fixed and reloaded"
                );
            }
            LoadOutcome::Unreadable(reason) => {
                tracing::error!("cannot read config {path}: {reason}; using defaults");
            }
        }
    }

    pub fn reloaded(&mut self, outcome: LoadOutcome) -> Option<Config> {
        let path = self.path.display().to_string();
        match outcome {
            LoadOutcome::Loaded(parsed) => {
                self.log_warnings(&parsed.warnings);
                tracing::info!("config reloaded from {path}");
                Some(parsed.config)
            }
            LoadOutcome::MissingUsingDefaults(config) => {
                self.log_warnings(&[]);
                tracing::info!("config {path} does not exist, reloaded the defaults");
                Some(config)
            }
            LoadOutcome::Invalid { errors, warnings } => {
                self.log_warnings(&warnings);
                self.log_errors(&errors);
                tracing::error!("config reload rejected, keeping the previous config");
                None
            }
            LoadOutcome::Unreadable(reason) => {
                tracing::error!(
                    "cannot read config {path}: {reason}; config reload rejected, keeping the previous config"
                );
                None
            }
        }
    }

    fn log_warnings(&mut self, warnings: &[Diagnostic]) {
        if self.logged_warnings.as_deref() == Some(warnings) {
            return;
        }
        for warning in warnings {
            tracing::warn!("{}", format_diagnostic("warning", &self.path, warning));
        }
        self.logged_warnings = Some(warnings.to_vec());
    }

    fn log_errors(&self, errors: &[Diagnostic]) {
        for error in errors {
            tracing::error!("{}", format_diagnostic("error", &self.path, error));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use md_core::config::{LogLevel, ScrollAcceleration};
    use tracing::Level;

    use super::*;
    use crate::logging::capture::capture;

    fn source(dir: &tempfile::TempDir) -> ConfigSource {
        ConfigSource::new(dir.path().join("magic-driver/config.toml"))
    }

    fn write(source: &ConfigSource, content: &str) {
        fs::create_dir_all(source.path().parent().unwrap()).unwrap();
        fs::write(source.path(), content).unwrap();
    }

    #[test]
    fn a_missing_file_means_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = source(&dir);
        let outcome = source.load();
        assert_eq!(initial_config(&outcome), Config::default());
        let records = capture(|| source.report_initial(&outcome));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].level, Level::INFO);
        assert!(
            records[0]
                .message
                .ends_with("does not exist, using defaults")
        );
    }

    #[test]
    fn a_valid_file_is_used_at_start() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = source(&dir);
        write(&source, "[scroll]\nacceleration = \"high\"\n");
        let outcome = source.load();
        let config = initial_config(&outcome);
        assert_eq!(config.scroll.acceleration, ScrollAcceleration::High);
        let records = capture(|| source.report_initial(&outcome));
        assert_eq!(records.len(), 1);
        assert!(records[0].message.starts_with("config loaded from "));
    }

    #[test]
    fn an_invalid_file_at_start_falls_back_to_defaults_with_line_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = source(&dir);
        write(&source, "version = 1\n\n[scroll]\nenabled = 3\n");
        let outcome = source.load();
        assert_eq!(initial_config(&outcome), Config::default());
        let records = capture(|| source.report_initial(&outcome));
        assert!(records.iter().all(|record| record.level == Level::ERROR));
        assert!(
            records[0].message.contains("config.toml:4: scroll.enabled"),
            "{records:?}"
        );
        assert!(records.last().unwrap().message.contains("using defaults"));
    }

    #[test]
    fn a_valid_reload_returns_the_new_config() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = source(&dir);
        write(&source, "[log]\nlevel = \"warn\"\n");
        let records = capture(|| {
            let config = source.reloaded(source.load()).expect("valid");
            assert_eq!(config.log.level, LogLevel::Warn);
        });
        assert_eq!(records.len(), 1);
        assert!(records[0].message.starts_with("config reloaded from "));
    }

    #[test]
    fn an_invalid_reload_is_rejected_with_line_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = source(&dir);
        write(&source, "[touch]\npalm_rejection = \"extreme\"\n");
        let records = capture(|| assert_eq!(source.reloaded(source.load()), None));
        assert_eq!(records.len(), 2, "{records:?}");
        assert!(records[0].message.starts_with("error: "));
        assert!(
            records[0]
                .message
                .contains("config.toml:2: touch.palm_rejection")
        );
        assert_eq!(
            records[1].message,
            "config reload rejected, keeping the previous config"
        );
    }

    #[test]
    fn an_unreadable_reload_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = source(&dir);
        fs::create_dir_all(source.path()).unwrap();
        let records = capture(|| assert_eq!(source.reloaded(source.load()), None));
        assert_eq!(records.len(), 1);
        assert!(records[0].message.contains("keeping the previous config"));
    }

    #[test]
    fn a_missing_file_on_reload_restores_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = source(&dir);
        let records = capture(|| {
            assert_eq!(source.reloaded(source.load()), Some(Config::default()));
        });
        assert!(records[0].message.ends_with("reloaded the defaults"));
    }

    #[test]
    fn unchanged_warnings_are_logged_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = source(&dir);
        write(&source, "colour = \"red\"\n");
        let outcome = source.load();
        let records = capture(|| {
            source.report_initial(&outcome);
            source.reloaded(source.load()).expect("valid");
            source.reloaded(source.load()).expect("valid");
        });
        let warnings: Vec<_> = records
            .iter()
            .filter(|record| record.level == Level::WARN)
            .collect();
        assert_eq!(warnings.len(), 1, "{records:?}");
        assert!(
            warnings[0]
                .message
                .contains("config.toml:1: colour: unknown key")
        );

        write(&source, "colour = \"red\"\nsize = 2\n");
        let records = capture(|| {
            source.reloaded(source.load()).expect("valid");
        });
        assert_eq!(
            records
                .iter()
                .filter(|record| record.level == Level::WARN)
                .count(),
            2
        );

        write(&source, "");
        let records = capture(|| {
            source.reloaded(source.load()).expect("valid");
            write(&source, "colour = \"red\"\n");
            source.reloaded(source.load()).expect("valid");
        });
        assert_eq!(
            records
                .iter()
                .filter(|record| record.level == Level::WARN)
                .count(),
            1
        );
    }
}
