use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::MouseButtonInput;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use frame::{McClick, McSlot, McStack, MinecraftUi};

use crate::binds::{BindButton, parse_key_name};
use crate::{ConsoleCommand, ConsoleRegistry, ConsoleSettings, ConsoleState};

const MC_USAGE: &str = "mc give <item> [count] | mc clear | mc click <inv N|grid N|result> [right] [shift] | mc fill <x1 y1 z1 x2 y2 z2> <block|air> (from the feet) | mc set <x1 y1 z1 x2 y2 z2> <block|air> (from the world's spawn block) | mc block <label> <x y z> (from the spawn block) | mc state <label> | mc summon <mob> [dx dy dz] (from the feet) | mc mobs <label> [mob] (the mobs near the player) | mc sounds <label> (the world sounds heard) | mc mobrange <label> (a host's spawns and despawns around a player) | mc spawning <on|off> (a host's natural spawning) | mc world [name] (the world a host keeps) — the Minecraft map";
const FILL_MAX: i32 = 4096;
const KEY_USAGE: &str =
    "key <key|mouse1|mouse2> [seconds] — press a key or mouse button as the device does";

pub(crate) fn register_debug_minecraft_commands(registry: &mut ConsoleRegistry) {
    if registry.resolve("mc").is_none() {
        registry.register(crate::CommandSpec::new("mc").usage(MC_USAGE));
    }
    if registry.resolve("key").is_none() {
        registry.register(crate::CommandSpec::new("key").usage(KEY_USAGE));
    }
}

/// Whether `mc <verb>` is taken now. A world in play with its player alive
/// (or a dedicated server's, which has none) takes every verb. Between the
/// player's lives (dead, waiting to spawn again) the world is still there:
/// a question about it (`state`, `block`, `mobs`, `sounds`, `mobrange`) is
/// taken and answered, at once or once the player lives again, and only
/// what the player would do waits for it to.
fn accepts(verb: Option<&str>, active: bool, dedicated: bool, world: bool) -> bool {
    if active || (dedicated && world) {
        return true;
    }
    world
        && matches!(
            verb,
            Some("state" | "block" | "mobs" | "sounds" | "mobrange")
        )
}

/// Keys and mouse buttons `key` holds down, and the seconds left on each.
#[derive(Default)]
pub(crate) struct HeldDeviceButtons(Vec<(BindButton, f32)>);

#[allow(clippy::too_many_arguments)]
pub(crate) fn route_debug_minecraft_commands(
    mut events: MessageReader<ConsoleCommand>,
    mut console: ResMut<ConsoleState>,
    settings: Res<ConsoleSettings>,
    ui: Option<ResMut<MinecraftUi>>,
    time: Res<Time>,
    mut held: Local<HeldDeviceButtons>,
    window: Query<Entity, With<PrimaryWindow>>,
    (mut keys, mut mouse): (
        MessageWriter<KeyboardInput>,
        MessageWriter<MouseButtonInput>,
    ),
    (presented, local, menus): (
        Res<net::PresentedSnapshot>,
        Res<net::LocalPresentClient>,
        Option<Res<hud::ScriptMenus>>,
    ),
) {
    let capacity = settings.log_capacity;
    let mut ui = ui;
    // A headless game has no window: its input still reaches the game,
    // which reads keys and buttons whatever window they came from.
    let window = window.single().unwrap_or(Entity::PLACEHOLDER);
    let mut send = |button: BindButton, state: ButtonState| {
        match button {
            BindButton::Key(key_code) => {
                keys.write(KeyboardInput {
                    key_code,
                    logical_key: Key::Unidentified(bevy::input::keyboard::NativeKey::Unidentified),
                    state,
                    text: None,
                    repeat: false,
                    window,
                });
            }
            BindButton::Mouse(button) => {
                mouse.write(MouseButtonInput {
                    button,
                    state,
                    window,
                });
            }
            _ => {}
        }
    };

    let dt = time.delta_secs();
    held.0.retain_mut(|(button, left)| {
        *left -= dt;
        if *left <= 0.0 {
            send(*button, ButtonState::Released);
            return false;
        }
        true
    });

    for cmd in events.read() {
        match cmd.name.as_str() {
            "key" => {
                let button = cmd
                    .args
                    .first()
                    .and_then(|name| parse_key_name(name))
                    .and_then(|buttons| {
                        buttons
                            .into_iter()
                            .find(|b| matches!(b, BindButton::Key(_) | BindButton::Mouse(_)))
                    });
                let seconds = match cmd.args.get(1) {
                    Some(raw) => raw.trim_end_matches(['s', 'S']).parse::<f32>().ok(),
                    None => Some(crate::PRESS_SECONDS),
                };
                match (button, seconds, cmd.args.len()) {
                    (Some(button), Some(seconds), 1..=2) => {
                        send(button, ButtonState::Pressed);
                        held.0.retain(|(b, _)| *b != button);
                        held.0.push((button, seconds.clamp(0.0, 60.0)));
                        console.echo(format!("key: {} ({seconds:.2}s)", cmd.args[0]), capacity);
                    }
                    _ => console.echo(format!("usage: {KEY_USAGE}"), capacity),
                }
            }
            "mc" if cmd.args.first().is_some_and(|verb| verb == "world") => {
                // The world a host keeps, for the next Minecraft match it hosts.
                let Some(ui) = ui.as_mut() else {
                    continue;
                };
                match cmd.args.get(1..).unwrap_or_default() {
                    [] => {}
                    [name] => ui.world_name = Some(name.clone()),
                    _ => {
                        console.echo(format!("usage: {MC_USAGE}"), capacity);
                        continue;
                    }
                }
                let name = ui
                    .world_name
                    .clone()
                    .or_else(|| std::env::var("IW4L_MINECRAFT_WORLD").ok());
                console.echo(
                    format!("mc: kept world `{}`", name.as_deref().unwrap_or("world")),
                    capacity,
                );
            }
            "mc" => {
                let verb = cmd.args.first().map(String::as_str);
                let Some(ui) = ui.as_mut().filter(|ui| {
                    accepts(
                        verb,
                        ui.active,
                        ui.net_role == "dedicated",
                        ui.seed.is_some(),
                    )
                }) else {
                    console.echo("mc: no Minecraft world in play", capacity);
                    continue;
                };
                let args: Vec<&str> = cmd.args.iter().map(String::as_str).collect();
                match args.as_slice() {
                    ["give", item, rest @ ..] if rest.len() <= 1 => {
                        let count = rest
                            .first()
                            .map_or(Some(1), |n| n.parse::<u8>().ok().filter(|&n| n > 0));
                        match count {
                            Some(count) => {
                                let id = if item.contains(':') {
                                    (*item).to_owned()
                                } else {
                                    format!("minecraft:{item}")
                                };
                                console.echo(format!("mc: give {id} {count}"), capacity);
                                ui.give.push((id, count));
                            }
                            None => console.echo(format!("usage: {MC_USAGE}"), capacity),
                        }
                    }
                    ["clear"] => ui.clear_items = true,
                    [verb @ ("fill" | "set"), x1, y1, z1, x2, y2, z2, block] => {
                        let corners: Option<Vec<i32>> = [x1, y1, z1, x2, y2, z2]
                            .iter()
                            .map(|v| v.parse().ok())
                            .collect();
                        match corners {
                            Some(c)
                                if ((c[0] - c[3]).abs() + 1)
                                    * ((c[1] - c[4]).abs() + 1)
                                    * ((c[2] - c[5]).abs() + 1)
                                    <= FILL_MAX =>
                            {
                                let id = match *block {
                                    "air" | "minecraft:air" => None,
                                    id if id.contains(':') => Some(id.to_owned()),
                                    id => Some(format!("minecraft:{id}")),
                                };
                                let corners = ([c[0], c[1], c[2]], [c[3], c[4], c[5]], id);
                                if *verb == "set" {
                                    ui.set.push(corners);
                                } else {
                                    ui.fill.push(corners);
                                }
                            }
                            _ => console.echo(
                                format!("usage: {MC_USAGE} (at most {FILL_MAX} blocks)"),
                                capacity,
                            ),
                        }
                    }
                    ["block", label, x, y, z] => match (x.parse(), y.parse(), z.parse()) {
                        (Ok(x), Ok(y), Ok(z)) => ui.probes.push(((*label).to_owned(), [x, y, z])),
                        _ => console.echo(format!("usage: {MC_USAGE}"), capacity),
                    },
                    ["summon", kind, rest @ ..] if rest.is_empty() || rest.len() == 3 => {
                        let offset: Option<Vec<f64>> = rest
                            .iter()
                            .map(|v| v.parse::<f64>().ok().filter(|v| v.is_finite() && v.abs() <= 64.0))
                            .collect();
                        match offset {
                            Some(o) => {
                                let at = if o.is_empty() { [2.0, 0.0, 0.0] } else { [o[0], o[1], o[2]] };
                                console.echo(format!("mc: summon {kind}"), capacity);
                                ui.summon.push(((*kind).to_owned(), at));
                            }
                            None => console.echo(format!("usage: {MC_USAGE} (offsets within 64 blocks)"), capacity),
                        }
                    }
                    ["mobs", label, kind @ ..] if kind.len() <= 1 => {
                        ui.mob_probes.push(((*label).to_owned(), kind.first().map(|k| (*k).to_owned())));
                    }
                    ["sounds", label] => ui.sound_probes.push((*label).to_owned()),
                    ["mobrange", label] => ui.range_probes.push((*label).to_owned()),
                    ["spawning", state @ ("on" | "off")] => {
                        ui.natural_spawning = Some(*state == "on");
                        console.echo(format!("mc: natural spawning {state}"), capacity);
                    }
                    ["click", rest @ ..] => match parse_click(rest) {
                        Some(click) => ui.clicks.push(click),
                        None => console.echo(format!("usage: {MC_USAGE}"), capacity),
                    },
                    ["state", label] => {
                        let player = presented.player(local.0);
                        let flags = player.map_or(0, |ps| ps.pm_flags);
                        let origin = player.map_or([0.0; 3], |ps| ps.origin);
                        let angles = player.map_or([0.0; 3], |ps| ps.viewangles);
                        let menu = menus.as_ref().is_some_and(|m| m.captures_input());
                        // MW2's action slots as the scripts set them (0 none,
                        // 1 a weapon, 2 the attachment, 3 night vision): the
                        // D-pad they sit on.
                        let slots = player.map_or_else(
                            || "-".to_owned(),
                            |ps| ps.action_slot_type.map(|kind| kind.to_string()).join(","),
                        );
                        let line = format!(
                            "{} pad={} action_slots={slots}",
                            state_line(label, ui, flags, menu, origin, angles),
                            pad_slot_text(ui.pad_slot),
                        );
                        diag::info!(Console, "{line}");
                        println!("{line}");
                        console.echo(line, capacity);
                    }
                    _ => console.echo(format!("usage: {MC_USAGE}"), capacity),
                }
            }
            _ => {}
        }
    }
}

fn parse_click(args: &[&str]) -> Option<McClick> {
    let (slot, flags) = match args {
        ["inv", n, flags @ ..] => (McSlot::Inventory(n.parse().ok()?), flags),
        ["grid", n, flags @ ..] => (McSlot::Crafting(n.parse().ok()?), flags),
        ["result", flags @ ..] => (McSlot::Result, flags),
        _ => return None,
    };
    let mut right = false;
    let mut shift = false;
    for flag in flags {
        match *flag {
            "right" => right = true,
            "shift" => shift = true,
            _ => return None,
        }
    }
    Some(McClick::Slot { slot, right, shift })
}

/// The controller's cursor on the open screen, as `mc click` names slots.
fn pad_slot_text(slot: Option<McSlot>) -> String {
    match slot {
        None => "-".to_owned(),
        Some(McSlot::Inventory(n)) => format!("inv{n}"),
        Some(McSlot::Crafting(n)) => format!("grid{n}"),
        Some(McSlot::Result) => "result".to_owned(),
    }
}

fn stack_text(stack: &Option<McStack>) -> String {
    stack
        .as_ref()
        .map_or_else(|| "-".to_owned(), |s| format!("{}*{}", s.id, s.count))
}

/// One line the approved tests read: the open screen, the grid, the result,
/// the cursor and every inventory slot.
fn state_line(label: &str, ui: &MinecraftUi, pm_flags: u32, menu: bool, origin: [f32; 3], angles: [f32; 3]) -> String {
    let in_view = ui.table_in_view.map_or_else(|| "-".to_owned(), |(x, y, z)| format!("{x},{y},{z}"));
    let stance = if pm_flags & playerstate_iw4::pm_flags::PRONE != 0 {
        "prone"
    } else if pm_flags & playerstate_iw4::pm_flags::CROUCH != 0 {
        "crouch"
    } else {
        "stand"
    };
    let screen = match (ui.inventory_open, ui.workbench) {
        (false, _) => "none".to_owned(),
        (true, None) => "inventory".to_owned(),
        (true, Some((x, y, z))) => format!("table@{x},{y},{z}"),
    };
    let list =
        |stacks: &[Option<McStack>]| stacks.iter().map(stack_text).collect::<Vec<_>>().join(",");
    format!(
        "minecraft-state: label={label} stance={stance} menu={menu} origin={:.0},{:.0},{:.0} view={:.0},{:.0} table_in_view={in_view} screen={screen} grid={} result={} cursor={} slots={} seed={} role={} feet={} no_icon={} hand={} ground={} no_model={} map={}",
        origin[0],
        origin[1],
        origin[2],
        angles[1],
        angles[0],
        list(&ui.crafting),
        stack_text(&ui.result),
        stack_text(&ui.cursor),
        list(&ui.slots),
        ui.seed
            .map_or_else(|| "-".to_owned(), |seed| seed.to_string()),
        ui.net_role,
        ui.feet
            .map_or_else(|| "-".to_owned(), |[x, y, z]| format!("{x},{y},{z}")),
        no_icon(ui),
        ui.hand
            .as_ref()
            .map_or_else(|| "-".to_owned(), |(id, faces)| format!("{id}:{faces}")),
        id_list(&ui.ground_items),
        id_list(&ui.no_model),
        ui.held_map.as_deref().unwrap_or("-"),
    )
}

/// Item ids, comma-separated, or `-`.
fn id_list(ids: &[String]) -> String {
    if ids.is_empty() {
        "-".to_owned()
    } else {
        ids.join(",")
    }
}

/// The Minecraft items on the screen (slots, grid, result, cursor) the
/// inventory has no icon for, comma-separated, or `-`: guns draw their own.
fn no_icon(ui: &MinecraftUi) -> String {
    let mut missing: Vec<&str> = Vec::new();
    let shown = ui
        .slots
        .iter()
        .chain(&ui.crafting)
        .chain([&ui.result, &ui.cursor]);
    for stack in shown.flatten() {
        let id = stack.id.as_str();
        if stack.weapon.is_none() && !ui.icon_rects.contains_key(id) && !missing.contains(&id) {
            missing.push(id);
        }
    }
    if missing.is_empty() {
        "-".to_owned()
    } else {
        missing.join(",")
    }
}

#[cfg(test)]
mod accepts_tests {
    use super::accepts;

    #[test]
    fn a_living_player_or_a_dedicated_world_takes_every_verb() {
        for verb in ["give", "set", "summon", "state", "mobs", "click"] {
            assert!(accepts(Some(verb), true, false, true), "{verb}");
            assert!(accepts(Some(verb), false, true, true), "{verb}");
        }
    }

    /// Run 39688016120: a client shot dead by a witch was asked `mc mobs`
    /// before it spawned again, was refused, and its step waited forever.
    #[test]
    fn between_lives_a_question_is_taken_and_an_action_is_not() {
        for verb in ["state", "block", "mobs", "sounds", "mobrange"] {
            assert!(accepts(Some(verb), false, false, true), "{verb}");
        }
        for verb in ["give", "clear", "set", "fill", "summon", "click", "spawning"] {
            assert!(!accepts(Some(verb), false, false, true), "{verb}");
        }
    }

    #[test]
    fn without_a_world_nothing_is_taken() {
        for verb in [Some("state"), Some("mobs"), Some("give"), None] {
            assert!(!accepts(verb, false, false, false), "{verb:?}");
            assert!(!accepts(verb, false, true, false), "{verb:?}");
        }
    }
}
