use raw_window_handle::RawDisplayHandle;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::{
    delegate_pointer, delegate_registry, delegate_seat, registry_handlers,
};
use wayland_client::backend::{Backend, WaylandError};
use wayland_client::globals::{GlobalError, registry_queue_init};
use wayland_client::protocol::{wl_pointer, wl_seat};
use wayland_client::{Connection, Dispatch, DispatchError, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::pointer_gestures::zv1::client::{
    zwp_pointer_gesture_hold_v1, zwp_pointer_gesture_pinch_v1, zwp_pointer_gesture_swipe_v1,
    zwp_pointer_gestures_v1,
};

use super::{Received, hold_event, pinch_event, pointer_received, swipe_event};

pub const GESTURES_VERSION: u32 = 3;
pub const HOLD_GESTURES_SINCE: u32 = 3;
const GESTURES_RELEASE_SINCE: u32 = 2;
const POINTER_RELEASE_SINCE: u32 = 3;

#[derive(Debug, thiserror::Error)]
pub enum ReceiverError {
    #[error("the window is not on Wayland, so the Received panel has no data")]
    NotWayland,
    #[error("cannot read the Wayland globals: {0}")]
    Globals(#[from] GlobalError),
    #[error("Wayland dispatch failed: {0}")]
    Dispatch(#[from] DispatchError),
    #[error("Wayland connection failed: {0}")]
    Connection(#[from] WaylandError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiverInfo {
    pub seat_version: Option<u32>,
    pub pointer_version: Option<u32>,
    pub gestures_version: Option<u32>,
    pub hold: bool,
}

pub struct WaylandReceiver {
    connection: Connection,
    queue: EventQueue<ReceiverState>,
    state: ReceiverState,
}

struct ReceiverState {
    registry: RegistryState,
    seats: SeatState,
    gestures: Option<zwp_pointer_gestures_v1::ZwpPointerGesturesV1>,
    pointer: Option<PointerObjects>,
    received: Vec<Received>,
}

struct PointerObjects {
    seat: wl_seat::WlSeat,
    pointer: wl_pointer::WlPointer,
    swipe: Option<zwp_pointer_gesture_swipe_v1::ZwpPointerGestureSwipeV1>,
    pinch: Option<zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1>,
    hold: Option<zwp_pointer_gesture_hold_v1::ZwpPointerGestureHoldV1>,
}

impl PointerObjects {
    fn release(self) {
        if let Some(swipe) = self.swipe {
            swipe.destroy();
        }
        if let Some(pinch) = self.pinch {
            pinch.destroy();
        }
        if let Some(hold) = self.hold {
            hold.destroy();
        }
        if self.pointer.version() >= POINTER_RELEASE_SINCE {
            self.pointer.release();
        }
    }
}

impl WaylandReceiver {
    pub fn attach(handle: RawDisplayHandle) -> Result<WaylandReceiver, ReceiverError> {
        let RawDisplayHandle::Wayland(handle) = handle else {
            return Err(ReceiverError::NotWayland);
        };
        let backend = unsafe { Backend::from_foreign_display(handle.display.as_ptr().cast()) };
        let connection = Connection::from_backend(backend);
        let (globals, mut queue) = registry_queue_init::<ReceiverState>(&connection)?;
        let qh = queue.handle();
        let gestures = globals
            .bind::<zwp_pointer_gestures_v1::ZwpPointerGesturesV1, _, _>(
                &qh,
                1..=GESTURES_VERSION,
                (),
            )
            .ok();
        let mut state = ReceiverState {
            registry: RegistryState::new(&globals),
            seats: SeatState::new(&globals, &qh),
            gestures,
            pointer: None,
            received: Vec::new(),
        };
        queue.roundtrip(&mut state)?;
        Ok(WaylandReceiver {
            connection,
            queue,
            state,
        })
    }

    pub fn poll(&mut self) -> Result<Vec<Received>, ReceiverError> {
        self.queue.dispatch_pending(&mut self.state)?;
        self.connection.flush()?;
        Ok(std::mem::take(&mut self.state.received))
    }

    pub fn info(&self) -> ReceiverInfo {
        let pointer = self.state.pointer.as_ref();
        ReceiverInfo {
            seat_version: pointer.map(|objects| objects.seat.version()),
            pointer_version: pointer.map(|objects| objects.pointer.version()),
            gestures_version: self.state.gestures.as_ref().map(Proxy::version),
            hold: pointer.is_some_and(|objects| objects.hold.is_some()),
        }
    }
}

impl Drop for WaylandReceiver {
    fn drop(&mut self) {
        if let Some(objects) = self.state.pointer.take() {
            objects.release();
        }
        if let Some(gestures) = self.state.gestures.take()
            && gestures.version() >= GESTURES_RELEASE_SINCE
        {
            gestures.release();
        }
        let _ = self.connection.flush();
    }
}

impl ReceiverState {
    fn create_pointer(&mut self, qh: &QueueHandle<ReceiverState>, seat: wl_seat::WlSeat) {
        let Ok(pointer) = self.seats.get_pointer(qh, &seat) else {
            return;
        };
        let (swipe, pinch, hold) = match &self.gestures {
            Some(gestures) => (
                Some(gestures.get_swipe_gesture(&pointer, qh, ())),
                Some(gestures.get_pinch_gesture(&pointer, qh, ())),
                (gestures.version() >= HOLD_GESTURES_SINCE)
                    .then(|| gestures.get_hold_gesture(&pointer, qh, ())),
            ),
            None => (None, None, None),
        };
        self.pointer = Some(PointerObjects {
            seat,
            pointer,
            swipe,
            pinch,
            hold,
        });
    }
}

impl SeatHandler for ReceiverState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.create_pointer(qh, seat);
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer
            && self
                .pointer
                .as_ref()
                .is_some_and(|objects| objects.seat == seat)
            && let Some(objects) = self.pointer.take()
        {
            objects.release();
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        if self
            .pointer
            .as_ref()
            .is_some_and(|objects| objects.seat == seat)
            && let Some(objects) = self.pointer.take()
        {
            objects.release();
        }
    }
}

impl PointerHandler for ReceiverState {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        self.received.extend(
            events
                .iter()
                .filter_map(|event| pointer_received(&event.kind)),
        );
    }
}

impl ProvidesRegistryState for ReceiverState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers![SeatState];
}

delegate_registry!(ReceiverState);
delegate_seat!(ReceiverState);
delegate_pointer!(ReceiverState);

impl Dispatch<zwp_pointer_gestures_v1::ZwpPointerGesturesV1, ()> for ReceiverState {
    fn event(
        _: &mut Self,
        _: &zwp_pointer_gestures_v1::ZwpPointerGesturesV1,
        _: zwp_pointer_gestures_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_pointer_gesture_swipe_v1::ZwpPointerGestureSwipeV1, ()> for ReceiverState {
    fn event(
        state: &mut Self,
        _: &zwp_pointer_gesture_swipe_v1::ZwpPointerGestureSwipeV1,
        event: zwp_pointer_gesture_swipe_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state
            .received
            .extend(swipe_event(&event).map(Received::Gesture));
    }
}

impl Dispatch<zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1, ()> for ReceiverState {
    fn event(
        state: &mut Self,
        _: &zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1,
        event: zwp_pointer_gesture_pinch_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state
            .received
            .extend(pinch_event(&event).map(Received::Gesture));
    }
}

impl Dispatch<zwp_pointer_gesture_hold_v1::ZwpPointerGestureHoldV1, ()> for ReceiverState {
    fn event(
        state: &mut Self,
        _: &zwp_pointer_gesture_hold_v1::ZwpPointerGestureHoldV1,
        event: zwp_pointer_gesture_hold_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state
            .received
            .extend(hold_event(&event).map(Received::Gesture));
    }
}
