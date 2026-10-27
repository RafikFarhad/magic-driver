use std::fmt::Write as _;

use md_core::config::Config;
use md_core::profile::MAGIC_MOUSE_2_BT;
use md_core::testkit::scenarios::{self, Scenario, drive};
use md_core::{Engine, Input, Micros, Mode, Output};
use md_trace::{OutputEvent, TraceRecord};

fn traced(scenario: &Scenario) -> Vec<(Micros, Output)> {
    let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, Config::default(), Mode::Normal);
    engine.set_tracing(true);
    drive(&mut engine, &scenario.timed())
}

fn untraced(scenario: &Scenario) -> Vec<(Micros, Output)> {
    let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, Config::default(), Mode::Normal);
    drive(&mut engine, &scenario.timed())
}

fn records(timeline: &[(Micros, Output)]) -> Vec<&TraceRecord> {
    timeline
        .iter()
        .filter_map(|(_, output)| match output {
            Output::Trace(record) => Some(record),
            _ => None,
        })
        .collect()
}

fn text(timeline: &[(Micros, Output)]) -> String {
    let mut text = String::new();
    for record in records(timeline) {
        if matches!(record, TraceRecord::Report(report) if report.touches.len() < 2) {
            continue;
        }
        writeln!(text, "{record}").unwrap();
    }
    text
}

fn json_lines(timeline: &[(Micros, Output)]) -> String {
    let mut text = String::new();
    for record in records(timeline) {
        writeln!(text, "{}", serde_json::to_string(record).unwrap()).unwrap();
    }
    text
}

#[test]
fn tracing_off_produces_no_records() {
    for (name, scenario) in scenarios::all() {
        let timeline = untraced(&scenario);
        assert!(records(&timeline).is_empty(), "{name}");
    }
}

#[test]
fn tracing_does_not_change_the_device_outputs() {
    for (name, scenario) in scenarios::all() {
        let plain = untraced(&scenario);
        let traced: Vec<_> = traced(&scenario)
            .into_iter()
            .filter(|(_, output)| !output.is_trace())
            .collect();
        assert_eq!(traced, plain, "{name}");
    }
}

#[test]
fn every_report_starts_with_its_report_record_and_every_output_is_recorded() {
    let scenario = scenarios::click_while_scrolling();
    let reports = scenario.timed();
    let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, Config::default(), Mode::Normal);
    engine.set_tracing(true);
    let mut out = Vec::new();
    for (t, bytes) in &reports {
        engine.handle(Input::Report { t: *t, bytes }, &mut out);
        match out.first() {
            Some(Output::Trace(TraceRecord::Report(report))) => {
                assert_eq!(report.t, t.as_u64());
                assert_eq!(report.processing_ns, None);
            }
            other => panic!("first output {other:?}"),
        }
        for (index, output) in out.iter().enumerate() {
            if output.is_trace() {
                continue;
            }
            let Some(Output::Trace(TraceRecord::Output(record))) = out.get(index + 1) else {
                panic!("{output:?} has no output record");
            };
            assert_eq!(record.t, t.as_u64());
            let matches = match (output, &record.output) {
                (Output::PointerMotion { dx, dy }, OutputEvent::PointerMotion { dx: x, dy: y }) => {
                    dx == x && dy == y
                }
                (Output::Button { pressed, .. }, OutputEvent::Button { pressed: p, .. }) => {
                    pressed == p
                }
                (Output::TouchFrame(frame), OutputEvent::TouchFrame { contacts }) => {
                    frame.contact_count() == contacts.len()
                }
                _ => false,
            };
            assert!(matches, "{output:?} vs {record:?}");
        }
        out.clear();
    }
}

#[test]
fn malformed_reports_are_traced_with_their_error() {
    let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, Config::default(), Mode::Normal);
    engine.set_tracing(true);
    let mut out = Vec::new();
    engine.handle(
        Input::Report {
            t: Micros::new(5),
            bytes: &[0x12, 0x00],
        },
        &mut out,
    );
    assert_eq!(out.len(), 1);
    let Output::Trace(TraceRecord::Report(report)) = &out[0] else {
        panic!("{out:?}");
    };
    assert_eq!(report.t, 5);
    assert!(report.error.as_deref().unwrap().contains("length 2"));
}

#[test]
fn reset_traces_the_lift_and_the_return_to_idle() {
    let scenario = scenarios::slow_scroll();
    let reports = scenario.timed();
    let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, Config::default(), Mode::Normal);
    engine.set_tracing(true);
    let mut out = Vec::new();
    for (t, bytes) in &reports[..40] {
        engine.handle(Input::Report { t: *t, bytes }, &mut out);
    }
    out.clear();
    engine.handle(Input::Reset, &mut out);
    let lines: Vec<String> = out
        .iter()
        .filter_map(|output| match output {
            Output::Trace(record) => Some(record.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(
        lines,
        ["0 output frame lifted", "0 recogniser scroll -> idle"]
    );
}

#[test]
fn click_while_scrolling_trace() {
    insta::assert_snapshot!(
        "click_while_scrolling_trace",
        text(&traced(&scenarios::click_while_scrolling()))
    );
}

#[test]
fn two_finger_double_tap_trace() {
    insta::assert_snapshot!(
        "two_finger_double_tap_trace",
        text(&traced(&scenarios::two_finger_double_tap()))
    );
}

#[test]
fn two_finger_swipe_trace_json() {
    insta::assert_snapshot!(
        "two_finger_swipe_trace_json",
        json_lines(&traced(&scenarios::two_finger_swipe()))
    );
}
