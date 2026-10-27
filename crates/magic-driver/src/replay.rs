use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

use md_core::config::Config;
use md_core::profile::Profile;
use md_core::recording::{RecordingError, RecordingReader};
use md_core::{Engine, EngineStats, Input, Micros, Mode, Output};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReplaySummary {
    pub stats: EngineStats,
    pub outputs: u64,
}

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
    #[error("cannot write the output: {0}")]
    Write(#[from] io::Error),
}

const MAX_TIMERS_BETWEEN_REPORTS: usize = 16;

#[derive(Serialize)]
struct JsonLine<'a> {
    t: u64,
    #[serde(flatten)]
    output: &'a Output,
}

pub fn open_recording(path: &Path) -> Result<BufReader<File>, ReplayError> {
    let metadata = std::fs::metadata(path).map_err(ReplayError::Open)?;
    if !metadata.is_file() {
        return Err(ReplayError::NotAFile);
    }
    File::open(path)
        .map(BufReader::new)
        .map_err(ReplayError::Open)
}

pub fn replay<R: BufRead, W: Write>(
    source: R,
    format: ReplayFormat,
    mut out: W,
) -> Result<ReplaySummary, ReplayError> {
    let reader = RecordingReader::new(source)?;
    let profile_id = &reader.header().profile_id;
    let profile = Profile::by_id(profile_id)
        .ok_or_else(|| ReplayError::UnknownProfile(profile_id.clone()))?;
    let mut engine = Engine::new(profile, Config::for_profile(profile), Mode::Normal);
    let mut outputs = Vec::new();
    let mut summary = ReplaySummary::default();
    let mut result = Ok(());
    for item in reader {
        let (t, bytes) = match item {
            Ok(report) => report,
            Err(error) => {
                result = Err(error.into());
                break;
            }
        };
        fire_deadlines(
            &mut engine,
            Some(t),
            &mut outputs,
            &mut out,
            format,
            &mut summary,
        )?;
        engine.handle(Input::Report { t, bytes: &bytes }, &mut outputs);
        write_outputs(&mut out, format, t, &mut outputs, &mut summary)?;
    }
    fire_deadlines(
        &mut engine,
        None,
        &mut outputs,
        &mut out,
        format,
        &mut summary,
    )?;
    out.flush()?;
    summary.stats = engine.stats();
    result.map(|()| summary)
}

fn fire_deadlines<W: Write>(
    engine: &mut Engine,
    before: Option<Micros>,
    outputs: &mut Vec<Output>,
    out: &mut W,
    format: ReplayFormat,
    summary: &mut ReplaySummary,
) -> Result<(), ReplayError> {
    for _ in 0..MAX_TIMERS_BETWEEN_REPORTS {
        let Some(deadline) = engine
            .next_deadline()
            .filter(|deadline| before.is_none_or(|before| *deadline < before))
        else {
            break;
        };
        engine.handle(Input::Timer { t: deadline }, outputs);
        write_outputs(out, format, deadline, outputs, summary)?;
    }
    Ok(())
}

fn write_outputs<W: Write>(
    out: &mut W,
    format: ReplayFormat,
    t: Micros,
    outputs: &mut Vec<Output>,
    summary: &mut ReplaySummary,
) -> Result<(), ReplayError> {
    for output in outputs.iter() {
        write_output(out, format, t, output)?;
    }
    summary.outputs += outputs.len() as u64;
    outputs.clear();
    Ok(())
}

fn write_output<W: Write>(
    out: &mut W,
    format: ReplayFormat,
    t: Micros,
    output: &Output,
) -> Result<(), ReplayError> {
    match format {
        ReplayFormat::Text => writeln!(out, "{} {output}", t.as_u64())?,
        ReplayFormat::Json => {
            serde_json::to_writer(
                &mut *out,
                &JsonLine {
                    t: t.as_u64(),
                    output,
                },
            )
            .map_err(io::Error::from)?;
            writeln!(out)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use md_core::recording::{RecordingHeader, RecordingWriter};

    fn recording(profile_id: &str, reports: &[(u64, Vec<u8>)]) -> Vec<u8> {
        let header = RecordingHeader {
            profile_id: profile_id.to_owned(),
            magic_driver_version: "0.0.0-test".to_owned(),
            kernel_version: "none".to_owned(),
            start_unix_s: 0,
        };
        let mut writer = RecordingWriter::new(Vec::new(), &header).expect("header");
        for (t, bytes) in reports {
            writer.write_report(Micros::new(*t), bytes).expect("report");
        }
        writer.into_inner()
    }

    fn header_only_report(clicks: u8, dx: u8) -> Vec<u8> {
        let mut bytes = vec![0; 14];
        bytes[0] = 0x12;
        bytes[1] = clicks;
        bytes[2] = dx;
        bytes
    }

    fn run(text: &[u8], format: ReplayFormat) -> (Result<ReplaySummary, ReplayError>, String) {
        let mut out = Vec::new();
        let result = replay(text, format, &mut out);
        (result, String::from_utf8(out).expect("utf8"))
    }

    #[test]
    fn text_output_has_one_line_per_output_with_its_timestamp() {
        let text = recording(
            "magic-mouse-2-bt",
            &[
                (0, header_only_report(0, 3)),
                (10, header_only_report(1, 0)),
                (20, vec![0x29, 0x00]),
                (30, vec![0x12, 0x00]),
                (40, header_only_report(0, 0)),
            ],
        );
        let (result, out) = run(&text, ReplayFormat::Text);
        let summary = result.expect("replay");
        assert_eq!(
            out,
            "0 pointer dx=3 dy=0\n10 button left press\n40 button left release\n"
        );
        assert_eq!(
            summary,
            ReplaySummary {
                stats: EngineStats {
                    reports: 5,
                    ignored: 1,
                    malformed: 1
                },
                outputs: 3
            }
        );
    }

    #[test]
    fn json_output_is_one_object_per_line() {
        let text = recording("magic-mouse-2-bt", &[(7, header_only_report(2, 0xff))]);
        let (result, out) = run(&text, ReplayFormat::Json);
        result.expect("replay");
        assert_eq!(
            out,
            "{\"t\":7,\"type\":\"pointer_motion\",\"dx\":255,\"dy\":0}\n{\"t\":7,\"type\":\"button\",\"button\":\"right\",\"pressed\":true}\n"
        );
    }

    #[test]
    fn unknown_profile_is_an_error() {
        let text = recording("magic-trackpad", &[]);
        let (result, out) = run(&text, ReplayFormat::Text);
        assert!(matches!(result, Err(ReplayError::UnknownProfile(id)) if id == "magic-trackpad"));
        assert!(out.is_empty());
    }

    #[test]
    fn recording_errors_stop_after_the_outputs_so_far() {
        let mut text = recording("magic-mouse-2-bt", &[(0, header_only_report(0, 1))]);
        text.extend_from_slice(b"5 12zz\n");
        let (result, out) = run(&text, ReplayFormat::Text);
        assert_eq!(out, "0 pointer dx=1 dy=0\n");
        match result {
            Err(ReplayError::Recording(RecordingError::InvalidReport { line, .. })) => {
                assert_eq!(line, 3);
            }
            other => panic!("expected a report error, got {other:?}"),
        }
    }

    #[test]
    fn missing_header_is_an_error() {
        let (result, _) = run(b"", ReplayFormat::Text);
        assert!(matches!(
            result,
            Err(ReplayError::Recording(RecordingError::MissingHeader))
        ));
    }
}
