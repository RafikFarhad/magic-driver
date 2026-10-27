use std::fmt::Write as _;

use md_core::config::Config;
use md_core::profile::MAGIC_MOUSE_2_BT;
use md_core::synth::{Frame, TOUCHPAD_SLOTS, VIRTUAL_TOUCHPAD};
use md_core::testkit::scenarios::{self, Scenario, drive};
use md_core::{Engine, Micros, Mode, Output};

fn engine() -> Engine {
    Engine::new(&MAGIC_MOUSE_2_BT, Config::default(), Mode::Normal)
}

fn run(scenario: &Scenario) -> Vec<(Micros, Output)> {
    drive(&mut engine(), &scenario.timed())
}

fn render(timeline: &[(Micros, Output)]) -> String {
    let mut text = String::new();
    for (t, output) in timeline {
        writeln!(text, "{:>8} {output}", t.as_u64()).unwrap();
    }
    text
}

fn frames(timeline: &[(Micros, Output)]) -> Vec<(Micros, Frame)> {
    timeline
        .iter()
        .filter_map(|(t, output)| match output {
            Output::TouchFrame(frame) => Some((*t, *frame)),
            _ => None,
        })
        .collect()
}

fn strokes(timeline: &[(Micros, Output)]) -> usize {
    let mut count = 0;
    let mut open = false;
    for (_, frame) in frames(timeline) {
        if !open && !frame.is_empty() {
            count += 1;
        }
        open = !frame.is_empty();
    }
    count
}

fn deliberate_promotions(scenario: &Scenario) -> usize {
    let mut engine = engine();
    let mut promoted = 0;
    let mut out = Vec::new();
    for (t, bytes) in scenario.timed() {
        engine.handle(md_core::Input::Report { t, bytes: &bytes }, &mut out);
        out.clear();
        promoted += engine
            .classifier()
            .unwrap()
            .decisions()
            .iter()
            .filter(|decision| decision.class == md_core::classify::Class::Deliberate)
            .count();
    }
    promoted
}

#[test]
fn palm_grip_motion_and_clicks_produce_no_deliberate_track_and_no_frame() {
    for (name, scenario) in [
        ("idle-resting-palm", scenarios::idle_resting_palm()),
        ("move-fingers-resting", scenarios::move_fingers_resting()),
        ("click-left", scenarios::click_left()),
        ("click-right", scenarios::click_right()),
        ("click-middle", scenarios::click_middle()),
    ] {
        assert_eq!(deliberate_promotions(&scenario), 0, "{name}");
        assert!(frames(&run(&scenario)).is_empty(), "{name}");
    }
}

#[test]
fn slow_scroll_flick_and_horizontal_scroll_produce_exactly_one_stroke() {
    for (name, scenario) in [
        ("slow-scroll", scenarios::slow_scroll()),
        ("flick", scenarios::flick()),
        ("horizontal-scroll", scenarios::horizontal_scroll()),
    ] {
        assert_eq!(deliberate_promotions(&scenario), 1, "{name}");
        let timeline = run(&scenario);
        assert_eq!(strokes(&timeline), 1, "{name}");
        assert!(frames(&timeline).last().unwrap().1.is_empty(), "{name}");
    }
}

#[test]
fn slow_scroll_frames() {
    insta::assert_snapshot!("slow_scroll", render(&run(&scenarios::slow_scroll())));
}

#[test]
fn flick_frames() {
    insta::assert_snapshot!("flick", render(&run(&scenarios::flick())));
}

#[test]
fn horizontal_scroll_frames() {
    insta::assert_snapshot!(
        "horizontal_scroll",
        render(&run(&scenarios::horizontal_scroll()))
    );
}

#[test]
fn touch_after_flick_frames() {
    insta::assert_snapshot!(
        "touch_after_flick",
        render(&run(&scenarios::touch_after_flick()))
    );
}

#[test]
fn click_while_scrolling_frames() {
    insta::assert_snapshot!(
        "click_while_scrolling",
        render(&run(&scenarios::click_while_scrolling()))
    );
}

#[test]
fn slow_scroll_moves_vertically_only() {
    let timeline = run(&scenarios::slow_scroll());
    let frames = frames(&timeline);
    let first = frames[0].1.slots[0].unwrap();
    for (_, frame) in &frames {
        if let Some(contact) = frame.slots[0] {
            assert_eq!(contact.x, first.x);
            assert!(contact.y <= first.y);
        }
    }
    let last = frames[frames.len() - 2].1.slots[0].unwrap();
    assert!(first.y - last.y > VIRTUAL_TOUCHPAD.mm_to_units(25.0));
}

#[test]
fn horizontal_scroll_moves_sideways_and_can_be_switched_off() {
    let timeline = run(&scenarios::horizontal_scroll());
    let moving = frames(&timeline);
    let first = moving[0].1.slots[0].unwrap();
    let last = moving[moving.len() - 2].1.slots[0].unwrap();
    assert!(last.x - first.x > VIRTUAL_TOUCHPAD.mm_to_units(50.0));
    assert_eq!(last.y, first.y);

    let mut config = Config::default();
    config.scroll.horizontal = false;
    let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, config, Mode::Normal);
    let timeline = drive(&mut engine, &scenarios::horizontal_scroll().timed());
    let frames = frames(&timeline);
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].1.contact_count(), 2);
    assert!(frames[1].1.is_empty());
}

#[test]
fn touch_after_flick_holds_until_the_touch_is_classified_as_resting() {
    let scenario = scenarios::touch_after_flick();
    let reports = scenario.timed();
    let timeline = run(&scenario);
    let frames = frames(&timeline);
    let flick_lift = frames
        .iter()
        .position(|(_, frame)| frame.is_empty())
        .unwrap();
    let (hold_t, hold) = frames[flick_lift + 1];
    let (lift_t, lift) = frames[flick_lift + 2];
    assert_eq!(frames.len(), flick_lift + 3);
    assert_eq!(hold.contact_count(), 2);
    let flick_ids: Vec<u16> = frames[0]
        .1
        .contacts()
        .map(|(_, contact)| contact.tracking_id)
        .collect();
    assert!(
        hold.contacts()
            .all(|(_, contact)| !flick_ids.contains(&contact.tracking_id))
    );
    let touch_down = reports
        .iter()
        .map(|(t, _)| *t)
        .find(|t| *t == hold_t)
        .unwrap();
    assert!(lift.is_empty());
    assert_eq!(lift_t, touch_down + Micros::from_millis(300));
    assert!(reports.iter().all(|(t, _)| *t != lift_t));
}

#[test]
fn scroll_after_flick_turns_the_hold_into_the_new_stroke() {
    let timeline = run(&scenarios::scroll_after_flick());
    let frames = frames(&timeline);
    let flick_lift = frames
        .iter()
        .position(|(_, frame)| frame.is_empty())
        .unwrap();
    let hold = frames[flick_lift + 1].1;
    let next = frames[flick_lift + 2].1;
    assert_eq!(hold.contact_count(), 2);
    assert_eq!(next.contact_count(), 2);
    for slot in 0..2 {
        let (hold, next) = (hold.slots[slot].unwrap(), next.slots[slot].unwrap());
        assert_eq!(hold.tracking_id, next.tracking_id);
        assert_eq!(hold.x, next.x);
        assert!(next.y < hold.y);
    }
}

#[test]
fn click_during_scroll_freezes_for_the_cancel_hold_then_lifts() {
    let timeline = run(&scenarios::click_while_scrolling());
    let press_t = timeline
        .iter()
        .find(|(_, output)| matches!(output, Output::Button { pressed: true, .. }))
        .unwrap()
        .0;
    let frames = frames(&timeline);
    let after_press: Vec<_> = frames.iter().filter(|(t, _)| *t >= press_t).collect();
    assert!(after_press[0].1.is_empty());
    assert_eq!(after_press[0].0, press_t + Micros::from_millis(60));
    let before_press = frames.iter().rfind(|(t, _)| *t < press_t).unwrap();
    assert_eq!(before_press.1.contact_count(), 2);
    assert!(press_t - before_press.0 <= Micros::new(11_111));
}

#[test]
fn cancel_hold_follows_the_config() {
    let mut config = Config::default();
    config.advanced.scroll.cancel_hold_ms = 150;
    let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, config, Mode::Normal);
    let timeline = drive(&mut engine, &scenarios::click_while_scrolling().timed());
    let press_t = timeline
        .iter()
        .find(|(_, output)| matches!(output, Output::Button { pressed: true, .. }))
        .unwrap()
        .0;
    let lift = frames(&timeline)
        .into_iter()
        .find(|(t, _)| *t >= press_t)
        .unwrap();
    assert!(lift.1.is_empty());
    assert_eq!(lift.0, press_t + Micros::from_millis(150));
}

#[test]
fn every_scenario_closes_every_slot() {
    for (name, scenario) in scenarios::all() {
        let timeline = run(&scenario);
        let mut open = [None; TOUCHPAD_SLOTS];
        for (_, frame) in frames(&timeline) {
            for (slot, contact) in frame.slots.iter().enumerate() {
                if let (Some(open), Some(contact)) = (open[slot], contact) {
                    assert_eq!(open, contact.tracking_id, "{name}: slot {slot} reused");
                }
                open[slot] = contact.map(|contact| contact.tracking_id);
            }
        }
        assert!(open.iter().all(Option::is_none), "{name}");
    }
}
