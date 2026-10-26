use md_core::profile::MAGIC_MOUSE_2_BT;

use super::*;

const MAGIC_MOUSE: &str = include_str!("fixtures/magic-mouse-2-bt.udev");
const USB_MOUSE: &str = include_str!("fixtures/usb-optical-mouse.udev");

const MM_HID: &str = "/devices/pci0000:00/0000:00:08.1/0000:c5:00.3/usb1/1-2/1-2.3/1-2.3:1.0/bluetooth/hci0/hci0:256/0005:004C:0269.0006";

fn classify_all(records: &[UdevRecord]) -> Vec<MouseNode> {
    records
        .iter()
        .filter_map(|record| classify(record, hid_parent_in(record, records)?))
        .collect()
}

fn replace_hid_path(text: &str, from: &str, to: &str) -> Vec<UdevRecord> {
    parse_udevadm_info(&text.replace(from, to))
}

#[test]
fn udevadm_info_fixtures_parse_into_records() {
    let records = parse_udevadm_info(MAGIC_MOUSE);
    assert_eq!(records.len(), 5);
    let hidraw = &records[0];
    assert_eq!(hidraw.subsystem.as_deref(), Some("hidraw"));
    assert_eq!(hidraw.sysname(), "hidraw5");
    assert_eq!(hidraw.devnode.as_deref(), Some(Path::new("/dev/hidraw5")));
    assert_eq!(
        hidraw.syspath(),
        PathBuf::from(format!("/sys{MM_HID}/hidraw/hidraw5"))
    );
    let hid = &records[1];
    assert_eq!(hid.property("HID_ID"), Some("0005:0000004C:00000269"));
    assert_eq!(hid.devnode, None);

    let usb = parse_udevadm_info(USB_MOUSE);
    assert_eq!(usb.len(), 5);
    assert_eq!(
        usb[3].devnode.as_deref(),
        Some(Path::new("/dev/input/event6"))
    );
}

#[test]
fn hid_id_parses_bus_vendor_and_product() {
    assert_eq!(
        parse_hid_id("0005:0000004C:00000269"),
        Some((0x0005, 0x004c, 0x0269))
    );
    assert_eq!(
        parse_hid_id("0003:0000093A:00002510"),
        Some((0x0003, 0x093a, 0x2510))
    );
    assert_eq!(parse_hid_id("0005:0000004C"), None);
    assert_eq!(parse_hid_id("0005:0000004C:00000269:1"), None);
    assert_eq!(parse_hid_id("0005:00010000:00000269"), None);
    assert_eq!(parse_hid_id("zz:0000004C:00000269"), None);
    assert_eq!(parse_hid_id(""), None);
}

#[test]
fn magic_mouse_fixture_yields_its_hidraw_and_event_nodes() {
    let records = parse_udevadm_info(MAGIC_MOUSE);
    let nodes = classify_all(&records);
    assert_eq!(nodes.len(), 2, "{nodes:?}");
    let key = PathBuf::from(format!("/sys{MM_HID}"));
    assert!(nodes.iter().all(|node| node.key == key));
    assert!(nodes.iter().all(|node| node.profile == &MAGIC_MOUSE_2_BT));
    assert_eq!(nodes[0].kind, NodeKind::Hidraw);
    assert_eq!(nodes[0].devnode, PathBuf::from("/dev/hidraw5"));
    assert_eq!(nodes[1].kind, NodeKind::Evdev);
    assert_eq!(nodes[1].devnode, PathBuf::from("/dev/input/event16"));
}

#[test]
fn mousedev_and_input_parent_nodes_are_not_candidates() {
    let records = parse_udevadm_info(MAGIC_MOUSE);
    let mouse1 = records.iter().find(|r| r.sysname() == "mouse1").unwrap();
    let input21 = records.iter().find(|r| r.sysname() == "input21").unwrap();
    assert_eq!(node_kind(mouse1), None);
    assert_eq!(node_kind(input21), None);
    assert_eq!(node_kind(&records[0]), Some(NodeKind::Hidraw));
}

#[test]
fn other_hid_devices_from_the_owner_machine_do_not_match() {
    let records = parse_udevadm_info(USB_MOUSE);
    assert!(classify_all(&records).is_empty());
    let hid = records.iter().find(|r| r.subsystem_is("hid")).unwrap();
    assert_eq!(hid_profile(hid), None);
}

#[test]
fn matching_does_not_depend_on_the_adapter_path() {
    let adapter_path = "/devices/pci0000:00/0000:00:08.1/0000:c5:00.3/usb1/1-2/1-2.3/1-2.3:1.0/bluetooth/hci0/hci0:256";
    for other in [
        "/devices/pci0000:00/0000:00:14.0/usb3/3-4/3-4:1.0/bluetooth/hci1/hci1:512",
        "/devices/virtual/misc/uhid",
        "/devices/platform/soc/bluetooth/hci2/hci2:11",
    ] {
        let records = replace_hid_path(MAGIC_MOUSE, adapter_path, other);
        let nodes = classify_all(&records);
        assert_eq!(nodes.len(), 2, "{other}");
        assert!(nodes[0].key.starts_with(format!("/sys{other}")));
    }
}

#[test]
fn other_apple_identities_do_not_match() {
    for hid_id in [
        "0003:0000004C:00000269",
        "0005:000005AC:00000269",
        "0005:0000004C:0000030D",
        "0005:0000004C:00000265",
    ] {
        let records = parse_udevadm_info(
            &MAGIC_MOUSE.replace("HID_ID=0005:0000004C:00000269", &format!("HID_ID={hid_id}")),
        );
        assert!(classify_all(&records).is_empty(), "{hid_id}");
    }
}

#[test]
fn a_node_outside_its_parent_is_not_classified() {
    let records = parse_udevadm_info(MAGIC_MOUSE);
    let usb = parse_udevadm_info(USB_MOUSE);
    let usb_event = usb.iter().find(|r| r.sysname() == "event6").unwrap();
    assert_eq!(classify(usb_event, &records[1]), None);
}

fn magic_mouse_nodes() -> (MouseNode, MouseNode) {
    let records = parse_udevadm_info(MAGIC_MOUSE);
    let mut nodes = classify_all(&records).into_iter();
    (nodes.next().unwrap(), nodes.next().unwrap())
}

#[test]
fn mouse_appears_once_both_nodes_are_known() {
    let (hidraw, evdev) = magic_mouse_nodes();
    let mut state = DiscoveryState::default();
    assert_eq!(state.node_added(hidraw.clone()), None);
    let Some(DiscoveryEvent::MouseAppeared(nodes)) = state.node_added(evdev.clone()) else {
        panic!("expected MouseAppeared");
    };
    assert_eq!(nodes.hidraw, PathBuf::from("/dev/hidraw5"));
    assert_eq!(nodes.evdev, PathBuf::from("/dev/input/event16"));
    assert_eq!(nodes.key, hidraw.key);
    assert_eq!(nodes.profile, &MAGIC_MOUSE_2_BT);
    assert_eq!(state.mice(), vec![nodes]);
}

#[test]
fn either_arrival_order_works_and_duplicates_are_silent() {
    let (hidraw, evdev) = magic_mouse_nodes();
    let mut state = DiscoveryState::default();
    assert_eq!(state.node_added(evdev.clone()), None);
    assert!(matches!(
        state.node_added(hidraw.clone()),
        Some(DiscoveryEvent::MouseAppeared(_))
    ));
    assert_eq!(state.node_added(hidraw), None);
    assert_eq!(state.node_added(evdev), None);
}

#[test]
fn change_events_announce_a_complete_mouse_again() {
    let (hidraw, evdev) = magic_mouse_nodes();
    let mut state = DiscoveryState::default();
    assert_eq!(state.node_changed(hidraw.clone()), None);
    assert!(state.node_added(evdev.clone()).is_some());
    assert_eq!(state.node_added(hidraw.clone()), None);
    let Some(DiscoveryEvent::MouseAppeared(nodes)) = state.node_changed(hidraw) else {
        panic!("expected MouseAppeared");
    };
    assert_eq!(nodes.evdev, evdev.devnode);
}

#[test]
fn removing_either_node_reports_the_mouse_gone_once() {
    let (hidraw, evdev) = magic_mouse_nodes();
    let key = hidraw.key.clone();
    let mut state = DiscoveryState::default();
    state.node_added(hidraw.clone());
    state.node_added(evdev.clone());
    assert_eq!(
        state.node_removed(&evdev.syspath),
        Some(DiscoveryEvent::MouseGone { key: key.clone() })
    );
    assert_eq!(state.node_removed(&hidraw.syspath), None);
    assert_eq!(state.node_removed(&hidraw.syspath), None);
    assert!(state.mice().is_empty());
    assert_eq!(state, DiscoveryState::default());
}

#[test]
fn reconnect_with_new_node_numbers_appears_again() {
    let (hidraw, evdev) = magic_mouse_nodes();
    let mut state = DiscoveryState::default();
    state.node_added(hidraw.clone());
    state.node_added(evdev.clone());
    state.node_removed(&hidraw.syspath);
    state.node_removed(&evdev.syspath);

    let records = parse_udevadm_info(
        &MAGIC_MOUSE
            .replace("0269.0006", "0269.0007")
            .replace("hidraw5", "hidraw6")
            .replace("event16", "event17"),
    );
    let mut nodes = classify_all(&records).into_iter();
    assert_eq!(state.node_added(nodes.next().unwrap()), None);
    let Some(DiscoveryEvent::MouseAppeared(mouse)) = state.node_added(nodes.next().unwrap()) else {
        panic!("expected MouseAppeared");
    };
    assert_eq!(mouse.hidraw, PathBuf::from("/dev/hidraw6"));
    assert_eq!(mouse.evdev, PathBuf::from("/dev/input/event17"));
    assert!(mouse.key.ends_with("0005:004C:0269.0007"));
}

#[test]
fn two_mice_on_two_adapters_are_tracked_separately() {
    let (hidraw, evdev) = magic_mouse_nodes();
    let other_records = replace_hid_path(
        MAGIC_MOUSE,
        "1-2.3/1-2.3:1.0/bluetooth/hci0/hci0:256/0005:004C:0269.0006",
        "1-4/1-4:1.0/bluetooth/hci1/hci1:256/0005:004C:0269.0009",
    );
    let other = classify_all(&other_records);
    let mut state = DiscoveryState::default();
    state.node_added(hidraw);
    state.node_added(other[0].clone());
    assert!(state.node_added(evdev).is_some());
    assert!(state.node_added(other[1].clone()).is_some());
    assert_eq!(state.mice().len(), 2);
}

#[test]
fn unknown_removals_are_ignored() {
    let mut state = DiscoveryState::default();
    assert_eq!(state.node_removed(Path::new("/sys/devices/none")), None);
}
