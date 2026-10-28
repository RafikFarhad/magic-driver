use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

use md_core::config::{Config, PalmRejection, ScrollAcceleration};
use md_core::profile::Profile;
use md_core::recording::{RecordingError, RecordingHeader, RecordingReader};
use md_core::{Engine, Input, Micros, Mode, Output};
use md_trace::TraceRecord;

use crate::timeline::Timeline;

pub const MAX_TIMERS_BETWEEN_REPORTS: usize = 16;
const MAX_TIMERS_AFTER_END: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("cannot open the recording: {0}")]
    Open(io::Error),
    #[error("not a regular file")]
    NotAFile,
    #[error("{0}")]
    Recording(#[from] RecordingError),
    #[error("unknown device profile {0:?}")]
    UnknownProfile(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Recording {
    pub header: RecordingHeader,
    pub profile: &'static Profile,
    pub reports: Vec<(Micros, Vec<u8>)>,
}

pub fn load_recording(path: &Path) -> Result<Recording, ReplayError> {
    let metadata = std::fs::metadata(path).map_err(ReplayError::Open)?;
    if !metadata.is_file() {
        return Err(ReplayError::NotAFile);
    }
    let file = File::open(path).map_err(ReplayError::Open)?;
    read_recording(BufReader::new(file))
}

pub fn read_recording<R: BufRead>(source: R) -> Result<Recording, ReplayError> {
    let reader = RecordingReader::new(source)?;
    let header = reader.header().clone();
    let profile = Profile::by_id(&header.profile_id)
        .ok_or_else(|| ReplayError::UnknownProfile(header.profile_id.clone()))?;
    let reports = reader.collect::<Result<Vec<_>, _>>()?;
    Ok(Recording {
        header,
        profile,
        reports,
    })
}

pub fn run_engine(
    profile: &'static Profile,
    config: &Config,
    reports: &[(Micros, Vec<u8>)],
) -> Timeline {
    let mut engine = Engine::new(profile, config.clone(), Mode::Normal);
    engine.set_tracing(true);
    let mut timeline = Timeline::new();
    let mut outputs = Vec::new();
    for (t, bytes) in reports {
        fire_deadlines(
            &mut engine,
            Some(*t),
            MAX_TIMERS_BETWEEN_REPORTS,
            &mut outputs,
            &mut timeline,
        );
        engine.handle(Input::Report { t: *t, bytes }, &mut outputs);
        collect(&mut outputs, &mut timeline);
    }
    fire_deadlines(
        &mut engine,
        None,
        MAX_TIMERS_AFTER_END,
        &mut outputs,
        &mut timeline,
    );
    timeline
}

fn fire_deadlines(
    engine: &mut Engine,
    before: Option<Micros>,
    limit: usize,
    outputs: &mut Vec<Output>,
    timeline: &mut Timeline,
) {
    for _ in 0..limit {
        let Some(deadline) = engine
            .next_deadline()
            .filter(|deadline| before.is_none_or(|before| *deadline < before))
        else {
            break;
        };
        engine.handle(Input::Timer { t: deadline }, outputs);
        collect(outputs, timeline);
    }
}

fn collect(outputs: &mut Vec<Output>, timeline: &mut Timeline) {
    for output in outputs.drain(..) {
        if let Output::Trace(record) = output {
            timeline.push(record);
        }
    }
}

pub fn trace_records(timeline: &Timeline) -> Vec<TraceRecord> {
    timeline
        .entries()
        .map(|entry| entry.record.clone())
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct OfflineSource {
    pub path: PathBuf,
    pub recording: Recording,
    pub chosen: ChosenConfig,
    pub config: Config,
    pub timeline: Timeline,
}

impl OfflineSource {
    pub fn new(path: PathBuf, recording: Recording, chosen: ChosenConfig) -> OfflineSource {
        let config = chosen.config.clone();
        let timeline = run_engine(recording.profile, &config, &recording.reports);
        OfflineSource {
            path,
            recording,
            chosen,
            config,
            timeline,
        }
    }

    pub fn open(
        path: &Path,
        explicit_config: Option<&Path>,
        default_config: Option<PathBuf>,
    ) -> Result<OfflineSource, String> {
        let recording =
            load_recording(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let chosen = choose_config(explicit_config, default_config, recording.profile)?;
        Ok(OfflineSource::new(path.to_path_buf(), recording, chosen))
    }

    pub fn rerun(&mut self, config: Config) {
        self.timeline = run_engine(self.recording.profile, &config, &self.recording.reports);
        self.config = config;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigOrigin {
    Defaults,
    File(PathBuf),
    MissingDefaultFile(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChosenConfig {
    pub config: Config,
    pub origin: ConfigOrigin,
    pub warnings: Vec<String>,
}

pub fn default_config_path(
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    let base = xdg_config_home
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            home.filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        })?;
    Some(base.join("magic-driver").join("config.toml"))
}

pub fn choose_config(
    explicit: Option<&Path>,
    default_path: Option<PathBuf>,
    profile: &'static Profile,
) -> Result<ChosenConfig, String> {
    match explicit {
        Some(path) => load_config_file(path, profile),
        None => match default_path {
            Some(path) if path.exists() => load_config_file(&path, profile),
            Some(path) => Ok(ChosenConfig {
                config: Config::for_profile(profile),
                origin: ConfigOrigin::MissingDefaultFile(path),
                warnings: Vec::new(),
            }),
            None => Ok(ChosenConfig {
                config: Config::for_profile(profile),
                origin: ConfigOrigin::Defaults,
                warnings: Vec::new(),
            }),
        },
    }
}

pub fn load_config_file(path: &Path, profile: &'static Profile) -> Result<ChosenConfig, String> {
    let source =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let parsed = Config::parse(&source, profile).map_err(|errors| {
        let details: Vec<String> = errors.errors.iter().map(ToString::to_string).collect();
        format!("{}: {errors}: {}", path.display(), details.join("; "))
    })?;
    Ok(ChosenConfig {
        config: parsed.config,
        origin: ConfigOrigin::File(path.to_path_buf()),
        warnings: parsed.warnings.iter().map(ToString::to_string).collect(),
    })
}

pub fn with_palm_rejection(config: &Config, preset: PalmRejection, profile: &Profile) -> Config {
    let mut config = config.clone();
    let values = preset.preset(profile);
    config.touch.palm_rejection = preset;
    let touch = &mut config.advanced.touch;
    touch.promote_distance_mm = values.promote_distance_mm;
    touch.rest_dwell_ms = values.rest_dwell_ms;
    touch.palm_size_mm = values.palm_size_mm;
    touch.rear_zone = values.rear_zone;
    touch.mouse_motion_threshold = values.mouse_motion_threshold;
    touch.click_guard_ms = values.click_guard_ms;
    config
}

pub fn with_acceleration(config: &Config, acceleration: ScrollAcceleration) -> Config {
    let mut config = config.clone();
    let values = acceleration.preset();
    config.scroll.acceleration = acceleration;
    let scroll = &mut config.advanced.scroll;
    scroll.gain_min = values.gain_min;
    scroll.gain_max = values.gain_max;
    scroll.gain_speed_mm_s = values.gain_speed_mm_s;
    config
}

#[cfg(test)]
mod tests;
