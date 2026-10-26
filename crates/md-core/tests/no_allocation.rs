use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use md_core::config::Config;
use md_core::profile::MAGIC_MOUSE_2_BT;
use md_core::report;
use md_core::testkit::synthetic_session;
use md_core::{Engine, Input, Micros, Mode, Output};

struct CountingAllocator;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

fn allocations_during(work: impl FnOnce()) -> usize {
    let before = ALLOCATIONS.with(Cell::get);
    work();
    ALLOCATIONS.with(Cell::get) - before
}

fn malformed_and_ignored() -> Vec<Vec<u8>> {
    vec![
        Vec::new(),
        vec![0x12],
        vec![0x12; 7],
        vec![0x12; 15],
        vec![0x12; 14 + 16 * 8],
        vec![0x29; 20],
        vec![0x12, 0x01, 0x02, 0x00, 0x03, 0x00, 0x00, 0x00],
    ]
}

#[test]
fn counter_detects_allocations() {
    let allocations = allocations_during(|| {
        let out: Vec<Output> = vec![Output::PointerMotion { dx: 1, dy: 1 }];
        assert_eq!(out.len(), 1);
    });
    assert!(allocations > 0);
}

#[test]
fn parse_does_not_allocate() {
    let session = synthetic_session(2_000);
    let extra = malformed_and_ignored();
    let mut touches = 0;
    let allocations = allocations_during(|| {
        for (_, bytes) in &session {
            touches += report::parse(&MAGIC_MOUSE_2_BT, bytes).map_or(0, |r| r.touches.len());
        }
        for bytes in &extra {
            let _ = report::parse(&MAGIC_MOUSE_2_BT, bytes);
        }
    });
    assert!(touches > 0);
    assert_eq!(allocations, 0);
}

#[test]
fn engine_reuses_the_output_vector_in_every_mode() {
    let session = synthetic_session(2_000);
    let extra = malformed_and_ignored();
    for mode in [Mode::PointerOnly, Mode::RawForward, Mode::ScrollProbe] {
        let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, Config::default(), mode);
        let mut out: Vec<Output> = Vec::with_capacity(16);
        let config = Config::default();
        let mut emitted = 0;
        let allocations = allocations_during(|| {
            for (t, bytes) in &session {
                engine.handle(Input::Report { t: *t, bytes }, &mut out);
                emitted += out.len();
                out.clear();
                engine.handle(Input::Timer { t: *t }, &mut out);
            }
            for bytes in &extra {
                engine.handle(
                    Input::Report {
                        t: Micros::MAX,
                        bytes,
                    },
                    &mut out,
                );
            }
            engine.handle(Input::ConfigChanged(config), &mut out);
            engine.handle(Input::Reset, &mut out);
            out.clear();
        });
        assert!(emitted > 0, "{mode:?}");
        assert_eq!(allocations, 0, "{mode:?}");
    }
}
