use serde::Deserialize;
use serde::de::{DeserializeOwned, Deserializer};
use toml::Spanned;

use super::{LogLevel, PalmRejection, ScrollAcceleration};

#[derive(Debug, Clone, PartialEq)]
pub struct Checked<T>(pub Result<T, String>);

impl<'de, T: DeserializeOwned> Deserialize<'de> for Checked<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = toml::Value::deserialize(deserializer)?;
        Ok(Checked(
            T::deserialize(value).map_err(|error| error.message().to_owned()),
        ))
    }
}

pub type Field<T> = Option<Spanned<Checked<T>>>;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ConfigFile {
    pub version: Field<u32>,
    pub pointer: PointerSection,
    pub scroll: ScrollSection,
    pub touch: TouchSection,
    pub gestures: GesturesSection,
    pub log: LogSection,
    pub advanced: AdvancedSection,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PointerSection {
    pub middle_click: Field<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ScrollSection {
    pub enabled: Field<bool>,
    pub horizontal: Field<bool>,
    pub axis_lock: Field<bool>,
    pub acceleration: Field<ScrollAcceleration>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct TouchSection {
    pub palm_rejection: Field<PalmRejection>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct GesturesSection {
    pub swipe_desktops: Field<bool>,
    pub double_tap_overview: Field<bool>,
    pub smart_zoom: Field<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct LogSection {
    pub level: Field<LogLevel>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct AdvancedSection {
    pub scroll: AdvancedScrollSection,
    pub touch: AdvancedTouchSection,
    pub gestures: AdvancedGesturesSection,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct AdvancedScrollSection {
    pub gain_min: Field<f64>,
    pub gain_max: Field<f64>,
    pub gain_speed_mm_s: Field<[f64; 2]>,
    pub cancel_hold_ms: Field<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct AdvancedTouchSection {
    pub promote_distance_mm: Field<f64>,
    pub promote_window_ms: Field<u32>,
    pub rest_dwell_ms: Field<u32>,
    pub palm_size_mm: Field<f64>,
    pub rear_zone: Field<f64>,
    pub mouse_motion_threshold: Field<f64>,
    pub click_guard_ms: Field<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct AdvancedGesturesSection {
    pub kwin_swipe_fingers: Field<u8>,
    pub double_tap_ms: Field<u32>,
    pub swipe_distance_mm: Field<f64>,
}
