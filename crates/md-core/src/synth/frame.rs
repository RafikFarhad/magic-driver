use std::fmt;

use serde::Serialize;

use crate::profile::SurfaceGeometry;
use crate::surface::{SurfacePoint, SurfacePointMm};

pub const TOUCHPAD_SLOTS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TouchpadGeometry {
    pub max_x: i32,
    pub max_y: i32,
    pub units_per_mm: i32,
}

pub const VIRTUAL_TOUCHPAD: TouchpadGeometry = TouchpadGeometry {
    max_x: 16_000,
    max_y: 16_000,
    units_per_mm: 40,
};

impl TouchpadGeometry {
    pub const fn centre(&self) -> TouchpadPoint {
        TouchpadPoint {
            x: self.max_x / 2,
            y: self.max_y / 2,
        }
    }

    pub fn width_mm(&self) -> f64 {
        f64::from(self.max_x) / f64::from(self.units_per_mm)
    }

    pub fn height_mm(&self) -> f64 {
        f64::from(self.max_y) / f64::from(self.units_per_mm)
    }

    #[allow(clippy::cast_possible_truncation)]
    pub fn mm_to_units(&self, mm: f64) -> i32 {
        (mm * f64::from(self.units_per_mm)).round() as i32
    }

    pub fn offset(&self, from: TouchpadPoint, mm: SurfacePointMm) -> TouchpadPoint {
        self.clamp(TouchpadPoint {
            x: from.x.saturating_add(self.mm_to_units(mm.x)),
            y: from.y.saturating_add(self.mm_to_units(mm.y)),
        })
    }

    pub fn map_surface_point(
        &self,
        surface: &SurfaceGeometry,
        point: SurfacePoint,
    ) -> TouchpadPoint {
        let point = surface.to_mm(point);
        let centre = surface_centre_mm(surface);
        self.offset(
            self.centre(),
            SurfacePointMm::new(point.x - centre.x, point.y - centre.y),
        )
    }

    pub fn clamp(&self, point: TouchpadPoint) -> TouchpadPoint {
        TouchpadPoint {
            x: point.x.clamp(0, self.max_x),
            y: point.y.clamp(0, self.max_y),
        }
    }

    pub fn contains(&self, point: TouchpadPoint) -> bool {
        (0..=self.max_x).contains(&point.x) && (0..=self.max_y).contains(&point.y)
    }
}

fn surface_centre_mm(surface: &SurfaceGeometry) -> SurfacePointMm {
    let min = surface.to_mm(SurfacePoint::new(surface.x.min, surface.y.min));
    let max = surface.to_mm(SurfacePoint::new(surface.x.max, surface.y.max));
    SurfacePointMm::new(f64::midpoint(min.x, max.x), f64::midpoint(min.y, max.y))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
pub struct TouchpadPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Contact {
    pub tracking_id: u16,
    pub x: i32,
    pub y: i32,
}

impl Contact {
    pub const fn new(tracking_id: u16, point: TouchpadPoint) -> Contact {
        Contact {
            tracking_id,
            x: point.x,
            y: point.y,
        }
    }

    pub const fn point(&self) -> TouchpadPoint {
        TouchpadPoint {
            x: self.x,
            y: self.y,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
pub struct Frame {
    pub slots: [Option<Contact>; TOUCHPAD_SLOTS],
}

impl Frame {
    pub const EMPTY: Frame = Frame {
        slots: [None; TOUCHPAD_SLOTS],
    };

    pub fn contact_count(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }

    pub fn contacts(&self) -> impl Iterator<Item = (usize, Contact)> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(slot, contact)| contact.map(|contact| (slot, contact)))
    }
}

impl fmt::Display for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "frame")?;
        for (slot, contact) in self.slots.iter().enumerate() {
            match contact {
                Some(contact) => write!(
                    f,
                    " {slot}:{}@{},{}",
                    contact.tracking_id, contact.x, contact.y
                )?,
                None => write!(f, " {slot}:-")?,
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrackingIds {
    next: u16,
}

impl TrackingIds {
    pub fn allocate(&mut self) -> u16 {
        let id = self.next;
        self.next = self.next.wrapping_add(1);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::MAGIC_MOUSE_2_BT;

    const SURFACE: SurfaceGeometry = MAGIC_MOUSE_2_BT.surface;

    #[test]
    fn virtual_touchpad_is_400_mm_square_centred() {
        assert!((VIRTUAL_TOUCHPAD.width_mm() - 400.0).abs() < 1e-9);
        assert!((VIRTUAL_TOUCHPAD.height_mm() - 400.0).abs() < 1e-9);
        assert_eq!(
            VIRTUAL_TOUCHPAD.centre(),
            TouchpadPoint { x: 8000, y: 8000 }
        );
    }

    #[test]
    fn surface_centre_maps_to_touchpad_centre() {
        let centre = SurfacePoint::new(
            i32::midpoint(SURFACE.x.min, SURFACE.x.max),
            i32::midpoint(SURFACE.y.min, SURFACE.y.max),
        );
        assert_eq!(
            VIRTUAL_TOUCHPAD.map_surface_point(&SURFACE, centre),
            VIRTUAL_TOUCHPAD.centre()
        );
    }

    #[test]
    fn surface_maps_one_to_one_in_millimetres() {
        let min = VIRTUAL_TOUCHPAD
            .map_surface_point(&SURFACE, SurfacePoint::new(SURFACE.x.min, SURFACE.y.min));
        let max = VIRTUAL_TOUCHPAD
            .map_surface_point(&SURFACE, SurfacePoint::new(SURFACE.x.max, SURFACE.y.max));
        assert!((max.x - min.x - VIRTUAL_TOUCHPAD.mm_to_units(90.56)).abs() <= 1);
        assert!((max.y - min.y - VIRTUAL_TOUCHPAD.mm_to_units(51.52)).abs() <= 1);
        assert!(VIRTUAL_TOUCHPAD.contains(min) && VIRTUAL_TOUCHPAD.contains(max));
        assert!(min.x < max.x && min.y < max.y);
    }

    #[test]
    fn whole_encodable_range_stays_on_the_touchpad() {
        for x in [-2048, 0, 2047] {
            for y in [-2047, 0, 2048] {
                let point = VIRTUAL_TOUCHPAD.map_surface_point(&SURFACE, SurfacePoint::new(x, y));
                assert!(VIRTUAL_TOUCHPAD.contains(point), "{x},{y} -> {point:?}");
            }
        }
    }

    #[test]
    fn offsets_are_clamped_to_the_surface() {
        let far = VIRTUAL_TOUCHPAD.offset(
            VIRTUAL_TOUCHPAD.centre(),
            SurfacePointMm::new(1000.0, -1000.0),
        );
        assert_eq!(far, TouchpadPoint { x: 16_000, y: 0 });
    }

    #[test]
    fn frame_counts_and_displays_contacts() {
        let mut frame = Frame::EMPTY;
        assert!(frame.is_empty());
        frame.slots[1] = Some(Contact::new(7, TouchpadPoint { x: 10, y: 20 }));
        frame.slots[3] = Some(Contact::new(8, TouchpadPoint { x: 30, y: 40 }));
        assert_eq!(frame.contact_count(), 2);
        assert_eq!(
            frame.contacts().map(|(slot, _)| slot).collect::<Vec<_>>(),
            vec![1, 3]
        );
        assert_eq!(frame.to_string(), "frame 0:- 1:7@10,20 2:- 3:8@30,40 4:-");
    }

    #[test]
    fn tracking_ids_increase_and_wrap() {
        let mut ids = TrackingIds::default();
        assert_eq!(ids.allocate(), 0);
        assert_eq!(ids.allocate(), 1);
        let mut ids = TrackingIds { next: u16::MAX };
        assert_eq!(ids.allocate(), u16::MAX);
        assert_eq!(ids.allocate(), 0);
    }
}
