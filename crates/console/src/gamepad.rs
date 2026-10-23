use bevy::input::gamepad::{Gamepad, GamepadAxis, GamepadButton, GamepadConnection, GamepadEvent};
use bevy::prelude::*;
use frame::{UiMenuKey, UiMenuRequest};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Sticks {
    /// Forward and right.
    pub movement: Vec2,
    /// Right and up.
    pub look: Vec2,
}

fn radial(stick: Vec2, deadzone: f32) -> Vec2 {
    let length = stick.length();
    if length <= deadzone || length <= f32::EPSILON {
        return Vec2::ZERO;
    }
    let scaled = ((length - deadzone) / (1.0 - deadzone).max(0.01)).min(1.0);
    stick / length * scaled
}

pub(crate) fn sticks(pad: &Gamepad, settings: &frame::GameSettings) -> Sticks {
    let left = radial(pad.left_stick(), settings.pad_deadzone_left);
    let right = radial(pad.right_stick(), settings.pad_deadzone_right);
    laid_out(left, right, settings.pad_stick_layout)
}

/// The sticks as the hand holds them, before any deadzone: what a
/// [`StickGate`] watches to tell a stick let go from one still pushed.
pub(crate) fn raw_sticks(pad: &Gamepad, settings: &frame::GameSettings) -> Sticks {
    laid_out(pad.left_stick(), pad.right_stick(), settings.pad_stick_layout)
}

fn laid_out(left: Vec2, right: Vec2, layout: u8) -> Sticks {
    match layout {
        1 => Sticks {
            movement: Vec2::new(right.y, right.x),
            look: left,
        },
        2 => Sticks {
            movement: Vec2::new(left.y, right.x),
            look: Vec2::new(left.x, right.y),
        },
        3 => Sticks {
            movement: Vec2::new(right.y, left.x),
            look: Vec2::new(right.x, left.y),
        },
        _ => Sticks {
            movement: Vec2::new(left.y, left.x),
            look: right,
        },
    }
}

/// How far a stick may still lean, before any deadzone, and count as let go.
///
/// Well past where a worn stick comes to rest (XInput's own suggested
/// left-stick deadzone is 0.24) and well short of the 0.6 that steps
/// through a menu.
pub(crate) const STICK_LET_GO: f32 = 0.3;

/// Keeps a stick out of gameplay from the moment a menu, the console or an
/// inventory takes the controller until the player lets that stick go, so
/// a push that was steering a menu doesn't walk or turn the player once the
/// menu closes.
///
/// "Let go" is the stick back under [`STICK_LET_GO`], not back at exactly
/// zero: a stick that comes to rest a little off centre, outside the
/// deadzone, never reads zero after the deadzone, and waiting for that kept
/// the player from moving at all after the pause menu was closed with the
/// controller, while the other stick (resting inside its deadzone) still
/// turned the view and the keyboard still walked.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct StickGate {
    held: bool,
}

impl StickGate {
    /// Something else has the controller now.
    pub(crate) fn capture(&mut self) {
        self.held = true;
    }

    /// Gameplay has the stick at once (a different controller took over).
    pub(crate) fn release(&mut self) {
        self.held = false;
    }

    /// The stick's value for gameplay this frame: `shaped` (after the
    /// deadzone) once the stick has been let go since the last capture,
    /// zero until then. `raw` is the same stick before the deadzone.
    pub(crate) fn pass(&mut self, raw: Vec2, shaped: Vec2) -> Vec2 {
        if self.held && raw.length() < STICK_LET_GO {
            self.held = false;
        }
        if self.held { Vec2::ZERO } else { shaped }
    }
}

fn curve(deflection: f32, kind: u8) -> f32 {
    let d = deflection.clamp(0.0, 1.0);
    match kind {
        1 => d,
        2 => 1.0 - (1.0 - d) * (1.0 - d),
        _ => 0.35 * d + 0.65 * d * d * d,
    }
}

pub(crate) fn shaped_look(look: Vec2, settings: &frame::GameSettings) -> Vec2 {
    let deflection = look.length();
    if deflection <= f32::EPSILON {
        return Vec2::ZERO;
    }
    let shaped = look / deflection * curve(deflection, settings.pad_curve);
    Vec2::new(
        shaped.x,
        if settings.pad_invert {
            -shaped.y
        } else {
            shaped.y
        },
    )
}

#[derive(Default)]
pub(crate) struct PadActivity {
    axes: std::collections::HashMap<(Entity, GamepadAxis), f32>,
    buttons: std::collections::HashMap<(Entity, GamepadButton), f32>,
}

pub(crate) fn track_active_pad(
    gamepads: Query<(Entity, &Gamepad, Option<&Name>)>,
    mut events: MessageReader<GamepadEvent>,
    mut active: ResMut<frame::ActivePad>,
    mut devices: ResMut<frame::InputDevices>,
    settings: Res<frame::GameSettings>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut activity: Local<PadActivity>,
) {
    devices.focused = windows.single().map_or(true, |window| window.focused);
    if active.0.is_some_and(|entity| gamepads.get(entity).is_err()) {
        active.0 = None;
    }
    let PadActivity { axes, buttons } = &mut *activity;
    axes.retain(|(entity, _), _| gamepads.contains(*entity));
    buttons.retain(|(entity, _), _| gamepads.contains(*entity));
    let mut connected = std::collections::HashSet::new();
    for event in events.read() {
        let activity = match event {
            GamepadEvent::Connection(event) => {
                if matches!(event.connection, GamepadConnection::Connected { .. }) {
                    connected.insert(event.gamepad);
                }
                if matches!(event.connection, GamepadConnection::Disconnected) {
                    axes.retain(|(entity, _), _| *entity != event.gamepad);
                    buttons.retain(|(entity, _), _| *entity != event.gamepad);
                    if active.0 == Some(event.gamepad) {
                        active.0 = None;
                    }
                }
                if matches!(event.connection, GamepadConnection::Connected { .. })
                    && let Ok((_, pad, _)) = gamepads.get(event.gamepad)
                {
                    for axis in [
                        GamepadAxis::LeftStickX,
                        GamepadAxis::LeftStickY,
                        GamepadAxis::RightStickX,
                        GamepadAxis::RightStickY,
                    ] {
                        axes.insert((event.gamepad, axis), pad.get(axis).unwrap_or(0.0));
                    }
                }
                None
            }
            GamepadEvent::Button(event) => {
                let previous = buttons
                    .insert((event.entity, event.button), event.value)
                    .unwrap_or(0.0);
                (event.value >= 0.55 && previous < 0.55).then_some(event.entity)
            }
            GamepadEvent::Axis(event) => {
                let previous = axes.entry((event.entity, event.axis)).or_insert(0.0);
                let deadzone = match event.axis {
                    GamepadAxis::LeftStickX | GamepadAxis::LeftStickY => settings.pad_deadzone_left,
                    _ => settings.pad_deadzone_right,
                }
                .max(0.15);
                let meaningful =
                    event.value.abs() > deadzone + 0.08 && (event.value - *previous).abs() > 0.12;
                if meaningful || event.value.abs() < deadzone {
                    *previous = event.value;
                }
                meaningful.then_some(event.entity)
            }
        };
        if let Some(entity) = activity.filter(|entity| {
            devices.focused && !connected.contains(entity) && gamepads.contains(*entity)
        }) {
            active.0 = Some(entity);
            devices.pad_prompts = true;
        }
    }
    if active.0.is_none() {
        devices.pad_prompts = false;
    }
    devices.style = match settings.pad_prompts {
        1 => frame::PromptStyle::Xbox,
        2 => frame::PromptStyle::PlayStation,
        3 => frame::PromptStyle::Generic,
        _ => active
            .0
            .and_then(|entity| gamepads.get(entity).ok())
            .map_or(frame::PromptStyle::Generic, |(_, pad, name)| {
                let name = name.map_or("", |name| name.as_str()).to_ascii_lowercase();
                if pad.vendor_id() == Some(0x054c)
                    || name.contains("dualshock")
                    || name.contains("dualsense")
                    || name == "wireless controller"
                {
                    frame::PromptStyle::PlayStation
                } else if pad.vendor_id() == Some(0x045e)
                    || name.contains("xbox")
                    || name.contains("xinput")
                {
                    frame::PromptStyle::Xbox
                } else {
                    frame::PromptStyle::Generic
                }
            }),
    };
}

const REPEAT_DELAY: f32 = 0.4;
const REPEAT_EVERY: f32 = 0.12;

pub(crate) fn drive_menus_with_pad(
    gamepads: Query<&Gamepad>,
    active: Res<frame::ActivePad>,
    script_menus: Option<Res<hud::ScriptMenus>>,
    (devices, console, capture): (
        Res<frame::InputDevices>,
        Res<crate::ConsoleState>,
        Res<frame::UiBindingCapture>,
    ),
    time: Res<Time>,
    mut requests: MessageWriter<UiMenuRequest>,
    mut repeat: Local<Option<(UiMenuKey, f32)>>,
) {
    if !devices.focused || console.open {
        *repeat = None;
        return;
    }
    let pad = active
        .0
        .and_then(|entity| gamepads.get(entity).ok())
        .filter(|_| capture.command.is_none());
    let Some(pad) = pad else {
        *repeat = None;
        return;
    };
    if pad.just_pressed(GamepadButton::Start) {
        requests.write(UiMenuRequest::Key(UiMenuKey::Escape));
        *repeat = None;
        return;
    }
    if !script_menus.is_some_and(|menus| menus.captures_input()) {
        *repeat = None;
        return;
    }
    if pad.just_pressed(GamepadButton::South) {
        requests.write(UiMenuRequest::Key(UiMenuKey::Enter));
    }
    if pad.just_pressed(GamepadButton::East) {
        requests.write(UiMenuRequest::Key(UiMenuKey::Escape));
    }
    let stick = pad.left_stick();
    let direction = if pad.pressed(GamepadButton::DPadUp) || stick.y > 0.6 {
        Some(UiMenuKey::Up)
    } else if pad.pressed(GamepadButton::DPadDown) || stick.y < -0.6 {
        Some(UiMenuKey::Down)
    } else if pad.pressed(GamepadButton::DPadLeft) || stick.x < -0.6 {
        Some(UiMenuKey::Left)
    } else if pad.pressed(GamepadButton::DPadRight) || stick.x > 0.6 {
        Some(UiMenuKey::Right)
    } else {
        None
    };
    let now = time.elapsed_secs();
    let fire = match (direction, *repeat) {
        (Some(key), Some((held, next))) if key == held => {
            if now >= next {
                *repeat = Some((key, now + REPEAT_EVERY));
            }
            now >= next
        }
        (Some(key), _) => {
            *repeat = Some((key, now + REPEAT_DELAY));
            true
        }
        (None, _) => {
            *repeat = None;
            false
        }
    };
    if fire && let Some(key) = direction {
        requests.write(UiMenuRequest::Key(key));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::input::gamepad::GamepadInput;

    fn deadzone(stick: Vec2) -> Vec2 {
        radial(stick, frame::GameSettings::default().pad_deadzone_left)
    }

    /// The owner's bug: a left stick that comes to rest a little off centre
    /// (outside the 0.12 deadzone, as a worn stick does) never read zero
    /// after the pause menu closed, so the player never walked again.
    #[test]
    fn a_stick_resting_off_centre_walks_again_after_a_menu() {
        let rest = Vec2::new(0.1, 0.1);
        assert_ne!(deadzone(rest), Vec2::ZERO, "the rest point is outside the deadzone");
        let mut gate = StickGate::default();
        gate.capture();
        gate.pass(rest, deadzone(rest));
        let forward = Vec2::new(0.0, 1.0);
        assert_eq!(gate.pass(forward, deadzone(forward)), deadzone(forward));
    }

    #[test]
    fn a_stick_still_pushed_when_a_menu_closes_waits_to_be_let_go() {
        let mut gate = StickGate::default();
        gate.capture();
        for push in [Vec2::new(0.0, 0.9), Vec2::new(0.1, 0.95), Vec2::new(0.0, 0.31)] {
            assert_eq!(gate.pass(push, deadzone(push)), Vec2::ZERO, "{push}");
        }
        let let_go = Vec2::new(0.02, 0.05);
        assert_eq!(gate.pass(let_go, deadzone(let_go)), Vec2::ZERO);
        let push = Vec2::new(0.0, 0.9);
        assert_eq!(gate.pass(push, deadzone(push)), deadzone(push));
    }

    #[test]
    fn a_stick_nothing_captured_passes_straight_through() {
        let mut gate = StickGate::default();
        let push = Vec2::new(0.7, -0.7);
        assert_eq!(gate.pass(push, deadzone(push)), deadzone(push));
        gate.capture();
        gate.release();
        assert_eq!(gate.pass(push, deadzone(push)), deadzone(push));
    }

    #[test]
    fn let_go_is_short_of_a_menu_step_and_past_a_worn_rest() {
        assert!(STICK_LET_GO < 0.6);
        assert!(STICK_LET_GO > 0.24);
    }

    #[test]
    fn raw_sticks_follow_the_stick_layout_before_the_deadzone() {
        let mut pad = Gamepad::default();
        let axes = pad.analog_mut();
        axes.set(GamepadInput::Axis(GamepadAxis::LeftStickX), 0.05);
        axes.set(GamepadInput::Axis(GamepadAxis::LeftStickY), 0.08);
        axes.set(GamepadInput::Axis(GamepadAxis::RightStickX), 0.5);
        axes.set(GamepadInput::Axis(GamepadAxis::RightStickY), -0.25);
        let mut settings = frame::GameSettings::default();
        let raw = raw_sticks(&pad, &settings);
        assert_eq!(raw.movement, Vec2::new(0.08, 0.05));
        assert_eq!(raw.look, Vec2::new(0.5, -0.25));
        assert_eq!(sticks(&pad, &settings).movement, Vec2::ZERO);
        settings.pad_stick_layout = 1;
        let raw = raw_sticks(&pad, &settings);
        assert_eq!(raw.movement, Vec2::new(-0.25, 0.5));
        assert_eq!(raw.look, Vec2::new(0.05, 0.08));
    }
}
