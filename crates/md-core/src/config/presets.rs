use super::{PalmRejection, ScrollAcceleration};
use crate::profile::Profile;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollPreset {
    pub gain_min: f64,
    pub gain_max: f64,
    pub gain_speed_mm_s: [f64; 2],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchPreset {
    pub promote_distance_mm: f64,
    pub rest_dwell_ms: u32,
    pub palm_size_mm: f64,
    pub rear_zone: f64,
    pub mouse_motion_threshold: f64,
    pub click_guard_ms: u32,
}

const GAIN_SPEED_MM_S: [f64; 2] = [20.0, 250.0];

impl ScrollAcceleration {
    pub fn preset(self) -> ScrollPreset {
        let gain_max = match self {
            ScrollAcceleration::Off => 1.0,
            ScrollAcceleration::Low => 1.75,
            ScrollAcceleration::Medium => 2.5,
            ScrollAcceleration::High => 4.0,
        };
        ScrollPreset {
            gain_min: 1.0,
            gain_max,
            gain_speed_mm_s: GAIN_SPEED_MM_S,
        }
    }
}

impl PalmRejection {
    pub fn preset(self, profile: &Profile) -> TouchPreset {
        let palm_size_mm = profile.default_palm_size_mm;
        let mouse_motion_threshold = profile.default_mouse_motion_threshold;
        match self {
            PalmRejection::Low => TouchPreset {
                promote_distance_mm: 1.0,
                rest_dwell_ms: 400,
                palm_size_mm: palm_size_mm * 1.25,
                rear_zone: 0.25,
                mouse_motion_threshold: mouse_motion_threshold * 1.5,
                click_guard_ms: 60,
            },
            PalmRejection::Medium => TouchPreset {
                promote_distance_mm: 1.5,
                rest_dwell_ms: 300,
                palm_size_mm,
                rear_zone: 0.35,
                mouse_motion_threshold,
                click_guard_ms: 100,
            },
            PalmRejection::High => TouchPreset {
                promote_distance_mm: 2.0,
                rest_dwell_ms: 200,
                palm_size_mm: palm_size_mm * 0.75,
                rear_zone: 0.45,
                mouse_motion_threshold: mouse_motion_threshold * 0.5,
                click_guard_ms: 150,
            },
        }
    }
}
