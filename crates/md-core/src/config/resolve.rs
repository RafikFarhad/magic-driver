use std::fmt::Display;
use std::ops::RangeInclusive;

use serde::Deserialize;
use toml::de::DeTable;

use super::file::{ConfigFile, Field};
use super::{CONFIG_VERSION, Config, ConfigErrors, Diagnostic, DiagnosticKind, ParsedConfig};
use crate::profile::Profile;

const GAIN_RANGE: RangeInclusive<f64> = 0.1..=10.0;
const GAIN_SPEED_RANGE: RangeInclusive<f64> = 0.0..=5_000.0;

pub(super) fn parse(source: &str, profile: &Profile) -> Result<ParsedConfig, ConfigErrors> {
    let table = match DeTable::parse(source) {
        Ok(table) => table,
        Err(error) => {
            let diagnostic = Diagnostic::new(
                source,
                error.span().map(|span| span.start),
                None,
                DiagnosticKind::Syntax(error.message().trim().to_owned()),
            );
            return Err(ConfigErrors {
                errors: vec![diagnostic],
                warnings: Vec::new(),
            });
        }
    };

    let defaults = Config::for_profile(profile);
    let known = toml::Table::try_from(&defaults).expect("config always serializes to a table");
    let mut warnings = Vec::new();
    collect_unknown_keys(source, table.get_ref(), &known, "", &mut warnings);
    warnings.sort_by_key(|warning| warning.line);

    let file = match ConfigFile::deserialize(toml::de::Deserializer::from(table)) {
        Ok(file) => file,
        Err(error) => {
            let diagnostic = Diagnostic::new(
                source,
                error.span().map(|span| span.start),
                None,
                DiagnosticKind::InvalidValue(error.message().trim().to_owned()),
            );
            return Err(ConfigErrors {
                errors: vec![diagnostic],
                warnings,
            });
        }
    };

    let mut resolver = Resolver {
        source,
        errors: Vec::new(),
    };
    let config = resolver.resolve(&file, defaults, profile);
    resolver.errors.sort_by_key(|error| error.line);
    if resolver.errors.is_empty() {
        Ok(ParsedConfig { config, warnings })
    } else {
        Err(ConfigErrors {
            errors: resolver.errors,
            warnings,
        })
    }
}

fn collect_unknown_keys(
    source: &str,
    table: &DeTable<'_>,
    known: &toml::Table,
    prefix: &str,
    warnings: &mut Vec<Diagnostic>,
) {
    for (key, value) in table {
        let name: &str = key.get_ref();
        let path = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}.{name}")
        };
        match known.get(name) {
            None => warnings.push(Diagnostic::new(
                source,
                Some(key.span().start),
                Some(&path),
                DiagnosticKind::UnknownKey,
            )),
            Some(toml::Value::Table(known_section)) => {
                if let Some(section) = value.get_ref().as_table() {
                    collect_unknown_keys(source, section, known_section, &path, warnings);
                }
            }
            Some(_) => {}
        }
    }
}

struct Resolver<'a> {
    source: &'a str,
    errors: Vec<Diagnostic>,
}

impl Resolver<'_> {
    fn resolve(&mut self, file: &ConfigFile, mut config: Config, profile: &Profile) -> Config {
        if let Some((version, offset)) = self.take(&file.version, "version")
            && version != CONFIG_VERSION
        {
            self.error(
                offset,
                "version",
                DiagnosticKind::UnsupportedVersion(version),
            );
        }

        self.set(
            &mut config.pointer.middle_click,
            &file.pointer.middle_click,
            "pointer.middle_click",
        );

        let scroll = &file.scroll;
        self.set(
            &mut config.scroll.enabled,
            &scroll.enabled,
            "scroll.enabled",
        );
        self.set(
            &mut config.scroll.horizontal,
            &scroll.horizontal,
            "scroll.horizontal",
        );
        self.set(
            &mut config.scroll.axis_lock,
            &scroll.axis_lock,
            "scroll.axis_lock",
        );
        self.set(
            &mut config.scroll.acceleration,
            &scroll.acceleration,
            "scroll.acceleration",
        );
        config.apply_scroll_preset(config.scroll.acceleration.preset());

        self.set(
            &mut config.touch.palm_rejection,
            &file.touch.palm_rejection,
            "touch.palm_rejection",
        );
        config.apply_touch_preset(config.touch.palm_rejection.preset(profile));

        let gestures = &file.gestures;
        self.set(
            &mut config.gestures.swipe_desktops,
            &gestures.swipe_desktops,
            "gestures.swipe_desktops",
        );
        self.set(
            &mut config.gestures.double_tap_overview,
            &gestures.double_tap_overview,
            "gestures.double_tap_overview",
        );
        self.set(
            &mut config.gestures.smart_zoom,
            &gestures.smart_zoom,
            "gestures.smart_zoom",
        );

        self.set(&mut config.log.level, &file.log.level, "log.level");

        self.resolve_advanced_scroll(file, &mut config);
        self.resolve_advanced_touch(file, &mut config);
        self.resolve_advanced_gestures(file, &mut config);
        config
    }

    fn resolve_advanced_scroll(&mut self, file: &ConfigFile, config: &mut Config) {
        let section = &file.advanced.scroll;
        let target = &mut config.advanced.scroll;
        let gain_min_ok = self.set_in_range(
            &mut target.gain_min,
            &section.gain_min,
            "advanced.scroll.gain_min",
            GAIN_RANGE,
        );
        let gain_max_ok = self.set_in_range(
            &mut target.gain_max,
            &section.gain_max,
            "advanced.scroll.gain_max",
            GAIN_RANGE,
        );
        if gain_min_ok && gain_max_ok && target.gain_max < target.gain_min {
            let (field, key) = if section.gain_max.is_some() {
                (&section.gain_max, "advanced.scroll.gain_max")
            } else {
                (&section.gain_min, "advanced.scroll.gain_min")
            };
            let offset = field.as_ref().map(|spanned| spanned.span().start);
            self.push(
                offset,
                key,
                DiagnosticKind::OutOfRange {
                    value: format!(
                        "gain_max {} below gain_min {}",
                        target.gain_max, target.gain_min
                    ),
                    expected: "gain_max >= gain_min".to_owned(),
                },
            );
        }

        let key = "advanced.scroll.gain_speed_mm_s";
        if let Some((speeds, offset)) = self.take(&section.gain_speed_mm_s, key) {
            let in_range = speeds.iter().all(|speed| GAIN_SPEED_RANGE.contains(speed));
            if in_range && speeds[0] < speeds[1] {
                target.gain_speed_mm_s = speeds;
            } else {
                self.error(
                    offset,
                    key,
                    DiagnosticKind::OutOfRange {
                        value: format!("[{}, {}]", speeds[0], speeds[1]),
                        expected: format!(
                            "[start, end] with {} <= start < end <= {}",
                            GAIN_SPEED_RANGE.start(),
                            GAIN_SPEED_RANGE.end()
                        ),
                    },
                );
            }
        }

        self.set_in_range(
            &mut target.cancel_hold_ms,
            &section.cancel_hold_ms,
            "advanced.scroll.cancel_hold_ms",
            0..=1_000,
        );
    }

    fn resolve_advanced_touch(&mut self, file: &ConfigFile, config: &mut Config) {
        let section = &file.advanced.touch;
        let target = &mut config.advanced.touch;
        self.set_in_range(
            &mut target.promote_distance_mm,
            &section.promote_distance_mm,
            "advanced.touch.promote_distance_mm",
            0.0..=20.0,
        );
        self.set_in_range(
            &mut target.promote_window_ms,
            &section.promote_window_ms,
            "advanced.touch.promote_window_ms",
            10..=2_000,
        );
        self.set_in_range(
            &mut target.rest_dwell_ms,
            &section.rest_dwell_ms,
            "advanced.touch.rest_dwell_ms",
            10..=5_000,
        );
        self.set_in_range(
            &mut target.palm_size_mm,
            &section.palm_size_mm,
            "advanced.touch.palm_size_mm",
            1.0..=100.0,
        );
        self.set_in_range(
            &mut target.rear_zone,
            &section.rear_zone,
            "advanced.touch.rear_zone",
            0.0..=0.9,
        );
        self.set_in_range(
            &mut target.mouse_motion_threshold,
            &section.mouse_motion_threshold,
            "advanced.touch.mouse_motion_threshold",
            0.0..=100_000.0,
        );
        self.set_in_range(
            &mut target.click_guard_ms,
            &section.click_guard_ms,
            "advanced.touch.click_guard_ms",
            0..=1_000,
        );
    }

    fn resolve_advanced_gestures(&mut self, file: &ConfigFile, config: &mut Config) {
        let section = &file.advanced.gestures;
        let target = &mut config.advanced.gestures;
        self.set_in_range(
            &mut target.kwin_swipe_fingers,
            &section.kwin_swipe_fingers,
            "advanced.gestures.kwin_swipe_fingers",
            3..=5,
        );
        self.set_in_range(
            &mut target.double_tap_ms,
            &section.double_tap_ms,
            "advanced.gestures.double_tap_ms",
            50..=2_000,
        );
        self.set_in_range(
            &mut target.swipe_distance_mm,
            &section.swipe_distance_mm,
            "advanced.gestures.swipe_distance_mm",
            1.0..=50.0,
        );
    }

    fn take<T: Clone>(&mut self, field: &Field<T>, key: &str) -> Option<(T, usize)> {
        let spanned = field.as_ref()?;
        let offset = spanned.span().start;
        match &spanned.get_ref().0 {
            Ok(value) => Some((value.clone(), offset)),
            Err(message) => {
                self.error(offset, key, DiagnosticKind::InvalidValue(message.clone()));
                None
            }
        }
    }

    fn set<T: Clone>(&mut self, target: &mut T, field: &Field<T>, key: &str) {
        if let Some((value, _)) = self.take(field, key) {
            *target = value;
        }
    }

    fn set_in_range<T: Copy + PartialOrd + Display>(
        &mut self,
        target: &mut T,
        field: &Field<T>,
        key: &str,
        range: RangeInclusive<T>,
    ) -> bool {
        let Some((value, offset)) = self.take(field, key) else {
            return field.is_none();
        };
        if range.contains(&value) {
            *target = value;
            true
        } else {
            self.error(
                offset,
                key,
                DiagnosticKind::OutOfRange {
                    value: value.to_string(),
                    expected: format!("{} to {}", range.start(), range.end()),
                },
            );
            false
        }
    }

    fn error(&mut self, offset: usize, key: &str, kind: DiagnosticKind) {
        self.push(Some(offset), key, kind);
    }

    fn push(&mut self, offset: Option<usize>, key: &str, kind: DiagnosticKind) {
        self.errors
            .push(Diagnostic::new(self.source, offset, Some(key), kind));
    }
}
