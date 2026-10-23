//! A controller on the inventory screen: a cursor on its slots that the
//! D-pad and the left stick move (repeating as the menus do), and buttons
//! that stand for the mouse's where the cursor is. A is the left button
//! (pick up, put down, swap; held while moving, it shares a carried stack
//! out), X the right (half a stack, or one), Y a shift click (quick move),
//! LB and RB a click outside the window (throw one, or the whole carried
//! stack). The clicks go through the mouse's own handling, so the screen
//! sends the same `McClick` a mouse would on that slot.
use bevy::input::ButtonInput;
use bevy::input::gamepad::{Gamepad, GamepadButton};
use bevy::input::mouse::MouseButton;
use bevy::math::Vec2;
use frame::McSlot;

/// The menus' repeat (`console::gamepad::drive_menus_with_pad`).
const REPEAT_DELAY: f64 = 0.4;
const REPEAT_EVERY: f64 = 0.12;
/// How far the left stick leans to move the cursor, as in the menus.
const STICK: f32 = 0.6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    /// The direction on the screen, y down.
    fn vector(self) -> Vec2 {
        match self {
            Self::Up => Vec2::new(0.0, -1.0),
            Self::Down => Vec2::new(0.0, 1.0),
            Self::Left => Vec2::new(-1.0, 0.0),
            Self::Right => Vec2::new(1.0, 0.0),
        }
    }
}

/// The direction the D-pad or the left stick holds, as the menus read it.
pub(crate) fn held_direction(pad: &Gamepad) -> Option<Dir> {
    let stick = pad.left_stick();
    if pad.pressed(GamepadButton::DPadUp) || stick.y > STICK {
        Some(Dir::Up)
    } else if pad.pressed(GamepadButton::DPadDown) || stick.y < -STICK {
        Some(Dir::Down)
    } else if pad.pressed(GamepadButton::DPadLeft) || stick.x < -STICK {
        Some(Dir::Left)
    } else if pad.pressed(GamepadButton::DPadRight) || stick.x > STICK {
        Some(Dir::Right)
    } else {
        None
    }
}

/// The slot next to `from` towards `dir` on a screen laid out as `slots`
/// (each slot's x, y and size): of the slots whose centre lies that way
/// (within a cone, no more than twice as far sideways as along), the
/// nearest, a step sideways counting double. `from` when none does.
pub(crate) fn step(slots: &[(McSlot, f32, f32, f32)], from: McSlot, dir: Dir) -> McSlot {
    let centre = |&(_, x, y, size): &(McSlot, f32, f32, f32)| Vec2::new(x + size * 0.5, y + size * 0.5);
    let Some(origin) = slots.iter().find(|slot| slot.0 == from).map(centre) else {
        return slots.first().map_or(from, |slot| slot.0);
    };
    let forward = dir.vector();
    slots
        .iter()
        .filter(|slot| slot.0 != from)
        .filter_map(|slot| {
            let offset = centre(slot) - origin;
            let along = offset.dot(forward);
            let across = (offset - forward * along).length();
            (along > 4.0 && across <= along * 2.0).then_some((along + 2.0 * across, slot.0))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map_or(from, |(_, slot)| slot)
}

/// What the controller did on the screen this frame, as the mouse would
/// have: its buttons (A the left, X the right), a shift click (Y), and a
/// click outside the window (LB throws one, RB the stack: `Some(right)`).
pub(crate) struct PadClicks {
    pub buttons: ButtonInput<MouseButton>,
    pub shift: bool,
    pub throw: Option<bool>,
}

/// The controller's cursor on the open screen.
#[derive(Default)]
pub(crate) struct PadScreen {
    pub slot: Option<McSlot>,
    table: bool,
    repeat: Option<(Dir, f64)>,
    /// A direction held as the screen opened (D-pad down opens it) moves
    /// nothing until it is let go.
    wait_release: bool,
    /// The mouse buttons A and X stand for, held as they are.
    buttons: ButtonInput<MouseButton>,
}

impl PadScreen {
    /// One frame of the controller on the open screen, laid out as `slots`
    /// (a crafting table's with `table`). The cursor starts on the selected
    /// hotbar slot.
    pub(crate) fn update(
        &mut self,
        pad: &Gamepad,
        slots: &[(McSlot, f32, f32, f32)],
        table: bool,
        selected: usize,
        now: f64,
    ) -> PadClicks {
        let slot = match self.slot {
            Some(slot) if self.table == table => slot,
            _ => {
                *self = Self {
                    table,
                    wait_release: true,
                    ..Self::default()
                };
                McSlot::Inventory(selected)
            }
        };
        let direction = held_direction(pad);
        let mut moved = slot;
        if self.wait_release {
            self.wait_release = direction.is_some();
        } else {
            let fire = match (direction, self.repeat) {
                (Some(dir), Some((held, next))) if dir == held => {
                    if now >= next {
                        self.repeat = Some((dir, now + REPEAT_EVERY));
                    }
                    now >= next
                }
                (Some(dir), _) => {
                    self.repeat = Some((dir, now + REPEAT_DELAY));
                    true
                }
                (None, _) => {
                    self.repeat = None;
                    false
                }
            };
            if fire && let Some(dir) = direction {
                moved = step(slots, slot, dir);
            }
        }
        self.slot = Some(moved);

        self.buttons.clear();
        for (button, mouse) in [(GamepadButton::South, MouseButton::Left), (GamepadButton::West, MouseButton::Right)] {
            if pad.just_pressed(button) {
                self.buttons.press(mouse);
            }
            if pad.just_released(button) {
                self.buttons.release(mouse);
            }
        }
        let mut buttons = self.buttons.clone();
        let mut shift = false;
        if pad.just_pressed(GamepadButton::North) && buttons.get_pressed().next().is_none() {
            buttons.press(MouseButton::Left);
            buttons.release(MouseButton::Left);
            shift = true;
        }
        let throw = if pad.just_pressed(GamepadButton::LeftTrigger) {
            Some(true)
        } else if pad.just_pressed(GamepadButton::RightTrigger) {
            Some(false)
        } else {
            None
        };
        PadClicks { buttons, shift, throw }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ScreenInput, handle_clicks, layout};
    use super::*;
    use frame::{McClick, McStack, MinecraftUi};

    fn walk(table: bool, from: McSlot, dirs: &[Dir]) -> McSlot {
        let slots = layout(table);
        dirs.iter().fold(from, |slot, dir| step(&slots, slot, *dir))
    }

    use Dir::*;
    use McSlot::{Crafting, Inventory, Result};

    #[test]
    fn the_cursor_walks_the_inventory() {
        // Along the hotbar and up through the backpack.
        assert_eq!(walk(false, Inventory(0), &[Right, Right]), Inventory(2));
        assert_eq!(walk(false, Inventory(0), &[Up]), Inventory(27));
        assert_eq!(walk(false, Inventory(0), &[Up, Up, Up]), Inventory(9));
        // Over the backpack: the boots, then up the armour to the helmet.
        assert_eq!(walk(false, Inventory(9), &[Up]), Inventory(36));
        assert_eq!(walk(false, Inventory(36), &[Up, Up, Up]), Inventory(39));
        // The offhand sits over the middle of the backpack.
        assert_eq!(walk(false, Inventory(13), &[Up]), Inventory(40));
        // The 2x2 grid and its result.
        assert_eq!(walk(false, Inventory(16), &[Up]), Crafting(3));
        assert_eq!(walk(false, Inventory(16), &[Up, Up]), Crafting(1));
        assert_eq!(walk(false, Crafting(2), &[Up, Right]), Crafting(1));
        assert_eq!(walk(false, Crafting(1), &[Right]), Result);
        assert_eq!(walk(false, Crafting(3), &[Right]), Result);
        // The edges hold.
        assert_eq!(walk(false, Inventory(8), &[Right, Down]), Inventory(8));
        assert_eq!(walk(false, Inventory(39), &[Left, Up]), Inventory(39));
    }

    #[test]
    fn the_cursor_walks_the_crafting_table() {
        assert_eq!(walk(true, Inventory(5), &[Up, Up, Up, Up]), Crafting(8));
        assert_eq!(walk(true, Crafting(8), &[Left, Left]), Crafting(6));
        assert_eq!(walk(true, Crafting(6), &[Up, Up]), Crafting(0));
        assert_eq!(walk(true, Crafting(0), &[Right, Right, Down]), Crafting(5));
        assert_eq!(walk(true, Crafting(5), &[Right]), Result);
        assert_eq!(walk(true, Crafting(8), &[Down]), Inventory(13));
        assert_eq!(walk(true, Crafting(8), &[Right]), Result);
        assert_eq!(walk(true, Result, &[Down]), Inventory(15));
        // Every slot of both screens is reached from the hotbar.
        for table in [false, true] {
            let slots = layout(table);
            let mut seen = vec![Inventory(0)];
            let mut at = 0;
            while at < seen.len() {
                for dir in [Up, Down, Left, Right] {
                    let next = step(&slots, seen[at], dir);
                    if !seen.contains(&next) {
                        seen.push(next);
                    }
                }
                at += 1;
            }
            assert_eq!(seen.len(), slots.len(), "table={table}");
        }
    }

    fn pad(pressed: &[GamepadButton]) -> Gamepad {
        let mut pad = Gamepad::default();
        for button in pressed {
            pad.digital_mut().press(*button);
        }
        pad
    }

    /// Steps the pad's frame and hands it to the mouse's clicks, as the
    /// screen does.
    fn frame(screen: &mut PadScreen, input: &mut ScreenInput, ui: &mut MinecraftUi, pad: &Gamepad, now: f64) {
        let clicks = screen.update(pad, &layout(true), true, ui.selected, now);
        handle_clicks(ui, input, &clicks.buttons, screen.slot, true, clicks.shift, None, now);
        if let Some(right) = clicks.throw {
            let mut button = ButtonInput::default();
            button.press(if right { MouseButton::Right } else { MouseButton::Left });
            handle_clicks(ui, input, &button, None, false, false, None, now);
        }
    }

    fn opened(selected: usize) -> (PadScreen, ScreenInput, MinecraftUi) {
        let mut screen = PadScreen::default();
        let mut input = ScreenInput::default();
        let mut ui = MinecraftUi { active: true, inventory_open: true, selected, ..Default::default() };
        // The screen opens with D-pad down still held: the cursor stays.
        frame(&mut screen, &mut input, &mut ui, &pad(&[GamepadButton::DPadDown]), 0.0);
        let mut held = pad(&[GamepadButton::DPadDown]);
        held.digital_mut().clear();
        frame(&mut screen, &mut input, &mut ui, &held, 0.5);
        assert_eq!(screen.slot, Some(Inventory(selected)));
        frame(&mut screen, &mut input, &mut ui, &pad(&[]), 0.6);
        (screen, input, ui)
    }

    #[test]
    fn the_cursor_starts_on_the_selected_slot_and_repeats_like_the_menus() {
        let (mut screen, mut input, mut ui) = opened(3);
        let mut right = pad(&[GamepadButton::DPadRight]);
        frame(&mut screen, &mut input, &mut ui, &right, 1.0);
        assert_eq!(screen.slot, Some(Inventory(4)));
        right.digital_mut().clear();
        // Held: nothing until the delay, then every 0.12 s.
        frame(&mut screen, &mut input, &mut ui, &right, 1.3);
        assert_eq!(screen.slot, Some(Inventory(4)));
        frame(&mut screen, &mut input, &mut ui, &right, 1.41);
        assert_eq!(screen.slot, Some(Inventory(5)));
        frame(&mut screen, &mut input, &mut ui, &right, 1.5);
        assert_eq!(screen.slot, Some(Inventory(5)));
        frame(&mut screen, &mut input, &mut ui, &right, 1.54);
        assert_eq!(screen.slot, Some(Inventory(6)));
        // The left stick leans the same way.
        let mut stick = Gamepad::default();
        stick.analog_mut().set(bevy::input::gamepad::GamepadAxis::LeftStickY, 0.9);
        frame(&mut screen, &mut input, &mut ui, &pad(&[]), 1.6);
        frame(&mut screen, &mut input, &mut ui, &stick, 1.7);
        assert_eq!(screen.slot, Some(Inventory(33)));
    }

    fn press(screen: &mut PadScreen, input: &mut ScreenInput, ui: &mut MinecraftUi, button: GamepadButton, now: f64) {
        frame(screen, input, ui, &pad(&[button]), now);
        let mut up = Gamepad::default();
        up.digital_mut().press(button);
        up.digital_mut().clear();
        up.digital_mut().release(button);
        frame(screen, input, ui, &up, now + 0.1);
    }

    fn stack(count: u8) -> Option<McStack> {
        Some(McStack { id: "minecraft:oak_planks".into(), count, weapon: None, durability: None })
    }

    #[test]
    fn the_buttons_click_as_the_mouse_does_on_the_cursor_s_slot() {
        let (mut screen, mut input, mut ui) = opened(5);
        // A: a left click on the slot.
        press(&mut screen, &mut input, &mut ui, GamepadButton::South, 1.0);
        assert_eq!(ui.clicks, [McClick::Slot { slot: Inventory(5), right: false, shift: false }]);
        // X: a right click; Y: a shift click.
        ui.clicks.clear();
        press(&mut screen, &mut input, &mut ui, GamepadButton::West, 2.0);
        press(&mut screen, &mut input, &mut ui, GamepadButton::North, 3.0);
        assert_eq!(
            ui.clicks,
            [
                McClick::Slot { slot: Inventory(5), right: true, shift: false },
                McClick::Slot { slot: Inventory(5), right: false, shift: true },
            ]
        );
        // On the result: a click there takes what was crafted.
        ui.clicks.clear();
        screen.slot = Some(Result);
        press(&mut screen, &mut input, &mut ui, GamepadButton::South, 4.0);
        press(&mut screen, &mut input, &mut ui, GamepadButton::North, 5.0);
        assert_eq!(
            ui.clicks,
            [
                McClick::Slot { slot: Result, right: false, shift: false },
                McClick::Slot { slot: Result, right: false, shift: true },
            ]
        );
        // On the grid with a carried stack: A puts it down, X one of it.
        ui.clicks.clear();
        ui.cursor = stack(8);
        screen.slot = Some(Crafting(4));
        press(&mut screen, &mut input, &mut ui, GamepadButton::West, 6.0);
        assert_eq!(ui.clicks, [McClick::Slot { slot: Crafting(4), right: true, shift: false }]);
        // LB and RB: a click outside the window throws one, or the stack.
        ui.clicks.clear();
        press(&mut screen, &mut input, &mut ui, GamepadButton::LeftTrigger, 7.0);
        press(&mut screen, &mut input, &mut ui, GamepadButton::RightTrigger, 8.0);
        assert_eq!(ui.clicks, [McClick::Outside { right: true }, McClick::Outside { right: false }]);
    }

    #[test]
    fn a_carried_stack_on_an_inventory_slot_goes_down_on_release_or_spreads() {
        let (mut screen, mut input, mut ui) = opened(0);
        ui.cursor = stack(8);
        // A pressed and let go on one slot: put down there, as a mouse
        // click is (its drag over one slot).
        frame(&mut screen, &mut input, &mut ui, &pad(&[GamepadButton::South]), 1.0);
        assert!(ui.clicks.is_empty());
        let mut up = pad(&[GamepadButton::South]);
        up.digital_mut().clear();
        up.digital_mut().release(GamepadButton::South);
        frame(&mut screen, &mut input, &mut ui, &up, 1.1);
        assert_eq!(ui.clicks, [McClick::Slot { slot: Inventory(0), right: false, shift: false }]);
        // A held while the cursor moves on: the stack is shared out.
        ui.clicks.clear();
        frame(&mut screen, &mut input, &mut ui, &pad(&[GamepadButton::South]), 2.0);
        let mut held = pad(&[GamepadButton::South, GamepadButton::DPadRight]);
        held.digital_mut().clear();
        held.digital_mut().press(GamepadButton::DPadRight);
        frame(&mut screen, &mut input, &mut ui, &held, 2.1);
        let mut up = pad(&[GamepadButton::South]);
        up.digital_mut().clear();
        up.digital_mut().release(GamepadButton::South);
        frame(&mut screen, &mut input, &mut ui, &up, 2.2);
        assert_eq!(ui.clicks, [McClick::Spread { slots: vec![0, 1], right: false }]);
    }

    #[test]
    fn a_double_a_gathers_as_a_double_click_does() {
        let (mut screen, mut input, mut ui) = opened(2);
        screen.slot = Some(Crafting(0));
        frame(&mut screen, &mut input, &mut ui, &pad(&[GamepadButton::South]), 1.0);
        ui.cursor = stack(3);
        let mut up = pad(&[GamepadButton::South]);
        up.digital_mut().clear();
        up.digital_mut().release(GamepadButton::South);
        frame(&mut screen, &mut input, &mut ui, &up, 1.05);
        frame(&mut screen, &mut input, &mut ui, &pad(&[GamepadButton::South]), 1.1);
        assert_eq!(
            ui.clicks,
            [McClick::Slot { slot: Crafting(0), right: false, shift: false }, McClick::Gather { right: false }]
        );
    }
}
