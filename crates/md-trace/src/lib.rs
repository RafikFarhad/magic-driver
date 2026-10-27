#![forbid(unsafe_code)]

use std::fmt;
use std::io::{self, Read};

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_RECORD_LEN: usize = 1 << 20;
pub const LENGTH_PREFIX_LEN: usize = 4;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TraceRecord {
    Hello(Hello),
    Report(ReportRecord),
    Decision(DecisionRecord),
    Recogniser(RecogniserRecord),
    Output(OutputRecord),
    Session(SessionRecord),
    Dropped(DroppedRecord),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol_version: u32,
    pub daemon_version: String,
    pub profile: ProfileInfo,
    pub config: serde_json::Value,
    pub session: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileInfo {
    pub id: String,
    pub name: String,
    pub surface: SurfaceInfo,
    pub touchpad: TouchpadInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SurfaceInfo {
    pub x_min: i32,
    pub x_max: i32,
    pub x_size_mm: f64,
    pub y_min: i32,
    pub y_max: i32,
    pub y_size_mm: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TouchpadInfo {
    pub max_x: i32,
    pub max_y: i32,
    pub units_per_mm: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportRecord {
    pub t: u64,
    pub clicks: u8,
    pub dx: i32,
    pub dy: i32,
    pub touches: Vec<TouchInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processing_ns: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TouchInfo {
    pub id: u8,
    pub x: i32,
    pub y: i32,
    pub touch_major: u8,
    pub touch_minor: u8,
    pub size: u8,
    pub orientation: i8,
    pub state: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackClass {
    Candidate,
    Deliberate,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionReason {
    Palm,
    RearZone,
    MouseMoving,
    ClickGuard,
    Resting,
    Promoted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub t: u64,
    pub track: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub touch_id: Option<u8>,
    pub previous: TrackClass,
    pub class: TrackClass,
    pub reason: DecisionReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GestureState {
    Idle,
    Scroll,
    Swipe,
    TapPending,
    Cancelling,
    Suppressed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecogniserRecord {
    pub t: u64,
    pub from: GestureState,
    pub to: GestureState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonName {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactInfo {
    pub slot: u8,
    pub tracking_id: u16,
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutputEvent {
    PointerMotion { dx: i32, dy: i32 },
    Button { button: ButtonName, pressed: bool },
    TouchFrame { contacts: Vec<ContactInfo> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputRecord {
    pub t: u64,
    pub output: OutputEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub t: u64,
    pub state: String,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DroppedRecord {
    pub count: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("cannot encode the trace record: {0}")]
    Encode(serde_json::Error),
    #[error("trace record of {0} bytes is larger than the limit of {MAX_RECORD_LEN} bytes")]
    TooLarge(usize),
    #[error("cannot read the trace stream: {0}")]
    Io(#[from] io::Error),
    #[error("invalid trace record: {0}")]
    Decode(serde_json::Error),
}

pub fn encode(record: &TraceRecord, out: &mut Vec<u8>) -> Result<(), WireError> {
    let start = out.len();
    out.extend_from_slice(&[0; LENGTH_PREFIX_LEN]);
    serde_json::to_writer(&mut *out, record).map_err(|error| {
        out.truncate(start);
        WireError::Encode(error)
    })?;
    let len = out.len() - start - LENGTH_PREFIX_LEN;
    let Some(prefix) = u32::try_from(len).ok().filter(|_| len <= MAX_RECORD_LEN) else {
        out.truncate(start);
        return Err(WireError::TooLarge(len));
    };
    out[start..start + LENGTH_PREFIX_LEN].copy_from_slice(&prefix.to_be_bytes());
    Ok(())
}

pub fn read_payload<R: Read>(reader: &mut R) -> Result<Option<Vec<u8>>, WireError> {
    let mut prefix = [0_u8; LENGTH_PREFIX_LEN];
    let mut filled = 0;
    while filled < LENGTH_PREFIX_LEN {
        match reader.read(&mut prefix[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => return Err(WireError::Io(io::ErrorKind::UnexpectedEof.into())),
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(WireError::Io(error)),
        }
    }
    let len = usize::try_from(u32::from_be_bytes(prefix)).unwrap_or(usize::MAX);
    if len > MAX_RECORD_LEN {
        return Err(WireError::TooLarge(len));
    }
    let mut payload = vec![0; len];
    reader.read_exact(&mut payload)?;
    Ok(Some(payload))
}

pub fn decode(payload: &[u8]) -> Result<TraceRecord, WireError> {
    serde_json::from_slice(payload).map_err(WireError::Decode)
}

impl fmt::Display for TrackClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TrackClass::Candidate => "candidate",
            TrackClass::Deliberate => "deliberate",
            TrackClass::Rejected => "rejected",
        })
    }
}

impl fmt::Display for DecisionReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DecisionReason::Palm => "palm",
            DecisionReason::RearZone => "rear zone",
            DecisionReason::MouseMoving => "mouse moving",
            DecisionReason::ClickGuard => "click guard",
            DecisionReason::Resting => "resting",
            DecisionReason::Promoted => "promoted",
        })
    }
}

impl fmt::Display for GestureState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            GestureState::Idle => "idle",
            GestureState::Scroll => "scroll",
            GestureState::Swipe => "swipe",
            GestureState::TapPending => "tap pending",
            GestureState::Cancelling => "cancelling",
            GestureState::Suppressed => "suppressed",
        })
    }
}

impl fmt::Display for ButtonName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ButtonName::Left => "left",
            ButtonName::Right => "right",
            ButtonName::Middle => "middle",
        })
    }
}

impl fmt::Display for OutputEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OutputEvent::PointerMotion { dx, dy } => write!(f, "pointer dx={dx} dy={dy}"),
            OutputEvent::Button { button, pressed } => write!(
                f,
                "button {button} {}",
                if *pressed { "press" } else { "release" }
            ),
            OutputEvent::TouchFrame { contacts } => {
                write!(f, "frame")?;
                if contacts.is_empty() {
                    return write!(f, " lifted");
                }
                for contact in contacts {
                    write!(
                        f,
                        " {}:{}@{},{}",
                        contact.slot, contact.tracking_id, contact.x, contact.y
                    )?;
                }
                Ok(())
            }
        }
    }
}

impl fmt::Display for TraceRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TraceRecord::Hello(hello) => write!(
                f,
                "hello protocol {} magic-driver {} profile {} session {}",
                hello.protocol_version, hello.daemon_version, hello.profile.id, hello.session
            ),
            TraceRecord::Report(report) => {
                write!(
                    f,
                    "{} report clicks={} dx={} dy={} touches={}",
                    report.t,
                    report.clicks,
                    report.dx,
                    report.dy,
                    report.touches.len()
                )?;
                for touch in &report.touches {
                    write!(
                        f,
                        " [{} {},{} {:#04x} {}x{}]",
                        touch.id,
                        touch.x,
                        touch.y,
                        touch.state,
                        touch.touch_major,
                        touch.touch_minor
                    )?;
                }
                if let Some(error) = &report.error {
                    write!(f, " error: {error}")?;
                }
                if let Some(ns) = report.processing_ns {
                    write!(f, " in {ns} ns")?;
                }
                Ok(())
            }
            TraceRecord::Decision(decision) => {
                write!(f, "{} track {}", decision.t, decision.track)?;
                if let Some(touch_id) = decision.touch_id {
                    write!(f, " (touch {touch_id})")?;
                }
                write!(
                    f,
                    " {} -> {}: {}",
                    decision.previous, decision.class, decision.reason
                )
            }
            TraceRecord::Recogniser(change) => {
                write!(
                    f,
                    "{} recogniser {} -> {}",
                    change.t, change.from, change.to
                )
            }
            TraceRecord::Output(output) => write!(f, "{} output {}", output.t, output.output),
            TraceRecord::Session(session) => {
                write!(
                    f,
                    "{} session {}: {}",
                    session.t, session.state, session.detail
                )
            }
            TraceRecord::Dropped(dropped) => write!(f, "dropped {} records", dropped.count),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello() -> TraceRecord {
        TraceRecord::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            daemon_version: "0.1.0".to_owned(),
            profile: ProfileInfo {
                id: "magic-mouse-2-bt".to_owned(),
                name: "Apple Magic Mouse 2 (Bluetooth)".to_owned(),
                surface: SurfaceInfo {
                    x_min: -1100,
                    x_max: 1258,
                    x_size_mm: 90.56,
                    y_min: -1589,
                    y_max: 2047,
                    y_size_mm: 51.52,
                },
                touchpad: TouchpadInfo {
                    max_x: 16_000,
                    max_y: 16_000,
                    units_per_mm: 40,
                },
            },
            config: serde_json::json!({"version": 1}),
            session: "active".to_owned(),
        })
    }

    fn samples() -> Vec<TraceRecord> {
        vec![
            hello(),
            TraceRecord::Report(ReportRecord {
                t: 11_111,
                clicks: 0,
                dx: 2,
                dy: -1,
                touches: vec![TouchInfo {
                    id: 2,
                    x: -100,
                    y: 600,
                    touch_major: 44,
                    touch_minor: 36,
                    size: 12,
                    orientation: 0,
                    state: 0x40,
                }],
                error: None,
                processing_ns: Some(2_300),
            }),
            TraceRecord::Decision(DecisionRecord {
                t: 22_222,
                track: 7,
                touch_id: Some(2),
                previous: TrackClass::Candidate,
                class: TrackClass::Deliberate,
                reason: DecisionReason::Promoted,
            }),
            TraceRecord::Recogniser(RecogniserRecord {
                t: 22_222,
                from: GestureState::Idle,
                to: GestureState::TapPending,
            }),
            TraceRecord::Output(OutputRecord {
                t: 22_222,
                output: OutputEvent::TouchFrame {
                    contacts: vec![ContactInfo {
                        slot: 0,
                        tracking_id: 3,
                        x: 7600,
                        y: 8000,
                    }],
                },
            }),
            TraceRecord::Output(OutputRecord {
                t: 22_223,
                output: OutputEvent::Button {
                    button: ButtonName::Left,
                    pressed: true,
                },
            }),
            TraceRecord::Session(SessionRecord {
                t: 30_000,
                state: "degraded".to_owned(),
                detail: "no valid hidraw report".to_owned(),
            }),
            TraceRecord::Dropped(DroppedRecord { count: 12 }),
        ]
    }

    #[test]
    fn records_round_trip_through_the_wire_format() {
        let mut wire = Vec::new();
        for record in samples() {
            encode(&record, &mut wire).unwrap();
        }
        let mut reader = wire.as_slice();
        let mut decoded = Vec::new();
        while let Some(payload) = read_payload(&mut reader).unwrap() {
            decoded.push(decode(&payload).unwrap());
        }
        assert_eq!(decoded, samples());
    }

    #[test]
    fn frames_are_big_endian_length_prefixed_json() {
        let mut wire = Vec::new();
        let record = TraceRecord::Dropped(DroppedRecord { count: 3 });
        encode(&record, &mut wire).unwrap();
        let json = br#"{"type":"dropped","count":3}"#;
        let len = u32::try_from(json.len()).unwrap();
        assert_eq!(&wire[..4], &len.to_be_bytes());
        assert_eq!(&wire[4..], json);
    }

    #[test]
    fn records_serialise_with_a_type_tag() {
        let json: Vec<String> = samples()
            .iter()
            .map(|record| serde_json::to_string(record).unwrap())
            .collect();
        assert!(json[0].starts_with(r#"{"type":"hello","protocol_version":1,"#));
        assert!(json[1].starts_with(r#"{"type":"report","t":11111,"#));
        assert!(json[1].ends_with(r#""processing_ns":2300}"#));
        assert_eq!(
            json[2],
            r#"{"type":"decision","t":22222,"track":7,"touch_id":2,"previous":"candidate","class":"deliberate","reason":"promoted"}"#
        );
        assert_eq!(
            json[3],
            r#"{"type":"recogniser","t":22222,"from":"idle","to":"tap_pending"}"#
        );
        assert_eq!(
            json[4],
            r#"{"type":"output","t":22222,"output":{"kind":"touch_frame","contacts":[{"slot":0,"tracking_id":3,"x":7600,"y":8000}]}}"#
        );
    }

    #[test]
    fn records_render_as_text_lines() {
        let lines: Vec<String> = samples().iter().map(ToString::to_string).collect();
        assert_eq!(
            lines,
            [
                "hello protocol 1 magic-driver 0.1.0 profile magic-mouse-2-bt session active",
                "11111 report clicks=0 dx=2 dy=-1 touches=1 [2 -100,600 0x40 44x36] in 2300 ns",
                "22222 track 7 (touch 2) candidate -> deliberate: promoted",
                "22222 recogniser idle -> tap pending",
                "22222 output frame 0:3@7600,8000",
                "22223 output button left press",
                "30000 session degraded: no valid hidraw report",
                "dropped 12 records",
            ]
        );
    }

    #[test]
    fn truncated_and_oversized_frames_are_errors() {
        let mut wire = Vec::new();
        encode(&TraceRecord::Dropped(DroppedRecord { count: 1 }), &mut wire).unwrap();
        let mut short = &wire[..wire.len() - 1];
        assert!(matches!(read_payload(&mut short), Err(WireError::Io(_))));
        let partial = [0_u8, 0];
        let mut partial_prefix = &partial[..];
        assert!(matches!(
            read_payload(&mut partial_prefix),
            Err(WireError::Io(_))
        ));
        let huge = u32::try_from(MAX_RECORD_LEN + 1).unwrap().to_be_bytes();
        let mut oversized: &[u8] = &huge;
        assert!(matches!(
            read_payload(&mut oversized),
            Err(WireError::TooLarge(_))
        ));
        let mut empty = &wire[..0];
        assert!(read_payload(&mut empty).unwrap().is_none());
        assert!(matches!(decode(b"{"), Err(WireError::Decode(_))));
    }
}
