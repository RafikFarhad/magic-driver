use crate::profile::{MAGIC_MOUSE_2_BT, ReportLayout};

const LAYOUT: ReportLayout = MAGIC_MOUSE_2_BT.report;
const TIMESTAMP_LIMIT: u32 = 1 << 18;

pub mod touch_state {
    pub const GONE: u8 = 0x00;
    pub const HOVER: u8 = 0x20;
    pub const START: u8 = 0x30;
    pub const DRAG: u8 = 0x40;
    pub const LIFT_50: u8 = 0x50;
    pub const LIFT_60: u8 = 0x60;
    pub const LIFT_70: u8 = 0x70;
}

#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TouchRecord {
    pub id: u8,
    pub x: i32,
    pub y: i32,
    pub touch_major: u8,
    pub touch_minor: u8,
    pub size: u8,
    pub orientation: i8,
    pub state: u8,
}

impl TouchRecord {
    pub fn new(id: u8, x: i32, y: i32) -> Self {
        TouchRecord {
            id,
            x,
            y,
            touch_major: 40,
            touch_minor: 32,
            size: 12,
            orientation: 0,
            state: touch_state::DRAG,
        }
    }

    pub fn state(mut self, state: u8) -> Self {
        self.state = state;
        self
    }

    pub fn touch_major(mut self, touch_major: u8) -> Self {
        self.touch_major = touch_major;
        self
    }

    pub fn touch_minor(mut self, touch_minor: u8) -> Self {
        self.touch_minor = touch_minor;
        self
    }

    pub fn size(mut self, size: u8) -> Self {
        self.size = size;
        self
    }

    pub fn orientation(mut self, orientation: i8) -> Self {
        self.orientation = orientation;
        self
    }

    pub fn encode(&self) -> [u8; 8] {
        assert!(self.id < 16, "touch id {} does not fit in 4 bits", self.id);
        assert!(
            (-2048..=2047).contains(&self.x),
            "touch x {} does not fit in 12 signed bits",
            self.x
        );
        let raw_y = -self.y;
        assert!(
            (-2048..=2047).contains(&raw_y),
            "touch y {} does not fit in 12 signed bits after inversion",
            self.y
        );
        assert!(
            self.size < 64,
            "touch size {} does not fit in 6 bits",
            self.size
        );
        assert!(
            (-32..=31).contains(&self.orientation),
            "touch orientation {} is outside -32..=31",
            self.orientation
        );
        let x = twelve_bits(self.x);
        let y = twelve_bits(raw_y);
        let orientation = u8::try_from(i16::from(self.orientation) + 32).expect("checked range");
        [
            low_byte(x),
            low_byte(x >> 8) | (low_byte(y) << 4),
            low_byte(y >> 4),
            self.touch_major,
            self.touch_minor,
            self.size | ((self.id & 0x03) << 6),
            (orientation << 2) | ((self.id >> 2) & 0x03),
            self.state,
        ]
    }
}

fn twelve_bits(value: i32) -> u16 {
    u16::try_from(value & 0x0fff).expect("masked to 12 bits")
}

fn low_byte(value: u16) -> u8 {
    u8::try_from(value & 0x00ff).expect("masked to 8 bits")
}

#[must_use]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReportBuilder {
    clicks: u8,
    dx: i16,
    dy: i16,
    timestamp: u32,
    touches: Vec<TouchRecord>,
}

impl ReportBuilder {
    pub fn new() -> Self {
        ReportBuilder::default()
    }

    pub fn clicks(mut self, clicks: u8) -> Self {
        self.clicks = clicks;
        self
    }

    pub fn left(mut self, pressed: bool) -> Self {
        self.clicks = (self.clicks & !0x01) | u8::from(pressed);
        self
    }

    pub fn right(mut self, pressed: bool) -> Self {
        self.clicks = (self.clicks & !0x02) | (u8::from(pressed) << 1);
        self
    }

    pub fn motion(mut self, dx: i16, dy: i16) -> Self {
        self.dx = dx;
        self.dy = dy;
        self
    }

    pub fn timestamp(mut self, timestamp: u32) -> Self {
        assert!(
            timestamp < TIMESTAMP_LIMIT,
            "timestamp {timestamp} does not fit in 18 bits"
        );
        self.timestamp = timestamp;
        self
    }

    pub fn touch(mut self, touch: TouchRecord) -> Self {
        self.touches.push(touch);
        self
    }

    pub fn build(&self) -> Vec<u8> {
        assert!(
            self.touches.len() <= LAYOUT.max_touches,
            "{} touches exceed the profile maximum of {}",
            self.touches.len(),
            LAYOUT.max_touches
        );
        let mut bytes = vec![0_u8; LAYOUT.header_len];
        bytes[0] = LAYOUT.report_id;
        bytes[1] = self.clicks;
        bytes[2..4].copy_from_slice(&self.dx.to_le_bytes());
        bytes[4..6].copy_from_slice(&self.dy.to_le_bytes());
        let timestamp = self.timestamp.to_le_bytes();
        bytes[11] = (timestamp[0] & 0x03) << 6;
        bytes[12] = u8::try_from((self.timestamp >> 2) & 0xff).expect("masked to 8 bits");
        bytes[13] = u8::try_from((self.timestamp >> 10) & 0xff).expect("masked to 8 bits");
        for touch in &self.touches {
            bytes.extend_from_slice(&touch.encode());
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    struct KernelTouch {
        id: i32,
        x: i32,
        y: i32,
        size: i32,
        orientation: i32,
        touch_major: i32,
        touch_minor: i32,
        state: i32,
    }

    fn kernel_decode_touch(tdata: &[u8]) -> KernelTouch {
        let t = |index: usize| i32::from(tdata[index]);
        KernelTouch {
            id: (t(6) << 2 | t(5) >> 6) & 0xf,
            x: (t(1) << 28 | t(0) << 20) >> 20,
            y: -((t(2) << 24 | t(1) << 16) >> 20),
            size: t(5) & 0x3f,
            orientation: (t(6) >> 2) - 32,
            touch_major: t(3),
            touch_minor: t(4),
            state: t(7) & 0xf0,
        }
    }

    fn kernel_decode_header(data: &[u8]) -> (i32, i32, i32, i32) {
        let d = |index: usize| i32::from(data[index]);
        let x = ((d(3) << 24) | (d(2) << 16)) >> 16;
        let y = ((d(5) << 24) | (d(4) << 16)) >> 16;
        let clicks = d(1);
        let timestamp = d(11) >> 6 | d(12) << 2 | d(13) << 10;
        (clicks, x, y, timestamp)
    }

    fn expected(touch: &TouchRecord) -> KernelTouch {
        KernelTouch {
            id: i32::from(touch.id),
            x: touch.x,
            y: touch.y,
            size: i32::from(touch.size),
            orientation: i32::from(touch.orientation),
            touch_major: i32::from(touch.touch_major),
            touch_minor: i32::from(touch.touch_minor),
            state: i32::from(touch.state),
        }
    }

    #[test]
    fn empty_report_is_header_only() {
        let bytes = ReportBuilder::new().build();
        assert_eq!(bytes.len(), 14);
        assert_eq!(bytes[0], 0x12);
        assert_eq!(kernel_decode_header(&bytes), (0, 0, 0, 0));
    }

    #[test]
    fn header_fields_decode_like_the_kernel() {
        for (dx, dy) in [(0, 0), (1, -1), (-300, 450), (i16::MIN, i16::MAX)] {
            for timestamp in [0, 1, 3, 4, 0x3_ffff, 0x2_a5a5] {
                let bytes = ReportBuilder::new()
                    .left(true)
                    .right(true)
                    .motion(dx, dy)
                    .timestamp(timestamp)
                    .build();
                assert_eq!(
                    kernel_decode_header(&bytes),
                    (
                        3,
                        i32::from(dx),
                        i32::from(dy),
                        i32::try_from(timestamp).unwrap()
                    )
                );
            }
        }
    }

    #[test]
    fn click_bits_toggle_independently() {
        let builder = ReportBuilder::new().left(true).right(true).left(false);
        assert_eq!(builder.build()[1], 0x02);
        assert_eq!(ReportBuilder::new().clicks(0x81).build()[1], 0x81);
    }

    #[test]
    fn touches_decode_like_the_kernel() {
        let touches = [
            TouchRecord::new(0, 0, 0),
            TouchRecord::new(15, -1100, 2047).state(touch_state::START),
            TouchRecord::new(5, 1258, -1589)
                .state(touch_state::LIFT_70)
                .orientation(-32)
                .size(63),
            TouchRecord::new(10, -2048, -2047)
                .orientation(31)
                .touch_major(255)
                .touch_minor(0)
                .state(touch_state::HOVER),
            TouchRecord::new(3, 2047, 2048).state(touch_state::GONE),
            TouchRecord::new(6, -1, 1).size(0),
        ];
        let builder = touches
            .iter()
            .fold(ReportBuilder::new(), |builder, touch| builder.touch(*touch));
        let bytes = builder.build();
        assert_eq!(bytes.len(), 14 + 8 * touches.len());
        for (index, touch) in touches.iter().enumerate() {
            let start = 14 + index * 8;
            assert_eq!(
                kernel_decode_touch(&bytes[start..start + 8]),
                expected(touch),
                "touch {index}"
            );
        }
    }

    #[test]
    fn profile_maximum_of_touches_fits() {
        let builder = (0..15).fold(ReportBuilder::new(), |builder, id| {
            builder.touch(TouchRecord::new(id, i32::from(id) * 10, 0))
        });
        assert_eq!(builder.build().len(), 14 + 8 * 15);
    }

    #[test]
    #[should_panic(expected = "exceed the profile maximum")]
    fn too_many_touches_panic() {
        let builder = (0..16).fold(ReportBuilder::new(), |builder, id| {
            builder.touch(TouchRecord::new(id, 0, 0))
        });
        let _ = builder.build();
    }

    #[test]
    #[should_panic(expected = "12 signed bits")]
    fn out_of_range_x_panics() {
        let _ = TouchRecord::new(0, 2048, 0).encode();
    }

    #[test]
    #[should_panic(expected = "18 bits")]
    fn out_of_range_timestamp_panics() {
        let _ = ReportBuilder::new().timestamp(1 << 18);
    }
}
