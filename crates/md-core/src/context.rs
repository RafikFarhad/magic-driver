use crate::Micros;
use crate::buttons::ButtonSet;
use crate::primitives::Button;

pub const MOTION_ENERGY_TAU: Micros = Micros::from_millis(50);
pub const MOTION_ENERGY_FLOOR: f64 = 1.0;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Context {
    motion_counts: f64,
    motion_t: Option<Micros>,
    pressed: ButtonSet,
    last_press: Option<Micros>,
    last_release: Option<Micros>,
}

impl Context {
    pub fn new() -> Context {
        Context::default()
    }

    pub fn on_motion(&mut self, t: Micros, dx: i32, dy: i32) {
        let decayed = self.decayed_counts(t);
        let counts = decayed + f64::from(dx).hypot(f64::from(dy));
        self.motion_counts = if counts / tau_s() < MOTION_ENERGY_FLOOR {
            0.0
        } else {
            counts
        };
        self.motion_t = Some(self.motion_t.map_or(t, |previous| previous.max(t)));
    }

    pub fn motion_energy(&self, t: Micros) -> f64 {
        let rate = self.decayed_counts(t) / tau_s();
        if rate < MOTION_ENERGY_FLOOR {
            0.0
        } else {
            rate
        }
    }

    pub fn motion_counts(&self, t: Micros) -> f64 {
        self.decayed_counts(t)
    }

    pub fn motion_energy_since(&self, t: Micros, baseline: f64, baseline_t: Micros) -> f64 {
        let elapsed = t.saturating_sub(baseline_t).as_secs_f64();
        let carried = baseline * (-elapsed / tau_s()).exp();
        let rate = (self.decayed_counts(t) - carried).max(0.0) / tau_s();
        if rate < MOTION_ENERGY_FLOOR {
            0.0
        } else {
            rate
        }
    }

    pub fn on_button(&mut self, t: Micros, button: Button, pressed: bool) {
        let was_down = !self.pressed.is_empty();
        self.pressed.set(button, pressed);
        let is_down = !self.pressed.is_empty();
        if is_down && !was_down {
            self.last_press = Some(t);
        }
        if was_down && !is_down {
            self.last_release = Some(t);
        }
    }

    pub const fn pressed(&self) -> ButtonSet {
        self.pressed
    }

    pub const fn is_button_down(&self) -> bool {
        !self.pressed.is_empty()
    }

    pub const fn last_press(&self) -> Option<Micros> {
        self.last_press
    }

    pub const fn last_release(&self) -> Option<Micros> {
        self.last_release
    }

    pub fn since_last_press(&self, t: Micros) -> Option<Micros> {
        self.last_press.map(|press| t - press)
    }

    pub fn since_last_release(&self, t: Micros) -> Option<Micros> {
        self.last_release.map(|release| t - release)
    }

    pub fn reset(&mut self) {
        *self = Context::default();
    }

    fn decayed_counts(&self, t: Micros) -> f64 {
        let Some(motion_t) = self.motion_t else {
            return 0.0;
        };
        let elapsed = (t - motion_t).as_secs_f64();
        self.motion_counts * (-elapsed / tau_s()).exp()
    }
}

fn tau_s() -> f64 {
    MOTION_ENERGY_TAU.as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: u64 = 11_111;

    fn moving(context: &mut Context, frames: u64, dx: i32, dy: i32) -> Micros {
        let mut t = Micros::ZERO;
        for frame in 0..frames {
            t = Micros::new(frame * FRAME);
            context.on_motion(t, dx, dy);
        }
        t
    }

    #[test]
    fn still_mouse_has_zero_energy() {
        let mut context = Context::new();
        assert!(context.motion_energy(Micros::ZERO).abs() < f64::EPSILON);
        let t = moving(&mut context, 20, 0, 0);
        assert!(context.motion_energy(t).abs() < f64::EPSILON);
    }

    #[test]
    fn energy_rises_on_movement_and_approaches_the_motion_rate() {
        let mut context = Context::new();
        let mut previous = 0.0;
        for frame in 0..5 {
            let t = Micros::new(frame * FRAME);
            context.on_motion(t, 3, 4);
            let energy = context.motion_energy(t);
            assert!(energy > previous, "frame {frame}: {energy} <= {previous}");
            previous = energy;
        }
        let t = moving(&mut context, 60, 3, 4);
        let rate = 5.0 / Micros::new(FRAME).as_secs_f64();
        let energy = context.motion_energy(t);
        assert!(
            (energy - rate).abs() / rate < 0.3,
            "energy {energy} rate {rate}"
        );
    }

    #[test]
    fn energy_decays_with_time_constant_and_falls_to_zero() {
        let mut context = Context::new();
        let t = moving(&mut context, 30, 10, 0);
        let start = context.motion_energy(t);
        let one_tau = context.motion_energy(t + MOTION_ENERGY_TAU);
        assert!((one_tau / start - (-1.0_f64).exp()).abs() < 1e-9);
        let mut previous = start;
        for ms in (10..=1000).step_by(10) {
            let energy = context.motion_energy(t + Micros::from_millis(ms));
            assert!(energy <= previous);
            previous = energy;
        }
        assert!(context.motion_energy(t + Micros::from_millis(1000)).abs() < f64::EPSILON);
    }

    #[test]
    fn still_reports_after_movement_decay_the_stored_energy_to_zero() {
        let mut context = Context::new();
        let t = moving(&mut context, 10, -20, 20);
        let mut now = t;
        for _ in 0..100 {
            now += Micros::new(FRAME);
            context.on_motion(now, 0, 0);
        }
        assert!(context.motion_energy(now).abs() < f64::EPSILON);
        context.on_motion(now + Micros::new(FRAME), 1, 0);
        assert!(context.motion_energy(now + Micros::new(FRAME)) > 0.0);
    }

    #[test]
    fn diagonal_and_negative_motion_count_by_magnitude() {
        let mut a = Context::new();
        let mut b = Context::new();
        a.on_motion(Micros::ZERO, 3, 4);
        b.on_motion(Micros::ZERO, -5, 0);
        assert!((a.motion_energy(Micros::ZERO) - b.motion_energy(Micros::ZERO)).abs() < 1e-9);
    }

    #[test]
    fn earlier_timestamps_do_not_increase_energy() {
        let mut context = Context::new();
        context.on_motion(Micros::from_millis(100), 10, 0);
        let energy = context.motion_energy(Micros::from_millis(100));
        assert!((context.motion_energy(Micros::from_millis(50)) - energy).abs() < 1e-9);
    }

    #[test]
    fn button_press_and_release_times() {
        let mut context = Context::new();
        assert!(!context.is_button_down());
        assert_eq!(context.last_press(), None);
        context.on_button(Micros::from_millis(10), Button::Left, true);
        assert!(context.is_button_down());
        assert!(context.pressed().contains(Button::Left));
        assert_eq!(context.last_press(), Some(Micros::from_millis(10)));
        assert_eq!(
            context.since_last_press(Micros::from_millis(25)),
            Some(Micros::from_millis(15))
        );
        context.on_button(Micros::from_millis(40), Button::Left, false);
        assert!(!context.is_button_down());
        assert_eq!(context.last_release(), Some(Micros::from_millis(40)));
        assert_eq!(
            context.since_last_release(Micros::from_millis(41)),
            Some(Micros::from_millis(1))
        );
    }

    #[test]
    fn press_time_is_the_first_of_several_buttons() {
        let mut context = Context::new();
        context.on_button(Micros::new(1), Button::Left, true);
        context.on_button(Micros::new(2), Button::Right, true);
        context.on_button(Micros::new(3), Button::Left, false);
        assert_eq!(context.last_press(), Some(Micros::new(1)));
        assert_eq!(context.last_release(), None);
        context.on_button(Micros::new(4), Button::Right, false);
        assert_eq!(context.last_release(), Some(Micros::new(4)));
    }

    #[test]
    fn reset_clears_everything() {
        let mut context = Context::new();
        context.on_motion(Micros::ZERO, 100, 0);
        context.on_button(Micros::ZERO, Button::Middle, true);
        context.reset();
        assert_eq!(context, Context::new());
    }
}
