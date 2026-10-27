use crate::Micros;
use crate::synth::frame::{TOUCHPAD_SLOTS, TouchpadGeometry, TouchpadPoint};
use crate::synth::scroll::StepLimiter;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chase {
    offset: TouchpadPoint,
    limiters: [StepLimiter; TOUCHPAD_SLOTS],
}

impl Chase {
    pub const fn new(t: Micros) -> Chase {
        Chase {
            offset: TouchpadPoint { x: 0, y: 0 },
            limiters: [StepLimiter::new(t); TOUCHPAD_SLOTS],
        }
    }

    pub const fn offset(&self) -> TouchpadPoint {
        self.offset
    }

    pub fn step(
        &mut self,
        t: Micros,
        target: TouchpadPoint,
        starts: &[TouchpadPoint],
        touchpad: &TouchpadGeometry,
    ) {
        let target = reachable(target, starts, touchpad);
        let wanted_x = f64::from(target.x - self.offset.x);
        let wanted_y = f64::from(target.y - self.offset.y);
        let wanted = wanted_x.hypot(wanted_y);
        if wanted == 0.0 {
            return;
        }
        let max_mm = self.limiters[..starts.len()]
            .iter()
            .map(|limiter| limiter.max_step_mm(t))
            .fold(f64::INFINITY, f64::min);
        let max_units = max_mm * f64::from(touchpad.units_per_mm);
        let scale = if wanted > max_units {
            max_units / wanted
        } else {
            1.0
        };
        let step = TouchpadPoint {
            x: truncate(wanted_x * scale),
            y: truncate(wanted_y * scale),
        };
        if step == TouchpadPoint::default() {
            return;
        }
        let before = self.offset;
        self.offset = TouchpadPoint {
            x: before.x + step.x,
            y: before.y + step.y,
        };
        for (limiter, start) in self.limiters.iter_mut().zip(starts) {
            let from = place(touchpad, *start, before);
            let to = place(touchpad, *start, self.offset);
            let moved_mm = f64::from(to.x - from.x).hypot(f64::from(to.y - from.y))
                / f64::from(touchpad.units_per_mm);
            limiter.record(t, moved_mm);
        }
    }
}

fn reachable(
    target: TouchpadPoint,
    starts: &[TouchpadPoint],
    touchpad: &TouchpadGeometry,
) -> TouchpadPoint {
    let min_x = starts.iter().map(|start| -start.x).max().unwrap_or(0);
    let max_x = starts
        .iter()
        .map(|start| touchpad.max_x - start.x)
        .min()
        .unwrap_or(0);
    let min_y = starts.iter().map(|start| -start.y).max().unwrap_or(0);
    let max_y = starts
        .iter()
        .map(|start| touchpad.max_y - start.y)
        .min()
        .unwrap_or(0);
    TouchpadPoint {
        x: target.x.clamp(min_x.min(0), max_x.max(0)),
        y: target.y.clamp(min_y.min(0), max_y.max(0)),
    }
}

pub fn place(
    touchpad: &TouchpadGeometry,
    start: TouchpadPoint,
    offset: TouchpadPoint,
) -> TouchpadPoint {
    touchpad.clamp(TouchpadPoint {
        x: start.x.saturating_add(offset.x),
        y: start.y.saturating_add(offset.y),
    })
}

#[allow(clippy::cast_possible_truncation)]
fn truncate(value: f64) -> i32 {
    value.trunc() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::VIRTUAL_TOUCHPAD;
    use crate::synth::scroll::{JUMP_MAX_STEP_INCREASE_MM, JUMP_REFERENCE_INTERVAL};

    #[test]
    fn the_first_step_is_limited_and_the_rest_carries_over() {
        let starts = [VIRTUAL_TOUCHPAD.centre()];
        let mut chase = Chase::new(Micros::ZERO);
        let target = TouchpadPoint { x: 0, y: -2_000 };
        chase.step(Micros::ZERO, target, &starts, &VIRTUAL_TOUCHPAD);
        assert_eq!(chase.offset(), TouchpadPoint::default());
        chase.step(JUMP_REFERENCE_INTERVAL, target, &starts, &VIRTUAL_TOUCHPAD);
        assert_eq!(
            chase.offset().y,
            -VIRTUAL_TOUCHPAD.mm_to_units(JUMP_MAX_STEP_INCREASE_MM)
        );
        let mut t = JUMP_REFERENCE_INTERVAL;
        for _ in 0..20 {
            t += JUMP_REFERENCE_INTERVAL;
            chase.step(t, target, &starts, &VIRTUAL_TOUCHPAD);
        }
        assert_eq!(chase.offset(), target);
    }

    #[test]
    fn the_offset_stops_where_the_outermost_finger_meets_the_edge() {
        let centre = VIRTUAL_TOUCHPAD.centre();
        let starts = [
            TouchpadPoint {
                x: centre.x - 400,
                y: centre.y,
            },
            TouchpadPoint {
                x: centre.x + 400,
                y: centre.y,
            },
        ];
        let mut chase = Chase::new(Micros::ZERO);
        let mut t = Micros::ZERO;
        for _ in 0..200 {
            t += JUMP_REFERENCE_INTERVAL;
            chase.step(
                t,
                TouchpadPoint {
                    x: 100_000,
                    y: -100_000,
                },
                &starts,
                &VIRTUAL_TOUCHPAD,
            );
        }
        assert_eq!(
            chase.offset(),
            TouchpadPoint {
                x: VIRTUAL_TOUCHPAD.max_x - starts[1].x,
                y: -centre.y,
            }
        );
    }
}
