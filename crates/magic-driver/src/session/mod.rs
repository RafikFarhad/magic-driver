pub mod manager;

use std::fmt;
use std::os::fd::BorrowedFd;
use std::sync::atomic::{AtomicU64, Ordering};

use md_core::config::Config;
use md_core::{Engine, Input, Micros, Mode, Output};

use crate::discovery::MouseNodes;
use crate::io::hidraw::REPORT_BUFFER_LEN;
use crate::io::{
    DeviceError, DeviceOpener, EvdevEvent, RawSource, StockDevice, VirtualPointer, VirtualTouchpad,
};

pub const ACQUIRE_WINDOW: Micros = Micros::from_millis(2_000);
pub const FIRST_RETRY_DELAY: Micros = Micros::from_millis(50);
pub const MAX_REPORTS_PER_WAKE: usize = 64;

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
}

impl fmt::Display for ReleaseReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ReleaseReason::DeviceGone => "device gone",
            ReleaseReason::Removed => "device removed",
            ReleaseReason::Replaced => "device nodes changed",
            ReleaseReason::Shutdown => "shutdown",
            ReleaseReason::ReadFailed => "device read failed",
            ReleaseReason::VirtualDeviceFailed => "virtual device write failed",
        })
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

#[derive(Debug)]
pub enum SessionEvent<'a> {
    Acquired {
        nodes: &'a MouseNodes,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Acquisition {
    started: Micros,
    next_attempt: Micros,
    delay: Micros,
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
    pub fn new(nodes: MouseNodes, mode: Mode, now: Micros) -> Session<O> {
        let engine = Engine::new(nodes.profile, Config::for_profile(nodes.profile), mode);
        Session {
            nodes,
            state: SessionState::Acquiring,
            acquisition: Some(Acquisition {
                started: now,
                next_attempt: now,
                delay: FIRST_RETRY_DELAY,
            }),
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
            SessionState::Active => self.engine.next_deadline(),
            SessionState::Absent | SessionState::Degraded => None,
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
                if self
                    .engine
                    .next_deadline()
                    .is_some_and(|deadline| deadline <= now)
                {
                    self.engine
                        .handle(Input::Timer { t: now }, &mut self.outputs);
                    self.flush_outputs(observer);
                }
            }
            SessionState::Absent | SessionState::Degraded => {}
        }
    }

    fn try_acquire(&mut self, now: Micros, opener: &mut O, observer: &mut dyn SessionObserver) {
        match acquire(opener, &self.nodes) {
            Ok(mut devices) => {
                self.stock_events.clear();
                let _ = devices.stock.drain(&mut self.stock_events);
                self.stock_events.clear();
                self.devices = Some(devices);
                self.acquisition = None;
                self.generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
                self.engine.handle(Input::Reset, &mut self.outputs);
                self.outputs.clear();
                self.state = SessionState::Active;
                observer.observe(&SessionEvent::Acquired { nodes: &self.nodes });
            }
            Err((step, error)) => {
                let Some(acquisition) = self.acquisition.as_mut() else {
                    return;
                };
                let deadline = acquisition.started + ACQUIRE_WINDOW;
                let retry_in = if now >= deadline {
                    None
                } else {
                    let delay = acquisition.delay.min(deadline - now);
                    acquisition.next_attempt = now + delay;
                    acquisition.delay = acquisition.delay + acquisition.delay;
                    Some(delay)
                };
                if retry_in.is_none() {
                    self.acquisition = None;
                    self.state = SessionState::Degraded;
                }
                observer.observe(&SessionEvent::AcquireFailed {
                    nodes: &self.nodes,
                    step,
                    error: &error,
                    retry_in,
                });
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
                    self.engine.handle(
                        Input::Report {
                            t: now,
                            bytes: &self.report[..len],
                        },
                        &mut self.outputs,
                    );
                    observer.observe(&SessionEvent::Report {
                        t: now,
                        bytes: &self.report[..len],
                        outputs: &self.outputs,
                    });
                    if !self.flush_outputs(observer) {
                        return;
                    }
                }
                Err(error) => {
                    self.release(read_failure(&error), observer);
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
        if !self.stock_events.is_empty() {
            observer.observe(&SessionEvent::StockEvents {
                t: now,
                events: &self.stock_events,
            });
        }
        if let Err(error) = result {
            self.release(read_failure(&error), observer);
        }
    }

    fn flush_outputs(&mut self, observer: &mut dyn SessionObserver) -> bool {
        let Some(devices) = self.devices.as_mut() else {
            self.outputs.clear();
            return false;
        };
        let result = write_outputs(devices, &self.outputs);
        self.outputs.clear();
        if result.is_err() {
            self.release(ReleaseReason::VirtualDeviceFailed, observer);
            return false;
        }
        true
    }

    pub fn release(&mut self, reason: ReleaseReason, observer: &mut dyn SessionObserver) {
        self.acquisition = None;
        let Some(devices) = self.devices.take() else {
            self.state = next_state(reason);
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
        self.state = next_state(reason);
        observer.observe(&SessionEvent::Released {
            nodes: &self.nodes,
            reason,
        });
    }
}

fn read_failure(error: &DeviceError) -> ReleaseReason {
    if error.is_gone() {
        ReleaseReason::DeviceGone
    } else {
        ReleaseReason::ReadFailed
    }
}

fn next_state(reason: ReleaseReason) -> SessionState {
    match reason {
        ReleaseReason::ReadFailed | ReleaseReason::VirtualDeviceFailed => SessionState::Degraded,
        ReleaseReason::DeviceGone
        | ReleaseReason::Removed
        | ReleaseReason::Replaced
        | ReleaseReason::Shutdown => SessionState::Absent,
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
