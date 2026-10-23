//! The Minecraft map's bindings: the inventory's binding (E, D-pad down on a
//! controller, rebindable in Options → Controls) opens and closes the
//! inventory screen, and the screen's hints name the keys and buttons it
//! takes. The screen itself, and the controller's cursor on it, are the
//! HUD's (`hud::minecraft_inventory`).
use bevy::input::ButtonInput;
use bevy::input::gamepad::Gamepad;
use bevy::input::keyboard::KeyCode;
use bevy::input::mouse::MouseButton;
use bevy::prelude::*;

use crate::binds::{BindButton, BindInputs, KeyBinds, PadButton, display_button};

/// Whether the open screen takes this button for its own, so that the
/// inventory's binding on it only opens the screen: the controller's
/// screen buttons and the mouse's clicks.
fn screen_takes(button: BindButton) -> bool {
    match button {
        BindButton::Pad(pad) => frame::MC_SCREEN_PAD_BUTTONS.contains(&pad.gamepad_button()),
        BindButton::Mouse(mouse) => matches!(mouse, MouseButton::Left | MouseButton::Right),
        BindButton::Key(key) => key == KeyCode::Escape,
        _ => false,
    }
}

/// Whether the inventory's binding went down this frame, to open the screen
/// or (`open`) close it.
pub(crate) fn toggle_pressed(binds: &KeyBinds, inputs: &BindInputs<'_>, open: bool) -> bool {
    binds
        .iter_minecraft()
        .any(|(button, _)| inputs.just_pressed(button) && !(open && screen_takes(button)))
}

/// What the open screen says closes it, and with a controller in hand the
/// line naming its buttons, in the controller's own prompts.
pub(crate) fn screen_hints(
    binds: &KeyBinds,
    pad: bool,
    style: frame::PromptStyle,
) -> (String, Option<String>) {
    if pad {
        let p = |button: PadButton| button.prompt(style);
        let line = format!(
            "{} TAKE / PLACE    {} HALF / ONE    {} QUICK MOVE    {} THROW ONE    {} THROW ALL    {} CLOSE",
            p(PadButton::South),
            p(PadButton::West),
            p(PadButton::North),
            p(PadButton::LeftBumper),
            p(PadButton::RightBumper),
            p(PadButton::East),
        );
        return (format!("{} TO CLOSE", p(PadButton::East)), Some(line));
    }
    let mut keys: Vec<String> = binds
        .iter_minecraft()
        .filter(|(button, _)| matches!(button, BindButton::Key(_)))
        .map(|(button, _)| display_button(button).to_uppercase())
        .collect();
    keys.sort();
    let key = keys.into_iter().next().unwrap_or_else(|| "ESC".to_owned());
    (format!("{key} TO CLOSE"), None)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn publish_minecraft_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBinds>,
    console: Res<crate::ConsoleState>,
    script_menus: Option<Res<hud::ScriptMenus>>,
    capture: Res<frame::UiBindingCapture>,
    (gamepads, active, devices): (Query<&Gamepad>, Res<frame::ActivePad>, Res<frame::InputDevices>),
    ui: Option<ResMut<frame::MinecraftUi>>,
) {
    let Some(mut ui) = ui else { return };
    if !ui.active {
        ui.inventory_toggle = false;
        return;
    }
    let (close, pad_line) = screen_hints(&binds, devices.pad_prompts, devices.style);
    if ui.close_hint != close {
        ui.close_hint = close;
    }
    if ui.pad_hint != pad_line {
        ui.pad_hint = pad_line;
    }
    let script_menu = script_menus.is_some_and(|menus| menus.captures_input());
    if console.open || script_menu || capture.command.is_some() || !devices.focused {
        return;
    }
    let pad = active.0.and_then(|entity| gamepads.get(entity).ok());
    let inputs = BindInputs::new(&keys, &mouse).with_pad(pad);
    if toggle_pressed(&binds, &inputs, ui.inventory_open) {
        ui.inventory_toggle = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use input_iw4::command_id_lookup;

    fn binds() -> KeyBinds {
        let mut binds = KeyBinds::default();
        binds.apply_defaults();
        binds
    }

    fn keys(pressed: &[KeyCode]) -> ButtonInput<KeyCode> {
        let mut input = ButtonInput::default();
        for key in pressed {
            input.press(*key);
        }
        input
    }

    fn pad(pressed: &[PadButton]) -> Gamepad {
        let mut pad = Gamepad::default();
        for button in pressed {
            pad.digital_mut().press(button.gamepad_button());
        }
        pad
    }

    #[test]
    fn e_and_d_pad_down_open_the_inventory() {
        let binds = binds();
        let mouse = ButtonInput::default();
        let e = keys(&[KeyCode::KeyE]);
        assert!(toggle_pressed(&binds, &BindInputs::new(&e, &mouse), false));
        // E closes the open screen too.
        assert!(toggle_pressed(&binds, &BindInputs::new(&e, &mouse), true));
        let none = keys(&[]);
        let down = pad(&[PadButton::DpadDown]);
        let inputs = BindInputs::new(&none, &mouse).with_pad(Some(&down));
        assert!(toggle_pressed(&binds, &inputs, false));
        // Inside the screen D-pad down moves the cursor: B closes it.
        assert!(!toggle_pressed(&binds, &inputs, true));
        let other = pad(&[PadButton::DpadUp, PadButton::North, PadButton::Select]);
        let inputs = BindInputs::new(&none, &mouse).with_pad(Some(&other));
        assert!(!toggle_pressed(&binds, &inputs, false));
    }

    #[test]
    fn a_rebound_inventory_follows_its_binding() {
        let mut binds = binds();
        let id = command_id_lookup("inventory").unwrap();
        binds.clear_command_on(id, true);
        binds.set(BindButton::Pad(PadButton::Select), id);
        let none = keys(&[]);
        let mouse = ButtonInput::default();
        let back = pad(&[PadButton::Select]);
        let inputs = BindInputs::new(&none, &mouse).with_pad(Some(&back));
        // BACK is no screen button: it opens and closes.
        assert!(toggle_pressed(&binds, &inputs, false));
        assert!(toggle_pressed(&binds, &inputs, true));
        let down = pad(&[PadButton::DpadDown]);
        let inputs = BindInputs::new(&none, &mouse).with_pad(Some(&down));
        assert!(!toggle_pressed(&binds, &inputs, false));
        // BACK stays the scoreboard in MW2's layer.
        assert_eq!(binds.binding_name(BindButton::Pad(PadButton::Select)), Some("+scores"));
        binds.clear_command_on(id, false);
        binds.set(BindButton::Key(KeyCode::KeyI), id);
        let i = keys(&[KeyCode::KeyI]);
        assert!(toggle_pressed(&binds, &BindInputs::new(&i, &mouse), false));
        let e = keys(&[KeyCode::KeyE]);
        assert!(!toggle_pressed(&binds, &BindInputs::new(&e, &mouse), false));
        assert_eq!(screen_hints(&binds, false, frame::PromptStyle::Xbox).0, "I TO CLOSE");
    }

    #[test]
    fn the_hints_name_the_keys_or_the_controller_s_buttons() {
        let binds = binds();
        assert_eq!(
            screen_hints(&binds, false, frame::PromptStyle::Xbox),
            ("E TO CLOSE".to_owned(), None)
        );
        let (close, line) = screen_hints(&binds, true, frame::PromptStyle::Xbox);
        assert_eq!(close, "B TO CLOSE");
        assert_eq!(
            line.as_deref(),
            Some("A TAKE / PLACE    X HALF / ONE    Y QUICK MOVE    LB THROW ONE    RB THROW ALL    B CLOSE")
        );
        let (close, line) = screen_hints(&binds, true, frame::PromptStyle::PlayStation);
        assert_eq!(close, "○ TO CLOSE");
        assert!(line.unwrap().starts_with("× TAKE / PLACE"));
    }
}
