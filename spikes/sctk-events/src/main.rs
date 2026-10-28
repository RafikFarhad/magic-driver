use std::time::Instant;

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::xdg::window::{
    Window, WindowConfigure, WindowDecorations, WindowHandler,
};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_output, delegate_pointer, delegate_registry, delegate_seat,
    delegate_shm, delegate_xdg_shell, delegate_xdg_window, registry_handlers,
};
use wayland_client::globals::{GlobalList, registry_queue_init};
use wayland_client::protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::pointer_gestures::zv1::client::{
    zwp_pointer_gesture_hold_v1, zwp_pointer_gesture_pinch_v1, zwp_pointer_gesture_swipe_v1,
    zwp_pointer_gestures_v1,
};

struct Spike {
    start: Instant,
    registry: RegistryState,
    seat: SeatState,
    output: OutputState,
    shm: Shm,
    pool: SlotPool,
    window: Window,
    gestures: Option<zwp_pointer_gestures_v1::ZwpPointerGesturesV1>,
    pointer: Option<wl_pointer::WlPointer>,
    gesture_objects: Vec<Box<dyn std::any::Any>>,
    width: u32,
    height: u32,
    exit: bool,
}

impl Spike {
    fn log(&self, text: &str) {
        println!(
            "{:>10.3} ms  {text}",
            self.start.elapsed().as_secs_f64() * 1e3
        );
    }

    fn draw(&mut self) {
        let stride = self.width as i32 * 4;
        let (buffer, canvas) = self
            .pool
            .create_buffer(
                self.width as i32,
                self.height as i32,
                stride,
                wl_shm::Format::Argb8888,
            )
            .expect("shm buffer");
        for pixel in canvas.as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&0xff30_3040_u32.to_le_bytes());
        }
        let surface = self.window.wl_surface();
        surface.damage_buffer(0, 0, self.width as i32, self.height as i32);
        buffer.attach_to(surface).expect("attach");
        self.window.commit();
    }

    fn bind_gestures(&mut self, globals: &GlobalList, qh: &QueueHandle<Spike>) {
        match globals.bind::<zwp_pointer_gestures_v1::ZwpPointerGesturesV1, _, _>(qh, 1..=3, ()) {
            Ok(gestures) => {
                self.log(&format!(
                    "zwp_pointer_gestures_v1 version {}",
                    gestures.version()
                ));
                self.gestures = Some(gestures);
            }
            Err(error) => self.log(&format!("zwp_pointer_gestures_v1 not offered: {error}")),
        }
    }
}

fn main() {
    let connection = Connection::connect_to_env().expect("a Wayland session");
    let (globals, mut queue) = registry_queue_init::<Spike>(&connection).expect("registry");
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let shell = XdgShell::bind(&globals, &qh).expect("xdg_wm_base");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm");
    let surface = compositor.create_surface(&qh);
    let window = shell.create_window(surface, WindowDecorations::RequestServer, &qh);
    window.set_title("sctk-events spike");
    window.set_app_id("magic-driver.sctk-events-spike");
    window.set_min_size(Some((480, 360)));
    window.commit();
    let pool = SlotPool::new(640 * 480 * 4, &shm).expect("shm pool");
    let mut spike = Spike {
        start: Instant::now(),
        registry: RegistryState::new(&globals),
        seat: SeatState::new(&globals, &qh),
        output: OutputState::new(&globals, &qh),
        shm,
        pool,
        window,
        gestures: None,
        pointer: None,
        gesture_objects: Vec::new(),
        width: 640,
        height: 480,
        exit: false,
    };
    spike.bind_gestures(&globals, &qh);
    while !spike.exit {
        queue.blocking_dispatch(&mut spike).expect("dispatch");
    }
}

impl PointerHandler for Spike {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            match &event.kind {
                PointerEventKind::Enter { .. } => {
                    self.log(&format!("enter (wl_pointer v{})", pointer.version()));
                }
                PointerEventKind::Leave { .. } => self.log("leave"),
                PointerEventKind::Axis {
                    time,
                    horizontal,
                    vertical,
                    source,
                } => self.log(&format!(
                    "axis time={time} source={source:?} vertical={{value {:.3}, discrete {}, value120 {}, stop {}, direction {:?}}} horizontal={{value {:.3}, discrete {}, value120 {}, stop {}, direction {:?}}}",
                    vertical.absolute,
                    vertical.discrete,
                    vertical.value120,
                    vertical.stop,
                    vertical.relative_direction,
                    horizontal.absolute,
                    horizontal.discrete,
                    horizontal.value120,
                    horizontal.stop,
                    horizontal.relative_direction
                )),
                _ => {}
            }
        }
    }
}

impl SeatHandler for Spike {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        self.log(&format!("wl_seat version {}", seat.version()));
    }

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability != Capability::Pointer || self.pointer.is_some() {
            return;
        }
        self.log(&format!("pointer on wl_seat version {}", seat.version()));
        let pointer = self.seat.get_pointer(qh, &seat).expect("wl_pointer");
        if let Some(gestures) = &self.gestures {
            self.gesture_objects
                .push(Box::new(gestures.get_swipe_gesture(&pointer, qh, ())));
            self.gesture_objects
                .push(Box::new(gestures.get_pinch_gesture(&pointer, qh, ())));
            if gestures.version() >= 3 {
                self.gesture_objects
                    .push(Box::new(gestures.get_hold_gesture(&pointer, qh, ())));
            }
        }
        self.pointer = Some(pointer);
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        _: Capability,
    ) {
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl Dispatch<zwp_pointer_gestures_v1::ZwpPointerGesturesV1, ()> for Spike {
    fn event(
        _: &mut Self,
        _: &zwp_pointer_gestures_v1::ZwpPointerGesturesV1,
        _: zwp_pointer_gestures_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_pointer_gesture_swipe_v1::ZwpPointerGestureSwipeV1, ()> for Spike {
    fn event(
        state: &mut Self,
        _: &zwp_pointer_gesture_swipe_v1::ZwpPointerGestureSwipeV1,
        event: zwp_pointer_gesture_swipe_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.log(&format!("swipe {event:?}"));
    }
}

impl Dispatch<zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1, ()> for Spike {
    fn event(
        state: &mut Self,
        _: &zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1,
        event: zwp_pointer_gesture_pinch_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.log(&format!("pinch {event:?}"));
    }
}

impl Dispatch<zwp_pointer_gesture_hold_v1::ZwpPointerGestureHoldV1, ()> for Spike {
    fn event(
        state: &mut Self,
        _: &zwp_pointer_gesture_hold_v1::ZwpPointerGestureHoldV1,
        event: zwp_pointer_gesture_hold_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.log(&format!("hold {event:?}"));
    }
}

impl WindowHandler for Spike {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &Window,
        configure: WindowConfigure,
        _: u32,
    ) {
        self.width = configure.new_size.0.map_or(640, |width| width.get());
        self.height = configure.new_size.1.map_or(480, |height| height.get());
        self.draw();
    }
}

impl CompositorHandler for Spike {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Spike {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for Spike {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Spike {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(Spike);
delegate_output!(Spike);
delegate_shm!(Spike);
delegate_seat!(Spike);
delegate_pointer!(Spike);
delegate_xdg_shell!(Spike);
delegate_xdg_window!(Spike);
delegate_registry!(Spike);
