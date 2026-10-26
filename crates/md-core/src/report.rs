use crate::bounded::BoundedVec;
use crate::profile::Profile;
use crate::surface::SurfacePoint;

pub const MAX_TOUCHES: usize = 15;

const TOUCH_STATE_MASK: u8 = 0xf0;
const CLICK_MASK: u8 = 0x03;

pub type Touches = BoundedVec<Touch, MAX_TOUCHES>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum TouchState {
    #[default]
    None,
    Hover,
    Start,
    Drag,
    Lift(u8),
    Unknown(u8),
}

impl TouchState {
    pub const fn from_bits(bits: u8) -> TouchState {
        match bits & TOUCH_STATE_MASK {
            0x00 => TouchState::None,
            0x20 => TouchState::Hover,
            0x30 => TouchState::Start,
            0x40 => TouchState::Drag,
            state @ (0x50 | 0x60 | 0x70) => TouchState::Lift(state),
            state => TouchState::Unknown(state),
        }
    }

    pub const fn bits(self) -> u8 {
        match self {
            TouchState::None => 0x00,
            TouchState::Hover => 0x20,
            TouchState::Start => 0x30,
            TouchState::Drag => 0x40,
            TouchState::Lift(state) | TouchState::Unknown(state) => state,
        }
    }

    pub const fn is_down(self) -> bool {
        !matches!(self, TouchState::None)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Touch {
    pub id: u8,
    pub x: i32,
    pub y: i32,
    pub touch_major: u8,
    pub touch_minor: u8,
    pub size: u8,
    pub orientation: i8,
    pub state: TouchState,
}

impl Touch {
    pub const fn position(&self) -> SurfacePoint {
        SurfacePoint::new(self.x, self.y)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Header {
    pub clicks: u8,
    pub dx: i32,
    pub dy: i32,
    pub timestamp: Option<u32>,
}

impl Header {
    pub const fn click_bits(&self) -> u8 {
        self.clicks & CLICK_MASK
    }

    pub const fn is_clicked(&self) -> bool {
        self.click_bits() != 0
    }

    pub const fn left(&self) -> bool {
        self.clicks & 0x01 != 0
    }

    pub const fn right(&self) -> bool {
        self.clicks & 0x02 != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    Short,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub format: ReportFormat,
    pub header: Header,
    pub touches: Touches,
}

impl Report {
    pub const fn has_touch_area(&self) -> bool {
        matches!(self.format, ReportFormat::Full)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("report id {report_id:#04x} is not handled")]
    Ignored { report_id: u8 },
    #[error("report length {len} is not a valid report length")]
    InvalidLength { len: usize },
    #[error("report carries {touches} touches, more than the maximum of {max}")]
    TooManyTouches { touches: usize, max: usize },
}

impl ParseError {
    pub const fn is_malformed(&self) -> bool {
        !matches!(self, ParseError::Ignored { .. })
    }
}

pub fn parse(profile: &Profile, bytes: &[u8]) -> Result<Report, ParseError> {
    let layout = &profile.report;
    let len = bytes.len();
    let Some(&report_id) = bytes.first() else {
        return Err(ParseError::InvalidLength { len });
    };
    if report_id != layout.report_id {
        return Err(ParseError::Ignored { report_id });
    }
    if len == layout.short_report_len {
        return Ok(Report {
            format: ReportFormat::Short,
            header: decode_header(bytes, None)?,
            touches: Touches::new(),
        });
    }
    if len < layout.header_len
        || layout.touch_len == 0
        || !(len - layout.header_len).is_multiple_of(layout.touch_len)
    {
        return Err(ParseError::InvalidLength { len });
    }
    let count = (len - layout.header_len) / layout.touch_len;
    let max = layout.max_touches.min(MAX_TOUCHES);
    if count > max {
        return Err(ParseError::TooManyTouches {
            touches: count,
            max,
        });
    }
    let header = decode_header(bytes, decode_timestamp(bytes))?;
    let mut touches = Touches::new();
    for record in bytes[layout.header_len..].chunks_exact(layout.touch_len) {
        let record: [u8; 8] = record
            .try_into()
            .map_err(|_| ParseError::InvalidLength { len })?;
        touches
            .push(decode_touch(record))
            .map_err(|_| ParseError::TooManyTouches {
                touches: count,
                max,
            })?;
    }
    Ok(Report {
        format: ReportFormat::Full,
        header,
        touches,
    })
}

fn decode_header(bytes: &[u8], timestamp: Option<u32>) -> Result<Header, ParseError> {
    let [_, clicks, x0, x1, y0, y1, ..] = *bytes else {
        return Err(ParseError::InvalidLength { len: bytes.len() });
    };
    Ok(Header {
        clicks,
        dx: i32::from(i16::from_le_bytes([x0, x1])),
        dy: i32::from(i16::from_le_bytes([y0, y1])),
        timestamp,
    })
}

fn decode_timestamp(bytes: &[u8]) -> Option<u32> {
    let [t0, t1, t2] = *bytes.get(11..14)? else {
        return None;
    };
    Some(u32::from(t0) >> 6 | u32::from(t1) << 2 | u32::from(t2) << 10)
}

fn decode_touch(tdata: [u8; 8]) -> Touch {
    let t = |index: usize| u32::from(tdata[index]);
    let x = (t(1) << 28 | t(0) << 20).cast_signed() >> 20;
    let y = -((t(2) << 24 | t(1) << 16).cast_signed() >> 20);
    let id = (tdata[6] << 2 | tdata[5] >> 6) & 0x0f;
    let orientation = (tdata[6] >> 2).cast_signed() - 32;
    Touch {
        id,
        x,
        y,
        touch_major: tdata[3],
        touch_minor: tdata[4],
        size: tdata[5] & 0x3f,
        orientation,
        state: TouchState::from_bits(tdata[7]),
    }
}

#[cfg(test)]
mod tests;
