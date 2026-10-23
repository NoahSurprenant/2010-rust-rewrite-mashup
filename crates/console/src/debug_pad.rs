//! `pad`: a controller the console drives, for tests. It is plugged in the
//! first time it is used, as a real one is, and its buttons and sticks go
//! through Bevy's gamepad input like a real controller's: the same
//! filtering, the same `Gamepad` the game reads, the same "last controller
//! touched" rule that makes it the active one.

use bevy::input::gamepad::{
    GamepadAxis, GamepadConnection, GamepadConnectionEvent, RawGamepadAxisChangedEvent,
    RawGamepadButtonChangedEvent, RawGamepadEvent,
};
use bevy::prelude::*;

use crate::binds::PadButton;
use crate::{ConsoleCommand, ConsoleRegistry, ConsoleSettings, ConsoleState};

const PAD_USAGE: &str = "pad <button> [seconds] | pad stick <left|right> <x> <y> (held until set again; 0 0 lets go) — \
     a controller the console drives (buttons: A B X Y LB RB LT RT LS RS D-UP D-DOWN D-LEFT D-RIGHT BACK START, or their BUTTON_ names)";

pub(crate) fn register_debug_pad_commands(registry: &mut ConsoleRegistry) {
    if registry.resolve("pad").is_none() {
        registry.register(crate::CommandSpec::new("pad").usage(PAD_USAGE));
    }
}

/// What one `pad` command asks of the controller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PadRequest {
    /// Hold a button down this long.
    Press(PadButton, f32),
    /// Lean a stick (left or right) to this point and leave it there.
    Stick { left: bool, at: Vec2 },
}

fn parse_button(name: &str) -> Option<PadButton> {
    PadButton::ALL.into_iter().find(|button| {
        button.label().eq_ignore_ascii_case(name) || button.console_name().eq_ignore_ascii_case(name)
    })
}

pub(crate) fn parse(args: &[String]) -> Option<PadRequest> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["stick", side, x, y] => {
            let left = match side.to_ascii_lowercase().as_str() {
                "left" => true,
                "right" => false,
                _ => return None,
            };
            let at = Vec2::new(x.parse().ok()?, y.parse().ok()?);
            (at.is_finite() && at.x.abs() <= 1.0 && at.y.abs() <= 1.0)
                .then_some(PadRequest::Stick { left, at })
        }
        [button] => Some(PadRequest::Press(parse_button(button)?, crate::PRESS_SECONDS)),
        [button, seconds] => {
            let seconds = seconds.trim_end_matches(['s', 'S']).parse::<f32>().ok()?;
            let button = parse_button(button)?;
            seconds.is_finite().then_some(PadRequest::Press(button, seconds))
        }
        _ => None,
    }
}

/// The console's controller: its entity once plugged in, the requests
/// waiting for it to be, and the buttons it holds with the seconds left.
#[derive(Default)]
pub(crate) struct ConsolePad {
    entity: Option<Entity>,
    waiting: Vec<PadRequest>,
    held: Vec<(PadButton, f32)>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn route_debug_pad_commands(
    mut commands: Commands,
    mut events: MessageReader<ConsoleCommand>,
    mut console: ResMut<ConsoleState>,
    settings: Res<ConsoleSettings>,
    time: Res<Time>,
    gamepads: Query<(), With<Gamepad>>,
    mut pad: Local<ConsolePad>,
    (mut raw, mut connections, mut buttons, mut axes): (
        MessageWriter<RawGamepadEvent>,
        MessageWriter<GamepadConnectionEvent>,
        MessageWriter<RawGamepadButtonChangedEvent>,
        MessageWriter<RawGamepadAxisChangedEvent>,
    ),
) {
    let capacity = settings.log_capacity;
    for cmd in events.read() {
        if cmd.name != "pad" {
            continue;
        }
        match parse(&cmd.args) {
            Some(request) => {
                console.echo(format!("pad: {}", cmd.args.join(" ")), capacity);
                pad.waiting.push(request);
            }
            None => console.echo(format!("usage: {PAD_USAGE}"), capacity),
        }
    }
    let entity = match pad.entity {
        Some(entity) => entity,
        None if pad.waiting.is_empty() => return,
        None => {
            // Plugged in as gilrs plugs a controller in: the connection on
            // both streams, the Gamepad on the entity the frame after.
            let entity = commands.spawn_empty().id();
            let connection = GamepadConnectionEvent::new(
                entity,
                GamepadConnection::Connected {
                    name: "iw4l console pad".to_owned(),
                    vendor_id: Some(0x045e),
                    product_id: None,
                },
            );
            raw.write(connection.clone().into());
            connections.write(connection);
            pad.entity = Some(entity);
            return;
        }
    };
    if !gamepads.contains(entity) {
        return;
    }
    let button = |button: PadButton, value: f32| {
        RawGamepadEvent::Button(RawGamepadButtonChangedEvent::new(
            entity,
            button.gamepad_button(),
            value,
        ))
    };
    let mut changes = Vec::new();
    let dt = time.delta_secs();
    pad.held.retain_mut(|(held, left)| {
        *left -= dt;
        if *left <= 0.0 {
            changes.push(button(*held, 0.0));
            return false;
        }
        true
    });
    for request in std::mem::take(&mut pad.waiting) {
        match request {
            PadRequest::Press(pressed, seconds) => {
                changes.push(button(pressed, 1.0));
                pad.held.retain(|(held, _)| *held != pressed);
                pad.held.push((pressed, seconds.clamp(0.0, 60.0)));
            }
            PadRequest::Stick { left, at } => {
                let (x, y) = if left {
                    (GamepadAxis::LeftStickX, GamepadAxis::LeftStickY)
                } else {
                    (GamepadAxis::RightStickX, GamepadAxis::RightStickY)
                };
                for (axis, value) in [(x, at.x), (y, at.y)] {
                    changes.push(RawGamepadEvent::Axis(RawGamepadAxisChangedEvent::new(
                        entity, axis, value,
                    )));
                }
            }
        }
    }
    for change in changes {
        match change {
            RawGamepadEvent::Button(event) => {
                buttons.write(event);
            }
            RawGamepadEvent::Axis(event) => {
                axes.write(event);
            }
            RawGamepadEvent::Connection(_) => {}
        }
        raw.write(change);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn buttons_are_named_by_label_or_console_name() {
        assert_eq!(
            parse(&args("start")),
            Some(PadRequest::Press(PadButton::Start, crate::PRESS_SECONDS))
        );
        assert_eq!(
            parse(&args("BUTTON_B 0.5")),
            Some(PadRequest::Press(PadButton::East, 0.5))
        );
        assert_eq!(
            parse(&args("d-up 2s")),
            Some(PadRequest::Press(PadButton::DpadUp, 2.0))
        );
        assert_eq!(parse(&args("nope")), None);
        assert_eq!(parse(&args("nope 1")), None);
        assert_eq!(parse(&args("a soon")), None);
    }

    #[test]
    fn sticks_lean_within_the_unit_square() {
        assert_eq!(
            parse(&args("stick left 0.1 -1")),
            Some(PadRequest::Stick {
                left: true,
                at: Vec2::new(0.1, -1.0)
            })
        );
        assert_eq!(
            parse(&args("stick RIGHT 0 0")),
            Some(PadRequest::Stick {
                left: false,
                at: Vec2::ZERO
            })
        );
        assert_eq!(parse(&args("stick left 2 0")), None);
        assert_eq!(parse(&args("stick up 0 0")), None);
        assert_eq!(parse(&args("stick left 0")), None);
    }
}
