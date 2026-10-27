use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GainCurve {
    pub gain_min: f64,
    pub gain_max: f64,
    pub speed_start_mm_s: f64,
    pub speed_end_mm_s: f64,
}

impl GainCurve {
    pub const fn constant(gain: f64) -> GainCurve {
        GainCurve {
            gain_min: gain,
            gain_max: gain,
            speed_start_mm_s: 0.0,
            speed_end_mm_s: 1.0,
        }
    }

    pub fn from_config(config: &Config) -> GainCurve {
        let scroll = &config.advanced.scroll;
        let [speed_start_mm_s, speed_end_mm_s] = scroll.gain_speed_mm_s;
        GainCurve {
            gain_min: scroll.gain_min,
            gain_max: scroll.gain_max,
            speed_start_mm_s,
            speed_end_mm_s,
        }
    }

    pub fn gain(&self, speed_mm_s: f64) -> f64 {
        if !speed_mm_s.is_finite() || speed_mm_s <= self.speed_start_mm_s {
            return self.gain_min;
        }
        if speed_mm_s >= self.speed_end_mm_s {
            return self.gain_max;
        }
        let progress =
            (speed_mm_s - self.speed_start_mm_s) / (self.speed_end_mm_s - self.speed_start_mm_s);
        self.gain_min + (self.gain_max - self.gain_min) * progress
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ScrollAcceleration;

    fn medium() -> GainCurve {
        GainCurve::from_config(&Config::default())
    }

    fn with_acceleration(acceleration: ScrollAcceleration) -> GainCurve {
        let preset = acceleration.preset();
        let [speed_start_mm_s, speed_end_mm_s] = preset.gain_speed_mm_s;
        GainCurve {
            gain_min: preset.gain_min,
            gain_max: preset.gain_max,
            speed_start_mm_s,
            speed_end_mm_s,
        }
    }

    fn speeds() -> impl Iterator<Item = f64> {
        (0..=4_000).map(|step| f64::from(step) * 0.25)
    }

    #[test]
    fn default_curve_is_the_medium_preset() {
        assert_eq!(medium(), with_acceleration(ScrollAcceleration::Medium));
        assert_eq!(
            medium(),
            GainCurve {
                gain_min: 1.0,
                gain_max: 2.5,
                speed_start_mm_s: 20.0,
                speed_end_mm_s: 250.0,
            }
        );
    }

    #[test]
    fn slow_speeds_get_gain_min_and_fast_speeds_gain_max() {
        let curve = medium();
        for speed in [0.0, 5.0, 19.9, 20.0] {
            assert!((curve.gain(speed) - 1.0).abs() < f64::EPSILON, "{speed}");
        }
        for speed in [250.0, 300.0, 5_000.0, f64::MAX] {
            assert!((curve.gain(speed) - 2.5).abs() < f64::EPSILON, "{speed}");
        }
        assert!((curve.gain(135.0) - 1.75).abs() < 1e-12);
    }

    #[test]
    fn curve_is_monotonic_and_bounded() {
        for acceleration in [
            ScrollAcceleration::Off,
            ScrollAcceleration::Low,
            ScrollAcceleration::Medium,
            ScrollAcceleration::High,
        ] {
            let curve = with_acceleration(acceleration);
            let mut previous = curve.gain(0.0);
            for speed in speeds() {
                let gain = curve.gain(speed);
                assert!(gain >= previous, "{acceleration:?} at {speed}");
                assert!(
                    (curve.gain_min..=curve.gain_max).contains(&gain),
                    "{acceleration:?} at {speed}"
                );
                previous = gain;
            }
        }
    }

    #[test]
    fn curve_is_continuous() {
        let curve = with_acceleration(ScrollAcceleration::High);
        let slope =
            (curve.gain_max - curve.gain_min) / (curve.speed_end_mm_s - curve.speed_start_mm_s);
        let step = 0.25;
        let mut previous = curve.gain(0.0);
        for speed in speeds().skip(1) {
            let gain = curve.gain(speed);
            assert!((gain - previous).abs() <= slope * step + 1e-12, "{speed}");
            previous = gain;
        }
        for edge in [curve.speed_start_mm_s, curve.speed_end_mm_s] {
            let below = curve.gain(edge - 1e-9);
            let above = curve.gain(edge + 1e-9);
            assert!((above - below).abs() < 1e-6, "{edge}");
        }
    }

    #[test]
    fn acceleration_off_is_constant_gain_min() {
        let curve = with_acceleration(ScrollAcceleration::Off);
        for speed in speeds() {
            assert!((curve.gain(speed) - curve.gain_min).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn presets_amplify_more_from_low_to_high() {
        let fast = 400.0;
        let gains: Vec<f64> = [
            ScrollAcceleration::Off,
            ScrollAcceleration::Low,
            ScrollAcceleration::Medium,
            ScrollAcceleration::High,
        ]
        .into_iter()
        .map(|acceleration| with_acceleration(acceleration).gain(fast))
        .collect();
        assert!(gains.windows(2).all(|pair| pair[0] < pair[1]), "{gains:?}");
    }

    #[test]
    fn invalid_speeds_fall_back_to_gain_min() {
        let curve = medium();
        assert!((curve.gain(f64::NAN) - 1.0).abs() < f64::EPSILON);
        assert!((curve.gain(-10.0) - 1.0).abs() < f64::EPSILON);
        assert!((curve.gain(f64::INFINITY) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn constant_curve_ignores_speed() {
        let curve = GainCurve::constant(2.0);
        for speed in [0.0, 50.0, 1_000.0] {
            assert!((curve.gain(speed) - 2.0).abs() < f64::EPSILON);
        }
    }
}
