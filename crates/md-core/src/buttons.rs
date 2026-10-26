use crate::bounded::BoundedVec;
use crate::primitives::Button;
use crate::profile::ButtonZones;
use crate::report::Touch;

const LEFT_BIT: u8 = 0x01;
const RIGHT_BIT: u8 = 0x02;
const MIDDLE_BIT: u8 = 0x04;
const CLICK_MASK: u8 = LEFT_BIT | RIGHT_BIT;
const ORDER: [Button; 3] = [Button::Left, Button::Right, Button::Middle];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ButtonSet(u8);

impl ButtonSet {
    pub const EMPTY: ButtonSet = ButtonSet(0);

    pub const fn only(button: Button) -> ButtonSet {
        ButtonSet(bit(button))
    }

    pub const fn from_click_bits(clicks: u8) -> ButtonSet {
        ButtonSet(clicks & CLICK_MASK)
    }

    pub const fn contains(self, button: Button) -> bool {
        self.0 & bit(button) != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn set(&mut self, button: Button, pressed: bool) {
        if pressed {
            self.0 |= bit(button);
        } else {
            self.0 &= !bit(button);
        }
    }

    pub fn iter(self) -> impl Iterator<Item = Button> {
        ORDER
            .into_iter()
            .filter(move |button| self.contains(*button))
    }
}

const fn bit(button: Button) -> u8 {
    match button {
        Button::Left => LEFT_BIT,
        Button::Right => RIGHT_BIT,
        Button::Middle => MIDDLE_BIT,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ButtonEdge {
    pub button: Button,
    pub pressed: bool,
}

impl Default for ButtonEdge {
    fn default() -> Self {
        ButtonEdge {
            button: Button::Left,
            pressed: false,
        }
    }
}

pub type ButtonEdges = BoundedVec<ButtonEdge, 3>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Buttons {
    pressed: ButtonSet,
}

impl Buttons {
    pub fn new() -> Buttons {
        Buttons::default()
    }

    pub const fn pressed(&self) -> ButtonSet {
        self.pressed
    }

    pub fn update(
        &mut self,
        zones: &ButtonZones,
        clicks: u8,
        touches: &[Touch],
        middle_click: bool,
    ) -> ButtonEdges {
        let next = decide(self.pressed, zones, clicks, touches, middle_click);
        self.transition(next)
    }

    pub fn release_all(&mut self) -> ButtonEdges {
        self.transition(ButtonSet::EMPTY)
    }

    fn transition(&mut self, next: ButtonSet) -> ButtonEdges {
        let mut edges = ButtonEdges::new();
        for button in ORDER {
            let was = self.pressed.contains(button);
            let is = next.contains(button);
            if was != is {
                edges
                    .push(ButtonEdge {
                        button,
                        pressed: is,
                    })
                    .expect("at most one edge per button");
            }
        }
        self.pressed = next;
        edges
    }
}

fn decide(
    pressed: ButtonSet,
    zones: &ButtonZones,
    clicks: u8,
    touches: &[Touch],
    middle_click: bool,
) -> ButtonSet {
    if clicks & CLICK_MASK == 0 {
        return ButtonSet::EMPTY;
    }
    if !pressed.is_empty() {
        return pressed;
    }
    match firm_touch(zones, touches) {
        Some(touch) if touch.x < zones.middle_start => ButtonSet::only(Button::Left),
        Some(touch) if touch.x > zones.middle_stop => ButtonSet::only(Button::Right),
        Some(_) if middle_click => ButtonSet::only(Button::Middle),
        Some(_) => ButtonSet::only(Button::Left),
        None => ButtonSet::from_click_bits(clicks),
    }
}

fn firm_touch<'a>(zones: &ButtonZones, touches: &'a [Touch]) -> Option<&'a Touch> {
    let down = touches.iter().filter(|touch| touch.state.is_down()).count();
    let mut firm = None;
    for record in &touches[..down] {
        let touch = touches
            .iter()
            .rev()
            .find(|other| other.id == record.id)
            .unwrap_or(record);
        if touch.size < zones.firm_touch_size {
            continue;
        }
        if firm.is_some() {
            return None;
        }
        firm = Some(touch);
    }
    firm
}

#[cfg(test)]
mod tests;
