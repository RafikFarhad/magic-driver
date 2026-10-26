use std::collections::BTreeMap;
use std::path::PathBuf;

use md_core::synth::{Contact, Frame, TOUCHPAD_SLOTS, TouchpadPoint, VIRTUAL_TOUCHPAD};

use super::*;
use crate::io::device_tests_enabled;

#[derive(Debug, Default)]
struct MemorySink {
    writes: Vec<Vec<EvdevEvent>>,
}

impl EventSink for MemorySink {
    fn write_events(&mut self, events: &[EvdevEvent]) -> Result<(), DeviceError> {
        self.writes.push(events.to_vec());
        Ok(())
    }
}

fn abs_event(code: AbsoluteAxisCode, value: i32) -> EvdevEvent {
    EvdevEvent::new(EV_ABS, code.0, value)
}

fn key_event(code: KeyCode, value: i32) -> EvdevEvent {
    EvdevEvent::new(EV_KEY, code.0, value)
}

#[allow(clippy::unnecessary_wraps)]
fn contact(id: u16, x: i32, y: i32) -> Option<Contact> {
    Some(Contact::new(id, TouchpadPoint { x, y }))
}

fn frame(slots: [Option<Contact>; TOUCHPAD_SLOTS]) -> Frame {
    Frame { slots }
}

#[derive(Debug, Default)]
struct KernelModel {
    slot: i32,
    tracking: BTreeMap<i32, i32>,
    x: BTreeMap<i32, i32>,
    y: BTreeMap<i32, i32>,
    keys: BTreeMap<u16, i32>,
    abs_x: Option<i32>,
    abs_y: Option<i32>,
}

impl KernelModel {
    fn apply(&mut self, events: &[EvdevEvent]) {
        let (last, body) = events.split_last().expect("non-empty write");
        assert_eq!(*last, SYN_REPORT, "every write ends with SYN_REPORT");
        for event in body {
            assert_ne!(event.kind, EV_SYN, "only one SYN per write");
            match (event.kind, event.code) {
                (EV_ABS, code) if code == AbsoluteAxisCode::ABS_MT_SLOT.0 => {
                    self.slot = event.value;
                }
                (EV_ABS, code) if code == AbsoluteAxisCode::ABS_MT_TRACKING_ID.0 => {
                    self.tracking.insert(self.slot, event.value);
                }
                (EV_ABS, code) if code == AbsoluteAxisCode::ABS_MT_POSITION_X.0 => {
                    self.x.insert(self.slot, event.value);
                }
                (EV_ABS, code) if code == AbsoluteAxisCode::ABS_MT_POSITION_Y.0 => {
                    self.y.insert(self.slot, event.value);
                }
                (EV_ABS, code) if code == AbsoluteAxisCode::ABS_X.0 => {
                    self.abs_x = Some(event.value);
                }
                (EV_ABS, code) if code == AbsoluteAxisCode::ABS_Y.0 => {
                    self.abs_y = Some(event.value);
                }
                (EV_KEY, code) => {
                    self.keys.insert(code, event.value);
                }
                other => panic!("unexpected event {other:?}"),
            }
        }
    }

    fn key(&self, code: KeyCode) -> i32 {
        self.keys.get(&code.0).copied().unwrap_or(0)
    }

    fn assert_matches(&self, frame: &Frame) {
        for (slot, expected) in frame.slots.iter().enumerate() {
            let slot = i32::try_from(slot).unwrap();
            let tracking = self.tracking.get(&slot).copied().unwrap_or(-1);
            match expected {
                Some(contact) => {
                    assert_eq!(tracking, i32::from(contact.tracking_id));
                    assert_eq!(self.x.get(&slot), Some(&contact.x));
                    assert_eq!(self.y.get(&slot), Some(&contact.y));
                }
                None => assert_eq!(tracking, -1, "slot {slot} closed"),
            }
        }
        let count = frame.contact_count();
        assert_eq!(self.key(KeyCode::BTN_TOUCH), i32::from(count > 0));
        for (index, tool) in TOOL_KEYS.iter().enumerate() {
            assert_eq!(self.key(*tool), i32::from(count == index + 1), "{tool:?}");
        }
        if let Some((_, first)) = frame.contacts().next() {
            assert_eq!(self.abs_x, Some(first.x));
            assert_eq!(self.abs_y, Some(first.y));
        }
        assert_eq!(self.key(KeyCode::BTN_LEFT), 0);
    }
}

#[test]
fn first_contact_opens_slot_and_sets_touch_and_one_finger_tool() {
    let mut touchpad = EncodedTouchpad::new(MemorySink::default());
    let first = frame([contact(7, 8000, 8000), None, None, None, None]);
    touchpad.frame(&first).unwrap();
    assert_eq!(
        touchpad.sink_mut().writes,
        vec![vec![
            abs_event(AbsoluteAxisCode::ABS_MT_SLOT, 0),
            abs_event(AbsoluteAxisCode::ABS_MT_TRACKING_ID, 7),
            abs_event(AbsoluteAxisCode::ABS_MT_POSITION_X, 8000),
            abs_event(AbsoluteAxisCode::ABS_MT_POSITION_Y, 8000),
            abs_event(AbsoluteAxisCode::ABS_X, 8000),
            abs_event(AbsoluteAxisCode::ABS_Y, 8000),
            key_event(KeyCode::BTN_TOUCH, 1),
            key_event(KeyCode::BTN_TOOL_FINGER, 1),
            SYN_REPORT,
        ]]
    );
}

#[test]
fn moving_contact_emits_only_changed_axes() {
    let mut touchpad = EncodedTouchpad::new(MemorySink::default());
    touchpad
        .frame(&frame([contact(7, 8000, 8000), None, None, None, None]))
        .unwrap();
    touchpad
        .frame(&frame([contact(7, 8000, 8120), None, None, None, None]))
        .unwrap();
    assert_eq!(
        touchpad.sink_mut().writes[1],
        vec![
            abs_event(AbsoluteAxisCode::ABS_MT_POSITION_Y, 8120),
            abs_event(AbsoluteAxisCode::ABS_Y, 8120),
            SYN_REPORT,
        ]
    );
}

#[test]
fn second_finger_switches_tool_from_finger_to_doubletap() {
    let mut touchpad = EncodedTouchpad::new(MemorySink::default());
    touchpad
        .frame(&frame([contact(1, 7600, 8000), None, None, None, None]))
        .unwrap();
    touchpad
        .frame(&frame([
            contact(1, 7600, 8000),
            contact(2, 8400, 8000),
            None,
            None,
            None,
        ]))
        .unwrap();
    assert_eq!(
        touchpad.sink_mut().writes[1],
        vec![
            abs_event(AbsoluteAxisCode::ABS_MT_SLOT, 1),
            abs_event(AbsoluteAxisCode::ABS_MT_TRACKING_ID, 2),
            abs_event(AbsoluteAxisCode::ABS_MT_POSITION_X, 8400),
            abs_event(AbsoluteAxisCode::ABS_MT_POSITION_Y, 8000),
            key_event(KeyCode::BTN_TOOL_FINGER, 0),
            key_event(KeyCode::BTN_TOOL_DOUBLETAP, 1),
            SYN_REPORT,
        ]
    );
}

#[test]
fn empty_frame_lifts_every_slot_and_releases_touch_and_tool() {
    let mut touchpad = EncodedTouchpad::new(MemorySink::default());
    touchpad
        .frame(&frame([
            contact(1, 7600, 8000),
            contact(2, 8400, 8000),
            None,
            None,
            None,
        ]))
        .unwrap();
    touchpad.frame(&Frame::EMPTY).unwrap();
    assert_eq!(
        touchpad.sink_mut().writes[1],
        vec![
            abs_event(AbsoluteAxisCode::ABS_MT_SLOT, 0),
            abs_event(AbsoluteAxisCode::ABS_MT_TRACKING_ID, -1),
            abs_event(AbsoluteAxisCode::ABS_MT_SLOT, 1),
            abs_event(AbsoluteAxisCode::ABS_MT_TRACKING_ID, -1),
            key_event(KeyCode::BTN_TOUCH, 0),
            key_event(KeyCode::BTN_TOOL_DOUBLETAP, 0),
            SYN_REPORT,
        ]
    );
}

#[test]
fn unchanged_frames_write_nothing() {
    let mut touchpad = EncodedTouchpad::new(MemorySink::default());
    touchpad.frame(&Frame::EMPTY).unwrap();
    let one = frame([None, contact(3, 100, 200), None, None, None]);
    touchpad.frame(&one).unwrap();
    touchpad.frame(&one).unwrap();
    assert_eq!(touchpad.sink_mut().writes.len(), 1);
}

#[test]
fn every_finger_count_sets_exactly_one_tool() {
    let mut touchpad = EncodedTouchpad::new(MemorySink::default());
    let mut model = KernelModel::default();
    let mut slots = [None; TOUCHPAD_SLOTS];
    let mut frames = Vec::new();
    for count in 1..=TOUCHPAD_SLOTS {
        let id = u16::try_from(count).unwrap();
        slots[count - 1] = contact(id, 1000 * i32::from(id), 2000);
        frames.push(frame(slots));
    }
    for count in (0..TOUCHPAD_SLOTS).rev() {
        slots[count] = None;
        frames.push(frame(slots));
    }
    for next in &frames {
        touchpad.frame(next).unwrap();
        let write = touchpad.sink_mut().writes.last().unwrap().clone();
        model.apply(&write);
        model.assert_matches(next);
    }
}

#[test]
fn kernel_state_follows_every_frame_of_a_long_sequence() {
    let mut touchpad = EncodedTouchpad::new(MemorySink::default());
    let mut model = KernelModel::default();
    let mut current = Frame::EMPTY;
    let mut next_id: u16 = 0;
    let mut seed: u32 = 0x1234_5678;
    let mut random = move |bound: u32| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed % bound
    };
    for _ in 0..2_000 {
        let mut next = current;
        for slot in &mut next.slots {
            match (*slot, random(6)) {
                (None, 0) => {
                    *slot = Some(Contact::new(
                        next_id,
                        TouchpadPoint {
                            x: i32::try_from(random(16_001)).unwrap(),
                            y: i32::try_from(random(16_001)).unwrap(),
                        },
                    ));
                    next_id = next_id.wrapping_add(1);
                }
                (Some(_), 0) => *slot = None,
                (Some(contact), 1 | 2) => {
                    *slot = Some(Contact::new(
                        contact.tracking_id,
                        TouchpadPoint {
                            x: (contact.x + i32::try_from(random(41)).unwrap() - 20)
                                .clamp(0, 16_000),
                            y: (contact.y + i32::try_from(random(41)).unwrap() - 20)
                                .clamp(0, 16_000),
                        },
                    ));
                }
                _ => {}
            }
        }
        let writes_before = touchpad.sink_mut().writes.len();
        touchpad.frame(&next).unwrap();
        let writes = &touchpad.sink_mut().writes;
        if next == current {
            assert_eq!(writes.len(), writes_before);
        } else {
            assert_eq!(writes.len(), writes_before + 1);
            model.apply(writes.last().unwrap());
        }
        model.assert_matches(&next);
        current = next;
    }
}

#[test]
fn pointer_motion_and_buttons_end_with_syn_report() {
    let mut pointer = EncodedPointer::new(MemorySink::default());
    pointer.motion(3, -2).unwrap();
    pointer.motion(0, 5).unwrap();
    pointer.motion(0, 0).unwrap();
    pointer.button(Button::Left, true).unwrap();
    pointer.button(Button::Right, false).unwrap();
    pointer.button(Button::Middle, true).unwrap();
    assert_eq!(
        pointer.sink_mut().writes,
        vec![
            vec![
                EvdevEvent::new(EV_REL, RelativeAxisCode::REL_X.0, 3),
                EvdevEvent::new(EV_REL, RelativeAxisCode::REL_Y.0, -2),
                SYN_REPORT,
            ],
            vec![
                EvdevEvent::new(EV_REL, RelativeAxisCode::REL_Y.0, 5),
                SYN_REPORT,
            ],
            vec![key_event(KeyCode::BTN_LEFT, 1), SYN_REPORT],
            vec![key_event(KeyCode::BTN_RIGHT, 0), SYN_REPORT],
            vec![key_event(KeyCode::BTN_MIDDLE, 1), SYN_REPORT],
        ]
    );
}

#[test]
fn touchpad_axes_use_the_core_geometry() {
    let axes = touchpad_axes();
    let find = |code| *axes.iter().find(|setup| setup.code == code).unwrap();
    for code in [AbsoluteAxisCode::ABS_X, AbsoluteAxisCode::ABS_MT_POSITION_X] {
        let horizontal = find(code);
        assert_eq!(
            (
                horizontal.minimum,
                horizontal.maximum,
                horizontal.resolution
            ),
            (0, VIRTUAL_TOUCHPAD.max_x, VIRTUAL_TOUCHPAD.units_per_mm)
        );
    }
    for code in [AbsoluteAxisCode::ABS_Y, AbsoluteAxisCode::ABS_MT_POSITION_Y] {
        let vertical = find(code);
        assert_eq!(
            (vertical.minimum, vertical.maximum, vertical.resolution),
            (0, VIRTUAL_TOUCHPAD.max_y, VIRTUAL_TOUCHPAD.units_per_mm)
        );
    }
    let slot = find(AbsoluteAxisCode::ABS_MT_SLOT);
    assert_eq!(
        (slot.minimum, slot.maximum),
        (0, i32::try_from(TOUCHPAD_SLOTS).unwrap() - 1)
    );
    let tracking = find(AbsoluteAxisCode::ABS_MT_TRACKING_ID);
    assert_eq!((tracking.minimum, tracking.maximum), (0, 65_535));
    let width_mm = f64::from(find(AbsoluteAxisCode::ABS_X).maximum)
        / f64::from(find(AbsoluteAxisCode::ABS_X).resolution);
    assert!((width_mm - 400.0).abs() < 1e-9);
}

#[test]
fn touchpad_declares_no_pressure_or_size_axes() {
    let codes: Vec<_> = touchpad_axes().iter().map(|axis| axis.code).collect();
    for absent in [
        AbsoluteAxisCode::ABS_PRESSURE,
        AbsoluteAxisCode::ABS_MT_PRESSURE,
        AbsoluteAxisCode::ABS_MT_TOUCH_MAJOR,
        AbsoluteAxisCode::ABS_MT_TOUCH_MINOR,
        AbsoluteAxisCode::ABS_MT_WIDTH_MAJOR,
    ] {
        assert!(!codes.contains(&absent), "{absent:?}");
    }
}

#[test]
fn identities_are_fixed_virtual_and_not_apple() {
    for identity in [POINTER_IDENTITY, TOUCHPAD_IDENTITY] {
        assert_eq!(identity.bus, 0x06);
        assert_ne!(identity.vendor, md_core::profile::VENDOR_APPLE_BLUETOOTH);
        assert_ne!(identity.vendor, 0x05ac);
        assert_eq!(identity.vendor, POINTER_IDENTITY.vendor);
    }
    assert_ne!(POINTER_IDENTITY.product, TOUCHPAD_IDENTITY.product);
    assert_eq!(POINTER_IDENTITY.name, "magic-driver pointer");
    assert_eq!(TOUCHPAD_IDENTITY.name, "magic-driver touchpad");
}

#[test]
fn tool_keys_match_the_slot_count() {
    assert_eq!(TOOL_KEYS.len(), TOUCHPAD_SLOTS);
    assert_eq!(tool_key(0), None);
    assert_eq!(tool_key(1), Some(KeyCode::BTN_TOOL_FINGER));
    assert_eq!(tool_key(5), Some(KeyCode::BTN_TOOL_QUINTTAP));
    assert_eq!(tool_key(6), None);
    for tool in TOOL_KEYS {
        assert!(TOUCHPAD_KEYS.contains(&tool));
    }
    assert!(TOUCHPAD_KEYS.contains(&KeyCode::BTN_LEFT));
    assert!(!TOUCHPAD_KEYS.contains(&KeyCode::BTN_RIGHT));
}

fn wait_for_event_node(sink: &mut UinputSink) -> PathBuf {
    for _ in 0..100 {
        if let Some(node) = sink.dev_nodes().unwrap().into_iter().next()
            && node.exists()
        {
            return node;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("virtual device node did not appear");
}

#[test]
#[ignore = "creates uinput devices; owner or isolated VM only, needs MAGIC_DRIVER_DEVICE_TESTS=1"]
fn device_virtual_devices_have_the_declared_capabilities() {
    if !device_tests_enabled() {
        eprintln!("skipped: set MAGIC_DRIVER_DEVICE_TESTS=1");
        return;
    }
    let mut pointer = UinputPointer::create().expect("create pointer");
    let mut touchpad = UinputTouchpad::create().expect("create touchpad");

    let node = wait_for_event_node(pointer.sink_mut());
    let device = evdev::Device::open(&node).expect("open pointer node");
    assert_eq!(device.name(), Some(POINTER_IDENTITY.name));
    assert_eq!(device.input_id(), POINTER_IDENTITY.input_id());
    let axes: Vec<_> = device.supported_relative_axes().unwrap().iter().collect();
    assert_eq!(axes, POINTER_AXES.to_vec());
    let keys: Vec<_> = device.supported_keys().unwrap().iter().collect();
    assert_eq!(keys, POINTER_BUTTONS.to_vec());
    assert!(device.supported_absolute_axes().is_none());

    let node = wait_for_event_node(touchpad.sink_mut());
    let device = evdev::Device::open(&node).expect("open touchpad node");
    assert_eq!(device.name(), Some(TOUCHPAD_IDENTITY.name));
    assert_eq!(device.input_id(), TOUCHPAD_IDENTITY.input_id());
    let keys: Vec<_> = device.supported_keys().unwrap().iter().collect();
    let mut expected_keys = TOUCHPAD_KEYS.to_vec();
    expected_keys.sort_by_key(|key| key.0);
    assert_eq!(keys, expected_keys);
    assert!(device.supported_relative_axes().is_none());
    assert!(device.properties().contains(PropType::POINTER));
    let absinfo: BTreeMap<u16, (i32, i32, i32)> = device
        .get_absinfo()
        .unwrap()
        .map(|(code, info)| (code.0, (info.minimum(), info.maximum(), info.resolution())))
        .collect();
    let expected: BTreeMap<u16, (i32, i32, i32)> = touchpad_axes()
        .iter()
        .map(|axis| (axis.code.0, (axis.minimum, axis.maximum, axis.resolution)))
        .collect();
    assert_eq!(absinfo, expected);

    touchpad
        .frame(&frame([contact(1, 8000, 8000), None, None, None, None]))
        .unwrap();
    touchpad.frame(&Frame::EMPTY).unwrap();
    pointer.motion(1, 1).unwrap();
    touchpad.destroy();
    pointer.destroy();
}
