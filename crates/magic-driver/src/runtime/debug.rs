use std::fmt::Write as _;
use std::io::Write;

use evdev::{EventType, KeyCode, RelativeAxisCode, SynchronizationCode};
use md_core::profile::Profile;
use md_core::report::{self, ReportFormat};
use md_core::{Button, Micros, Output};

use crate::io::EvdevEvent;
use crate::session::{SessionEvent, SessionObserver};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ButtonCounts {
    pub presses: u64,
    pub releases: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StreamTotals {
    pub dx: i64,
    pub dy: i64,
    pub left: ButtonCounts,
    pub right: ButtonCounts,
    pub middle: ButtonCounts,
}

impl StreamTotals {
    fn button(&mut self, button: Button, pressed: bool) {
        let counts = match button {
            Button::Left => &mut self.left,
            Button::Right => &mut self.right,
            Button::Middle => &mut self.middle,
        };
        if pressed {
            counts.presses += 1;
        } else {
            counts.releases += 1;
        }
    }

    fn render(&self) -> String {
        format!(
            "dx={} dy={} left={}/{} right={}/{} middle={}/{}",
            self.dx,
            self.dy,
            self.left.presses,
            self.left.releases,
            self.right.presses,
            self.right.releases,
            self.middle.presses,
            self.middle.releases
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CrossCheck {
    pub reports: u64,
    pub stock_frames: u64,
    pub hidraw: StreamTotals,
    pub stock: StreamTotals,
}

impl CrossCheck {
    pub fn on_hidraw(&mut self, dx: i32, dy: i32, outputs: &[Output]) {
        self.reports += 1;
        self.hidraw.dx += i64::from(dx);
        self.hidraw.dy += i64::from(dy);
        for output in outputs {
            if let Output::Button { button, pressed } = output {
                self.hidraw.button(*button, *pressed);
            }
        }
    }

    pub fn on_stock(&mut self, events: &[EvdevEvent]) {
        for event in events {
            match stock_meaning(*event) {
                StockMeaning::RelX(value) => self.stock.dx += i64::from(value),
                StockMeaning::RelY(value) => self.stock.dy += i64::from(value),
                StockMeaning::Button(button, pressed) => self.stock.button(button, pressed),
                StockMeaning::Report => self.stock_frames += 1,
                StockMeaning::Other => {}
            }
        }
    }

    pub fn motion_matches(&self) -> bool {
        self.hidraw.dx == self.stock.dx && self.hidraw.dy == self.stock.dy
    }

    pub fn buttons_match(&self) -> bool {
        self.hidraw.left == self.stock.left
            && self.hidraw.right == self.stock.right
            && self.hidraw.middle == self.stock.middle
    }

    pub fn summary(&self) -> String {
        format!(
            "cross-check: hidraw reports={} {} | stock frames={} {} | motion {} | buttons {}",
            self.reports,
            self.hidraw.render(),
            self.stock_frames,
            self.stock.render(),
            if self.motion_matches() {
                "match"
            } else {
                "differ"
            },
            if self.buttons_match() {
                "match"
            } else {
                "differ"
            },
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StockMeaning {
    RelX(i32),
    RelY(i32),
    Button(Button, bool),
    Report,
    Other,
}

fn stock_meaning(event: EvdevEvent) -> StockMeaning {
    let pressed = event.value != 0;
    match (event.kind, event.code) {
        (kind, code) if kind == EventType::RELATIVE.0 && code == RelativeAxisCode::REL_X.0 => {
            StockMeaning::RelX(event.value)
        }
        (kind, code) if kind == EventType::RELATIVE.0 && code == RelativeAxisCode::REL_Y.0 => {
            StockMeaning::RelY(event.value)
        }
        (kind, code) if kind == EventType::KEY.0 && code == KeyCode::BTN_LEFT.0 => {
            StockMeaning::Button(Button::Left, pressed)
        }
        (kind, code) if kind == EventType::KEY.0 && code == KeyCode::BTN_RIGHT.0 => {
            StockMeaning::Button(Button::Right, pressed)
        }
        (kind, code) if kind == EventType::KEY.0 && code == KeyCode::BTN_MIDDLE.0 => {
            StockMeaning::Button(Button::Middle, pressed)
        }
        (kind, code)
            if kind == EventType::SYNCHRONIZATION.0
                && code == SynchronizationCode::SYN_REPORT.0 =>
        {
            StockMeaning::Report
        }
        _ => StockMeaning::Other,
    }
}

pub fn render_stock_events(t: Micros, events: &[EvdevEvent]) -> String {
    let mut line = format!("{} stock", t.as_u64());
    for event in events {
        let _ = match stock_meaning(*event) {
            StockMeaning::RelX(value) => write!(line, " REL_X={value}"),
            StockMeaning::RelY(value) => write!(line, " REL_Y={value}"),
            StockMeaning::Button(button, pressed) => {
                write!(line, " {button}={}", u8::from(pressed))
            }
            StockMeaning::Report => write!(line, " SYN"),
            StockMeaning::Other => write!(line, " {}:{}={}", event.kind, event.code, event.value),
        };
    }
    line
}

pub fn render_report(profile: &Profile, t: Micros, bytes: &[u8], outputs: &[Output]) -> String {
    let mut line = format!("{} hidraw", t.as_u64());
    let _ = match report::parse(profile, bytes) {
        Ok(report) => write!(
            line,
            " clicks={} dx={} dy={} touches={}{}",
            report.header.clicks,
            report.header.dx,
            report.header.dy,
            report.touches.len(),
            if report.format == ReportFormat::Short {
                " short"
            } else {
                ""
            }
        ),
        Err(error) => write!(line, " {error}"),
    };
    for output in outputs {
        let _ = write!(line, " | {output}");
    }
    line
}

pub struct TerminalObserver<W: Write> {
    out: W,
    debug: bool,
    cross_check: CrossCheck,
}

impl<W: Write> TerminalObserver<W> {
    pub fn new(out: W, debug: bool) -> TerminalObserver<W> {
        TerminalObserver {
            out,
            debug,
            cross_check: CrossCheck::default(),
        }
    }

    #[cfg(test)]
    pub fn into_inner(self) -> W {
        self.out
    }

    fn line(&mut self, line: &str) {
        let _ = writeln!(self.out, "{line}");
    }
}

impl<W: Write> SessionObserver for TerminalObserver<W> {
    fn observe(&mut self, event: &SessionEvent<'_>) {
        match event {
            SessionEvent::Acquired { nodes } => {
                self.cross_check = CrossCheck::default();
                self.line(&format!(
                    "magic-driver: active: {} (raw {}, stock {} grabbed)",
                    nodes.profile.name,
                    nodes.hidraw.display(),
                    nodes.evdev.display()
                ));
            }
            SessionEvent::AcquireFailed {
                nodes,
                step,
                error,
                retry_in,
            } => {
                let next = match retry_in {
                    Some(delay) => format!("retrying in {} ms", delay.as_millis()),
                    None => "giving up, the mouse keeps its stock behaviour".to_owned(),
                };
                self.line(&format!(
                    "magic-driver: cannot {step} for {}: {error}; {next}",
                    nodes.hidraw.display()
                ));
            }
            SessionEvent::Released { nodes, reason } => {
                self.line(&format!(
                    "magic-driver: released {}: {reason}; stock behaviour restored",
                    nodes.hidraw.display()
                ));
                if self.debug {
                    let summary = self.cross_check.summary();
                    self.line(&summary);
                }
            }
            SessionEvent::Report { t, bytes, outputs } => {
                if !self.debug {
                    return;
                }
                let profile = &md_core::profile::MAGIC_MOUSE_2_BT;
                if let Ok(report) = report::parse(profile, bytes) {
                    self.cross_check
                        .on_hidraw(report.header.dx, report.header.dy, outputs);
                }
                let line = render_report(profile, *t, bytes, outputs);
                self.line(&line);
            }
            SessionEvent::StockEvents { t, events } => {
                if !self.debug {
                    return;
                }
                self.cross_check.on_stock(events);
                let line = render_stock_events(*t, events);
                self.line(&line);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use md_core::profile::MAGIC_MOUSE_2_BT;
    use md_core::testkit::{ReportBuilder, TouchRecord};
    use nix::errno::Errno;

    use super::*;
    use crate::discovery::MouseNodes;
    use crate::io::DeviceError;
    use crate::session::{AcquireStep, ReleaseReason};

    const SYN: EvdevEvent = EvdevEvent::new(0, 0, 0);

    fn rel(code: RelativeAxisCode, value: i32) -> EvdevEvent {
        EvdevEvent::new(EventType::RELATIVE.0, code.0, value)
    }

    fn key(code: KeyCode, value: i32) -> EvdevEvent {
        EvdevEvent::new(EventType::KEY.0, code.0, value)
    }

    fn press(button: Button, pressed: bool) -> Output {
        Output::Button { button, pressed }
    }

    #[test]
    fn matching_streams_report_match() {
        let mut check = CrossCheck::default();
        check.on_hidraw(3, -2, &[Output::PointerMotion { dx: 3, dy: -2 }]);
        check.on_hidraw(0, 0, &[press(Button::Left, true)]);
        check.on_hidraw(1, 0, &[press(Button::Left, false)]);
        check.on_stock(&[
            rel(RelativeAxisCode::REL_X, 3),
            rel(RelativeAxisCode::REL_Y, -2),
            SYN,
            key(KeyCode::BTN_LEFT, 1),
            SYN,
            rel(RelativeAxisCode::REL_X, 1),
            key(KeyCode::BTN_LEFT, 0),
            rel(RelativeAxisCode::REL_WHEEL, 1),
            SYN,
        ]);
        assert!(check.motion_matches());
        assert!(check.buttons_match());
        assert_eq!(
            check.summary(),
            "cross-check: hidraw reports=3 dx=4 dy=-2 left=1/1 right=0/0 middle=0/0 | stock frames=3 dx=4 dy=-2 left=1/1 right=0/0 middle=0/0 | motion match | buttons match"
        );
    }

    #[test]
    fn a_different_button_decision_is_reported() {
        let mut check = CrossCheck::default();
        check.on_hidraw(0, 0, &[press(Button::Middle, true)]);
        check.on_stock(&[key(KeyCode::BTN_LEFT, 1), SYN]);
        assert!(check.motion_matches());
        assert!(!check.buttons_match());
        assert!(check.summary().ends_with("buttons differ"));
    }

    #[test]
    fn missing_stock_motion_is_reported() {
        let mut check = CrossCheck::default();
        check.on_hidraw(5, 0, &[]);
        assert!(!check.motion_matches());
    }

    #[test]
    fn stock_events_render_with_names() {
        let line = render_stock_events(
            Micros::new(42),
            &[
                rel(RelativeAxisCode::REL_X, 3),
                key(KeyCode::BTN_RIGHT, 1),
                rel(RelativeAxisCode::REL_WHEEL_HI_RES, 120),
                SYN,
            ],
        );
        assert_eq!(line, "42 stock REL_X=3 right=1 2:11=120 SYN");
    }

    #[test]
    fn reports_render_header_and_outputs() {
        let bytes = ReportBuilder::new()
            .motion(2, -1)
            .left(true)
            .touch(TouchRecord::new(1, -700, 0).size(12))
            .build();
        let line = render_report(
            &MAGIC_MOUSE_2_BT,
            Micros::new(7),
            &bytes,
            &[
                Output::PointerMotion { dx: 2, dy: -1 },
                press(Button::Left, true),
            ],
        );
        assert_eq!(
            line,
            "7 hidraw clicks=1 dx=2 dy=-1 touches=1 | pointer dx=2 dy=-1 | button left press"
        );
        let short = render_report(&MAGIC_MOUSE_2_BT, Micros::new(8), &[0x10, 1, 2], &[]);
        assert!(short.starts_with("8 hidraw "));
    }

    fn nodes() -> MouseNodes {
        MouseNodes {
            key: PathBuf::from("/sys/devices/x/0005:004C:0269.0001"),
            hidraw: PathBuf::from("/dev/hidraw5"),
            evdev: PathBuf::from("/dev/input/event16"),
            profile: &MAGIC_MOUSE_2_BT,
        }
    }

    fn render(debug: bool, events: &[SessionEvent<'_>]) -> String {
        let mut observer = TerminalObserver::new(Vec::new(), debug);
        for event in events {
            observer.observe(event);
        }
        String::from_utf8(observer.into_inner()).unwrap()
    }

    #[test]
    fn lifecycle_lines_are_printed_without_debug() {
        let nodes = nodes();
        let error = DeviceError::from(Errno::EACCES);
        let bytes = ReportBuilder::new().motion(1, 1).build();
        let text = render(
            false,
            &[
                SessionEvent::AcquireFailed {
                    nodes: &nodes,
                    step: AcquireStep::OpenRaw,
                    error: &error,
                    retry_in: Some(Micros::from_millis(50)),
                },
                SessionEvent::Acquired { nodes: &nodes },
                SessionEvent::Report {
                    t: Micros::new(1),
                    bytes: &bytes,
                    outputs: &[],
                },
                SessionEvent::StockEvents {
                    t: Micros::new(1),
                    events: &[SYN],
                },
                SessionEvent::Released {
                    nodes: &nodes,
                    reason: ReleaseReason::Shutdown,
                },
            ],
        );
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert!(lines[0].starts_with(
            "magic-driver: cannot open raw device for /dev/hidraw5: permission denied"
        ));
        assert!(lines[0].ends_with("; retrying in 50 ms"));
        assert_eq!(
            lines[1],
            "magic-driver: active: Apple Magic Mouse 2 (Bluetooth) (raw /dev/hidraw5, stock /dev/input/event16 grabbed)"
        );
        assert_eq!(
            lines[2],
            "magic-driver: released /dev/hidraw5: shutdown; stock behaviour restored"
        );
    }

    #[test]
    fn debug_prints_reports_stock_events_and_the_cross_check() {
        let nodes = nodes();
        let bytes = ReportBuilder::new().motion(1, 1).build();
        let text = render(
            true,
            &[
                SessionEvent::Acquired { nodes: &nodes },
                SessionEvent::Report {
                    t: Micros::new(1),
                    bytes: &bytes,
                    outputs: &[Output::PointerMotion { dx: 1, dy: 1 }],
                },
                SessionEvent::StockEvents {
                    t: Micros::new(2),
                    events: &[
                        rel(RelativeAxisCode::REL_X, 1),
                        rel(RelativeAxisCode::REL_Y, 1),
                        SYN,
                    ],
                },
                SessionEvent::Released {
                    nodes: &nodes,
                    reason: ReleaseReason::Shutdown,
                },
            ],
        );
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 5, "{text}");
        assert!(lines[1].starts_with("1 hidraw clicks=0 dx=1 dy=1"));
        assert_eq!(lines[2], "2 stock REL_X=1 REL_Y=1 SYN");
        assert!(lines[4].starts_with("cross-check: hidraw reports=1 dx=1 dy=1"));
        assert!(lines[4].ends_with("motion match | buttons match"));
    }

    #[test]
    fn giving_up_says_the_stock_behaviour_stays() {
        let nodes = nodes();
        let error = DeviceError::from(Errno::EBUSY);
        let text = render(
            false,
            &[SessionEvent::AcquireFailed {
                nodes: &nodes,
                step: AcquireStep::Grab,
                error: &error,
                retry_in: None,
            }],
        );
        assert!(text.contains("cannot grab stock device for /dev/hidraw5: busy"));
        assert!(
            text.trim_end()
                .ends_with("giving up, the mouse keeps its stock behaviour")
        );
    }
}
