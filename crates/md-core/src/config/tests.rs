#![allow(clippy::float_cmp)]

use super::*;

fn parse(source: &str) -> Result<ParsedConfig, ConfigErrors> {
    Config::parse(source, &MAGIC_MOUSE_2_BT)
}

fn parse_ok(source: &str) -> Config {
    let parsed = parse(source).expect("config parses");
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    parsed.config
}

fn parse_err(source: &str) -> ConfigErrors {
    parse(source).expect_err("config is rejected")
}

fn error_summary(errors: &ConfigErrors) -> Vec<(Option<usize>, Option<String>)> {
    errors
        .errors
        .iter()
        .map(|error| (error.line, error.key.clone()))
        .collect()
}

#[test]
fn rendered_defaults_parse_back_to_default() {
    let rendered = Config::default().render();
    assert_eq!(parse_ok(&rendered), Config::default());
}

#[test]
fn rendered_defaults_snapshot() {
    insta::assert_snapshot!(Config::default().render());
}

#[test]
fn rendered_non_default_config_round_trips() {
    let mut config = Config::default();
    config.scroll.acceleration = ScrollAcceleration::High;
    config.touch.palm_rejection = PalmRejection::Low;
    config.pointer.middle_click = false;
    config.log.level = LogLevel::Debug;
    config.advanced.scroll.gain_max = 3.25;
    config.advanced.touch.rear_zone = 0.4;
    config.advanced.gestures.kwin_swipe_fingers = 3;
    assert_eq!(parse_ok(&config.render()), config);
}

#[test]
fn empty_file_means_defaults() {
    assert_eq!(parse_ok(""), Config::default());
    assert_eq!(parse_ok("\n\n"), Config::default());
}

#[test]
fn default_values_match_the_documented_defaults() {
    let config = Config::default();
    assert_eq!(config.version, 1);
    assert!(config.pointer.middle_click);
    assert!(config.scroll.enabled && config.scroll.horizontal && config.scroll.axis_lock);
    assert_eq!(config.scroll.acceleration, ScrollAcceleration::Medium);
    assert_eq!(config.touch.palm_rejection, PalmRejection::Medium);
    assert!(config.gestures.swipe_desktops && config.gestures.double_tap_overview);
    assert!(!config.gestures.smart_zoom);
    assert_eq!(config.log.level, LogLevel::Info);
    let advanced = &config.advanced;
    assert_eq!(advanced.scroll.gain_min, 1.0);
    assert_eq!(advanced.scroll.gain_max, 2.5);
    assert_eq!(advanced.scroll.gain_speed_mm_s, [20.0, 250.0]);
    assert_eq!(advanced.scroll.cancel_hold_ms, 60);
    assert_eq!(advanced.touch.promote_distance_mm, 1.5);
    assert_eq!(advanced.touch.promote_window_ms, 150);
    assert_eq!(advanced.touch.rest_dwell_ms, 300);
    assert_eq!(
        advanced.touch.palm_size_mm,
        MAGIC_MOUSE_2_BT.default_palm_size_mm
    );
    assert_eq!(advanced.touch.rear_zone, 0.35);
    assert_eq!(
        advanced.touch.mouse_motion_threshold,
        MAGIC_MOUSE_2_BT.default_mouse_motion_threshold
    );
    assert_eq!(advanced.touch.click_guard_ms, 100);
    assert_eq!(advanced.gestures.double_tap_ms, 300);
    assert_eq!(advanced.gestures.swipe_distance_mm, 8.0);
}

#[test]
fn the_minimal_example_parses() {
    let config = parse_ok(
        "version = 1\n\n[scroll]\nacceleration = \"high\"\n\n[touch]\npalm_rejection = \"high\"\n\n[gestures]\ndouble_tap_overview = false\n",
    );
    assert_eq!(config.scroll.acceleration, ScrollAcceleration::High);
    assert_eq!(config.touch.palm_rejection, PalmRejection::High);
    assert!(!config.gestures.double_tap_overview);
    assert!(config.gestures.swipe_desktops);
}

#[test]
fn every_scroll_acceleration_preset_expands_to_advanced_values() {
    for (name, level) in [
        ("off", ScrollAcceleration::Off),
        ("low", ScrollAcceleration::Low),
        ("medium", ScrollAcceleration::Medium),
        ("high", ScrollAcceleration::High),
    ] {
        let config = parse_ok(&format!("[scroll]\nacceleration = \"{name}\"\n"));
        let preset = level.preset();
        assert_eq!(config.scroll.acceleration, level);
        assert_eq!(config.advanced.scroll.gain_min, preset.gain_min);
        assert_eq!(config.advanced.scroll.gain_max, preset.gain_max);
        assert_eq!(
            config.advanced.scroll.gain_speed_mm_s,
            preset.gain_speed_mm_s
        );
    }
}

#[test]
fn scroll_acceleration_presets_increase_gain() {
    let gains: Vec<f64> = [
        ScrollAcceleration::Off,
        ScrollAcceleration::Low,
        ScrollAcceleration::Medium,
        ScrollAcceleration::High,
    ]
    .iter()
    .map(|level| level.preset().gain_max)
    .collect();
    assert_eq!(gains[0], 1.0);
    assert!(gains.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn every_palm_rejection_preset_expands_to_advanced_values() {
    for (name, level) in [
        ("low", PalmRejection::Low),
        ("medium", PalmRejection::Medium),
        ("high", PalmRejection::High),
    ] {
        let config = parse_ok(&format!("[touch]\npalm_rejection = \"{name}\"\n"));
        let preset = level.preset(&MAGIC_MOUSE_2_BT);
        let touch = &config.advanced.touch;
        assert_eq!(config.touch.palm_rejection, level);
        assert_eq!(touch.promote_distance_mm, preset.promote_distance_mm);
        assert_eq!(touch.rest_dwell_ms, preset.rest_dwell_ms);
        assert_eq!(touch.palm_size_mm, preset.palm_size_mm);
        assert_eq!(touch.rear_zone, preset.rear_zone);
        assert_eq!(touch.mouse_motion_threshold, preset.mouse_motion_threshold);
        assert_eq!(touch.click_guard_ms, preset.click_guard_ms);
        assert_eq!(touch.promote_window_ms, 150);
    }
}

#[test]
fn palm_rejection_presets_get_stricter() {
    let low = PalmRejection::Low.preset(&MAGIC_MOUSE_2_BT);
    let medium = PalmRejection::Medium.preset(&MAGIC_MOUSE_2_BT);
    let high = PalmRejection::High.preset(&MAGIC_MOUSE_2_BT);
    assert!(low.palm_size_mm > medium.palm_size_mm && medium.palm_size_mm > high.palm_size_mm);
    assert!(low.rear_zone < medium.rear_zone && medium.rear_zone < high.rear_zone);
    assert!(
        low.mouse_motion_threshold > medium.mouse_motion_threshold
            && medium.mouse_motion_threshold > high.mouse_motion_threshold
    );
    assert!(
        low.click_guard_ms < medium.click_guard_ms && medium.click_guard_ms < high.click_guard_ms
    );
}

#[test]
fn palm_rejection_presets_scale_with_profile_defaults() {
    let profile = Profile {
        default_palm_size_mm: 30.0,
        default_mouse_motion_threshold: 400.0,
        ..MAGIC_MOUSE_2_BT
    };
    let config = Config::for_profile(&profile);
    assert_eq!(config.advanced.touch.palm_size_mm, 30.0);
    assert_eq!(config.advanced.touch.mouse_motion_threshold, 400.0);
    let high = Config::parse("[touch]\npalm_rejection = \"high\"\n", &profile)
        .expect("config parses")
        .config;
    assert_eq!(high.advanced.touch.palm_size_mm, 22.5);
    assert_eq!(high.advanced.touch.mouse_motion_threshold, 200.0);
}

#[test]
fn explicit_advanced_keys_override_presets() {
    let config = parse_ok(
        "[advanced.scroll]\ngain_max = 3.0\n\n[scroll]\nacceleration = \"off\"\n\n[touch]\npalm_rejection = \"high\"\n\n[advanced.touch]\nrear_zone = 0.2\npalm_size_mm = 12.5\n",
    );
    let off = ScrollAcceleration::Off.preset();
    let high = PalmRejection::High.preset(&MAGIC_MOUSE_2_BT);
    assert_eq!(config.advanced.scroll.gain_max, 3.0);
    assert_eq!(config.advanced.scroll.gain_min, off.gain_min);
    assert_eq!(config.advanced.touch.rear_zone, 0.2);
    assert_eq!(config.advanced.touch.palm_size_mm, 12.5);
    assert_eq!(config.advanced.touch.click_guard_ms, high.click_guard_ms);
    assert_eq!(config.advanced.touch.rest_dwell_ms, high.rest_dwell_ms);
}

#[test]
fn integers_are_accepted_for_float_keys() {
    let config = parse_ok("[advanced.scroll]\ngain_speed_mm_s = [20, 300]\ngain_max = 3\n");
    assert_eq!(config.advanced.scroll.gain_speed_mm_s, [20.0, 300.0]);
    assert_eq!(config.advanced.scroll.gain_max, 3.0);
}

#[test]
fn dotted_and_inline_keys_are_supported() {
    let config = parse_ok(
        "scroll.acceleration = \"low\"\nadvanced = { gestures = { kwin_swipe_fingers = 3 } }\n",
    );
    assert_eq!(config.scroll.acceleration, ScrollAcceleration::Low);
    assert_eq!(config.advanced.gestures.kwin_swipe_fingers, 3);
}

#[test]
fn out_of_range_values_are_all_reported_with_lines() {
    let errors = parse_err(
        "[advanced.scroll]\ncancel_hold_ms = 5000\ngain_max = 50.0\n\n[advanced.touch]\nrear_zone = 1.5\npromote_window_ms = 1\n\n[advanced.gestures]\nkwin_swipe_fingers = 2\ndouble_tap_ms = 10\nswipe_distance_mm = 0.5\n",
    );
    assert_eq!(
        error_summary(&errors),
        vec![
            (Some(2), Some("advanced.scroll.cancel_hold_ms".to_owned())),
            (Some(3), Some("advanced.scroll.gain_max".to_owned())),
            (Some(6), Some("advanced.touch.rear_zone".to_owned())),
            (Some(7), Some("advanced.touch.promote_window_ms".to_owned())),
            (
                Some(10),
                Some("advanced.gestures.kwin_swipe_fingers".to_owned())
            ),
            (Some(11), Some("advanced.gestures.double_tap_ms".to_owned())),
            (
                Some(12),
                Some("advanced.gestures.swipe_distance_mm".to_owned())
            ),
        ]
    );
    assert!(
        errors
            .errors
            .iter()
            .all(|error| matches!(error.kind, DiagnosticKind::OutOfRange { .. }))
    );
    assert_eq!(
        errors.errors[2].to_string(),
        "line 6: advanced.touch.rear_zone: value 1.5 is out of range, expected 0 to 0.9"
    );
}

#[test]
fn non_finite_floats_are_out_of_range() {
    let errors = parse_err("[advanced.touch]\npalm_size_mm = nan\nmouse_motion_threshold = inf\n");
    assert_eq!(
        error_summary(&errors),
        vec![
            (Some(2), Some("advanced.touch.palm_size_mm".to_owned())),
            (
                Some(3),
                Some("advanced.touch.mouse_motion_threshold".to_owned())
            ),
        ]
    );
}

#[test]
fn gain_max_below_gain_min_is_an_error() {
    let errors = parse_err("[advanced.scroll]\ngain_min = 2.0\ngain_max = 1.5\n");
    assert_eq!(
        error_summary(&errors),
        vec![(Some(3), Some("advanced.scroll.gain_max".to_owned()))]
    );
    let errors = parse_err("[scroll]\nacceleration = \"off\"\n[advanced.scroll]\ngain_min = 2.0\n");
    assert_eq!(
        error_summary(&errors),
        vec![(Some(4), Some("advanced.scroll.gain_min".to_owned()))]
    );
}

#[test]
fn gain_speed_must_be_increasing_and_in_range() {
    for value in ["[250, 20]", "[20, 20]", "[-1, 20]", "[20, 9000]"] {
        let errors = parse_err(&format!("[advanced.scroll]\ngain_speed_mm_s = {value}\n"));
        assert_eq!(
            error_summary(&errors),
            vec![(Some(2), Some("advanced.scroll.gain_speed_mm_s".to_owned()))],
            "{value}"
        );
    }
    let errors = parse_err("[advanced.scroll]\ngain_speed_mm_s = [20, 30, 40]\n");
    assert!(matches!(
        errors.errors[0].kind,
        DiagnosticKind::InvalidValue(_)
    ));
}

#[test]
fn wrong_types_are_all_reported_with_lines() {
    let errors = parse_err(
        "[scroll]\nenabled = \"yes\"\nacceleration = \"fast\"\n\n[log]\nlevel = 3\n\n[advanced.touch]\nclick_guard_ms = -5\n",
    );
    assert_eq!(
        error_summary(&errors),
        vec![
            (Some(2), Some("scroll.enabled".to_owned())),
            (Some(3), Some("scroll.acceleration".to_owned())),
            (Some(6), Some("log.level".to_owned())),
            (Some(9), Some("advanced.touch.click_guard_ms".to_owned())),
        ]
    );
    assert!(
        errors
            .errors
            .iter()
            .all(|error| matches!(error.kind, DiagnosticKind::InvalidValue(_)))
    );
    let acceleration = errors.errors[1].to_string();
    assert!(acceleration.contains("fast"), "{acceleration}");
    assert!(acceleration.contains("medium"), "{acceleration}");
}

#[test]
fn errors_keep_warnings() {
    let errors = parse_err("colour = \"red\"\n[scroll]\nenabled = 1\n");
    assert_eq!(errors.errors.len(), 1);
    assert_eq!(errors.warnings.len(), 1);
    assert_eq!(errors.warnings[0].key.as_deref(), Some("colour"));
}

#[test]
fn unknown_keys_are_warnings_with_lines() {
    let parsed = parse(
        "version = 1\ncolour = \"red\"\n\n[scroll]\nspeed = 3\nenabled = false\n\n[advanced.touch]\npalm_size = 10\n\n[advanced.extra]\nx = 1\n\n[themes]\ndark = true\n",
    )
    .expect("unknown keys do not fail");
    assert!(!parsed.config.scroll.enabled);
    let warnings: Vec<(Option<usize>, Option<String>, DiagnosticKind)> = parsed
        .warnings
        .iter()
        .map(|warning| (warning.line, warning.key.clone(), warning.kind.clone()))
        .collect();
    assert_eq!(
        warnings,
        vec![
            (
                Some(2),
                Some("colour".to_owned()),
                DiagnosticKind::UnknownKey
            ),
            (
                Some(5),
                Some("scroll.speed".to_owned()),
                DiagnosticKind::UnknownKey
            ),
            (
                Some(9),
                Some("advanced.touch.palm_size".to_owned()),
                DiagnosticKind::UnknownKey
            ),
            (
                Some(11),
                Some("advanced.extra".to_owned()),
                DiagnosticKind::UnknownKey
            ),
            (
                Some(14),
                Some("themes".to_owned()),
                DiagnosticKind::UnknownKey
            ),
        ]
    );
}

#[test]
fn syntax_errors_report_the_line() {
    let errors = parse_err("version = 1\n[scroll\nenabled = true\n");
    assert_eq!(errors.errors.len(), 1);
    assert_eq!(errors.errors[0].line, Some(2));
    assert!(matches!(errors.errors[0].kind, DiagnosticKind::Syntax(_)));
}

#[test]
fn section_with_wrong_type_is_an_error() {
    let errors = parse_err("scroll = 3\n");
    assert_eq!(errors.errors.len(), 1);
    assert_eq!(errors.errors[0].line, Some(1));
}

#[test]
fn unsupported_version_is_an_error() {
    let errors = parse_err("version = 2\n");
    assert_eq!(
        errors.errors,
        vec![Diagnostic {
            line: Some(1),
            key: Some("version".to_owned()),
            kind: DiagnosticKind::UnsupportedVersion(2),
        }]
    );
    assert_eq!(parse_ok("version = 1\n"), Config::default());
}
