use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use md_core::config::{Config, PalmRejection, ScrollAcceleration};
use md_core::profile::Profile;
use md_trace::TouchpadInfo;
use raw_window_handle::HasDisplayHandle;

use crate::clock::LocalClock;
use crate::live::{ConnectionStatus, DEFAULT_RETRY, LiveClient, LiveSession};
use crate::player::{PlayState, Player, SPEEDS};
use crate::received::ReceivedLog;
use crate::received::wayland::{ReceiverInfo, WaylandReceiver};
use crate::replay::{ConfigOrigin, OfflineSource, with_acceleration, with_palm_rejection};
use crate::timeline::Timeline;
use crate::view::{SurfaceMapper, Thresholds, ViewCursor};

mod panels;

pub const REPAINT_INTERVAL: Duration = Duration::from_millis(33);
const MAX_LIVE_EVENTS_PER_FRAME: usize = 20_000;

pub struct LiveSource {
    pub socket: PathBuf,
    client: Option<LiveClient>,
    pub session: LiveSession,
    pub error: Option<String>,
}

pub enum Source {
    Offline(Box<OfflineSource>),
    Live(Box<LiveSource>),
}

pub struct ReceivedSource {
    receiver: Option<WaylandReceiver>,
    pub status: String,
    pub info: Option<ReceiverInfo>,
    pub log: ReceivedLog,
}

pub struct InspectorApp {
    clock: LocalClock,
    source: Source,
    player: Player,
    cursor: ViewCursor,
    received: ReceivedSource,
    last_frame: Option<Instant>,
}

impl InspectorApp {
    pub fn offline(cc: &eframe::CreationContext<'_>, source: OfflineSource) -> InspectorApp {
        let player = Player::offline(&source.timeline);
        InspectorApp::new(cc, Source::Offline(Box::new(source)), player)
    }

    pub fn live(cc: &eframe::CreationContext<'_>, socket: PathBuf) -> InspectorApp {
        let clock = LocalClock::new();
        let ctx = cc.egui_ctx.clone();
        let (client, error) =
            match LiveClient::spawn_with_waker(socket.clone(), DEFAULT_RETRY, clock, move || {
                ctx.request_repaint();
            }) {
                Ok(client) => (Some(client), None),
                Err(error) => (
                    None,
                    Some(format!("cannot start the trace client: {error}")),
                ),
            };
        let live = LiveSource {
            socket,
            client,
            session: LiveSession::new(),
            error,
        };
        let mut app = InspectorApp::new(cc, Source::Live(Box::new(live)), Player::live());
        app.clock = clock;
        app
    }

    fn new(cc: &eframe::CreationContext<'_>, source: Source, player: Player) -> InspectorApp {
        let received = match cc.display_handle() {
            Ok(handle) => match WaylandReceiver::attach(handle.as_raw()) {
                Ok(receiver) => ReceivedSource {
                    info: Some(receiver.info()),
                    receiver: Some(receiver),
                    status: "listening on this window's Wayland pointer".to_owned(),
                    log: ReceivedLog::new(),
                },
                Err(error) => ReceivedSource::unavailable(error.to_string()),
            },
            Err(error) => ReceivedSource::unavailable(format!("no display handle: {error}")),
        };
        InspectorApp {
            clock: LocalClock::new(),
            source,
            player,
            cursor: ViewCursor::new(),
            received,
            last_frame: None,
        }
    }

    fn profile(&self) -> Option<&'static Profile> {
        match &self.source {
            Source::Offline(offline) => Some(offline.recording.profile),
            Source::Live(live) => live.session.profile,
        }
    }

    fn thresholds(&self) -> Option<Thresholds> {
        match &self.source {
            Source::Offline(offline) => Some(Thresholds::from_config(&offline.config)),
            Source::Live(live) => live.session.thresholds,
        }
    }

    fn touchpad(&self) -> TouchpadInfo {
        match &self.source {
            Source::Live(live) => live
                .session
                .hello
                .as_ref()
                .map_or_else(default_touchpad, |hello| hello.profile.touchpad),
            Source::Offline(offline) => {
                md_core::trace::profile_info(offline.recording.profile).touchpad
            }
        }
    }

    fn pump_live(&mut self) {
        let Source::Live(live) = &mut self.source else {
            return;
        };
        let Some(client) = &live.client else {
            return;
        };
        let mut reconnected = false;
        for event in client.drain(MAX_LIVE_EVENTS_PER_FRAME) {
            reconnected |= live.session.apply(event);
        }
        if reconnected {
            self.cursor.reset();
            self.player = Player::live();
        }
    }

    fn pump_received(&mut self) {
        let Some(receiver) = &mut self.received.receiver else {
            return;
        };
        match receiver.poll() {
            Ok(items) => {
                let now = self.clock.now_us();
                for item in items {
                    self.received.log.record(item, now);
                }
            }
            Err(error) => {
                self.received.status = format!("stopped: {error}");
                self.received.receiver = None;
            }
        }
    }

    fn rerun(&mut self, config: Config) {
        if let Source::Offline(offline) = &mut self.source {
            let position = self.player.position();
            offline.rerun(config);
            self.cursor.reset();
            self.player = Player::offline(&offline.timeline);
            self.player.seek(&offline.timeline, position);
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            self.source_label(ui);
            ui.separator();
            self.transport(ui);
        });
        let mut new_config = None;
        if let Source::Offline(offline) = &self.source {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("config: {}", origin_label(&offline.chosen.origin)));
                for warning in &offline.chosen.warnings {
                    ui.colored_label(egui::Color32::YELLOW, warning);
                }
                ui.separator();
                let profile = offline.recording.profile;
                ui.label("palm rejection");
                for preset in [
                    PalmRejection::Low,
                    PalmRejection::Medium,
                    PalmRejection::High,
                ] {
                    let selected = offline.config.touch.palm_rejection == preset;
                    if ui.selectable_label(selected, palm_label(preset)).clicked() && !selected {
                        new_config = Some(with_palm_rejection(&offline.config, preset, profile));
                    }
                }
                ui.separator();
                ui.label("acceleration");
                for preset in [
                    ScrollAcceleration::Off,
                    ScrollAcceleration::Low,
                    ScrollAcceleration::Medium,
                    ScrollAcceleration::High,
                ] {
                    let selected = offline.config.scroll.acceleration == preset;
                    if ui
                        .selectable_label(selected, acceleration_label(preset))
                        .clicked()
                        && !selected
                    {
                        new_config = Some(with_acceleration(&offline.config, preset));
                    }
                }
                if offline.config != offline.chosen.config && ui.button("reset config").clicked() {
                    new_config = Some(offline.chosen.config.clone());
                }
            });
        }
        if let Some(config) = new_config {
            self.rerun(config);
        }
    }

    fn source_label(&self, ui: &mut egui::Ui) {
        match &self.source {
            Source::Offline(offline) => {
                ui.label(format!(
                    "replay {} ({} reports, profile {}, recorded with magic-driver {} on kernel {})",
                    offline.path.display(),
                    offline.recording.reports.len(),
                    offline.recording.header.profile_id,
                    offline.recording.header.magic_driver_version,
                    offline.recording.header.kernel_version
                ));
            }
            Source::Live(live) => {
                let status = match &live.session.status {
                    ConnectionStatus::Connecting => "connecting".to_owned(),
                    ConnectionStatus::Connected => live.session.hello.as_ref().map_or_else(
                        || "connected".to_owned(),
                        |hello| {
                            format!(
                                "connected to magic-driver {} ({}), {}",
                                hello.daemon_version, hello.profile.name, hello.session
                            )
                        },
                    ),
                    ConnectionStatus::Disconnected(reason) => {
                        format!("disconnected: {reason}; reconnecting")
                    }
                    ConnectionStatus::Unavailable(reason) => {
                        format!("{reason}; retrying")
                    }
                };
                ui.label(format!("live {}: {status}", live.socket.display()));
                if let Some(error) = &live.error {
                    ui.colored_label(egui::Color32::RED, error);
                }
                let dropped = live.session.dropped_total;
                let colour = if dropped > 0 {
                    egui::Color32::YELLOW
                } else {
                    ui.visuals().text_color()
                };
                ui.colored_label(
                    colour,
                    format!(
                        "dropped records: {dropped} (this connection {}), connections: {}",
                        live.session.dropped_this_connection, live.session.connections
                    ),
                );
                if live.session.hello.is_some() && live.session.profile.is_none() {
                    ui.colored_label(egui::Color32::RED, "unknown device profile");
                }
            }
        }
    }

    fn transport(&mut self, ui: &mut egui::Ui) {
        let timeline = timeline_of(&self.source);
        let player = &mut self.player;
        let play_label = if player.is_playing() { "pause" } else { "play" };
        if ui.button(play_label).clicked() {
            player.toggle(timeline);
        }
        if ui.button("step back").clicked() {
            player.step_back(timeline);
        }
        if ui.button("step").clicked() {
            player.step_forward(timeline);
        }
        if let Source::Live(_) = &self.source
            && ui
                .add_enabled(
                    player.state() != PlayState::Following,
                    egui::Button::new("go live"),
                )
                .clicked()
        {
            player.follow(timeline);
        }
        egui::ComboBox::from_id_salt("speed")
            .selected_text(format!("{}x", player.speed()))
            .show_ui(ui, |ui| {
                for speed in SPEEDS {
                    if ui
                        .selectable_label(
                            (player.speed() - speed).abs() < f64::EPSILON,
                            format!("{speed}x"),
                        )
                        .clicked()
                    {
                        player.set_speed(speed);
                    }
                }
            });
        let start = timeline.start_time().unwrap_or(0);
        let end = timeline.end_time().unwrap_or(start);
        let mut position = player.position();
        let response = ui.add(
            egui::Slider::new(&mut position, start..=end.max(start))
                .show_value(false)
                .text(format!(
                    "{} / {}",
                    panels::format_time(player.position()),
                    panels::format_time(end)
                )),
        );
        if response.changed() {
            player.seek(timeline, position);
        }
    }
}

impl ReceivedSource {
    fn unavailable(status: String) -> ReceivedSource {
        ReceivedSource {
            receiver: None,
            status,
            info: None,
            log: ReceivedLog::new(),
        }
    }
}

impl eframe::App for InspectorApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.received.receiver = None;
        if let Source::Live(live) = &mut self.source {
            live.client = None;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        let elapsed = self
            .last_frame
            .map_or(Duration::ZERO, |last| now.duration_since(last));
        self.last_frame = Some(now);
        self.pump_live();
        self.pump_received();
        let timeline = timeline_of(&self.source);
        self.player.tick(timeline, elapsed);
        ui.ctx().request_repaint_after(REPAINT_INTERVAL);

        egui::Panel::top("controls").show(ui, |ui| self.controls(ui));
        let position = self.player.position();
        let profile = self.profile();
        let thresholds = self.thresholds();
        let touchpad = self.touchpad();
        let live_alignment = match &self.source {
            Source::Live(live) => Some(live.session.alignment),
            Source::Offline(_) => None,
        };
        let timeline = timeline_of(&self.source);
        let state = self.cursor.seek(timeline, position).clone();
        let received_view = panels::ReceivedView {
            status: &self.received.status,
            info: self.received.info,
            log: &self.received.log,
            now_local_us: self.clock.now_us(),
            trace_alignment: live_alignment,
            position,
            following: self.player.state() == PlayState::Following,
        };
        egui::Panel::right("received")
            .default_size(420.0)
            .show(ui, |ui| panels::received_panel(ui, &received_view));
        egui::CentralPanel::default().show(ui, |ui| match profile {
            Some(profile) => {
                let mapper = SurfaceMapper::new(profile);
                let thresholds = thresholds
                    .unwrap_or_else(|| Thresholds::from_config(&Config::for_profile(profile)));
                ui.columns(2, |columns| {
                    panels::raw_panel(
                        &mut columns[0],
                        mapper,
                        &thresholds,
                        &state,
                        timeline,
                        position,
                    );
                    panels::decisions_panel(&mut columns[1], &state, timeline, position, touchpad);
                });
            }
            None => {
                ui.label("waiting for the daemon's hello record (device profile and config)");
            }
        });
    }
}

fn timeline_of(source: &Source) -> &Timeline {
    match source {
        Source::Offline(offline) => &offline.timeline,
        Source::Live(live) => &live.session.timeline,
    }
}

fn default_touchpad() -> TouchpadInfo {
    md_core::trace::profile_info(&md_core::profile::MAGIC_MOUSE_2_BT).touchpad
}

fn origin_label(origin: &ConfigOrigin) -> String {
    match origin {
        ConfigOrigin::Defaults => "defaults".to_owned(),
        ConfigOrigin::File(path) => path.display().to_string(),
        ConfigOrigin::MissingDefaultFile(path) => {
            format!("defaults ({} does not exist)", path.display())
        }
    }
}

const fn palm_label(preset: PalmRejection) -> &'static str {
    match preset {
        PalmRejection::Low => "low",
        PalmRejection::Medium => "medium",
        PalmRejection::High => "high",
    }
}

const fn acceleration_label(preset: ScrollAcceleration) -> &'static str {
    match preset {
        ScrollAcceleration::Off => "off",
        ScrollAcceleration::Low => "low",
        ScrollAcceleration::Medium => "medium",
        ScrollAcceleration::High => "high",
    }
}
