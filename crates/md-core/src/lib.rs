#![forbid(unsafe_code)]

mod bounded;
pub mod buttons;
pub mod classify;
pub mod config;
pub mod context;
pub mod engine;
mod primitives;
pub mod profile;
pub mod recognise;
pub mod recording;
pub mod report;
pub mod surface;
pub mod synth;
#[cfg(any(test, feature = "test-support"))]
pub mod testkit;
pub mod trace;
pub mod tracker;

pub use bounded::BoundedVec;
pub use engine::{Engine, EngineStats, Input, Mode, Output};
pub use primitives::{Axis, Button, Micros};
