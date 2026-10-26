use std::fmt::{Debug, Write as _};

use crate::Micros;
use crate::recording::render_report_line;

pub fn render_timeline<T: Debug>(items: impl IntoIterator<Item = (Micros, T)>) -> String {
    let mut text = String::new();
    for (t, item) in items {
        writeln!(text, "{:>10} {item:?}", t.as_u64()).expect("writing to a String cannot fail");
    }
    text
}

pub fn render_reports<'a>(reports: impl IntoIterator<Item = &'a (Micros, Vec<u8>)>) -> String {
    let mut text = String::new();
    for (t, bytes) in reports {
        writeln!(text, "{}", render_report_line(*t, bytes))
            .expect("writing to a String cannot fail");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_has_one_aligned_line_per_item() {
        let text = render_timeline([(Micros::new(0), "start"), (Micros::new(11_111), "drag")]);
        assert_eq!(text, "         0 \"start\"\n     11111 \"drag\"\n");
    }

    #[test]
    fn reports_render_as_recording_lines() {
        let reports = vec![(Micros::new(5), vec![0x12, 0xab])];
        assert_eq!(render_reports(&reports), "5 12ab\n");
    }
}
