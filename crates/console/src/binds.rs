use input_iw4::{ClientInput, command_id_lookup, command_name, key_event};

use std::collections::HashMap;

use bevy::input::ButtonInput;
use bevy::input::gamepad::{Gamepad, GamepadButton};
use bevy::input::keyboard::KeyCode;
use bevy::input::mouse::{MouseButton, MouseScrollUnit};
use bevy::prelude::Resource;

pub const DEFAULT_CONTROLS: &str = include_str!("../assets/default_controls.cfg");

mod key_names;

pub use key_names::{
    BINDABLE_KEYS, display_button, host_keynum, parse_button_name, parse_key_name,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindButton {
    Key(KeyCode),
    Mouse(MouseButton),
    WheelUp,
    WheelDown,
    Pad(PadButton),
}

impl BindButton {
    pub fn is_pad(self) -> bool {
        matches!(self, Self::Pad(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PadButton {
    South,
    East,
    West,
    North,
    LeftBumper,
    RightBumper,
    LeftTrigger,
    RightTrigger,
    LeftStick,
    RightStick,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    Select,
    Start,
}

impl PadButton {
    pub const ALL: [Self; 16] = [
        Self::South,
        Self::East,
        Self::West,
        Self::North,
        Self::LeftBumper,
        Self::RightBumper,
        Self::LeftTrigger,
        Self::RightTrigger,
        Self::LeftStick,
        Self::RightStick,
        Self::DpadUp,
        Self::DpadDown,
        Self::DpadLeft,
        Self::DpadRight,
        Self::Select,
        Self::Start,
    ];

    pub const fn gamepad_button(self) -> GamepadButton {
        match self {
            Self::South => GamepadButton::South,
            Self::East => GamepadButton::East,
            Self::West => GamepadButton::West,
            Self::North => GamepadButton::North,
            Self::LeftBumper => GamepadButton::LeftTrigger,
            Self::RightBumper => GamepadButton::RightTrigger,
            Self::LeftTrigger => GamepadButton::LeftTrigger2,
            Self::RightTrigger => GamepadButton::RightTrigger2,
            Self::LeftStick => GamepadButton::LeftThumb,
            Self::RightStick => GamepadButton::RightThumb,
            Self::DpadUp => GamepadButton::DPadUp,
            Self::DpadDown => GamepadButton::DPadDown,
            Self::DpadLeft => GamepadButton::DPadLeft,
            Self::DpadRight => GamepadButton::DPadRight,
            Self::Select => GamepadButton::Select,
            Self::Start => GamepadButton::Start,
        }
    }

    pub fn from_gamepad_button(button: GamepadButton) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|pad| pad.gamepad_button() == button)
    }

    pub const fn console_name(self) -> &'static str {
        match self {
            Self::South => "BUTTON_A",
            Self::East => "BUTTON_B",
            Self::West => "BUTTON_X",
            Self::North => "BUTTON_Y",
            Self::LeftBumper => "BUTTON_LSHLDR",
            Self::RightBumper => "BUTTON_RSHLDR",
            Self::LeftTrigger => "BUTTON_LTRIG",
            Self::RightTrigger => "BUTTON_RTRIG",
            Self::LeftStick => "BUTTON_LSTICK",
            Self::RightStick => "BUTTON_RSTICK",
            Self::DpadUp => "DPAD_UP",
            Self::DpadDown => "DPAD_DOWN",
            Self::DpadLeft => "DPAD_LEFT",
            Self::DpadRight => "DPAD_RIGHT",
            Self::Select => "BUTTON_BACK",
            Self::Start => "BUTTON_START",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::South => "A",
            Self::East => "B",
            Self::West => "X",
            Self::North => "Y",
            Self::LeftBumper => "LB",
            Self::RightBumper => "RB",
            Self::LeftTrigger => "LT",
            Self::RightTrigger => "RT",
            Self::LeftStick => "LS",
            Self::RightStick => "RS",
            Self::DpadUp => "D-UP",
            Self::DpadDown => "D-DOWN",
            Self::DpadLeft => "D-LEFT",
            Self::DpadRight => "D-RIGHT",
            Self::Select => "BACK",
            Self::Start => "START",
        }
    }

    pub const fn prompt(self, style: frame::PromptStyle) -> &'static str {
        use frame::PromptStyle::*;
        match (style, self) {
            (Xbox, _) => self.label(),
            (PlayStation, Self::South) => "×",
            (PlayStation, Self::East) => "○",
            (PlayStation, Self::West) => "□",
            (PlayStation, Self::North) => "△",
            (PlayStation, Self::LeftBumper) => "L1",
            (PlayStation, Self::RightBumper) => "R1",
            (PlayStation, Self::LeftTrigger) => "L2",
            (PlayStation, Self::RightTrigger) => "R2",
            (PlayStation, Self::LeftStick) => "L3",
            (PlayStation, Self::RightStick) => "R3",
            (PlayStation, Self::Select) => "SHARE",
            (PlayStation, Self::Start) => "OPTIONS",
            (Generic, Self::South) => "SOUTH",
            (Generic, Self::East) => "EAST",
            (Generic, Self::West) => "WEST",
            (Generic, Self::North) => "NORTH",
            (Generic, Self::LeftBumper) => "L-BUMPER",
            (Generic, Self::RightBumper) => "R-BUMPER",
            (Generic, Self::LeftTrigger) => "L-TRIGGER",
            (Generic, Self::RightTrigger) => "R-TRIGGER",
            (Generic, Self::LeftStick) => "L-STICK",
            (Generic, Self::RightStick) => "R-STICK",
            (Generic, Self::Select) => "SELECT",
            _ => self.label(),
        }
    }

    const fn keynum(self) -> usize {
        190 + self as usize
    }
}

pub(crate) fn gameplay_binding(button: BindButton, command: u32, akimbo: bool) -> u32 {
    if !akimbo || !button.is_pad() {
        return command;
    }
    let mapped = match command_name(command) {
        Some("+attack") => "+speed_throw",
        Some("+speed_throw") => "+attack",
        _ => return command,
    };
    command_id_lookup(mapped).expect("built-in controller action")
}

pub fn pad_layout(layout: usize) -> Vec<(PadButton, &'static str)> {
    use PadButton::*;
    let mut binds = vec![
        (RightTrigger, "+attack"),
        (LeftTrigger, "+speed_throw"),
        (RightBumper, "+frag"),
        (LeftBumper, "+smoke"),
        (South, "+gostand"),
        (East, "+stance"),
        (West, "+usereload"),
        (North, "weapnext"),
        (LeftStick, "+breath_sprint"),
        (RightStick, "+melee"),
        (DpadUp, "+actionslot 1"),
        (DpadDown, "+actionslot 2"),
        (DpadLeft, "+actionslot 3"),
        (DpadRight, "+actionslot 4"),
        (Select, "+scores"),
    ];
    let mut set = |button: PadButton, command: &'static str| {
        binds.retain(|(b, _)| *b != button);
        binds.push((button, command));
    };
    let tactical = |set: &mut dyn FnMut(PadButton, &'static str)| {
        set(East, "+melee");
        set(RightStick, "+stance");
    };
    match layout {
        1 => tactical(&mut set),
        2 => {
            set(LeftTrigger, "+attack");
            set(RightTrigger, "+speed_throw");
            set(LeftBumper, "+frag");
            set(RightBumper, "+smoke");
        }
        3 | 4 => {
            set(LeftBumper, "+gostand");
            set(South, "+smoke");
            if layout == 4 {
                tactical(&mut set);
            }
        }
        _ => {}
    }
    binds
}

pub(crate) fn wheel_button(y: f32) -> Option<BindButton> {
    if y > 0.0 {
        Some(BindButton::WheelUp)
    } else if y < 0.0 {
        Some(BindButton::WheelDown)
    } else {
        None
    }
}

pub(crate) fn wheel_detents(unit: MouseScrollUnit, y: f32, carry: &mut f32) -> i32 {
    if !y.is_finite() {
        return 0;
    }
    let delta = match unit {
        MouseScrollUnit::Line => y,
        MouseScrollUnit::Pixel => y / 100.0,
    };
    *carry = (*carry + delta).clamp(-32.0, 32.0);
    let detents = carry.trunc() as i32;
    *carry -= detents as f32;
    detents
}

pub struct BindInputs<'a> {
    pub keys: &'a ButtonInput<KeyCode>,
    pub mouse: &'a ButtonInput<MouseButton>,
    pub pad: Option<&'a Gamepad>,
}

impl<'a> BindInputs<'a> {
    pub fn new(keys: &'a ButtonInput<KeyCode>, mouse: &'a ButtonInput<MouseButton>) -> Self {
        Self {
            keys,
            mouse,
            pad: None,
        }
    }

    pub fn with_pad(mut self, pad: Option<&'a Gamepad>) -> Self {
        self.pad = pad;
        self
    }

    pub fn pressed(&self, button: BindButton) -> bool {
        match button {
            BindButton::Key(key) => self.keys.pressed(key),
            BindButton::Mouse(btn) => self.mouse.pressed(btn),
            BindButton::WheelUp | BindButton::WheelDown => false,
            BindButton::Pad(btn) => self
                .pad
                .is_some_and(|pad| pad.pressed(btn.gamepad_button())),
        }
    }

    pub fn just_pressed(&self, button: BindButton) -> bool {
        match button {
            BindButton::Key(key) => self.keys.just_pressed(key),
            BindButton::Mouse(btn) => self.mouse.just_pressed(btn),
            BindButton::WheelUp | BindButton::WheelDown => false,
            BindButton::Pad(btn) => self
                .pad
                .is_some_and(|pad| pad.just_pressed(btn.gamepad_button())),
        }
    }

    pub fn just_released(&self, button: BindButton) -> bool {
        match button {
            BindButton::Key(key) => self.keys.just_released(key),
            BindButton::Mouse(btn) => self.mouse.just_released(btn),
            BindButton::WheelUp | BindButton::WheelDown => false,
            BindButton::Pad(btn) => self
                .pad
                .is_some_and(|pad| pad.just_released(btn.gamepad_button())),
        }
    }
}

/// The Minecraft map's inventory (and the crafting table in view): the one
/// command of the Minecraft layer.
pub const INVENTORY_COMMAND: &str = "inventory";

/// The controller button that opens the inventory by default, in every
/// layout: D-pad down. Each layout binds all sixteen buttons, so none is
/// free; D-pad down is MW2's `+actionslot 2`, a slot MW2 multiplayer's
/// scripts leave empty (1 is night vision, 3 the attachment, 4 the
/// killstreak), and the inventory sits beside it in the Minecraft layer
/// rather than in its place. D-pad left and right and Y already pick from
/// the hotbar there, and BACK is the scoreboard.
pub const INVENTORY_PAD_BUTTON: PadButton = PadButton::DpadDown;

/// Whether a command belongs to the Minecraft layer: bound beside a
/// button's MW2 command rather than in its place, and acted on only on the
/// Minecraft map, where it takes the press from MW2.
pub fn is_minecraft_command(id: u32) -> bool {
    command_name(id) == Some(INVENTORY_COMMAND)
}

#[derive(Resource, Debug, Clone, Default)]
pub struct KeyBinds {
    map: HashMap<BindButton, u32>,
    /// The Minecraft layer: E opens the inventory and stays MW2's Use.
    minecraft: HashMap<BindButton, u32>,
}

impl KeyBinds {
    pub fn apply_defaults(&mut self) {
        self.clear_all();
        let _ = self.apply_script(DEFAULT_CONTROLS);
        self.apply_pad_layout(0);
    }

    pub fn apply_script(&mut self, script: &str) -> Vec<String> {
        self.apply_script_inner(script, true)
    }

    pub(crate) fn apply_config_script(&mut self, script: &str) -> Vec<String> {
        self.apply_script_inner(script, false)
    }

    fn apply_script_inner(&mut self, script: &str, echo_success: bool) -> Vec<String> {
        let mut output = Vec::new();
        for raw in script.split([';', '\n']) {
            let line = raw.trim();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            let Some(command) = crate::ConsoleCommand::parse(line) else {
                continue;
            };
            match command.name.as_str() {
                "bind" => match self.cmd_bind(&command.args) {
                    Ok(Some(msg)) if echo_success => output.push(msg),
                    Ok(Some(_)) => {}
                    Ok(None) => {}
                    Err(msg) => output.push(msg),
                },
                "unbind" => match self.cmd_unbind(&command.args) {
                    Ok(Some(msg)) if echo_success => output.push(msg),
                    Ok(Some(_)) => {}
                    Ok(None) => {}
                    Err(msg) => output.push(msg),
                },
                "unbindall" => {
                    self.clear_all();
                    if echo_success {
                        output.push("unbindall".into());
                    }
                }
                other => output.push(format!("unknown bind-script command `{other}`")),
            }
        }
        output
    }

    fn layer_of(&mut self, id: u32) -> &mut HashMap<BindButton, u32> {
        if is_minecraft_command(id) {
            &mut self.minecraft
        } else {
            &mut self.map
        }
    }

    /// Binds a button to a command: in place of its MW2 command, or for the
    /// Minecraft layer's, beside it.
    pub fn set(&mut self, button: BindButton, id: u32) {
        self.layer_of(id).insert(button, id);
    }

    pub fn clear_button(&mut self, button: BindButton) -> bool {
        let gameplay = self.map.remove(&button).is_some();
        self.minecraft.remove(&button).is_some() || gameplay
    }

    pub fn clear_command(&mut self, id: u32) -> bool {
        let layer = self.layer_of(id);
        let before = layer.len();
        layer.retain(|_, bound| *bound != id);
        layer.len() != before
    }

    pub fn clear_all(&mut self) {
        self.map.clear();
        self.minecraft.clear();
    }

    pub fn clear_command_on(&mut self, id: u32, pad: bool) -> bool {
        let layer = self.layer_of(id);
        let before = layer.len();
        layer.retain(|button, bound| *bound != id || button.is_pad() != pad);
        layer.len() != before
    }

    /// A controller layout's MW2 buttons, and the inventory on
    /// `INVENTORY_PAD_BUTTON` beside them.
    pub fn apply_pad_layout(&mut self, layout: usize) {
        self.map.retain(|button, _| !button.is_pad());
        self.minecraft.retain(|button, _| !button.is_pad());
        for (button, command) in pad_layout(layout) {
            if let Some(id) = command_id_lookup(command) {
                self.set(BindButton::Pad(button), id);
            }
        }
        if let Some(id) = command_id_lookup(INVENTORY_COMMAND) {
            self.set(BindButton::Pad(INVENTORY_PAD_BUTTON), id);
        }
    }

    pub fn has_pad_binds(&self) -> bool {
        self.map.keys().any(|button| button.is_pad())
    }

    /// The button's MW2 command.
    pub fn get(&self, button: BindButton) -> Option<u32> {
        self.map.get(&button).copied()
    }

    pub fn binding_name(&self, button: BindButton) -> Option<&'static str> {
        self.get(button).and_then(command_name)
    }

    /// Whether the button opens the Minecraft inventory.
    pub fn opens_inventory(&self, button: BindButton) -> bool {
        self.minecraft.contains_key(&button)
    }

    /// The buttons bound to MW2's commands.
    pub fn iter(&self) -> impl Iterator<Item = (BindButton, u32)> + '_ {
        self.map.iter().map(|(b, id)| (*b, *id))
    }

    /// The buttons bound to the Minecraft layer's commands.
    pub fn iter_minecraft(&self) -> impl Iterator<Item = (BindButton, u32)> + '_ {
        self.minecraft.iter().map(|(b, id)| (*b, *id))
    }

    /// Every binding of both layers.
    pub fn iter_all(&self) -> impl Iterator<Item = (BindButton, u32)> + '_ {
        self.iter().chain(self.iter_minecraft())
    }

    pub fn list_lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .iter_all()
            .filter_map(|(button, id)| {
                command_name(id).map(|name| format!("bind {} {name}", display_button(button)))
            })
            .collect();
        lines.sort();
        lines.dedup();
        lines
    }

    fn cmd_bind(&mut self, args: &[String]) -> Result<Option<String>, String> {
        match args {
            [] => Ok(None),
            [key] => {
                let buttons =
                    parse_button_name(key).ok_or_else(|| format!("unknown key `{key}`"))?;
                let names: Vec<&str> = buttons
                    .iter()
                    .flat_map(|button| {
                        let minecraft = self.minecraft.get(button).copied().and_then(command_name);
                        self.binding_name(*button).into_iter().chain(minecraft)
                    })
                    .collect();
                if names.is_empty() {
                    Ok(Some(format!("`{key}` is unbound")))
                } else {
                    Ok(Some(format!("bind {key} {}", names.join(" + "))))
                }
            }
            [key, action @ ..] => {
                let action = action.join(" ");
                let id = command_id_lookup(&action)
                    .ok_or_else(|| format!("unknown command `{action}`"))?;
                let buttons =
                    parse_button_name(key).ok_or_else(|| format!("unknown key `{key}`"))?;
                for button in buttons {
                    self.set(button, id);
                }
                let name = command_name(id).unwrap_or(action.as_str());
                Ok(Some(format!("bind {key} {name}")))
            }
        }
    }

    fn cmd_unbind(&mut self, args: &[String]) -> Result<Option<String>, String> {
        match args {
            [key] => {
                let buttons =
                    parse_button_name(key).ok_or_else(|| format!("unknown key `{key}`"))?;
                let mut any = false;
                for button in buttons {
                    any |= self.clear_button(button);
                }
                if any {
                    Ok(Some(format!("unbind {key}")))
                } else {
                    Ok(Some(format!("`{key}` is unbound")))
                }
            }
            _ => Err("usage: unbind <key>".into()),
        }
    }
}

pub(crate) fn pulse_wheel_binding(
    binds: &KeyBinds,
    client: &mut ClientInput,
    button: BindButton,
    now_msec: i32,
    frame_msec: u32,
) {
    let Some(id) = binds.get(button) else { return };
    let key_num = host_keynum(button);
    client.keys[key_num].binding = id;
    key_event(client, key_num, true, now_msec, frame_msec);
    key_event(client, key_num, false, now_msec, frame_msec);
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYOUTS: [&str; 5] = [
        "Default",
        "Tactical",
        "Lefty",
        "Bumper Jumper",
        "Bumper Jumper Tactical",
    ];

    fn inventory() -> u32 {
        command_id_lookup(INVENTORY_COMMAND).unwrap()
    }

    /// The inventory's controller button takes no MW2 action in any
    /// layout: every layout binds every button (so none is free), and D-pad
    /// down is `+actionslot 2` in each, the slot MW2 multiplayer leaves
    /// empty, which keeps its binding beside the inventory's. It is none of
    /// the buttons the Minecraft map already uses (D-pad left and right and
    /// Y pick from the hotbar) and not BACK (the scoreboard) or START.
    #[test]
    fn the_inventory_button_steals_nothing_in_any_layout() {
        assert_eq!(INVENTORY_PAD_BUTTON, PadButton::DpadDown);
        for (layout, name) in LAYOUTS.iter().enumerate() {
            let binds = pad_layout(layout);
            let bound = |button: PadButton| {
                binds
                    .iter()
                    .find(|(b, _)| *b == button)
                    .map(|(_, command)| *command)
            };
            assert_eq!(bound(INVENTORY_PAD_BUTTON), Some("+actionslot 2"), "{name}");
            for taken in [PadButton::DpadLeft, PadButton::DpadRight, PadButton::North] {
                assert_ne!(taken, INVENTORY_PAD_BUTTON, "{name}");
            }
            assert_eq!(bound(PadButton::Select), Some("+scores"), "{name}");
            assert_ne!(INVENTORY_PAD_BUTTON, PadButton::Start);

            let mut keys = KeyBinds::default();
            keys.apply_defaults();
            keys.apply_pad_layout(layout);
            // Every MW2 binding of the layout is still there, unchanged.
            for (button, command) in &binds {
                assert_eq!(
                    keys.binding_name(BindButton::Pad(*button)),
                    Some(*command),
                    "{name}: {button:?}"
                );
            }
            assert_eq!(
                keys.iter().filter(|(button, _)| button.is_pad()).count(),
                binds.len(),
                "{name}"
            );
            // The inventory sits beside it.
            assert!(keys.opens_inventory(BindButton::Pad(INVENTORY_PAD_BUTTON)), "{name}");
            let pads: Vec<_> = keys
                .iter_minecraft()
                .filter(|(button, _)| button.is_pad())
                .collect();
            assert_eq!(pads, [(BindButton::Pad(INVENTORY_PAD_BUTTON), inventory())], "{name}");
        }
    }

    #[test]
    fn e_opens_the_inventory_and_stays_use() {
        let mut binds = KeyBinds::default();
        binds.apply_defaults();
        let e = BindButton::Key(KeyCode::KeyE);
        assert_eq!(binds.binding_name(e), Some("+activate"));
        assert!(binds.opens_inventory(e));
        // Nothing else opens it.
        assert_eq!(binds.iter_minecraft().count(), 2);
        // Its gameplay iteration never carries the inventory.
        assert!(binds.iter().all(|(_, id)| !is_minecraft_command(id)));
    }

    #[test]
    fn rebinding_the_inventory_leaves_mw2_s_bindings_alone() {
        let mut binds = KeyBinds::default();
        binds.apply_defaults();
        let id = inventory();
        // As Options → Controls rebinds: the controller's, then the keys'.
        binds.clear_command_on(id, true);
        binds.set(BindButton::Pad(PadButton::North), id);
        assert_eq!(binds.binding_name(BindButton::Pad(PadButton::North)), Some("weapnext"));
        assert!(binds.opens_inventory(BindButton::Pad(PadButton::North)));
        assert!(!binds.opens_inventory(BindButton::Pad(PadButton::DpadDown)));
        assert_eq!(binds.binding_name(BindButton::Pad(PadButton::DpadDown)), Some("+actionslot 2"));
        binds.clear_command_on(id, false);
        binds.set(BindButton::Key(KeyCode::KeyI), id);
        assert!(!binds.opens_inventory(BindButton::Key(KeyCode::KeyE)));
        assert_eq!(binds.binding_name(BindButton::Key(KeyCode::KeyE)), Some("+activate"));
        // An MW2 command bound onto the inventory's key replaces only MW2's.
        binds.set(BindButton::Key(KeyCode::KeyI), command_id_lookup("+reload").unwrap());
        assert!(binds.opens_inventory(BindButton::Key(KeyCode::KeyI)));
        assert_eq!(binds.binding_name(BindButton::Key(KeyCode::KeyI)), Some("+reload"));
        // `unbind` clears both layers.
        assert!(binds.clear_button(BindButton::Key(KeyCode::KeyI)));
        assert!(!binds.opens_inventory(BindButton::Key(KeyCode::KeyI)));
        assert_eq!(binds.get(BindButton::Key(KeyCode::KeyI)), None);
    }

    #[test]
    fn the_inventory_bindings_are_saved_and_read_back() {
        let mut binds = KeyBinds::default();
        binds.apply_defaults();
        let lines = binds.list_lines();
        assert!(lines.contains(&"bind e inventory".to_owned()), "{lines:?}");
        assert!(lines.contains(&"bind e +activate".to_owned()), "{lines:?}");
        assert!(lines.contains(&"bind DPAD_DOWN inventory".to_owned()), "{lines:?}");
        assert!(lines.contains(&"bind DPAD_DOWN +actionslot 2".to_owned()), "{lines:?}");
        let mut read = KeyBinds::default();
        let script = std::iter::once("unbindall".to_owned()).chain(lines).collect::<Vec<_>>().join("\n");
        assert!(read.apply_config_script(&script).is_empty());
        assert_eq!(read.list_lines(), binds.list_lines());
        assert!(read.opens_inventory(BindButton::Key(KeyCode::KeyE)));
        assert_eq!(read.binding_name(BindButton::Key(KeyCode::KeyE)), Some("+activate"));
    }
}
