use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, BusType, EventType, InputEvent, InputId, KeyCode,
    PropType, RelativeAxisCode, SynchronizationCode, UinputAbsSetup,
};
use md_core::Button;
use md_core::synth::{Frame, TOUCHPAD_SLOTS, TouchpadPoint, VIRTUAL_TOUCHPAD};

use super::{DeviceError, EvdevEvent, VirtualPointer, VirtualTouchpad};

pub const VIRTUAL_BUS: u16 = BusType::BUS_VIRTUAL.0;
pub const VIRTUAL_VENDOR: u16 = 0x4d44;
pub const VIRTUAL_VERSION: u16 = 0x0001;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualDeviceIdentity {
    pub name: &'static str,
    pub bus: u16,
    pub vendor: u16,
    pub product: u16,
    pub version: u16,
}

impl VirtualDeviceIdentity {
    pub fn input_id(&self) -> InputId {
        InputId::new(BusType(self.bus), self.vendor, self.product, self.version)
    }
}

pub const POINTER_IDENTITY: VirtualDeviceIdentity = VirtualDeviceIdentity {
    name: "magic-driver pointer",
    bus: VIRTUAL_BUS,
    vendor: VIRTUAL_VENDOR,
    product: 0x0001,
    version: VIRTUAL_VERSION,
};

pub const TOUCHPAD_IDENTITY: VirtualDeviceIdentity = VirtualDeviceIdentity {
    name: "magic-driver touchpad",
    bus: VIRTUAL_BUS,
    vendor: VIRTUAL_VENDOR,
    product: 0x0002,
    version: VIRTUAL_VERSION,
};

const EV_SYN: u16 = EventType::SYNCHRONIZATION.0;
const EV_KEY: u16 = EventType::KEY.0;
const EV_REL: u16 = EventType::RELATIVE.0;
const EV_ABS: u16 = EventType::ABSOLUTE.0;

pub const SYN_REPORT: EvdevEvent = EvdevEvent::new(EV_SYN, SynchronizationCode::SYN_REPORT.0, 0);

pub const POINTER_BUTTONS: [KeyCode; 3] =
    [KeyCode::BTN_LEFT, KeyCode::BTN_RIGHT, KeyCode::BTN_MIDDLE];
pub const POINTER_AXES: [RelativeAxisCode; 2] = [RelativeAxisCode::REL_X, RelativeAxisCode::REL_Y];

pub const TOOL_KEYS: [KeyCode; TOUCHPAD_SLOTS] = [
    KeyCode::BTN_TOOL_FINGER,
    KeyCode::BTN_TOOL_DOUBLETAP,
    KeyCode::BTN_TOOL_TRIPLETAP,
    KeyCode::BTN_TOOL_QUADTAP,
    KeyCode::BTN_TOOL_QUINTTAP,
];

pub const TOUCHPAD_KEYS: [KeyCode; 7] = [
    KeyCode::BTN_LEFT,
    KeyCode::BTN_TOUCH,
    KeyCode::BTN_TOOL_FINGER,
    KeyCode::BTN_TOOL_DOUBLETAP,
    KeyCode::BTN_TOOL_TRIPLETAP,
    KeyCode::BTN_TOOL_QUADTAP,
    KeyCode::BTN_TOOL_QUINTTAP,
];

pub const TOUCHPAD_PROPERTIES: [PropType; 1] = [PropType::POINTER];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisSetup {
    pub code: AbsoluteAxisCode,
    pub minimum: i32,
    pub maximum: i32,
    pub resolution: i32,
}

impl AxisSetup {
    fn uinput(&self) -> UinputAbsSetup {
        UinputAbsSetup::new(
            self.code,
            AbsInfo::new(
                self.minimum,
                self.minimum,
                self.maximum,
                0,
                0,
                self.resolution,
            ),
        )
    }
}

pub fn touchpad_axes() -> [AxisSetup; 6] {
    let geometry = VIRTUAL_TOUCHPAD;
    let position = |code, maximum| AxisSetup {
        code,
        minimum: 0,
        maximum,
        resolution: geometry.units_per_mm,
    };
    let last_slot = i32::try_from(TOUCHPAD_SLOTS - 1).expect("slot count fits in i32");
    [
        position(AbsoluteAxisCode::ABS_X, geometry.max_x),
        position(AbsoluteAxisCode::ABS_Y, geometry.max_y),
        AxisSetup {
            code: AbsoluteAxisCode::ABS_MT_SLOT,
            minimum: 0,
            maximum: last_slot,
            resolution: 0,
        },
        AxisSetup {
            code: AbsoluteAxisCode::ABS_MT_TRACKING_ID,
            minimum: 0,
            maximum: i32::from(u16::MAX),
            resolution: 0,
        },
        position(AbsoluteAxisCode::ABS_MT_POSITION_X, geometry.max_x),
        position(AbsoluteAxisCode::ABS_MT_POSITION_Y, geometry.max_y),
    ]
}

pub const fn button_code(button: Button) -> KeyCode {
    match button {
        Button::Left => KeyCode::BTN_LEFT,
        Button::Right => KeyCode::BTN_RIGHT,
        Button::Middle => KeyCode::BTN_MIDDLE,
    }
}

pub fn encode_motion(dx: i32, dy: i32, out: &mut Vec<EvdevEvent>) {
    if dx == 0 && dy == 0 {
        return;
    }
    if dx != 0 {
        out.push(EvdevEvent::new(EV_REL, RelativeAxisCode::REL_X.0, dx));
    }
    if dy != 0 {
        out.push(EvdevEvent::new(EV_REL, RelativeAxisCode::REL_Y.0, dy));
    }
    out.push(SYN_REPORT);
}

pub fn encode_button(button: Button, pressed: bool, out: &mut Vec<EvdevEvent>) {
    out.push(EvdevEvent::new(
        EV_KEY,
        button_code(button).0,
        i32::from(pressed),
    ));
    out.push(SYN_REPORT);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchpadEncoder {
    current: Frame,
    slot: Option<usize>,
    contacts: usize,
    pointer: Option<TouchpadPoint>,
}

impl Default for TouchpadEncoder {
    fn default() -> TouchpadEncoder {
        TouchpadEncoder::new()
    }
}

impl TouchpadEncoder {
    pub const fn new() -> TouchpadEncoder {
        TouchpadEncoder {
            current: Frame::EMPTY,
            slot: None,
            contacts: 0,
            pointer: None,
        }
    }

    pub fn encode(&mut self, frame: &Frame, out: &mut Vec<EvdevEvent>) {
        let start = out.len();
        for slot in 0..TOUCHPAD_SLOTS {
            self.encode_slot(slot, frame, out);
        }
        self.encode_pointer_emulation(frame, out);
        self.encode_keys(frame.contact_count(), out);
        self.current = *frame;
        if out.len() > start {
            out.push(SYN_REPORT);
        }
    }

    fn select_slot(&mut self, slot: usize, out: &mut Vec<EvdevEvent>) {
        if self.slot != Some(slot) {
            let value = i32::try_from(slot).expect("slot index fits in i32");
            out.push(abs(AbsoluteAxisCode::ABS_MT_SLOT, value));
            self.slot = Some(slot);
        }
    }

    fn encode_slot(&mut self, slot: usize, frame: &Frame, out: &mut Vec<EvdevEvent>) {
        match (self.current.slots[slot], frame.slots[slot]) {
            (None, None) => {}
            (Some(_), None) => {
                self.select_slot(slot, out);
                out.push(abs(AbsoluteAxisCode::ABS_MT_TRACKING_ID, -1));
            }
            (None, Some(next)) => {
                self.select_slot(slot, out);
                out.push(abs(
                    AbsoluteAxisCode::ABS_MT_TRACKING_ID,
                    i32::from(next.tracking_id),
                ));
                out.push(abs(AbsoluteAxisCode::ABS_MT_POSITION_X, next.x));
                out.push(abs(AbsoluteAxisCode::ABS_MT_POSITION_Y, next.y));
            }
            (Some(previous), Some(next)) => {
                let new_contact = previous.tracking_id != next.tracking_id;
                if new_contact || previous.x != next.x || previous.y != next.y {
                    self.select_slot(slot, out);
                }
                if new_contact {
                    out.push(abs(
                        AbsoluteAxisCode::ABS_MT_TRACKING_ID,
                        i32::from(next.tracking_id),
                    ));
                }
                if new_contact || previous.x != next.x {
                    out.push(abs(AbsoluteAxisCode::ABS_MT_POSITION_X, next.x));
                }
                if new_contact || previous.y != next.y {
                    out.push(abs(AbsoluteAxisCode::ABS_MT_POSITION_Y, next.y));
                }
            }
        }
    }

    fn encode_pointer_emulation(&mut self, frame: &Frame, out: &mut Vec<EvdevEvent>) {
        let Some((_, first)) = frame.contacts().next() else {
            return;
        };
        let point = first.point();
        if self.pointer.is_none_or(|previous| previous.x != point.x) {
            out.push(abs(AbsoluteAxisCode::ABS_X, point.x));
        }
        if self.pointer.is_none_or(|previous| previous.y != point.y) {
            out.push(abs(AbsoluteAxisCode::ABS_Y, point.y));
        }
        self.pointer = Some(point);
    }

    fn encode_keys(&mut self, contacts: usize, out: &mut Vec<EvdevEvent>) {
        if contacts == self.contacts {
            return;
        }
        if (contacts > 0) != (self.contacts > 0) {
            out.push(key(KeyCode::BTN_TOUCH, contacts > 0));
        }
        if let Some(previous) = tool_key(self.contacts) {
            out.push(key(previous, false));
        }
        if let Some(next) = tool_key(contacts) {
            out.push(key(next, true));
        }
        self.contacts = contacts;
    }
}

fn tool_key(contacts: usize) -> Option<KeyCode> {
    contacts
        .checked_sub(1)
        .and_then(|index| TOOL_KEYS.get(index).copied())
}

fn abs(code: AbsoluteAxisCode, value: i32) -> EvdevEvent {
    EvdevEvent::new(EV_ABS, code.0, value)
}

fn key(code: KeyCode, pressed: bool) -> EvdevEvent {
    EvdevEvent::new(EV_KEY, code.0, i32::from(pressed))
}

pub trait EventSink {
    fn write_events(&mut self, events: &[EvdevEvent]) -> Result<(), DeviceError>;
}

#[derive(Debug)]
pub struct EncodedPointer<S: EventSink> {
    sink: S,
    events: Vec<EvdevEvent>,
}

impl<S: EventSink> EncodedPointer<S> {
    pub fn new(sink: S) -> EncodedPointer<S> {
        EncodedPointer {
            sink,
            events: Vec::with_capacity(8),
        }
    }

    #[cfg(test)]
    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    fn flush(&mut self) -> Result<(), DeviceError> {
        let result = if self.events.is_empty() {
            Ok(())
        } else {
            self.sink.write_events(&self.events)
        };
        self.events.clear();
        result
    }
}

impl<S: EventSink> VirtualPointer for EncodedPointer<S> {
    fn motion(&mut self, dx: i32, dy: i32) -> Result<(), DeviceError> {
        encode_motion(dx, dy, &mut self.events);
        self.flush()
    }

    fn button(&mut self, button: Button, pressed: bool) -> Result<(), DeviceError> {
        encode_button(button, pressed, &mut self.events);
        self.flush()
    }
}

#[derive(Debug)]
pub struct EncodedTouchpad<S: EventSink> {
    sink: S,
    encoder: TouchpadEncoder,
    events: Vec<EvdevEvent>,
}

impl<S: EventSink> EncodedTouchpad<S> {
    pub fn new(sink: S) -> EncodedTouchpad<S> {
        EncodedTouchpad {
            sink,
            encoder: TouchpadEncoder::new(),
            events: Vec::with_capacity(64),
        }
    }

    #[cfg(test)]
    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }
}

impl<S: EventSink> VirtualTouchpad for EncodedTouchpad<S> {
    fn frame(&mut self, frame: &Frame) -> Result<(), DeviceError> {
        self.encoder.encode(frame, &mut self.events);
        let result = if self.events.is_empty() {
            Ok(())
        } else {
            self.sink.write_events(&self.events)
        };
        self.events.clear();
        result
    }
}

#[derive(Debug)]
pub struct UinputSink {
    device: VirtualDevice,
    buffer: Vec<InputEvent>,
}

impl UinputSink {
    #[cfg(test)]
    pub fn dev_nodes(&mut self) -> Result<Vec<std::path::PathBuf>, DeviceError> {
        let mut nodes = Vec::new();
        for node in self.device.enumerate_dev_nodes_blocking()? {
            nodes.push(node?);
        }
        Ok(nodes)
    }
}

impl EventSink for UinputSink {
    fn write_events(&mut self, events: &[EvdevEvent]) -> Result<(), DeviceError> {
        let Some((last, body)) = events.split_last() else {
            return Ok(());
        };
        debug_assert_eq!(*last, SYN_REPORT);
        self.buffer.clear();
        self.buffer.extend(
            body.iter()
                .map(|event| InputEvent::new(event.kind, event.code, event.value)),
        );
        self.device.emit(&self.buffer).map_err(DeviceError::from)
    }
}

pub type UinputPointer = EncodedPointer<UinputSink>;
pub type UinputTouchpad = EncodedTouchpad<UinputSink>;

impl UinputPointer {
    pub fn create() -> Result<UinputPointer, DeviceError> {
        let keys: AttributeSet<KeyCode> = POINTER_BUTTONS.iter().collect();
        let axes: AttributeSet<RelativeAxisCode> = POINTER_AXES.iter().collect();
        let device = VirtualDevice::builder()?
            .name(POINTER_IDENTITY.name)
            .input_id(POINTER_IDENTITY.input_id())
            .with_keys(&keys)?
            .with_relative_axes(&axes)?
            .build()?;
        Ok(EncodedPointer::new(UinputSink {
            device,
            buffer: Vec::with_capacity(8),
        }))
    }
}

impl UinputTouchpad {
    pub fn create() -> Result<UinputTouchpad, DeviceError> {
        let keys: AttributeSet<KeyCode> = TOUCHPAD_KEYS.iter().collect();
        let properties: AttributeSet<PropType> = TOUCHPAD_PROPERTIES.iter().collect();
        let mut builder = VirtualDevice::builder()?
            .name(TOUCHPAD_IDENTITY.name)
            .input_id(TOUCHPAD_IDENTITY.input_id())
            .with_keys(&keys)?
            .with_properties(&properties)?;
        for axis in touchpad_axes() {
            builder = builder.with_absolute_axis(&axis.uinput())?;
        }
        Ok(EncodedTouchpad::new(UinputSink {
            device: builder.build()?,
            buffer: Vec::with_capacity(64),
        }))
    }
}

#[cfg(test)]
mod tests;
