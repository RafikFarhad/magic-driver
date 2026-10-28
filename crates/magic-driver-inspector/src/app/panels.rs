use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use md_trace::{GestureState, OutputEvent, TouchpadInfo, TrackClass};

use crate::clock::ClockAlignment;
use crate::received::wayland::ReceiverInfo;
use crate::received::{ReceivedLog, ScrollAxis, describe};
use crate::timeline::Timeline;
use crate::view::{
    SurfaceMapper, Thresholds, TouchDot, TouchStateKind, ViewState, synthetic_fingers,
};

const STATE_WINDOW_US: u64 = 5_000_000;
const RATE_WINDOW_US: u64 = 1_000_000;
const RECEIVED_WINDOW_US: u64 = 5_000_000;
const SYNTHETIC_VIEW_MM: f64 = 120.0;
const RECEIVED_ROWS: usize = 200;

pub struct ReceivedView<'a> {
    pub status: &'a str,
    pub info: Option<ReceiverInfo>,
    pub log: &'a ReceivedLog,
    pub now_local_us: u64,
    pub trace_alignment: Option<ClockAlignment>,
    pub position: u64,
    pub following: bool,
}

pub fn format_time(us: u64) -> String {
    format!("{}.{:03} s", us / 1_000_000, (us / 1_000) % 1_000)
}

#[allow(clippy::cast_possible_truncation)]
fn f32_of(value: f64) -> f32 {
    value as f32
}

#[allow(clippy::cast_precision_loss)]
fn f32_of_u64(value: u64) -> f32 {
    value as f32
}

fn touch_colour(kind: TouchStateKind) -> Color32 {
    match kind {
        TouchStateKind::None => Color32::DARK_GRAY,
        TouchStateKind::Hover => Color32::from_rgb(120, 120, 160),
        TouchStateKind::Start => Color32::from_rgb(230, 200, 60),
        TouchStateKind::Drag => Color32::from_rgb(80, 190, 110),
        TouchStateKind::Lift => Color32::from_rgb(230, 130, 50),
        TouchStateKind::Unknown => Color32::from_rgb(200, 80, 200),
    }
}

fn class_colour(class: Option<TrackClass>) -> Color32 {
    match class {
        Some(TrackClass::Deliberate) => Color32::from_rgb(60, 220, 90),
        Some(TrackClass::Rejected) => Color32::from_rgb(230, 60, 60),
        Some(TrackClass::Candidate) | None => Color32::LIGHT_GRAY,
    }
}

fn state_colour(state: GestureState) -> Color32 {
    match state {
        GestureState::Idle => Color32::from_gray(70),
        GestureState::Scroll => Color32::from_rgb(70, 140, 220),
        GestureState::Swipe => Color32::from_rgb(170, 90, 220),
        GestureState::TapPending => Color32::from_rgb(220, 190, 70),
        GestureState::Cancelling => Color32::from_rgb(230, 110, 60),
        GestureState::Suppressed => Color32::from_rgb(150, 60, 60),
    }
}

pub fn raw_panel(
    ui: &mut egui::Ui,
    mapper: SurfaceMapper,
    thresholds: &Thresholds,
    state: &ViewState,
    timeline: &Timeline,
    position: u64,
) {
    ui.heading("Raw");
    let dots = mapper.dots(state);
    surface_view(ui, mapper, thresholds, &dots);
    let rate = timeline.report_rate_hz(position, RATE_WINDOW_US);
    match &state.report {
        Some(report) => {
            ui.label(format!(
                "t {}   clicks {:#04x}   dx {}   dy {}   {rate:.0} reports/s",
                format_time(report.t),
                report.clicks,
                report.dx,
                report.dy
            ));
            if let Some(ns) = report.processing_ns {
                ui.label(format!(
                    "processing {:.1} µs",
                    f64::from(u32::try_from(ns).unwrap_or(u32::MAX)) / 1_000.0
                ));
            }
            if let Some(error) = &report.error {
                ui.colored_label(Color32::RED, format!("report error: {error}"));
            }
        }
        None => {
            ui.label("no report yet");
        }
    }
    ui.horizontal(|ui| {
        for (name, down) in [
            ("left", state.buttons.left),
            ("middle", state.buttons.middle),
            ("right", state.buttons.right),
        ] {
            let colour = if down {
                Color32::from_rgb(230, 90, 60)
            } else {
                Color32::from_gray(90)
            };
            ui.colored_label(
                colour,
                if down {
                    format!("[{name} down]")
                } else {
                    format!("[{name}]")
                },
            );
        }
    });
    egui::Grid::new("touches").striped(true).show(ui, |ui| {
        for header in [
            "id", "state", "x", "y", "major mm", "minor mm", "class", "reason",
        ] {
            ui.strong(header);
        }
        ui.end_row();
        let touches = state
            .report
            .as_ref()
            .map(|report| report.touches.as_slice())
            .unwrap_or_default();
        for (dot, touch) in dots.iter().zip(touches) {
            ui.label(dot.id.to_string());
            ui.colored_label(
                touch_colour(dot.kind),
                format!("{} {:#04x}", dot.kind.label(), dot.state),
            );
            ui.label(touch.x.to_string());
            ui.label(touch.y.to_string());
            let palm = thresholds.is_palm_sized(dot);
            let size_colour = if palm {
                Color32::RED
            } else {
                ui.visuals().text_color()
            };
            ui.colored_label(size_colour, format!("{:.1}", dot.major_mm));
            ui.colored_label(size_colour, format!("{:.1}", dot.minor_mm));
            ui.colored_label(
                class_colour(dot.class),
                dot.class
                    .map_or("candidate".to_owned(), |class| class.to_string()),
            );
            ui.label(
                dot.reason
                    .map_or(String::new(), |reason| reason.to_string()),
            );
            ui.end_row();
        }
    });
    ui.label(format!(
        "thresholds: palm {:.1} mm, rear zone {:.0} %, promote {:.1} mm, mouse motion {:.0}",
        thresholds.palm_size_mm,
        thresholds.rear_zone * 100.0,
        thresholds.promote_distance_mm,
        thresholds.mouse_motion_threshold
    ));
}

fn surface_view(
    ui: &mut egui::Ui,
    mapper: SurfaceMapper,
    thresholds: &Thresholds,
    dots: &[TouchDot],
) {
    let (width_mm, height_mm) = mapper.size_mm();
    let width = ui.available_width().max(120.0);
    let scale = width / f32_of(width_mm);
    let size = Vec2::new(width, f32_of(height_mm) * scale);
    let (response, painter) = ui.allocate_painter(size, Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 12.0, Color32::from_gray(28));
    painter.rect_stroke(
        rect,
        12.0,
        Stroke::new(1.0, Color32::GRAY),
        StrokeKind::Inside,
    );
    let rear_y = rect.top() + rect.height() * f32_of(mapper.rear_zone_start(thresholds.rear_zone));
    painter.rect_filled(
        Rect::from_min_max(Pos2::new(rect.left(), rear_y), rect.max),
        0.0,
        Color32::from_rgba_unmultiplied(160, 60, 60, 40),
    );
    painter.line_segment(
        [
            Pos2::new(rect.left(), rear_y),
            Pos2::new(rect.right(), rear_y),
        ],
        Stroke::new(1.0, Color32::from_rgb(160, 60, 60)),
    );
    painter.text(
        rect.left_top() + Vec2::new(6.0, 4.0),
        Align2::LEFT_TOP,
        "front",
        FontId::proportional(11.0),
        Color32::GRAY,
    );
    painter.text(
        Pos2::new(rect.left() + 6.0, rear_y + 2.0),
        Align2::LEFT_TOP,
        "rear zone",
        FontId::proportional(11.0),
        Color32::from_rgb(200, 100, 100),
    );
    for dot in dots {
        let centre = Pos2::new(
            rect.left() + rect.width() * f32_of(dot.x),
            rect.top() + rect.height() * f32_of(dot.y),
        );
        let radius = (f32_of(dot.major_mm.max(dot.minor_mm)) * scale / 2.0).max(4.0);
        painter.circle_filled(centre, radius, touch_colour(dot.kind).gamma_multiply(0.6));
        painter.circle_stroke(centre, radius, Stroke::new(2.0, class_colour(dot.class)));
        painter.text(
            centre,
            Align2::CENTER_CENTER,
            dot.id.to_string(),
            FontId::monospace(12.0),
            Color32::WHITE,
        );
    }
}

pub fn decisions_panel(
    ui: &mut egui::Ui,
    state: &ViewState,
    timeline: &Timeline,
    position: u64,
    touchpad: TouchpadInfo,
) {
    ui.heading("Decisions and output");
    ui.label(format!("recogniser: {}", state.recogniser));
    state_timeline(ui, timeline, position);
    ui.label("recent decisions");
    egui::Grid::new("decisions").striped(true).show(ui, |ui| {
        for header in ["t", "touch", "track", "from", "to", "reason"] {
            ui.strong(header);
        }
        ui.end_row();
        for decision in state.decisions.iter().rev().take(8) {
            ui.label(format_time(decision.t));
            ui.label(
                decision
                    .touch_id
                    .map_or("-".to_owned(), |id| id.to_string()),
            );
            ui.label(decision.track.to_string());
            ui.label(decision.previous.to_string());
            ui.colored_label(
                class_colour(Some(decision.class)),
                decision.class.to_string(),
            );
            ui.label(decision.reason.to_string());
            ui.end_row();
        }
    });
    ui.label("synthetic touchpad fingers");
    synthetic_view(ui, touchpad, state);
    ui.label("pointer and button events");
    egui::ScrollArea::vertical()
        .id_salt("pointer-events")
        .max_height(160.0)
        .show(ui, |ui| {
            for event in state.pointer_events.iter().rev() {
                let colour = match event.event {
                    OutputEvent::Button { .. } => Color32::from_rgb(230, 150, 60),
                    _ => ui.visuals().text_color(),
                };
                ui.colored_label(colour, format!("{}  {}", format_time(event.t), event.event));
            }
        });
    if let Some(session) = &state.session {
        ui.label(format!("session: {} {}", session.state, session.detail));
    }
}

fn state_timeline(ui: &mut egui::Ui, timeline: &Timeline, position: u64) {
    let from = position.saturating_sub(STATE_WINDOW_US);
    let segments = timeline.recogniser_segments(from, position);
    let (response, painter) =
        ui.allocate_painter(Vec2::new(ui.available_width(), 22.0), Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 2.0, Color32::from_gray(20));
    let span = f32_of_u64(STATE_WINDOW_US);
    for segment in &segments {
        let x0 = rect.left() + rect.width() * f32_of_u64(segment.start - from) / span;
        let x1 = rect.left() + rect.width() * f32_of_u64(segment.end - from) / span;
        let segment_rect = Rect::from_min_max(
            Pos2::new(x0, rect.top()),
            Pos2::new(x1.max(x0 + 1.0), rect.bottom()),
        );
        painter.rect_filled(segment_rect, 0.0, state_colour(segment.state));
        if x1 - x0 > 50.0 {
            painter.text(
                segment_rect.center(),
                Align2::CENTER_CENTER,
                segment.state.to_string(),
                FontId::proportional(11.0),
                Color32::WHITE,
            );
        }
    }
    painter.text(
        rect.left_bottom() + Vec2::new(2.0, 10.0),
        Align2::LEFT_TOP,
        "-5 s",
        FontId::proportional(10.0),
        Color32::GRAY,
    );
}

fn synthetic_view(ui: &mut egui::Ui, touchpad: TouchpadInfo, state: &ViewState) {
    let side = ui.available_width().min(220.0);
    let (response, painter) = ui.allocate_painter(Vec2::splat(side), Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 4.0, Color32::from_gray(24));
    painter.line_segment(
        [rect.center_top(), rect.center_bottom()],
        Stroke::new(1.0, Color32::from_gray(50)),
    );
    painter.line_segment(
        [rect.left_center(), rect.right_center()],
        Stroke::new(1.0, Color32::from_gray(50)),
    );
    let scale = side / f32_of(SYNTHETIC_VIEW_MM);
    for finger in synthetic_fingers(touchpad, &state.synthetic) {
        let centre = rect.center() + Vec2::new(f32_of(finger.x_mm), f32_of(finger.y_mm)) * scale;
        let centre = rect.clamp(centre);
        painter.circle_filled(centre, 6.0, Color32::from_rgb(90, 160, 240));
        painter.text(
            centre + Vec2::new(8.0, 0.0),
            Align2::LEFT_CENTER,
            format!("{}:{}", finger.slot, finger.tracking_id),
            FontId::monospace(10.0),
            Color32::LIGHT_GRAY,
        );
    }
    painter.text(
        rect.left_top() + Vec2::new(4.0, 2.0),
        Align2::LEFT_TOP,
        format!(
            "{} contacts, centre {} mm view",
            state.synthetic.len(),
            SYNTHETIC_VIEW_MM
        ),
        FontId::proportional(10.0),
        Color32::GRAY,
    );
}

pub fn received_panel(ui: &mut egui::Ui, view: &ReceivedView<'_>) {
    ui.heading("Received");
    ui.label(view.status);
    if let Some(info) = view.info {
        ui.label(format!(
            "wl_seat {}, wl_pointer {}, pointer gestures {}{}",
            info.seat_version
                .map_or("none".to_owned(), |version| format!("v{version}")),
            info.pointer_version
                .map_or("none".to_owned(), |version| format!("v{version}")),
            info.gestures_version
                .map_or("not offered".to_owned(), |version| format!("v{version}")),
            if info.hold { ", hold" } else { "" }
        ));
    }
    ui.label("Scroll and gestures this window receives from KWin while the pointer is over it.");
    let window_end = match (view.trace_alignment, view.following) {
        (Some(alignment), false) => alignment
            .to_local(view.position)
            .unwrap_or(view.now_local_us),
        _ => view.now_local_us,
    };
    let window_start = window_end.saturating_sub(RECEIVED_WINDOW_US);
    scroll_graph(ui, view.log, window_start, window_end);
    egui::ScrollArea::vertical()
        .id_salt("received-events")
        .show(ui, |ui| {
            egui::Grid::new("received").striped(true).show(ui, |ui| {
                for header in ["time", "trace t", "event"] {
                    ui.strong(header);
                }
                ui.end_row();
                for entry in view
                    .log
                    .entries()
                    .rev()
                    .filter(|entry| entry.local_us <= window_end)
                    .take(RECEIVED_ROWS)
                {
                    ui.label(
                        entry
                            .wayland_ms
                            .map_or("-".to_owned(), |ms| format!("{ms} ms")),
                    );
                    let trace_t = view
                        .trace_alignment
                        .and_then(|alignment| alignment.to_remote(entry.local_us));
                    ui.label(trace_t.map_or("-".to_owned(), format_time));
                    ui.label(describe(&entry.item));
                    ui.end_row();
                }
            });
        });
}

fn scroll_graph(ui: &mut egui::Ui, log: &ReceivedLog, from: u64, to: u64) {
    let (response, painter) =
        ui.allocate_painter(Vec2::new(ui.available_width(), 140.0), Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 2.0, Color32::from_gray(20));
    painter.line_segment(
        [rect.left_center(), rect.right_center()],
        Stroke::new(1.0, Color32::from_gray(60)),
    );
    let span = f32_of_u64(to.saturating_sub(from).max(1));
    for (axis, colour) in [
        (ScrollAxis::Vertical, Color32::from_rgb(90, 170, 240)),
        (ScrollAxis::Horizontal, Color32::from_rgb(240, 170, 80)),
    ] {
        let points = log.scroll_series(axis, from, to);
        let max = points
            .iter()
            .map(|point| point.value.abs())
            .fold(10.0_f64, f64::max);
        let mut line = Vec::new();
        for point in &points {
            let x = rect.left() + rect.width() * f32_of_u64(point.local_us - from) / span;
            let y = rect.center().y - f32_of(point.value / max) * rect.height() / 2.0;
            line.push(Pos2::new(x, y));
            if point.stop {
                painter.line_segment(
                    [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                    Stroke::new(1.0, Color32::from_rgb(230, 60, 60)),
                );
            }
            if point.discrete.is_some() || point.value120.is_some() {
                painter.circle_stroke(Pos2::new(x, y), 3.0, Stroke::new(1.0, colour));
            }
        }
        if line.len() > 1 {
            painter.line(line, Stroke::new(1.5, colour));
        } else if let Some(point) = line.first() {
            painter.circle_filled(*point, 2.0, colour);
        }
    }
    painter.text(
        rect.left_top() + Vec2::new(4.0, 2.0),
        Align2::LEFT_TOP,
        "vertical (blue), horizontal (orange), stop (red line), last 5 s",
        FontId::proportional(10.0),
        Color32::GRAY,
    );
}

#[cfg(test)]
mod tests;
