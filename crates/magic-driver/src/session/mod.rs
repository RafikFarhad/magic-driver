pub mod manager;
pub mod selfcheck;

use std::fmt;
use std::os::fd::BorrowedFd;
use std::sync::atomic::{AtomicU64, Ordering};

use md_core::config::Config;
use md_core::{Engine, Input, Micros, Mode, Output};

use self::selfcheck::{SelfCheck, SelfCheckFailure};
use crate::discovery::MouseNodes;
use crate::io::hidraw::REPORT_BUFFER_LEN;
use crate::io::{
    DeviceError, DeviceErrorKind, DeviceOpener, EvdevEvent, RawSource, StockDevice, VirtualPointer,
    VirtualTouchpad,
};

pub const ACQUIRE_WINDOW: Micros = Micros::from_millis(2_000);
pub const FIRST_RETRY_DELAY: Micros = Micros::from_millis(50);
pub const MAX_REPORTS_PER_WAKE: usize = 64;
pub const DEGRADED_MIN_DWELL: Micros = Micros::from_millis(5_000);
pub const DEGRADED_MAX_RETRY_DELAY: Micros = Micros::from_millis(60_000);
pub const STABLE_ACTIVE: Micros = Micros::from_millis(60_000);

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Absent,
    Acquiring,
    Active,
    Degraded,
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SessionState::Absent => "absent",
            SessionState::Acquiring => "acquiring",
            SessionState::Active => "active",
            SessionState::Degraded => "degraded",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseReason {
    DeviceGone,
    Removed,
    Replaced,
    Shutdown,
    ReadFailed,
    VirtualDeviceFailed,
    SelfCheck(SelfCheckFailure),
}

impl fmt::Display for ReleaseReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReleaseReason::DeviceGone => f.write_str("device gone"),
            ReleaseReason::Removed => f.write_str("device removed"),
            ReleaseReason::Replaced => f.write_str("device nodes changed"),
            ReleaseReason::Shutdown => f.write_str("shutdown"),
            ReleaseReason::ReadFailed => f.write_str("device read failed"),
            ReleaseReason::VirtualDeviceFailed => f.write_str("virtual device write failed"),
            ReleaseReason::SelfCheck(failure) => write!(f, "self-check failed: {failure}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquireStep {
    OpenRaw,
    CreatePointer,
    CreateTouchpad,
    OpenStock,
    Grab,
}

impl fmt::Display for AcquireStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AcquireStep::OpenRaw => "open raw device",
            AcquireStep::CreatePointer => "create virtual pointer",
            AcquireStep::CreateTouchpad => "create virtual touchpad",
            AcquireStep::OpenStock => "open stock device",
            AcquireStep::Grab => "grab stock device",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegradeCause {
    SelfCheck(SelfCheckFailure),
    ReadFailed,
    VirtualDeviceFailed,
    AcquireGaveUp {
        step: AcquireStep,
        kind: DeviceErrorKind,
    },
}

impl DegradeCause {
    fn release_reason(self) -> Option<ReleaseReason> {
        match self {
            DegradeCause::SelfCheck(failure) => Some(ReleaseReason::SelfCheck(failure)),
            DegradeCause::ReadFailed => Some(ReleaseReason::ReadFailed),
            DegradeCause::VirtualDeviceFailed => Some(ReleaseReason::VirtualDeviceFailed),
            DegradeCause::AcquireGaveUp { .. } => None,
        }
    }
}

impl fmt::Display for DegradeCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DegradeCause::SelfCheck(failure) => write!(f, "{failure}"),
            DegradeCause::ReadFailed => f.write_str("device read failed"),
            DegradeCause::VirtualDeviceFailed => f.write_str("virtual device write failed"),
            DegradeCause::AcquireGaveUp { step, kind } => write!(
                f,
                "cannot {step}: {kind} after {} ms of retries",
                ACQUIRE_WINDOW.as_millis()
            ),
        }
    }
}

#[derive(Debug)]
pub enum SessionEvent<'a> {
    Acquired {
        nodes: &'a MouseNodes,
        elapsed: Micros,
        attempts: u32,
    },
    AcquireFailed {
        nodes: &'a MouseNodes,
        step: AcquireStep,
        error: &'a DeviceError,
        retry_in: Option<Micros>,
    },
    Released {
        nodes: &'a MouseNodes,
        reason: ReleaseReason,
    },
    Degraded {
        nodes: &'a MouseNodes,
        cause: DegradeCause,
        retry_in: Micros,
    },
    Retrying {
        nodes: &'a MouseNodes,
        cause: DegradeCause,
    },
    Report {
        t: Micros,
        bytes: &'a [u8],
        outputs: &'a [Output],
    },
    StockEvents {
        t: Micros,
        events: &'a [EvdevEvent],
    },
}

pub trait SessionObserver {
    fn observe(&mut self, event: &SessionEvent<'_>);
}

#[cfg(test)]
#[derive(Debug, Default)]
pub struct NoObserver;

#[cfg(test)]
impl SessionObserver for NoObserver {
    fn observe(&mut self, _event: &SessionEvent<'_>) {}
}

pub fn degraded_retry_delay(level: u32) -> Micros {
    let factor = 1_u64 << level.min(8);
    Micros::new(DEGRADED_MIN_DWELL.as_u64().saturating_mul(factor)).min(DEGRADED_MAX_RETRY_DELAY)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Acquisition {
    started: Micros,
    next_attempt: Micros,
    delay: Micros,
    attempts: u32,
}

impl Acquisition {
    fn new(now: Micros) -> Acquisition {
        Acquisition {
            started: now,
            next_attempt: now,
            delay: FIRST_RETRY_DELAY,
            attempts: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Degradation {
    cause: DegradeCause,
    retry_at: Micros,
}

struct Devices<O: DeviceOpener> {
    raw: O::Raw,
    pointer: O::Pointer,
    touchpad: O::Touchpad,
    stock: O::Stock,
}

pub struct Session<O: DeviceOpener> {
    nodes: MouseNodes,
    state: SessionState,
    acquisition: Option<Acquisition>,
    degradation: Option<Degradation>,
    degrade_level: u32,
    active_since: Option<Micros>,
    self_check: SelfCheck,
    devices: Option<Devices<O>>,
    engine: Engine,
    generation: u64,
    outputs: Vec<Output>,
    report: Vec<u8>,
    stock_events: Vec<EvdevEvent>,
}

impl<O: DeviceOpener> fmt::Debug for Session<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("nodes", &self.nodes)
            .field("state", &self.state)
            .field("acquisition", &self.acquisition)
            .field("degradation", &self.degradation)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SessionFds<'a> {
    pub generation: u64,
    pub raw: Option<BorrowedFd<'a>>,
    pub stock: Option<BorrowedFd<'a>>,
}

impl<O: DeviceOpener> Session<O> {
    pub fn new(nodes: MouseNodes, mode: Mode, config: Config, now: Micros) -> Session<O> {
        let engine = Engine::new(nodes.profile, config, mode);
        Session {
            nodes,
            state: SessionState::Acquiring,
            acquisition: Some(Acquisition::new(now)),
            degradation: None,
            degrade_level: 0,
            active_since: None,
            self_check: SelfCheck::default(),
            devices: None,
            engine,
            generation: 0,
            outputs: Vec::with_capacity(16),
            report: vec![0; REPORT_BUFFER_LEN],
            stock_events: Vec::with_capacity(64),
        }
    }

    pub fn nodes(&self) -> &MouseNodes {
        &self.nodes
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    #[cfg(test)]
    pub fn degrade_cause(&self) -> Option<DegradeCause> {
        self.degradation.map(|degradation| degradation.cause)
    }

    pub fn status_line(&self) -> String {
        let node = self.nodes.hidraw.file_name().map_or_else(
            || self.nodes.hidraw.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let name = self.nodes.profile.name;
        match (self.state, self.degradation) {
            (SessionState::Degraded, Some(degradation)) => {
                format!("degraded: {}; stock behaviour, retrying", degradation.cause)
            }
            (SessionState::Degraded, None) => "degraded: stock behaviour".to_owned(),
            (SessionState::Acquiring, _) => format!("acquiring: {name} on {node}"),
            (SessionState::Active, _) => format!("active: {name} on {node}"),
            (SessionState::Absent, _) => format!("absent: {name} disconnected"),
        }
    }

    #[cfg(test)]
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn fds(&self) -> SessionFds<'_> {
        SessionFds {
            generation: self.generation,
            raw: self
                .devices
                .as_ref()
                .and_then(|devices| devices.raw.poll_fd()),
            stock: self
                .devices
                .as_ref()
                .and_then(|devices| devices.stock.poll_fd()),
        }
    }

    pub fn next_deadline(&self) -> Option<Micros> {
        match self.state {
            SessionState::Acquiring => self.acquisition.map(|acquisition| acquisition.next_attempt),
            SessionState::Active => {
                earliest(self.engine.next_deadline(), self.self_check.deadline())
            }
            SessionState::Degraded => self.degradation.map(|degradation| degradation.retry_at),
            SessionState::Absent => None,
        }
    }

    pub fn on_timer(&mut self, now: Micros, opener: &mut O, observer: &mut dyn SessionObserver) {
        match self.state {
            SessionState::Acquiring => {
                if self
                    .acquisition
                    .is_some_and(|acquisition| acquisition.next_attempt <= now)
                {
                    self.try_acquire(now, opener, observer);
                }
            }
            SessionState::Active => {
                if let Some(failure) = self.self_check.on_timer(now) {
                    self.degrade(now, DegradeCause::SelfCheck(failure), observer);
                    return;
                }
                if self
                    .engine
                    .next_deadline()
                    .is_some_and(|deadline| deadline <= now)
                {
                    self.engine
                        .handle(Input::Timer { t: now }, &mut self.outputs);
                    self.flush_outputs(now, observer);
                }
            }
            SessionState::Degraded => {
                if self
                    .degradation
                    .is_some_and(|degradation| degradation.retry_at <= now)
                {
                    self.retry(now, opener, observer);
                }
            }
            SessionState::Absent => {}
        }
    }

    pub fn apply_config(
        &mut self,
        now: Micros,
        config: &Config,
        observer: &mut dyn SessionObserver,
    ) {
        self.engine
            .handle(Input::ConfigChanged(config.clone()), &mut self.outputs);
        if self.state == SessionState::Active {
            self.flush_outputs(now, observer);
        } else {
            self.outputs.clear();
        }
    }

    fn retry(&mut self, now: Micros, opener: &mut O, observer: &mut dyn SessionObserver) {
        let Some(degradation) = self.degradation.take() else {
            return;
        };
        self.state = SessionState::Acquiring;
        self.acquisition = Some(Acquisition::new(now));
        observer.observe(&SessionEvent::Retrying {
            nodes: &self.nodes,
            cause: degradation.cause,
        });
        self.try_acquire(now, opener, observer);
    }

    fn try_acquire(&mut self, now: Micros, opener: &mut O, observer: &mut dyn SessionObserver) {
        let Some(acquisition) = self.acquisition.as_mut() else {
            return;
        };
        acquisition.attempts += 1;
        match acquire(opener, &self.nodes) {
            Ok(mut devices) => {
                let Acquisition {
                    started, attempts, ..
                } = *acquisition;
                self.stock_events.clear();
                let _ = devices.stock.drain(&mut self.stock_events);
                self.stock_events.clear();
                self.devices = Some(devices);
                self.acquisition = None;
                self.generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
                self.engine.handle(Input::Reset, &mut self.outputs);
                self.outputs.clear();
                self.self_check.reset();
                self.active_since = Some(now);
                self.state = SessionState::Active;
                observer.observe(&SessionEvent::Acquired {
                    nodes: &self.nodes,
                    elapsed: now.saturating_sub(started),
                    attempts,
                });
            }
            Err((step, error)) => {
                let deadline = acquisition.started + ACQUIRE_WINDOW;
                let retry_in = if now >= deadline {
                    None
                } else {
                    let delay = acquisition.delay.min(deadline - now);
                    acquisition.next_attempt = now + delay;
                    acquisition.delay = acquisition.delay + acquisition.delay;
                    Some(delay)
                };
                observer.observe(&SessionEvent::AcquireFailed {
                    nodes: &self.nodes,
                    step,
                    error: &error,
                    retry_in,
                });
                if retry_in.is_none() {
                    self.acquisition = None;
                    self.enter_degraded(
                        now,
                        DegradeCause::AcquireGaveUp {
                            step,
                            kind: error.kind(),
                        },
                        observer,
                    );
                }
            }
        }
    }

    pub fn on_raw_readable(&mut self, now: Micros, observer: &mut dyn SessionObserver) {
        for _ in 0..MAX_REPORTS_PER_WAKE {
            let Some(devices) = self.devices.as_mut() else {
                return;
            };
            match devices.raw.read_report(&mut self.report) {
                Ok(None) => return,
                Ok(Some(len)) => {
                    let before = self.engine.stats();
                    self.engine.handle(
                        Input::Report {
                            t: now,
                            bytes: &self.report[..len],
                        },
                        &mut self.outputs,
                    );
                    let after = self.engine.stats();
                    let failure = if after.malformed > before.malformed {
                        self.self_check.on_malformed_report(now)
                    } else {
                        if after.ignored == before.ignored {
                            self.self_check.on_valid_report();
                        }
                        None
                    };
                    observer.observe(&SessionEvent::Report {
                        t: now,
                        bytes: &self.report[..len],
                        outputs: &self.outputs,
                    });
                    if !self.flush_outputs(now, observer) {
                        return;
                    }
                    if let Some(failure) = failure {
                        self.degrade(now, DegradeCause::SelfCheck(failure), observer);
                        return;
                    }
                }
                Err(error) => {
                    self.read_failed(now, &error, observer);
                    return;
                }
            }
        }
    }

    pub fn on_stock_readable(&mut self, now: Micros, observer: &mut dyn SessionObserver) {
        let Some(devices) = self.devices.as_mut() else {
            return;
        };
        self.stock_events.clear();
        let result = devices.stock.drain(&mut self.stock_events);
        let active = !self.stock_events.is_empty();
        if active {
            observer.observe(&SessionEvent::StockEvents {
                t: now,
                events: &self.stock_events,
            });
        }
        if let Err(error) = result {
            self.read_failed(now, &error, observer);
            return;
        }
        if active && let Some(failure) = self.self_check.on_stock_activity(now) {
            self.degrade(now, DegradeCause::SelfCheck(failure), observer);
        }
    }

    fn read_failed(
        &mut self,
        now: Micros,
        error: &DeviceError,
        observer: &mut dyn SessionObserver,
    ) {
        if error.is_gone() {
            self.release(ReleaseReason::DeviceGone, observer);
        } else {
            self.degrade(now, DegradeCause::ReadFailed, observer);
        }
    }

    fn flush_outputs(&mut self, now: Micros, observer: &mut dyn SessionObserver) -> bool {
        let Some(devices) = self.devices.as_mut() else {
            self.outputs.clear();
            return false;
        };
        let result = write_outputs(devices, &self.outputs);
        self.outputs.clear();
        if result.is_err() {
            self.degrade(now, DegradeCause::VirtualDeviceFailed, observer);
            return false;
        }
        true
    }

    pub fn release(&mut self, reason: ReleaseReason, observer: &mut dyn SessionObserver) {
        self.acquisition = None;
        self.degradation = None;
        self.release_devices(reason, observer);
        self.state = SessionState::Absent;
    }

    fn degrade(&mut self, now: Micros, cause: DegradeCause, observer: &mut dyn SessionObserver) {
        self.acquisition = None;
        if let Some(reason) = cause.release_reason() {
            self.release_devices(reason, observer);
        }
        self.enter_degraded(now, cause, observer);
    }

    fn enter_degraded(
        &mut self,
        now: Micros,
        cause: DegradeCause,
        observer: &mut dyn SessionObserver,
    ) {
        let stable = self
            .active_since
            .take()
            .is_some_and(|since| now.saturating_sub(since) >= STABLE_ACTIVE);
        if stable {
            self.degrade_level = 0;
        }
        let retry_in = degraded_retry_delay(self.degrade_level);
        self.degrade_level = self.degrade_level.saturating_add(1);
        self.degradation = Some(Degradation {
            cause,
            retry_at: now + retry_in,
        });
        self.self_check.reset();
        self.state = SessionState::Degraded;
        observer.observe(&SessionEvent::Degraded {
            nodes: &self.nodes,
            cause,
            retry_in,
        });
    }

    fn release_devices(&mut self, reason: ReleaseReason, observer: &mut dyn SessionObserver) {
        let Some(devices) = self.devices.take() else {
            return;
        };
        let Devices {
            raw,
            mut pointer,
            mut touchpad,
            mut stock,
        } = devices;
        let _ = stock.ungrab();
        drop(stock);
        self.outputs.clear();
        self.engine.handle(Input::Reset, &mut self.outputs);
        for output in &self.outputs {
            let _ = write_output(&mut pointer, &mut touchpad, output);
        }
        self.outputs.clear();
        VirtualTouchpad::destroy(touchpad);
        VirtualPointer::destroy(pointer);
        drop(raw);
        observer.observe(&SessionEvent::Released {
            nodes: &self.nodes,
            reason,
        });
    }
}

fn earliest(a: Option<Micros>, b: Option<Micros>) -> Option<Micros> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

fn acquire<O: DeviceOpener>(
    opener: &mut O,
    nodes: &MouseNodes,
) -> Result<Devices<O>, (AcquireStep, DeviceError)> {
    let raw = opener
        .open_raw(&nodes.hidraw)
        .map_err(|error| (AcquireStep::OpenRaw, error))?;
    let pointer = opener
        .create_pointer()
        .map_err(|error| (AcquireStep::CreatePointer, error))?;
    let touchpad = opener
        .create_touchpad()
        .map_err(|error| (AcquireStep::CreateTouchpad, error))?;
    let stock = opener
        .open_stock(&nodes.evdev)
        .map_err(|error| (AcquireStep::OpenStock, error))?;
    let mut devices = Devices {
        raw,
        pointer,
        touchpad,
        stock,
    };
    if let Err(error) = StockDevice::grab(&mut devices.stock) {
        let Devices {
            raw,
            pointer,
            touchpad,
            stock,
        } = devices;
        drop(stock);
        VirtualTouchpad::destroy(touchpad);
        VirtualPointer::destroy(pointer);
        drop(raw);
        return Err((AcquireStep::Grab, error));
    }
    Ok(devices)
}

fn write_outputs<O: DeviceOpener>(
    devices: &mut Devices<O>,
    outputs: &[Output],
) -> Result<(), DeviceError> {
    for output in outputs {
        write_output(&mut devices.pointer, &mut devices.touchpad, output)?;
    }
    Ok(())
}

fn write_output<P: VirtualPointer, T: VirtualTouchpad>(
    pointer: &mut P,
    touchpad: &mut T,
    output: &Output,
) -> Result<(), DeviceError> {
    match output {
        Output::PointerMotion { dx, dy } => pointer.motion(*dx, *dy),
        Output::Button { button, pressed } => pointer.button(*button, *pressed),
        Output::TouchFrame(frame) => touchpad.frame(frame),
    }
}

#[cfg(test)]
mod tests;
