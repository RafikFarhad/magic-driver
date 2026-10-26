mod diagnostic;
mod file;
mod presets;
mod resolve;

use serde::{Deserialize, Serialize};

use crate::profile::{MAGIC_MOUSE_2_BT, Profile};

pub use diagnostic::{ConfigErrors, Diagnostic, DiagnosticKind};
pub use presets::{ScrollPreset, TouchPreset};

pub const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Config {
    pub version: u32,
    pub pointer: PointerConfig,
    pub scroll: ScrollConfig,
    pub touch: TouchConfig,
    pub gestures: GesturesConfig,
    pub log: LogConfig,
    pub advanced: AdvancedConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PointerConfig {
    pub middle_click: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScrollConfig {
    pub enabled: bool,
    pub horizontal: bool,
    pub axis_lock: bool,
    pub acceleration: ScrollAcceleration,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TouchConfig {
    pub palm_rejection: PalmRejection,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GesturesConfig {
    pub swipe_desktops: bool,
    pub double_tap_overview: bool,
    pub smart_zoom: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogConfig {
    pub level: LogLevel,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AdvancedConfig {
    pub scroll: AdvancedScrollConfig,
    pub touch: AdvancedTouchConfig,
    pub gestures: AdvancedGesturesConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AdvancedScrollConfig {
    pub gain_min: f64,
    pub gain_max: f64,
    pub gain_speed_mm_s: [f64; 2],
    pub cancel_hold_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AdvancedTouchConfig {
    pub promote_distance_mm: f64,
    pub promote_window_ms: u32,
    pub rest_dwell_ms: u32,
    pub palm_size_mm: f64,
    pub rear_zone: f64,
    pub mouse_motion_threshold: f64,
    pub click_guard_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AdvancedGesturesConfig {
    pub kwin_swipe_fingers: u8,
    pub double_tap_ms: u32,
    pub swipe_distance_mm: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScrollAcceleration {
    Off,
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PalmRejection {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Warn,
    Info,
    Debug,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedConfig {
    pub config: Config,
    pub warnings: Vec<Diagnostic>,
}

impl Config {
    pub fn for_profile(profile: &Profile) -> Config {
        let acceleration = ScrollAcceleration::Medium;
        let palm_rejection = PalmRejection::Medium;
        let scroll_preset = acceleration.preset();
        let touch_preset = palm_rejection.preset(profile);
        Config {
            version: CONFIG_VERSION,
            pointer: PointerConfig { middle_click: true },
            scroll: ScrollConfig {
                enabled: true,
                horizontal: true,
                axis_lock: true,
                acceleration,
            },
            touch: TouchConfig { palm_rejection },
            gestures: GesturesConfig {
                swipe_desktops: true,
                double_tap_overview: true,
                smart_zoom: false,
            },
            log: LogConfig {
                level: LogLevel::Info,
            },
            advanced: AdvancedConfig {
                scroll: AdvancedScrollConfig {
                    gain_min: scroll_preset.gain_min,
                    gain_max: scroll_preset.gain_max,
                    gain_speed_mm_s: scroll_preset.gain_speed_mm_s,
                    cancel_hold_ms: 60,
                },
                touch: AdvancedTouchConfig {
                    promote_distance_mm: touch_preset.promote_distance_mm,
                    promote_window_ms: 150,
                    rest_dwell_ms: touch_preset.rest_dwell_ms,
                    palm_size_mm: touch_preset.palm_size_mm,
                    rear_zone: touch_preset.rear_zone,
                    mouse_motion_threshold: touch_preset.mouse_motion_threshold,
                    click_guard_ms: touch_preset.click_guard_ms,
                },
                gestures: AdvancedGesturesConfig {
                    kwin_swipe_fingers: 4,
                    double_tap_ms: 300,
                    swipe_distance_mm: 8.0,
                },
            },
        }
    }

    pub fn parse(source: &str, profile: &Profile) -> Result<ParsedConfig, ConfigErrors> {
        resolve::parse(source, profile)
    }

    pub fn render(&self) -> String {
        toml::to_string(self).expect("config always serializes to TOML")
    }

    pub(crate) fn apply_scroll_preset(&mut self, preset: ScrollPreset) {
        let scroll = &mut self.advanced.scroll;
        scroll.gain_min = preset.gain_min;
        scroll.gain_max = preset.gain_max;
        scroll.gain_speed_mm_s = preset.gain_speed_mm_s;
    }

    pub(crate) fn apply_touch_preset(&mut self, preset: TouchPreset) {
        let touch = &mut self.advanced.touch;
        touch.promote_distance_mm = preset.promote_distance_mm;
        touch.rest_dwell_ms = preset.rest_dwell_ms;
        touch.palm_size_mm = preset.palm_size_mm;
        touch.rear_zone = preset.rear_zone;
        touch.mouse_motion_threshold = preset.mouse_motion_threshold;
        touch.click_guard_ms = preset.click_guard_ms;
    }
}

impl Default for Config {
    fn default() -> Self {
        Config::for_profile(&MAGIC_MOUSE_2_BT)
    }
}

#[cfg(test)]
mod tests;
