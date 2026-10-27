mod report_builder;
pub mod scenarios;
mod stream;
mod timeline;

pub use report_builder::{ReportBuilder, TouchRecord, touch_state};
pub use stream::{SYNTHETIC_REPORT_INTERVAL_US, synthetic_session};
pub use timeline::{render_reports, render_timeline};
