use std::time::Instant;

use eframe::egui;

struct Spike {
    start: Instant,
    lines: Vec<String>,
}

impl Spike {
    fn log(&mut self, text: String) {
        let line = format!(
            "{:>10.3} ms  {text}",
            self.start.elapsed().as_secs_f64() * 1e3
        );
        println!("{line}");
        self.lines.push(line);
        if self.lines.len() > 40 {
            self.lines.remove(0);
        }
    }
}

impl eframe::App for Spike {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let events = ui.ctx().input(|input| input.raw.events.clone());
        for event in events {
            match event {
                egui::Event::MouseWheel {
                    unit,
                    delta,
                    phase,
                    source,
                    modifiers,
                } => self.log(format!(
                    "MouseWheel unit={unit:?} delta=({:.3}, {:.3}) phase={phase:?} source={source:?} modifiers={modifiers:?}",
                    delta.x, delta.y
                )),
                egui::Event::Zoom(factor) => self.log(format!("Zoom {factor:.4}")),
                egui::Event::Rotate(angle) => self.log(format!("Rotate {angle:.4}")),
                egui::Event::Touch {
                    device_id,
                    id,
                    phase,
                    pos,
                    force,
                } => self.log(format!(
                    "Touch device={device_id:?} id={id:?} phase={phase:?} pos={pos:?} force={force:?}"
                )),
                egui::Event::PointerGone => self.log("PointerGone".to_owned()),
                _ => {}
            }
        }
        ui.heading("eframe-events spike");
        ui.label("Scroll, swipe, pinch and hold over this window. Every egui event is printed to stdout.");
        ui.separator();
        for line in &self.lines {
            ui.monospace(line);
        }
        ui.ctx().request_repaint();
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("eframe-events spike")
            .with_inner_size([720.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "eframe-events",
        options,
        Box::new(|_cc| {
            Ok(Box::new(Spike {
                start: Instant::now(),
                lines: Vec::new(),
            }))
        }),
    )
}
