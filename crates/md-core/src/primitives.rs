use std::fmt;
use std::ops::{Add, AddAssign, Sub, SubAssign};

use serde::Serialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Micros(u64);

impl Micros {
    pub const ZERO: Micros = Micros(0);
    pub const MAX: Micros = Micros(u64::MAX);

    pub const fn new(micros: u64) -> Self {
        Micros(micros)
    }

    pub const fn from_millis(millis: u64) -> Self {
        Micros(millis.saturating_mul(1_000))
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }

    pub const fn as_millis(self) -> u64 {
        self.0 / 1_000
    }

    #[allow(clippy::cast_precision_loss)]
    pub fn as_secs_f64(self) -> f64 {
        self.0 as f64 / 1_000_000.0
    }

    #[must_use]
    pub const fn saturating_add(self, other: Micros) -> Micros {
        Micros(self.0.saturating_add(other.0))
    }

    #[must_use]
    pub const fn saturating_sub(self, other: Micros) -> Micros {
        Micros(self.0.saturating_sub(other.0))
    }
}

impl Add for Micros {
    type Output = Micros;

    fn add(self, other: Micros) -> Micros {
        self.saturating_add(other)
    }
}

impl AddAssign for Micros {
    fn add_assign(&mut self, other: Micros) {
        *self = self.saturating_add(other);
    }
}

impl Sub for Micros {
    type Output = Micros;

    fn sub(self, other: Micros) -> Micros {
        self.saturating_sub(other)
    }
}

impl SubAssign for Micros {
    fn sub_assign(&mut self, other: Micros) {
        *self = self.saturating_sub(other);
    }
}

impl fmt::Display for Button {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Button::Left => "left",
            Button::Right => "right",
            Button::Middle => "middle",
        })
    }
}

impl fmt::Display for Micros {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}us", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    Vertical,
    Horizontal,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn micros_add_saturates_at_max() {
        assert_eq!(Micros::MAX + Micros::new(1), Micros::MAX);
        assert_eq!(Micros::new(u64::MAX - 1) + Micros::new(5), Micros::MAX);
        assert_eq!(Micros::new(2) + Micros::new(3), Micros::new(5));
    }

    #[test]
    fn micros_sub_saturates_at_zero() {
        assert_eq!(Micros::new(3) - Micros::new(5), Micros::ZERO);
        assert_eq!(Micros::new(5) - Micros::new(3), Micros::new(2));
    }

    #[test]
    fn micros_assign_operators_saturate() {
        let mut t = Micros::new(10);
        t -= Micros::new(20);
        assert_eq!(t, Micros::ZERO);
        t += Micros::MAX;
        t += Micros::new(1);
        assert_eq!(t, Micros::MAX);
    }

    #[test]
    fn micros_millis_conversions() {
        assert_eq!(Micros::from_millis(150), Micros::new(150_000));
        assert_eq!(Micros::from_millis(u64::MAX), Micros::MAX);
        assert_eq!(Micros::new(150_999).as_millis(), 150);
        assert!((Micros::new(1_500_000).as_secs_f64() - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn micros_orders_by_time() {
        assert!(Micros::new(1) < Micros::new(2));
        assert_eq!(Micros::default(), Micros::ZERO);
        assert_eq!(Micros::new(42).to_string(), "42us");
    }

    #[test]
    fn buttons_display_in_lowercase() {
        assert_eq!(Button::Left.to_string(), "left");
        assert_eq!(Button::Right.to_string(), "right");
        assert_eq!(Button::Middle.to_string(), "middle");
    }
}
