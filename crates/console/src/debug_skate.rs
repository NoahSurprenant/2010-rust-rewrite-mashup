//! `skate`: get on and off the board, a controller the console holds, and
//! the line the approved tests read (`skate-state:`).
use bevy::prelude::*;
use net::{LocalPresentClient, PresentedSnapshot};

use crate::{ConsoleCommand, ConsoleLine, ConsoleSettings, ConsoleState};

pub(crate) const USAGE: &str = "skate [on|off|status] | skate pad push|stop|<buttons> <lx> <ly> | skate state <label> - Skate gameplay (J toggles)";

#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct SkateNet<'w> {
    outbox: Res<'w, net::SkateOutbox>,
    relay: Option<Res<'w, net::SkaterRelay>>,
    remote: Res<'w, frame::RemoteSkaters>,
    downlink: Res<'w, net::SkaterDownlink>,
    authority: Option<Res<'w, net::AuthorityWorld>>,
    role: Res<'w, frame::RuntimeRole>,
    dedicated: Option<Res<'w, frame::DedicatedServer>>,
}

pub(crate) fn route_skate_commands(
    mut skate: ResMut<frame::SkateMode>,
    mut events: MessageReader<ConsoleCommand>,
    mut console: ResMut<ConsoleState>,
    settings: Res<ConsoleSettings>,
    mut line: ResMut<ConsoleLine>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    net: SkateNet,
) {
    let capacity = settings.log_capacity;
    let mut echo = |msg: String| {
        diag::info!(Console, "{msg}");
        line.0 = msg.clone();
        console.echo(msg, capacity);
    };
    for cmd in events.read() {
        if cmd.name != "skate" {
            continue;
        }
        let args: Vec<&str> = cmd.args.iter().map(String::as_str).collect();
        match args.as_slice() {
            [] => skate.toggle_requested = true,
            ["status"] => {}
            ["on"] => skate.toggle_requested = !skate.active && !skate.entering,
            ["off"] => skate.toggle_requested = skate.active || skate.entering,
            ["pad", rest @ ..] => match pad(rest) {
                Ok(pad) => {
                    skate.virtual_pad = pad;
                    echo(format!("skate pad {:?}", skate.virtual_pad));
                    continue;
                }
                Err(e) => {
                    echo(e);
                    continue;
                }
            },
            ["state", label] => {
                let text = state_line(label, &skate, &presented, local.0, &net);
                println!("{text}");
                echo(text);
                continue;
            }
            _ => {
                echo(format!("usage: {USAGE}"));
                continue;
            }
        }
        echo(format!(
            "skate active={} ready={} controller={:?} tick={} {}",
            skate.active, skate.preloaded, skate.controller, skate.tick, skate.status
        ));
    }
}

/// `push`: hold A, which pushes off and keeps pushing; `stop`: let go of
/// the console's controller; or raw button bits and the left stick.
fn pad(args: &[&str]) -> Result<Option<frame::VirtualPad>, String> {
    const A: u16 = 0x1000;
    match args {
        ["push"] => Ok(Some(frame::VirtualPad {
            buttons: A,
            ..Default::default()
        })),
        ["idle"] => Ok(Some(frame::VirtualPad::default())),
        ["stop"] => Ok(None),
        [buttons, lx, ly] => {
            let buttons = u16::from_str_radix(buttons.trim_start_matches("0x"), 16)
                .map_err(|_| format!("bad buttons {buttons}"))?;
            let axis = |v: &str| v.parse::<i16>().map_err(|_| format!("bad axis {v}"));
            Ok(Some(frame::VirtualPad {
                buttons,
                left: [axis(lx)?, axis(ly)?],
                ..Default::default()
            }))
        }
        _ => Err("usage: skate pad push|idle|stop|<hex buttons> <lx> <ly>".into()),
    }
}

fn word(text: &str) -> String {
    if text.is_empty() {
        "-".into()
    } else {
        text.replace(char::is_whitespace, "_")
    }
}

fn xyz(v: impl Into<[f32; 3]>) -> String {
    let [x, y, z] = v.into();
    format!("{x:.0},{y:.0},{z:.0}")
}

/// One line: this game's own skater, the skaters it draws, and (on a host)
/// every player's skating as the host has it.
fn state_line(
    label: &str,
    skate: &frame::SkateMode,
    presented: &PresentedSnapshot,
    local: sim::ClientId,
    net: &SkateNet,
) -> String {
    // A dedicated server runs the listen runtime with no player of its own.
    let dedicated = net.dedicated.is_some() || *net.role == frame::RuntimeRole::Dedicated;
    let role = match *net.role {
        _ if dedicated => "dedicated",
        frame::RuntimeRole::Client => "client",
        _ => "listen",
    };
    let origin = presented
        .player(local)
        .map_or_else(|| "-".to_owned(), |ps| xyz(ps.origin));
    let skater = if skate.active {
        xyz(skate.root.w_axis.truncate())
    } else {
        "-".into()
    };
    let mut remote: Vec<_> = net.remote.poses.iter().collect();
    remote.sort_by_key(|(id, _)| **id);
    let remote = remote
        .iter()
        .map(|(id, pose)| {
            format!(
                "{id}:{}:{}:{}:{}:{}",
                xyz(pose.root.w_axis.truncate()),
                pose.bones.len(),
                pose.received,
                net.remote.drawn(**id),
                word(&pose.label)
            )
        })
        .collect::<Vec<_>>()
        .join("|");
    let mut peers = Vec::new();
    if let (Some(world), Some(relay)) = (net.authority.as_deref(), net.relay.as_deref()) {
        world.0.visit_players(|id, ps| {
            if dedicated && id.0 == 0 {
                return;
            }
            let skater = relay.skaters.get(&id.0);
            peers.push(format!(
                "{}:{}:{}:{}:{}:{}:{}",
                id.0,
                xyz(ps.origin),
                u8::from(skater.is_some_and(|s| s.gate.active)),
                skater.map_or(0, |s| s.gate.accepted),
                skater.map_or(0, |s| s.gate.rejected),
                skater.map_or(0, |s| s.bytes_in),
                skater.map_or(0, |s| s.bytes_out),
            ));
        });
    }
    format!(
        "skate-state: label={label} role={role} client={} active={} ready={} origin={origin} skater={skater} speed={:.0} sim={} sent={} sent_bytes={} sent_bytes_per_s={} pose_err_mm={:.2} received_bytes={} relayed_bytes={} remote={} peers={}",
        local.0,
        u8::from(skate.active),
        u8::from(skate.preloaded),
        skate.speed,
        word(&skate.status),
        net.outbox.sent,
        net.outbox.sent_bytes,
        net.outbox.bytes_per_s,
        net.outbox.worst_error_mm,
        net.downlink.bytes,
        net.relay.as_ref().map_or(0, |r| r.frame_bytes),
        if remote.is_empty() {
            "-".into()
        } else {
            remote
        },
        if peers.is_empty() {
            "-".into()
        } else {
            peers.join("|")
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_console_controller_parses() {
        assert_eq!(pad(&["push"]).unwrap().unwrap().buttons, 0x1000);
        assert_eq!(pad(&["stop"]).unwrap(), None);
        let raw = pad(&["0x1000", "0", "-32767"]).unwrap().unwrap();
        assert_eq!(raw.left, [0, -32767]);
        assert!(pad(&["nope"]).is_err());
        assert!(pad(&["1000", "x", "0"]).is_err());
    }

    #[test]
    fn words_hold_no_spaces() {
        assert_eq!(word("Skate rig.json missing"), "Skate_rig.json_missing");
        assert_eq!(word(""), "-");
    }
}
