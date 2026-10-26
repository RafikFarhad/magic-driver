#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HidIdentity {
    pub bus: u16,
    pub vendor: u16,
    pub product: u16,
}

pub const BUS_BLUETOOTH: u16 = 0x0005;
pub const VENDOR_APPLE_BLUETOOTH: u16 = 0x004c;
pub const PRODUCT_MAGIC_MOUSE_2: u16 = 0x0269;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportLayout {
    pub report_id: u8,
    pub header_len: usize,
    pub short_report_len: usize,
    pub touch_len: usize,
    pub max_touches: usize,
}

impl ReportLayout {
    pub const fn report_len(&self, touches: usize) -> usize {
        self.header_len + self.touch_len * touches
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceDirection {
    Left,
    Right,
    Front,
    Rear,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceAxis {
    pub min: i32,
    pub max: i32,
    pub size_mm: f64,
    pub increases_toward: SurfaceDirection,
}

impl SurfaceAxis {
    pub const fn span_units(&self) -> i32 {
        self.max - self.min
    }

    pub fn units_per_mm(&self) -> f64 {
        f64::from(self.span_units()) / self.size_mm
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceGeometry {
    pub x: SurfaceAxis,
    pub y: SurfaceAxis,
    pub raw_y_inverted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Profile {
    pub id: &'static str,
    pub name: &'static str,
    pub hid: HidIdentity,
    pub report: ReportLayout,
    pub surface: SurfaceGeometry,
    pub touch_size_scale: i32,
    pub default_palm_size_mm: f64,
    pub default_mouse_motion_threshold: f64,
}

pub const MAGIC_MOUSE_2_BT: Profile = Profile {
    id: "magic-mouse-2-bt",
    name: "Apple Magic Mouse 2 (Bluetooth)",
    hid: HidIdentity {
        bus: BUS_BLUETOOTH,
        vendor: VENDOR_APPLE_BLUETOOTH,
        product: PRODUCT_MAGIC_MOUSE_2,
    },
    report: ReportLayout {
        report_id: 0x12,
        header_len: 14,
        short_report_len: 8,
        touch_len: 8,
        max_touches: 15,
    },
    surface: SurfaceGeometry {
        x: SurfaceAxis {
            min: -1100,
            max: 1258,
            size_mm: 90.56,
            increases_toward: SurfaceDirection::Right,
        },
        y: SurfaceAxis {
            min: -1589,
            max: 2047,
            size_mm: 51.52,
            increases_toward: SurfaceDirection::Rear,
        },
        raw_y_inverted: true,
    },
    touch_size_scale: 4,
    default_palm_size_mm: 20.0,
    default_mouse_motion_threshold: 150.0,
};

pub const PROFILES: &[Profile] = &[MAGIC_MOUSE_2_BT];

impl Profile {
    pub fn lookup(bus: u16, vendor: u16, product: u16) -> Option<&'static Profile> {
        let wanted = HidIdentity {
            bus,
            vendor,
            product,
        };
        PROFILES.iter().find(|profile| profile.hid == wanted)
    }

    pub fn by_id(id: &str) -> Option<&'static Profile> {
        PROFILES.iter().find(|profile| profile.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_finds_magic_mouse_2_over_bluetooth() {
        let profile = Profile::lookup(0x0005, 0x004c, 0x0269).expect("profile");
        assert_eq!(profile.id, "magic-mouse-2-bt");
        assert_eq!(*profile, MAGIC_MOUSE_2_BT);
    }

    #[test]
    fn lookup_rejects_other_identities() {
        assert_eq!(Profile::lookup(0x0003, 0x004c, 0x0269), None);
        assert_eq!(Profile::lookup(0x0005, 0x05ac, 0x0269), None);
        assert_eq!(Profile::lookup(0x0005, 0x004c, 0x030d), None);
        assert_eq!(Profile::lookup(0x0005, 0x004c, 0x0323), None);
        assert_eq!(Profile::lookup(0, 0, 0), None);
    }

    #[test]
    fn by_id_matches_profile_ids() {
        assert_eq!(Profile::by_id("magic-mouse-2-bt"), Some(&MAGIC_MOUSE_2_BT));
        assert_eq!(Profile::by_id("magic-trackpad"), None);
    }

    #[test]
    fn report_layout_matches_reference() {
        let report = MAGIC_MOUSE_2_BT.report;
        assert_eq!(report.report_id, 0x12);
        assert_eq!(report.header_len, 14);
        assert_eq!(report.short_report_len, 8);
        assert_eq!(report.touch_len, 8);
        assert_eq!(report.max_touches, 15);
        assert_eq!(report.report_len(0), 14);
        assert_eq!(report.report_len(3), 38);
        assert_eq!(report.report_len(15), 134);
    }

    #[test]
    fn surface_matches_reference() {
        let surface = MAGIC_MOUSE_2_BT.surface;
        assert_eq!((surface.x.min, surface.x.max), (-1100, 1258));
        assert_eq!((surface.y.min, surface.y.max), (-1589, 2047));
        assert_eq!(surface.x.span_units(), 2358);
        assert_eq!(surface.y.span_units(), 3636);
        assert!((surface.x.size_mm - 90.56).abs() < 1e-9);
        assert!((surface.y.size_mm - 51.52).abs() < 1e-9);
        assert!(surface.raw_y_inverted);
        assert_eq!(surface.x.increases_toward, SurfaceDirection::Right);
        assert_eq!(surface.y.increases_toward, SurfaceDirection::Rear);
    }

    #[test]
    fn resolution_truncates_to_kernel_values() {
        let surface = MAGIC_MOUSE_2_BT.surface;
        assert!((surface.x.units_per_mm() - 2358.0 / 90.56).abs() < 1e-9);
        assert!((surface.y.units_per_mm() - 3636.0 / 51.52).abs() < 1e-9);
        assert!((26.0..27.0).contains(&surface.x.units_per_mm()));
        assert!((70.0..71.0).contains(&surface.y.units_per_mm()));
    }

    #[test]
    fn profile_ids_and_identities_are_unique() {
        for (index, profile) in PROFILES.iter().enumerate() {
            for other in &PROFILES[index + 1..] {
                assert_ne!(profile.id, other.id);
                assert_ne!(profile.hid, other.hid);
            }
        }
    }
}
