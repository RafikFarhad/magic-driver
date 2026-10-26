use crate::profile::{SurfaceAxis, SurfaceGeometry};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SurfacePoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SurfacePointMm {
    pub x: f64,
    pub y: f64,
}

impl SurfacePoint {
    pub const fn new(x: i32, y: i32) -> Self {
        SurfacePoint { x, y }
    }
}

impl SurfacePointMm {
    pub const fn new(x: f64, y: f64) -> Self {
        SurfacePointMm { x, y }
    }
}

impl SurfaceAxis {
    pub fn units_to_mm(&self, units: i32) -> f64 {
        f64::from(units) / self.units_per_mm()
    }

    #[allow(clippy::cast_possible_truncation)]
    pub fn mm_to_units(&self, mm: f64) -> i32 {
        (mm * self.units_per_mm()).round() as i32
    }

    pub fn contains(&self, units: i32) -> bool {
        (self.min..=self.max).contains(&units)
    }
}

impl SurfaceGeometry {
    pub fn to_mm(&self, point: SurfacePoint) -> SurfacePointMm {
        SurfacePointMm {
            x: self.x.units_to_mm(point.x),
            y: self.y.units_to_mm(point.y),
        }
    }

    pub fn to_device_units(&self, point: SurfacePointMm) -> SurfacePoint {
        SurfacePoint {
            x: self.x.mm_to_units(point.x),
            y: self.y.mm_to_units(point.y),
        }
    }

    pub fn contains(&self, point: SurfacePoint) -> bool {
        self.x.contains(point.x) && self.y.contains(point.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::MAGIC_MOUSE_2_BT;

    const SURFACE: SurfaceGeometry = MAGIC_MOUSE_2_BT.surface;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn origin_maps_to_origin() {
        assert_eq!(
            SURFACE.to_mm(SurfacePoint::new(0, 0)),
            SurfacePointMm::new(0.0, 0.0)
        );
        assert_eq!(
            SURFACE.to_device_units(SurfacePointMm::new(0.0, 0.0)),
            SurfacePoint::new(0, 0)
        );
    }

    #[test]
    fn full_span_maps_to_surface_size() {
        let min = SURFACE.to_mm(SurfacePoint::new(SURFACE.x.min, SURFACE.y.min));
        let max = SURFACE.to_mm(SurfacePoint::new(SURFACE.x.max, SURFACE.y.max));
        assert!(close(max.x - min.x, 90.56));
        assert!(close(max.y - min.y, 51.52));
    }

    #[test]
    fn known_points_convert_with_profile_resolution() {
        let point = SURFACE.to_mm(SurfacePoint::new(2358, 3636));
        assert!(close(point.x, 90.56));
        assert!(close(point.y, 51.52));
        let negative = SURFACE.to_mm(SurfacePoint::new(-2358, -3636));
        assert!(close(negative.x, -90.56));
        assert!(close(negative.y, -51.52));
    }

    #[test]
    fn device_units_round_trip_through_mm() {
        for x in [-1100, -350, -1, 0, 1, 350, 1258] {
            for y in [-1589, -1, 0, 1, 2047] {
                let point = SurfacePoint::new(x, y);
                assert_eq!(SURFACE.to_device_units(SURFACE.to_mm(point)), point);
            }
        }
    }

    #[test]
    fn mm_to_units_rounds_to_nearest() {
        assert_eq!(SURFACE.x.mm_to_units(1.0), 26);
        assert_eq!(SURFACE.y.mm_to_units(-1.0), -71);
        assert_eq!(SURFACE.x.mm_to_units(90.56), 2358);
    }

    #[test]
    fn contains_checks_both_axes() {
        assert!(SURFACE.contains(SurfacePoint::new(-1100, 2047)));
        assert!(SURFACE.contains(SurfacePoint::new(1258, -1589)));
        assert!(!SURFACE.contains(SurfacePoint::new(1259, 0)));
        assert!(!SURFACE.contains(SurfacePoint::new(0, -1590)));
    }
}
