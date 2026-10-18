//! The mob steps the multiplayer scenarios share: a walled stone arena far
//! from the world's spawn, a mob summoned beside the client there, what each
//! game says of its mobs (`mc mobs`), and the client shooting a mob with its
//! gun until the host sees it die.
//!
//! Each scenario's own file says which of these it takes and what each
//! checkpoint must show.

use std::time::{Duration, Instant};

use crate::duo::Pair;
use crate::runner::Event;

/// How long a mob summoned through the host takes to reach the client's view.
pub const SEE_SECS: u64 = 30;
/// How long a mob has to hurt the client.
pub const HURT_SECS: u64 = 30;
/// How long the client has to shoot a mob dead.
pub const KILL_SECS: u64 = 60;
/// How far apart the host's and the client's copies of a mob may stand at
/// one moment, in blocks: the client draws a few ticks behind the host.
pub const TRACK_BLOCKS: f64 = 1.5;

/// One mob as a `minecraft-mobs:` line lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Mob {
    pub kind: String,
    pub id: u64,
    /// Feet, in blocks from the world's spawn block.
    pub at: [f64; 3],
    pub health: f32,
    pub dead: bool,
}

fn triple(text: &str) -> Option<[f64; 3]> {
    let v: Vec<f64> = text.split(',').filter_map(|v| v.parse().ok()).collect();
    (v.len() == 3).then(|| [v[0], v[1], v[2]])
}

/// The mobs a `minecraft-mobs:` line lists, nearest first.
pub fn mobs(event: &Event) -> Vec<Mob> {
    event
        .get("mobs")
        .unwrap_or("-")
        .split(';')
        .filter_map(|entry| {
            let parts: Vec<&str> = entry.split(':').collect();
            let [kind, id, at, health, state] = parts.as_slice() else {
                return None;
            };
            Some(Mob {
                kind: (*kind).to_owned(),
                id: id.parse().ok()?,
                at: triple(at)?,
                health: health.parse().ok()?,
                dead: *state == "dead",
            })
        })
        .collect()
}

/// The player's eye in a `minecraft-mobs:` line.
pub fn eye(event: &Event) -> Option<[f64; 3]> {
    triple(event.get("eye")?)
}

/// The hotbar slots (from 1) holding a bullet gun in a `minecraft-mobs:`
/// line.
pub fn guns(event: &Event) -> Vec<u32> {
    event
        .get("guns")
        .unwrap_or("-")
        .split(',')
        .filter_map(|s| s.parse().ok())
        .collect()
}

/// The player's MW2 health in a `minecraft-mobs:` line.
pub fn health(event: &Event) -> Option<f64> {
    event.get("health")?.parse().ok()
}

/// The items on the ground near the player in a `minecraft-mobs:` line.
pub fn items(event: &Event) -> Vec<String> {
    event
        .get("items")
        .unwrap_or("-")
        .split(',')
        .filter(|s| *s != "-" && !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The view (MW2 degrees: yaw from map +X, pitch down) from `eye` to
/// `target`, both in blocks: map X is block X, map Y is block -Z and map
/// up is block Y.
pub fn aim(eye: [f64; 3], target: [f64; 3]) -> (f64, f64) {
    let (dx, dy, dz) = (target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]);
    let yaw = (-dz).atan2(dx).to_degrees();
    let pitch = -dy.atan2(dx.hypot(dz)).to_degrees();
    (yaw, pitch)
}

/// Horizontal and vertical distance between two points, in blocks.
pub fn apart(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt()
}

/// `mc set` lines for a stone arena around `centre` (blocks from the
/// world's spawn block): a 9 x 9 floor one block under the centre's feet
/// level, and a wall two blocks high around it, so the mobs stay in.
pub fn arena(centre: [i32; 3]) -> String {
    let [x, y, z] = centre;
    let set = |a: [i32; 3], b: [i32; 3], block: &str| {
        format!(
            "mc set {} {} {} {} {} {} {block}",
            a[0], a[1], a[2], b[0], b[1], b[2]
        )
    };
    [
        set([x - 4, y, z - 4], [x + 4, y, z + 4], "stone"),
        set([x - 4, y + 1, z - 4], [x + 4, y + 3, z + 4], "air"),
        set([x - 5, y + 1, z - 5], [x + 5, y + 2, z - 5], "stone"),
        set([x - 5, y + 1, z + 5], [x + 5, y + 2, z + 5], "stone"),
        set([x - 5, y + 1, z - 4], [x - 5, y + 2, z + 4], "stone"),
        set([x + 5, y + 1, z - 4], [x + 5, y + 2, z + 4], "stone"),
    ]
    .join("; ")
}

/// Asks one game for its mobs (of `kind`, if given) under `label`.
pub fn census(
    pair: &mut Pair,
    client: bool,
    label: &str,
    kind: Option<&str>,
) -> Result<Event, String> {
    let script = match kind {
        Some(kind) => format!("mc mobs {label} {kind}"),
        None => format!("mc mobs {label}"),
    };
    pair.ask(client, &script, label, Duration::from_secs(30))
}

/// Whether `mob` is alive within the arena around the player of a census
/// (eight blocks of its eye): one summoned there, not one of the level's own
/// further off.
pub fn summoned(census: &Event, mob: &Mob) -> bool {
    !mob.dead && eye(census).is_some_and(|eye| apart(eye, mob.at) < 8.0)
}

/// Asks until a listed mob passes `want` or `timeout` passes: the last
/// answer, and the mob if one passed.
pub fn poll(
    pair: &mut Pair,
    client: bool,
    label: &str,
    kind: Option<&str>,
    timeout: Duration,
    want: impl Fn(&Event, &Mob) -> bool,
) -> Result<(Event, Option<Mob>), String> {
    let start = Instant::now();
    loop {
        let event = census(pair, client, label, kind)?;
        if let Some(mob) = mobs(&event).into_iter().find(|mob| want(&event, mob)) {
            return Ok((event, Some(mob)));
        }
        if start.elapsed() > timeout {
            return Ok((event, None));
        }
        let at = Instant::now();
        pair.until("the next census", Duration::from_secs(5), |_| {
            at.elapsed() >= Duration::from_millis(500)
        })?;
    }
}

/// Asks the client (for its mobs of `kind`) until its health is under
/// `below` or `timeout` passes; the last answer.
pub fn poll_hurt(
    pair: &mut Pair,
    label: &str,
    kind: &str,
    below: f64,
    timeout: Duration,
) -> Result<Event, String> {
    let start = Instant::now();
    loop {
        let event = census(pair, true, label, Some(kind))?;
        if health(&event).is_some_and(|h| h < below) || start.elapsed() > timeout {
            return Ok(event);
        }
        let at = Instant::now();
        pair.until("the next health check", Duration::from_secs(5), |_| {
            at.elapsed() >= Duration::from_millis(300)
        })?;
    }
}

/// Asks the client until the item `id` lies on the ground near it or is in
/// its inventory, or `timeout` passes: `ground`, `picked` or `none`.
pub fn poll_drop(pair: &mut Pair, id: &str, timeout: Duration) -> Result<String, String> {
    let start = Instant::now();
    let stack = format!("{id}*");
    loop {
        let event = census(pair, true, "c_drop", Some("-"))?;
        if items(&event).iter().any(|s| s.starts_with(&stack)) {
            return Ok("ground".into());
        }
        let state = pair.ask(
            true,
            "mc state c_drop_state",
            "c_drop_state",
            Duration::from_secs(30),
        )?;
        if state
            .get("slots")
            .unwrap_or("")
            .split(',')
            .any(|s| s.starts_with(&stack))
        {
            return Ok("picked".into());
        }
        if start.elapsed() > timeout {
            return Ok("none".into());
        }
        let at = Instant::now();
        pair.until("the next drop check", Duration::from_secs(5), |_| {
            at.elapsed() >= Duration::from_millis(500)
        })?;
    }
}

/// What the client's shooting came to: the host's last census of the mob,
/// whether the host saw it die (dead, or gone from its level), and the
/// bursts fired.
pub struct Shot {
    pub host: Event,
    pub killed: bool,
    pub bursts: u32,
}

/// The client raises a bullet gun (not the launcher or a grenade, which
/// would take it with the mob): the first hotbar slot that holds one.
pub fn draw_gun(pair: &mut Pair) -> Result<u32, String> {
    let held = census(pair, true, "c_guns", Some("-"))?;
    let slot = guns(&held)
        .first()
        .copied()
        .ok_or("the client holds no bullet gun")?;
    pair.client
        .as_mut()
        .ok_or("no client")?
        .send(&format!("key {slot}; wait 1.5s"))?;
    Ok(slot)
}

/// The client shoots the mob `id` with the gun it holds (`draw_gun`),
/// starting from `seen`, a census of its in which it has the mob: it aims at
/// the mob's middle from its eye as both stand on the client, fires a burst
/// and looks again, until it sees the mob dead or gone; then the host is
/// asked. Again until the host sees the mob die or `timeout` passes.
pub fn shoot(
    pair: &mut Pair,
    kind: &str,
    id: u64,
    middle: f64,
    seen: &Event,
    timeout: Duration,
) -> Result<Shot, String> {
    let start = Instant::now();
    let mut bursts = 0;
    let mut seen = seen.clone();
    loop {
        while let (Some(eye), Some(mob)) = (
            eye(&seen),
            mobs(&seen).into_iter().find(|m| m.id == id && !m.dead),
        ) {
            if start.elapsed() > timeout {
                break;
            }
            let (yaw, pitch) = aim(eye, [mob.at[0], mob.at[1] + middle, mob.at[2]]);
            let script = format!(
                "look {yaw:.2} {pitch:.2}; wait 0.1s; press +attack 0.6; wait 0.7s; mc mobs c_aim {kind}"
            );
            seen = pair.ask(true, &script, "c_aim", Duration::from_secs(30))?;
            bursts += 1;
        }
        let host = census(pair, false, "h_shot", Some(kind))?;
        let killed = mobs(&host)
            .into_iter()
            .find(|m| m.id == id)
            .is_none_or(|m| m.dead);
        if killed || start.elapsed() > timeout {
            return Ok(Shot {
                host,
                killed,
                bursts,
            });
        }
        seen = census(pair, true, "c_aim", Some(kind))?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(fields: &str) -> Event {
        crate::runner::parse(&format!("minecraft-mobs: {fields}"), "minecraft-mobs:", 0).unwrap()
    }

    #[test]
    fn a_census_line_reads_back() {
        let e = event(
            "label=c role=client health=85 eye=320.50,62.62,0.50 guns=2,5 count=2 \
             mobs=husk:41:323.20,61.00,0.40:20.0:alive;pig:7:-3.00,70.50,2.00:0.0:dead \
             items=minecraft:porkchop*2,minecraft:stone*1",
        );
        assert_eq!(
            mobs(&e),
            vec![
                Mob {
                    kind: "husk".into(),
                    id: 41,
                    at: [323.2, 61.0, 0.4],
                    health: 20.0,
                    dead: false
                },
                Mob {
                    kind: "pig".into(),
                    id: 7,
                    at: [-3.0, 70.5, 2.0],
                    health: 0.0,
                    dead: true
                },
            ]
        );
        assert_eq!(eye(&e), Some([320.5, 62.62, 0.5]));
        assert_eq!(health(&e), Some(85.0));
        assert_eq!(guns(&e), [2, 5]);
        assert_eq!(items(&e), ["minecraft:porkchop*2", "minecraft:stone*1"]);
        let none = event("label=c mobs=- items=-");
        assert!(mobs(&none).is_empty());
        assert!(items(&none).is_empty());
    }

    #[test]
    fn only_a_mob_in_the_arena_counts_as_summoned() {
        let e = event(
            "label=c eye=320.50,62.67,0.50 mobs=pig:101:304.24,4.00,69.78:10.0:alive;pig:151:322.50,61.00,2.50:10.0:alive;pig:9:321.00,61.00,0.00:0.0:dead",
        );
        let picked: Vec<u64> = mobs(&e)
            .iter()
            .filter(|m| summoned(&e, m))
            .map(|m| m.id)
            .collect();
        assert_eq!(picked, [151]);
    }

    #[test]
    fn aim_turns_to_the_target_as_mw2_measures() {
        // East (+X) and level.
        let (yaw, pitch) = aim([0.0, 1.0, 0.0], [5.0, 1.0, 0.0]);
        assert!(yaw.abs() < 1e-9 && pitch.abs() < 1e-9);
        // Block -Z is map +Y: a quarter turn left.
        let (yaw, _) = aim([0.0, 1.0, 0.0], [0.0, 1.0, -5.0]);
        assert!((yaw - 90.0).abs() < 1e-9);
        // Below is a positive pitch.
        let (_, pitch) = aim([0.0, 2.0, 0.0], [1.0, 1.0, 0.0]);
        assert!((pitch - 45.0).abs() < 1e-9);
    }

    #[test]
    fn the_arena_keeps_to_the_console_box_limit() {
        let lines = arena([320, 60, 0]);
        assert_eq!(lines.matches("mc set").count(), 6);
        assert!(lines.starts_with("mc set 316 60 -4 324 60 4 stone"));
        // `mc set` takes at most 4096 blocks a box.
        for line in lines.split("; ") {
            let v: Vec<i32> = line
                .split(' ')
                .skip(2)
                .take(6)
                .map(|v| v.parse().unwrap())
                .collect();
            let blocks = (0..3).map(|i| (v[i] - v[i + 3]).abs() + 1).product::<i32>();
            assert!(blocks <= 4096, "{line}");
        }
    }
}
