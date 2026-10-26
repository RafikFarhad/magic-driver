use std::fmt::Write as _;
use std::io::{self, BufRead, Write};

use crate::Micros;

pub const RECORDING_TAG: &str = "magic-driver-recording";
pub const RECORDING_FORMAT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingHeader {
    pub profile_id: String,
    pub magic_driver_version: String,
    pub kernel_version: String,
    pub start_unix_s: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum RecordingError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("recording is empty, expected a header line")]
    MissingHeader,
    #[error("line {line}: invalid header: {reason}")]
    InvalidHeader { line: usize, reason: HeaderError },
    #[error("line {line}: invalid report: {reason}")]
    InvalidReport {
        line: usize,
        reason: ReportLineError,
    },
    #[error("header field {field} has invalid value {value:?}")]
    InvalidHeaderValue { field: &'static str, value: String },
    #[error("report at {t} is earlier than the previous report at {previous}")]
    TimestampOutOfOrder { previous: Micros, t: Micros },
    #[error("report has no bytes")]
    EmptyReport,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HeaderError {
    #[error("expected the line to start with {RECORDING_TAG}")]
    MissingTag,
    #[error("field {0:?} is not of the form name=value")]
    MalformedField(String),
    #[error("unknown field {0:?}")]
    UnknownField(String),
    #[error("duplicate field {0:?}")]
    DuplicateField(String),
    #[error("missing field {0:?}")]
    MissingField(&'static str),
    #[error("unsupported format {0:?}, expected {RECORDING_FORMAT}")]
    UnsupportedFormat(String),
    #[error("start {0:?} is not a number of seconds")]
    InvalidStart(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReportLineError {
    #[error("expected <microseconds> <hex bytes>")]
    MissingSeparator,
    #[error("timestamp {0:?} is not a number of microseconds")]
    InvalidTimestamp(String),
    #[error("no report bytes")]
    EmptyHex,
    #[error("odd number of hex digits")]
    OddHexLength,
    #[error("invalid character {character:?} at hex position {position}, expected lowercase hex")]
    InvalidHexDigit { position: usize, character: char },
    #[error("timestamp {t} is earlier than the previous report at {previous}")]
    TimestampOutOfOrder { previous: Micros, t: Micros },
}

const FIELD_FORMAT: &str = "format";
const FIELD_PROFILE: &str = "profile";
const FIELD_MAGIC_DRIVER: &str = "magic-driver";
const FIELD_KERNEL: &str = "kernel";
const FIELD_START: &str = "start";

impl RecordingHeader {
    pub fn render(&self) -> Result<String, RecordingError> {
        for (field, value) in [
            (FIELD_PROFILE, &self.profile_id),
            (FIELD_MAGIC_DRIVER, &self.magic_driver_version),
            (FIELD_KERNEL, &self.kernel_version),
        ] {
            if !is_valid_header_value(value) {
                return Err(RecordingError::InvalidHeaderValue {
                    field,
                    value: value.clone(),
                });
            }
        }
        Ok(format!(
            "{RECORDING_TAG} {FIELD_FORMAT}={RECORDING_FORMAT} {FIELD_PROFILE}={} {FIELD_MAGIC_DRIVER}={} {FIELD_KERNEL}={} {FIELD_START}={}",
            self.profile_id, self.magic_driver_version, self.kernel_version, self.start_unix_s
        ))
    }

    pub fn parse(line: &str) -> Result<RecordingHeader, HeaderError> {
        let mut tokens = line.split_whitespace();
        if tokens.next() != Some(RECORDING_TAG) {
            return Err(HeaderError::MissingTag);
        }
        let mut format = None;
        let mut profile_id = None;
        let mut magic_driver_version = None;
        let mut kernel_version = None;
        let mut start = None;
        for token in tokens {
            let Some((name, value)) = token.split_once('=') else {
                return Err(HeaderError::MalformedField(token.to_owned()));
            };
            if value.is_empty() || value.contains('=') {
                return Err(HeaderError::MalformedField(token.to_owned()));
            }
            let slot = match name {
                FIELD_FORMAT => &mut format,
                FIELD_PROFILE => &mut profile_id,
                FIELD_MAGIC_DRIVER => &mut magic_driver_version,
                FIELD_KERNEL => &mut kernel_version,
                FIELD_START => &mut start,
                _ => return Err(HeaderError::UnknownField(name.to_owned())),
            };
            if slot.replace(value.to_owned()).is_some() {
                return Err(HeaderError::DuplicateField(name.to_owned()));
            }
        }
        let format = format.ok_or(HeaderError::MissingField(FIELD_FORMAT))?;
        if format != RECORDING_FORMAT.to_string() {
            return Err(HeaderError::UnsupportedFormat(format));
        }
        let start = start.ok_or(HeaderError::MissingField(FIELD_START))?;
        let start_unix_s = start
            .parse::<u64>()
            .map_err(|_| HeaderError::InvalidStart(start.clone()))?;
        Ok(RecordingHeader {
            profile_id: profile_id.ok_or(HeaderError::MissingField(FIELD_PROFILE))?,
            magic_driver_version: magic_driver_version
                .ok_or(HeaderError::MissingField(FIELD_MAGIC_DRIVER))?,
            kernel_version: kernel_version.ok_or(HeaderError::MissingField(FIELD_KERNEL))?,
            start_unix_s,
        })
    }
}

fn is_valid_header_value(value: &str) -> bool {
    !value.is_empty() && !value.contains('=') && !value.chars().any(char::is_whitespace)
}

pub fn render_report_line(t: Micros, bytes: &[u8]) -> String {
    let mut line = String::with_capacity(21 + bytes.len() * 2);
    write!(line, "{} ", t.as_u64()).expect("writing to a String cannot fail");
    for byte in bytes {
        write!(line, "{byte:02x}").expect("writing to a String cannot fail");
    }
    line
}

pub fn parse_report_line(line: &str) -> Result<(Micros, Vec<u8>), ReportLineError> {
    let line = line.trim_end();
    let (timestamp, hex) = line
        .split_once(' ')
        .ok_or(ReportLineError::MissingSeparator)?;
    let t = parse_timestamp(timestamp)?;
    Ok((t, parse_hex(hex)?))
}

fn parse_timestamp(timestamp: &str) -> Result<Micros, ReportLineError> {
    if timestamp.is_empty() || !timestamp.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ReportLineError::InvalidTimestamp(timestamp.to_owned()));
    }
    timestamp
        .parse::<u64>()
        .map(Micros::new)
        .map_err(|_| ReportLineError::InvalidTimestamp(timestamp.to_owned()))
}

fn parse_hex(hex: &str) -> Result<Vec<u8>, ReportLineError> {
    if hex.is_empty() {
        return Err(ReportLineError::EmptyHex);
    }
    let mut nibbles = Vec::with_capacity(hex.len());
    for (position, character) in hex.chars().enumerate() {
        let nibble = match character {
            '0'..='9' | 'a'..='f' => character.to_digit(16),
            _ => None,
        };
        let Some(nibble) = nibble else {
            return Err(ReportLineError::InvalidHexDigit {
                position,
                character,
            });
        };
        nibbles.push(u8::try_from(nibble).expect("hex digit fits in a byte"));
    }
    if nibbles.len() % 2 != 0 {
        return Err(ReportLineError::OddHexLength);
    }
    Ok(nibbles
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| (pair[0] << 4) | pair[1])
        .collect())
}

pub struct RecordingWriter<W: Write> {
    out: W,
    previous: Option<Micros>,
}

impl<W: Write> RecordingWriter<W> {
    pub fn new(mut out: W, header: &RecordingHeader) -> Result<Self, RecordingError> {
        let line = header.render()?;
        writeln!(out, "{line}")?;
        Ok(RecordingWriter {
            out,
            previous: None,
        })
    }

    pub fn write_report(&mut self, t: Micros, bytes: &[u8]) -> Result<(), RecordingError> {
        if bytes.is_empty() {
            return Err(RecordingError::EmptyReport);
        }
        if let Some(previous) = self.previous
            && t < previous
        {
            return Err(RecordingError::TimestampOutOfOrder { previous, t });
        }
        writeln!(self.out, "{}", render_report_line(t, bytes))?;
        self.previous = Some(t);
        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), RecordingError> {
        self.out.flush()?;
        Ok(())
    }

    pub fn into_inner(self) -> W {
        self.out
    }
}

pub struct RecordingReader<R: BufRead> {
    lines: io::Lines<R>,
    header: RecordingHeader,
    line: usize,
    previous: Option<Micros>,
    failed: bool,
}

impl<R: BufRead> RecordingReader<R> {
    pub fn new(reader: R) -> Result<Self, RecordingError> {
        let mut lines = reader.lines();
        let first = lines.next().ok_or(RecordingError::MissingHeader)??;
        let header = RecordingHeader::parse(first.trim_end())
            .map_err(|reason| RecordingError::InvalidHeader { line: 1, reason })?;
        Ok(RecordingReader {
            lines,
            header,
            line: 1,
            previous: None,
            failed: false,
        })
    }

    pub fn header(&self) -> &RecordingHeader {
        &self.header
    }

    fn next_report(&mut self) -> Option<Result<(Micros, Vec<u8>), RecordingError>> {
        loop {
            let text = match self.lines.next()? {
                Ok(text) => text,
                Err(error) => return Some(Err(error.into())),
            };
            self.line += 1;
            if text.trim().is_empty() {
                continue;
            }
            let parsed = parse_report_line(&text).and_then(|(t, bytes)| match self.previous {
                Some(previous) if t < previous => {
                    Err(ReportLineError::TimestampOutOfOrder { previous, t })
                }
                _ => Ok((t, bytes)),
            });
            return Some(match parsed {
                Ok((t, bytes)) => {
                    self.previous = Some(t);
                    Ok((t, bytes))
                }
                Err(reason) => Err(RecordingError::InvalidReport {
                    line: self.line,
                    reason,
                }),
            });
        }
    }
}

impl<R: BufRead> Iterator for RecordingReader<R> {
    type Item = Result<(Micros, Vec<u8>), RecordingError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        let item = self.next_report();
        if matches!(item, Some(Err(_))) {
            self.failed = true;
        }
        item
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> RecordingHeader {
        RecordingHeader {
            profile_id: "magic-mouse-2-bt".to_owned(),
            magic_driver_version: "0.1.0".to_owned(),
            kernel_version: "7.1.13-2-MANJARO".to_owned(),
            start_unix_s: 1_767_225_600,
        }
    }

    const HEADER_LINE: &str = "magic-driver-recording format=1 profile=magic-mouse-2-bt magic-driver=0.1.0 kernel=7.1.13-2-MANJARO start=1767225600";

    fn write(reports: &[(u64, &[u8])]) -> String {
        let mut writer = RecordingWriter::new(Vec::new(), &header()).expect("header");
        for (t, bytes) in reports {
            writer.write_report(Micros::new(*t), bytes).expect("report");
        }
        String::from_utf8(writer.into_inner()).expect("utf8")
    }

    fn read_all(text: &str) -> Result<Vec<(Micros, Vec<u8>)>, RecordingError> {
        RecordingReader::new(text.as_bytes())?.collect()
    }

    fn report_error(text: &str) -> (usize, ReportLineError) {
        match read_all(text) {
            Err(RecordingError::InvalidReport { line, reason }) => (line, reason),
            other => panic!("expected a report error, got {other:?}"),
        }
    }

    fn header_error(text: &str) -> HeaderError {
        match RecordingReader::new(text.as_bytes()) {
            Err(RecordingError::InvalidHeader { line: 1, reason }) => reason,
            Err(other) => panic!("expected a header error, got {other:?}"),
            Ok(_) => panic!("expected a header error, got a reader"),
        }
    }

    #[test]
    fn writer_output_is_line_based_lowercase_hex() {
        let text = write(&[(0, &[0x12, 0x00, 0xab]), (11_111, &[0xff])]);
        assert_eq!(text, format!("{HEADER_LINE}\n0 1200ab\n11111 ff\n"));
    }

    #[test]
    fn round_trip_preserves_header_and_reports() {
        let reports: Vec<(u64, Vec<u8>)> = vec![
            (0, vec![0x12, 1, 2, 3]),
            (11_000, (0..=255).collect()),
            (11_000, vec![0x00]),
            (u64::MAX, vec![0xde, 0xad, 0xbe, 0xef]),
        ];
        let borrowed: Vec<(u64, &[u8])> = reports
            .iter()
            .map(|(t, bytes)| (*t, bytes.as_slice()))
            .collect();
        let text = write(&borrowed);
        let reader = RecordingReader::new(text.as_bytes()).expect("reader");
        assert_eq!(reader.header(), &header());
        let read: Vec<(Micros, Vec<u8>)> = reader.collect::<Result<_, _>>().expect("reports");
        let expected: Vec<(Micros, Vec<u8>)> = reports
            .into_iter()
            .map(|(t, bytes)| (Micros::new(t), bytes))
            .collect();
        assert_eq!(read, expected);
    }

    #[test]
    fn header_only_recording_has_no_reports() {
        assert_eq!(read_all(&write(&[])).expect("reports"), Vec::new());
    }

    #[test]
    fn reader_tolerates_trailing_whitespace_crlf_and_blank_lines() {
        let text = format!("{HEADER_LINE}  \r\n5 12ab \t\r\n\n   \n7 ff\r\n");
        assert_eq!(
            read_all(&text).expect("reports"),
            vec![
                (Micros::new(5), vec![0x12, 0xab]),
                (Micros::new(7), vec![0xff])
            ]
        );
    }

    #[test]
    fn header_fields_may_come_in_any_order() {
        let text = "magic-driver-recording start=5 kernel=k magic-driver=v profile=p format=1\n";
        let reader = RecordingReader::new(text.as_bytes()).expect("reader");
        assert_eq!(reader.header().start_unix_s, 5);
        assert_eq!(reader.header().profile_id, "p");
    }

    #[test]
    fn malformed_hex_is_rejected_with_line_numbers() {
        let base = format!("{HEADER_LINE}\n0 12\n");
        assert_eq!(
            report_error(&format!("{base}1 123\n")),
            (3, ReportLineError::OddHexLength)
        );
        assert_eq!(
            report_error(&format!("{base}1 12zz\n")),
            (
                3,
                ReportLineError::InvalidHexDigit {
                    position: 2,
                    character: 'z'
                }
            )
        );
        assert_eq!(
            report_error(&format!("{base}\n1 12AB\n")),
            (
                4,
                ReportLineError::InvalidHexDigit {
                    position: 2,
                    character: 'A'
                }
            )
        );
        assert_eq!(
            report_error(&format!("{base}1  12\n")),
            (
                3,
                ReportLineError::InvalidHexDigit {
                    position: 0,
                    character: ' '
                }
            )
        );
        assert_eq!(
            report_error(&format!("{base}1 12 34\n")),
            (
                3,
                ReportLineError::InvalidHexDigit {
                    position: 2,
                    character: ' '
                }
            )
        );
    }

    #[test]
    fn malformed_lines_are_rejected_with_line_numbers() {
        let base = format!("{HEADER_LINE}\n");
        assert_eq!(
            report_error(&format!("{base}1200ab\n")),
            (2, ReportLineError::MissingSeparator)
        );
        assert_eq!(
            report_error(&format!("{base}-1 12\n")),
            (2, ReportLineError::InvalidTimestamp("-1".to_owned()))
        );
        assert_eq!(
            report_error(&format!("{base}+1 12\n")),
            (2, ReportLineError::InvalidTimestamp("+1".to_owned()))
        );
        assert_eq!(
            report_error(&format!("{base} 1 12\n")),
            (2, ReportLineError::InvalidTimestamp(String::new()))
        );
        assert_eq!(
            report_error(&format!("{base}99999999999999999999 12\n")),
            (
                2,
                ReportLineError::InvalidTimestamp("99999999999999999999".to_owned())
            )
        );
        assert_eq!(
            report_error(&format!("{base}1 \n")),
            (2, ReportLineError::MissingSeparator)
        );
        assert_eq!(
            report_error(&format!("{base}5 12\n4 12\n")),
            (
                3,
                ReportLineError::TimestampOutOfOrder {
                    previous: Micros::new(5),
                    t: Micros::new(4)
                }
            )
        );
    }

    #[test]
    fn reader_stops_after_the_first_error() {
        let text = format!("{HEADER_LINE}\n1 zz\n2 12\n");
        let mut reader = RecordingReader::new(text.as_bytes()).expect("reader");
        assert!(matches!(reader.next(), Some(Err(_))));
        assert!(reader.next().is_none());
    }

    #[test]
    fn missing_header_is_rejected() {
        assert!(matches!(
            RecordingReader::new(&b""[..]),
            Err(RecordingError::MissingHeader)
        ));
    }

    #[test]
    fn malformed_headers_are_rejected() {
        assert_eq!(header_error("0 1200\n"), HeaderError::MissingTag);
        assert_eq!(
            header_error(
                "magic-driver-recording format=2 profile=p magic-driver=v kernel=k start=1\n"
            ),
            HeaderError::UnsupportedFormat("2".to_owned())
        );
        assert_eq!(
            header_error("magic-driver-recording format=1 magic-driver=v kernel=k start=1\n"),
            HeaderError::MissingField("profile")
        );
        assert_eq!(
            header_error("magic-driver-recording profile=p magic-driver=v kernel=k start=1\n"),
            HeaderError::MissingField("format")
        );
        assert_eq!(
            header_error(
                "magic-driver-recording format=1 profile=p profile=q magic-driver=v kernel=k start=1\n"
            ),
            HeaderError::DuplicateField("profile".to_owned())
        );
        assert_eq!(
            header_error(
                "magic-driver-recording format=1 profile=p magic-driver=v kernel=k start=1 host=x\n"
            ),
            HeaderError::UnknownField("host".to_owned())
        );
        assert_eq!(
            header_error(
                "magic-driver-recording format=1 profile magic-driver=v kernel=k start=1\n"
            ),
            HeaderError::MalformedField("profile".to_owned())
        );
        assert_eq!(
            header_error(
                "magic-driver-recording format=1 profile=p magic-driver=v kernel=k start=yesterday\n"
            ),
            HeaderError::InvalidStart("yesterday".to_owned())
        );
    }

    #[test]
    fn writer_rejects_header_values_that_cannot_round_trip() {
        for value in ["", "two words", "a=b", "tab\there"] {
            let mut bad = header();
            bad.kernel_version = value.to_owned();
            assert!(matches!(
                RecordingWriter::new(Vec::new(), &bad),
                Err(RecordingError::InvalidHeaderValue {
                    field: "kernel",
                    ..
                })
            ));
        }
    }

    #[test]
    fn writer_rejects_out_of_order_and_empty_reports() {
        let mut writer = RecordingWriter::new(Vec::new(), &header()).expect("header");
        writer.write_report(Micros::new(10), &[1]).expect("report");
        writer
            .write_report(Micros::new(10), &[2])
            .expect("equal time");
        assert!(matches!(
            writer.write_report(Micros::new(9), &[3]),
            Err(RecordingError::TimestampOutOfOrder { .. })
        ));
        assert!(matches!(
            writer.write_report(Micros::new(11), &[]),
            Err(RecordingError::EmptyReport)
        ));
    }

    #[test]
    fn report_line_render_and_parse_are_inverse() {
        let line = render_report_line(Micros::new(42), &[0x00, 0x0f, 0xf0, 0xff]);
        assert_eq!(line, "42 000ff0ff");
        assert_eq!(
            parse_report_line(&line),
            Ok((Micros::new(42), vec![0x00, 0x0f, 0xf0, 0xff]))
        );
    }
}
