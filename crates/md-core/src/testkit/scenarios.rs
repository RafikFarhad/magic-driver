use crate::Micros;
use crate::engine::{Engine, Input, Output};
use crate::testkit::report_builder::{ReportBuilder, TouchRecord, touch_state};
use crate::testkit::stream::SYNTHETIC_REPORT_INTERVAL_US;

pub const PALM_ID: u8 = 1;
pub const FINGER_ID: u8 = 2;
pub const GRIP_ID: u8 = 3;
pub const SECOND_FINGER_ID: u8 = 4;

pub fn palm() -> TouchRecord {
    TouchRecord::new(PALM_ID, 100, 1_700)
        .touch_major(170)
        .touch_minor(140)
        .size(40)
}

pub fn grip() -> TouchRecord {
    TouchRecord::new(GRIP_ID, -1_000, 300)
        .touch_major(60)
        .touch_minor(48)
        .size(14)
}

pub fn finger(x: i32, y: i32) -> TouchRecord {
    TouchRecord::new(FINGER_ID, x, y)
        .touch_major(44)
        .touch_minor(36)
        .size(12)
}

pub fn second_finger(x: i32, y: i32) -> TouchRecord {
    TouchRecord::new(SECOND_FINGER_ID, x, y)
        .touch_major(44)
        .touch_minor(36)
        .size(12)
}

#[derive(Debug, Clone, Default)]
pub struct Scenario {
    reports: Vec<ReportBuilder>,
}

impl Scenario {
    pub fn new() -> Scenario {
        Scenario::default()
    }

    pub fn push(&mut self, report: ReportBuilder) -> &mut Scenario {
        self.reports.push(report);
        self
    }

    pub fn repeat(&mut self, frames: usize, report: &ReportBuilder) -> &mut Scenario {
        for _ in 0..frames {
            self.reports.push(report.clone());
        }
        self
    }

    pub fn len(&self) -> usize {
        self.reports.len()
    }

    pub fn is_empty(&self) -> bool {
        self.reports.is_empty()
    }

    pub fn stroke(
        &mut self,
        background: &[TouchRecord],
        start: (i32, i32),
        steps: &[(i32, i32)],
    ) -> &mut Scenario {
        let (mut x, mut y) = start;
        self.push(with(background).touch(finger(x, y).state(touch_state::START)));
        for (dx, dy) in steps {
            x += dx;
            y += dy;
            self.push(with(background).touch(finger(x, y)));
        }
        self.push(with(background).touch(finger(x, y).state(touch_state::LIFT_50)));
        self.push(with(background).touch(finger(x, y).state(touch_state::GONE)));
        self
    }

    pub fn two_finger_stroke(
        &mut self,
        background: &[TouchRecord],
        starts: [(i32, i32); 2],
        steps: &[(i32, i32)],
    ) -> &mut Scenario {
        let [(mut ax, mut ay), (mut bx, mut by)] = starts;
        self.push(
            with(background)
                .touch(finger(ax, ay).state(touch_state::START))
                .touch(second_finger(bx, by).state(touch_state::START)),
        );
        for (dx, dy) in steps {
            ax += dx;
            ay += dy;
            bx += dx;
            by += dy;
            self.push(
                with(background)
                    .touch(finger(ax, ay))
                    .touch(second_finger(bx, by)),
            );
        }
        self.push(
            with(background)
                .touch(finger(ax, ay).state(touch_state::LIFT_50))
                .touch(second_finger(bx, by).state(touch_state::LIFT_50)),
        );
        self.push(with(background));
        self
    }

    pub fn two_finger_tap(
        &mut self,
        background: &[TouchRecord],
        frames: usize,
        left: bool,
    ) -> &mut Scenario {
        let a = finger(-400, 200);
        let b = second_finger(200, 200);
        self.push(
            with(background)
                .touch(a.state(touch_state::START))
                .touch(b.state(touch_state::START)),
        );
        for frame in 0..frames {
            let jitter = i32::try_from(frame % 3).unwrap_or(0) - 1;
            self.push(
                with(background)
                    .left(left)
                    .touch(TouchRecord {
                        x: a.x + jitter,
                        ..a
                    })
                    .touch(TouchRecord {
                        y: b.y + jitter,
                        ..b
                    }),
            );
        }
        self.push(
            with(background)
                .touch(a.state(touch_state::LIFT_50))
                .touch(b.state(touch_state::LIFT_50)),
        );
        self.push(with(background));
        self
    }

    pub fn timed(&self) -> Vec<(Micros, Vec<u8>)> {
        self.reports
            .iter()
            .zip(0_u64..)
            .map(|(builder, index)| {
                (
                    Micros::new(index * SYNTHETIC_REPORT_INTERVAL_US),
                    builder.build(),
                )
            })
            .collect()
    }
}

fn with(background: &[TouchRecord]) -> ReportBuilder {
    background
        .iter()
        .fold(ReportBuilder::new(), |builder, touch| builder.touch(*touch))
}

pub fn idle_resting_palm() -> Scenario {
    let mut scenario = Scenario::new();
    scenario.push(ReportBuilder::new().touch(palm().state(touch_state::START)));
    scenario.repeat(20, &ReportBuilder::new().touch(palm()));
    scenario.push(
        ReportBuilder::new()
            .touch(palm())
            .touch(grip().state(touch_state::START)),
    );
    for frame in 0..90 {
        let jitter = (frame % 3) - 1;
        scenario.push(
            ReportBuilder::new()
                .touch(TouchRecord {
                    x: palm().x + jitter * 4,
                    ..palm()
                })
                .touch(TouchRecord {
                    y: grip().y + jitter * 3,
                    ..grip()
                }),
        );
    }
    scenario.push(ReportBuilder::new().touch(palm().state(touch_state::LIFT_50)));
    scenario.push(ReportBuilder::new());
    scenario
}

pub fn move_fingers_resting() -> Scenario {
    let mut scenario = Scenario::new();
    let resting = finger(-200, 300);
    scenario.push(
        ReportBuilder::new()
            .touch(palm())
            .touch(grip().state(touch_state::START))
            .touch(resting.state(touch_state::START)),
    );
    for frame in 0..100_i16 {
        let drift = i32::from(frame);
        let dx = 6 + frame % 5;
        let dy = -4 + frame % 3;
        scenario.push(
            ReportBuilder::new()
                .motion(dx, dy)
                .touch(palm())
                .touch(TouchRecord {
                    x: grip().x + drift,
                    ..grip()
                })
                .touch(TouchRecord {
                    y: resting.y - drift * 3,
                    ..resting
                }),
        );
    }
    scenario.repeat(10, &ReportBuilder::new().touch(palm()));
    scenario.push(ReportBuilder::new());
    scenario
}

pub fn click(x: i32, right: bool) -> Scenario {
    let mut scenario = Scenario::new();
    let touch = finger(x, 200);
    scenario.push(
        ReportBuilder::new()
            .touch(palm())
            .touch(touch.state(touch_state::START)),
    );
    scenario.repeat(4, &ReportBuilder::new().touch(palm()).touch(touch));
    for frame in 0..9 {
        scenario.push(
            ReportBuilder::new()
                .left(!right)
                .right(right)
                .touch(palm())
                .touch(TouchRecord {
                    y: touch.y - frame * 12,
                    ..touch
                }),
        );
    }
    let released = TouchRecord {
        y: touch.y - 96,
        ..touch
    };
    scenario.repeat(4, &ReportBuilder::new().touch(palm()).touch(released));
    for frame in 1..=10 {
        scenario.push(ReportBuilder::new().touch(palm()).touch(TouchRecord {
            y: released.y - frame * 30,
            ..released
        }));
    }
    scenario.push(
        ReportBuilder::new()
            .touch(palm())
            .touch(released.state(touch_state::LIFT_50)),
    );
    scenario.push(ReportBuilder::new().touch(palm()));
    scenario
}

pub fn click_left() -> Scenario {
    click(-700, false)
}

pub fn click_right() -> Scenario {
    click(700, true)
}

pub fn click_middle() -> Scenario {
    click(0, false)
}

pub fn slow_scroll() -> Scenario {
    let mut scenario = Scenario::new();
    scenario.push(ReportBuilder::new().touch(palm()));
    scenario.stroke(&[palm()], (-100, 600), &[(0, -12); 180]);
    scenario.repeat(5, &ReportBuilder::new().touch(palm()));
    scenario
}

pub fn flick_steps() -> Vec<(i32, i32)> {
    let mut steps = vec![(0, -20), (0, -60), (0, -140), (0, -260)];
    steps.extend([(0, -340); 5]);
    steps
}

pub fn flick() -> Scenario {
    let mut scenario = Scenario::new();
    scenario.push(ReportBuilder::new().touch(palm()));
    scenario.stroke(&[palm()], (-100, 700), &flick_steps());
    scenario.repeat(5, &ReportBuilder::new().touch(palm()));
    scenario
}

pub fn horizontal_scroll() -> Scenario {
    let mut scenario = Scenario::new();
    scenario.push(ReportBuilder::new().touch(palm()));
    scenario.stroke(&[palm()], (-900, 0), &[(20, 0); 90]);
    scenario.repeat(5, &ReportBuilder::new().touch(palm()));
    scenario
}

pub fn click_while_scrolling() -> Scenario {
    let mut scenario = Scenario::new();
    let mut y = 600;
    let start = finger(-100, y);
    scenario.push(
        ReportBuilder::new()
            .touch(palm())
            .touch(start.state(touch_state::START)),
    );
    for frame in 0..40 {
        y -= 24;
        let pressed = (20..30).contains(&frame);
        scenario.push(
            ReportBuilder::new()
                .left(pressed)
                .touch(palm())
                .touch(finger(-100, y)),
        );
    }
    scenario.push(
        ReportBuilder::new()
            .touch(palm())
            .touch(finger(-100, y).state(touch_state::LIFT_50)),
    );
    scenario.push(ReportBuilder::new().touch(palm()));
    scenario.stroke(&[palm()], (-100, 600), &[(0, -24); 20]);
    scenario
}

pub fn touch_after_flick() -> Scenario {
    let mut scenario = flick();
    scenario.repeat(15, &ReportBuilder::new().touch(palm()));
    let rest = finger(-100, 300);
    scenario.push(
        ReportBuilder::new()
            .touch(palm())
            .touch(rest.state(touch_state::START)),
    );
    scenario.repeat(40, &ReportBuilder::new().touch(palm()).touch(rest));
    scenario.push(
        ReportBuilder::new()
            .touch(palm())
            .touch(rest.state(touch_state::LIFT_50)),
    );
    scenario.push(ReportBuilder::new().touch(palm()));
    scenario
}

pub fn scroll_after_flick() -> Scenario {
    let mut scenario = flick();
    scenario.repeat(15, &ReportBuilder::new().touch(palm()));
    scenario.stroke(&[palm()], (-100, 600), &[(0, -30); 30]);
    scenario
}

pub fn diagonal_drift() -> Scenario {
    let mut scenario = Scenario::new();
    scenario.push(ReportBuilder::new().touch(palm()));
    scenario.stroke(&[palm()], (-600, 700), &[(6, -30); 60]);
    scenario.repeat(5, &ReportBuilder::new().touch(palm()));
    scenario
}

pub fn horizontal_drift() -> Scenario {
    let mut scenario = Scenario::new();
    scenario.push(ReportBuilder::new().touch(palm()));
    scenario.stroke(&[palm()], (-900, 0), &[(20, 6); 60]);
    scenario.repeat(5, &ReportBuilder::new().touch(palm()));
    scenario
}

pub fn two_finger_swipe_steps(dx: i32) -> Vec<(i32, i32)> {
    let mut steps = vec![(dx / 4, 0), (dx / 2, 1)];
    steps.extend([(dx, 0), (dx, -1)].repeat(9));
    steps
}

pub fn two_finger_swipe() -> Scenario {
    let mut scenario = Scenario::new();
    scenario.push(ReportBuilder::new().touch(palm()));
    scenario.two_finger_stroke(
        &[palm()],
        [(-500, 200), (100, 200)],
        &two_finger_swipe_steps(40),
    );
    scenario.repeat(5, &ReportBuilder::new().touch(palm()));
    scenario
}

pub fn two_finger_swipe_left() -> Scenario {
    let mut scenario = Scenario::new();
    scenario.push(ReportBuilder::new().touch(palm()));
    scenario.two_finger_stroke(
        &[palm()],
        [(0, 200), (600, 200)],
        &two_finger_swipe_steps(-40),
    );
    scenario.repeat(5, &ReportBuilder::new().touch(palm()));
    scenario
}

pub fn two_finger_vertical() -> Scenario {
    let mut scenario = Scenario::new();
    scenario.push(ReportBuilder::new().touch(palm()));
    scenario.two_finger_stroke(&[palm()], [(-500, 600), (100, 600)], &[(0, -40); 20]);
    scenario.repeat(5, &ReportBuilder::new().touch(palm()));
    scenario
}

pub fn two_finger_tap_gap(scenario: &mut Scenario, frames: usize) {
    scenario.repeat(frames, &ReportBuilder::new().touch(palm()));
}

pub fn two_finger_double_tap() -> Scenario {
    let mut scenario = Scenario::new();
    scenario.push(ReportBuilder::new().touch(palm()));
    scenario.two_finger_tap(&[palm()], 6, false);
    two_finger_tap_gap(&mut scenario, 12);
    scenario.two_finger_tap(&[palm()], 6, false);
    scenario.repeat(30, &ReportBuilder::new().touch(palm()));
    scenario
}

pub fn two_finger_single_tap() -> Scenario {
    let mut scenario = Scenario::new();
    scenario.push(ReportBuilder::new().touch(palm()));
    scenario.two_finger_tap(&[palm()], 6, false);
    scenario.repeat(60, &ReportBuilder::new().touch(palm()));
    scenario
}

pub fn two_finger_tap_with_click() -> Scenario {
    let mut scenario = Scenario::new();
    scenario.push(ReportBuilder::new().touch(palm()));
    scenario.two_finger_tap(&[palm()], 6, false);
    two_finger_tap_gap(&mut scenario, 12);
    scenario.two_finger_tap(&[palm()], 6, true);
    scenario.repeat(30, &ReportBuilder::new().touch(palm()));
    scenario
}

pub fn two_finger_slow_second_tap() -> Scenario {
    let mut scenario = Scenario::new();
    scenario.push(ReportBuilder::new().touch(palm()));
    scenario.two_finger_tap(&[palm()], 6, false);
    two_finger_tap_gap(&mut scenario, 40);
    scenario.two_finger_tap(&[palm()], 6, false);
    scenario.repeat(30, &ReportBuilder::new().touch(palm()));
    scenario
}

pub fn all() -> Vec<(&'static str, Scenario)> {
    vec![
        ("idle-resting-palm", idle_resting_palm()),
        ("move-fingers-resting", move_fingers_resting()),
        ("click-left", click_left()),
        ("click-right", click_right()),
        ("click-middle", click_middle()),
        ("slow-scroll", slow_scroll()),
        ("flick", flick()),
        ("horizontal-scroll", horizontal_scroll()),
        ("click-while-scrolling", click_while_scrolling()),
        ("touch-after-flick", touch_after_flick()),
        ("scroll-after-flick", scroll_after_flick()),
        ("diagonal-drift", diagonal_drift()),
        ("horizontal-drift", horizontal_drift()),
        ("two-finger-swipe", two_finger_swipe()),
        ("two-finger-swipe-left", two_finger_swipe_left()),
        ("two-finger-vertical", two_finger_vertical()),
        ("two-finger-double-tap", two_finger_double_tap()),
        ("two-finger-single-tap", two_finger_single_tap()),
        ("two-finger-tap-with-click", two_finger_tap_with_click()),
        ("two-finger-slow-second-tap", two_finger_slow_second_tap()),
    ]
}

pub fn drive(engine: &mut Engine, reports: &[(Micros, Vec<u8>)]) -> Vec<(Micros, Output)> {
    let mut timeline = Vec::new();
    let mut out = Vec::new();
    for (t, bytes) in reports {
        fire_due(engine, *t, &mut timeline, &mut out);
        engine.handle(Input::Report { t: *t, bytes }, &mut out);
        timeline.extend(out.drain(..).map(|output| (*t, output)));
    }
    fire_due(engine, Micros::MAX, &mut timeline, &mut out);
    timeline
}

fn fire_due(
    engine: &mut Engine,
    until: Micros,
    timeline: &mut Vec<(Micros, Output)>,
    out: &mut Vec<Output>,
) {
    for _ in 0..64 {
        let Some(deadline) = engine.next_deadline().filter(|deadline| *deadline < until) else {
            return;
        };
        engine.handle(Input::Timer { t: deadline }, out);
        timeline.extend(out.drain(..).map(|output| (deadline, output)));
    }
}
