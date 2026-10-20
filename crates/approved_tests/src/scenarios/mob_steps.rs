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
pub const SEE_SECS: u64 = 60;
/// How long a mob has to hurt the client.
pub const HURT_SECS: u64 = 60;
/// How long the client has to shoot a mob dead.
pub const KILL_SECS: u64 = 120;
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

/// The hotbar slot (from 1) of the weapon the player holds in a
/// `minecraft-mobs:` line.
pub fn held(event: &Event) -> Option<u32> {
    event.get("held")?.parse().ok()
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

/// Summons `kind` through the host at `offset` (blocks from the client's
/// feet, `"dx dy dz"`) and asks the client until it sees it there
/// (`summoned`), or `see` passes. Once: a summon the client never sees is a
/// failure, not a retry.
pub fn summon_seen(
    pair: &mut Pair,
    kind: &str,
    offset: &str,
    label: &str,
    see: Duration,
) -> Result<(Event, Option<Mob>), String> {
    pair.client
        .as_mut()
        .ok_or("no client")?
        .send(&format!("mc summon {kind} {offset}"))?;
    poll(pair, true, label, Some(kind), see, summoned)
}

/// Asks the host until the arena around `centre` has its floor at its four
/// corners (the arena crosses chunk borders) and under the spot a husk is
/// summoned on. A host that generates the chunks there late sets the arena
/// late, and a mob summoned before falls through to the ground below (runs
/// 39235178267's husk and 39240931444's witch, on a dedicated server).
pub fn arena_floor(pair: &mut Pair, centre: [i32; 3], timeout: Duration) -> Result<(), String> {
    let [x, y, z] = centre;
    let stone = "minecraft:stone";
    for [dx, dz] in [[3, 0], [-4, -4], [4, -4], [-4, 4], [4, 4]] {
        let at = [x + dx, y, z + dz];
        let block = pair.poll_block(false, "h_arena", at, stone, timeout)?;
        if block != stone {
            return Err(format!("the host never had the arena's floor at {at:?}: {block}"));
        }
    }
    Ok(())
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
    pair.ask(client, &script, label, Duration::from_secs(60))
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
            Duration::from_secs(60),
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
/// would take it with the mob): the first hotbar slot that holds one. Its
/// key is pressed again only while the census says another slot is held:
/// once, on 2 CPUs, a press did not take and the client fired its launcher
/// at the husk beside it (run 39272980932 attempt 3), and a second press
/// made blindly, after the first had taken, put the launcher back in its
/// hands (run 39340605035).
pub fn draw_gun(pair: &mut Pair) -> Result<u32, String> {
    let mut seen = census(pair, true, "c_guns", Some("-"))?;
    let slot = guns(&seen)
        .first()
        .copied()
        .ok_or("the client holds no bullet gun")?;
    for _ in 0..DRAW_TRIES {
        if held(&seen) == Some(slot) {
            return Ok(slot);
        }
        pair.client
            .as_mut()
            .ok_or("no client")?
            .send(&format!("key {slot}; wait 2s"))?;
        seen = census(pair, true, "c_guns", Some("-"))?;
    }
    if held(&seen) == Some(slot) {
        return Ok(slot);
    }
    Err(format!(
        "the client never raised the gun in slot {slot} (holds slot {})",
        seen.get("held").unwrap_or("-")
    ))
}

/// How many times the client presses a gun's key before `draw_gun` gives up.
pub const DRAW_TRIES: u32 = 4;

/// The client shoots the mob `id` with the gun it holds (`draw_gun`),
/// starting from `seen`, a census of its in which it has the mob: it aims at
/// the mob's middle from its eye as both stand on the client, fires a burst
/// and looks again, until it sees the mob dead or gone; then the host is
/// asked. Again until the host sees the mob die or `timeout` passes. A
/// burst is 1.5 s: a game that draws 3 to 4 frames a second fires only a
/// few shots in it, and between two bursts (a census away) a husk beside
/// the client strikes; with 0.6 s bursts one killed the client before it
/// killed the husk (run 39228628595, dedicated_server mobs).
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
                "look {yaw:.2} {pitch:.2}; wait 0.1s; press +attack 1.5; wait 1.6s; mc mobs c_aim {kind}"
            );
            seen = pair.ask(true, &script, "c_aim", Duration::from_secs(60))?;
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

/// How long a mob has to come within a knife's reach of the client.
pub const CLOSE_SECS: u64 = 60;
/// How long the client has to punch a mob dead.
pub const PUNCH_SECS: u64 = 120;
/// How long a witch has to land a potion on the client.
pub const WITCH_SECS: u64 = 120;
/// How far a knife reaches from the eye, in blocks (MW2's 64 units),
/// with room for the mob's half width.
pub const KNIFE_BLOCKS: f64 = 1.3;

/// The world sounds of a `minecraft-sounds:` line: event and how far from
/// the player it was.
pub fn heard(event: &Event) -> Vec<(String, f64)> {
    event
        .get("events")
        .unwrap_or("-")
        .split(',')
        .filter_map(|entry| {
            let (name, distance) = entry.split_once('@')?;
            Some((name.to_owned(), distance.parse().ok()?))
        })
        .collect()
}

/// The player's status effects in a `minecraft-mobs:` line: effect, level
/// and ticks left.
pub fn effects(event: &Event) -> Vec<(String, u32, i32)> {
    event
        .get("effects")
        .unwrap_or("-")
        .split(',')
        .filter_map(|entry| {
            let parts: Vec<&str> = entry.split(':').collect();
            let [name, level, ticks] = parts.as_slice() else {
                return None;
            };
            Some(((*name).to_owned(), level.parse().ok()?, ticks.parse().ok()?))
        })
        .collect()
}

/// How many arrows or potions (`kind`) a game has drawn, from a
/// `minecraft-mobs:` line.
pub fn flown(event: &Event, kind: &str) -> u32 {
    event
        .get("flown")
        .unwrap_or("-")
        .split(',')
        .find_map(|entry| {
            let (name, count) = entry.split_once(':')?;
            (name == kind).then(|| count.parse().ok()).flatten()
        })
        .unwrap_or(0)
}

/// A number field of a line.
pub fn number(event: &Event, key: &str) -> Option<f64> {
    event.get(key)?.parse().ok()
}

/// The map point (36 units a block) over the middle of the block `at` (from
/// the world's spawn block), a little above its top: map origin is the
/// middle of the spawn block.
pub fn block_centre_map(at: [i32; 3]) -> [i32; 3] {
    [at[0] * 36, -at[2] * 36, (at[1] + 1) * 36 + 18]
}

/// `mc set` lines for a pen two blocks along +X from the block `feet` (from
/// the world's spawn block): one block across and two high inside stone,
/// roofed, so a mob in it cannot run from a fist that reaches through the
/// stone, nor be knocked out over the top (run 39228628595's pig was, by
/// the blows that had taken it to 2 health).
pub fn pen(feet: [i32; 3]) -> String {
    let [x, y, z] = feet;
    [
        format!(
            "mc set {} {y} {} {} {} {} stone",
            x + 1,
            z - 1,
            x + 3,
            y + 2,
            z + 1
        ),
        format!("mc set {} {y} {z} {} {} {z} air", x + 2, x + 2, y + 1),
    ]
    .join("; ")
}

/// What the client's knife or fists came to: the host's last census of the
/// mob, whether the host saw it die, and the blows struck.
pub struct Blows {
    pub host: Event,
    pub killed: bool,
    pub blows: u32,
}

fn host_saw_death(pair: &mut Pair, kind: &str, id: u64) -> Result<(Event, bool), String> {
    let host = census(pair, false, "h_blow", Some(kind))?;
    let killed = mobs(&host)
        .into_iter()
        .find(|m| m.id == id)
        .is_none_or(|m| m.dead);
    Ok((host, killed))
}

/// The client knifes the mob `id` (MW2's melee with the gun it holds; no
/// shot fired): it waits for the mob to come within `KNIFE_BLOCKS` of its
/// eye, looks at its `middle` and strikes, until the host sees it die or
/// `timeout` passes.
pub fn knife(
    pair: &mut Pair,
    kind: &str,
    id: u64,
    middle: f64,
    timeout: Duration,
) -> Result<Blows, String> {
    let start = Instant::now();
    let mut blows = 0;
    loop {
        let seen = census(pair, true, "c_knife", Some(kind))?;
        if let (Some(eye), Some(mob)) = (
            eye(&seen),
            mobs(&seen).into_iter().find(|m| m.id == id && !m.dead),
        ) {
            let target = [mob.at[0], mob.at[1] + middle, mob.at[2]];
            let across = (target[0] - eye[0]).hypot(target[2] - eye[2]);
            if across <= KNIFE_BLOCKS {
                let (yaw, pitch) = aim(eye, target);
                pair.ask(
                    true,
                    &format!(
                        "look {yaw:.2} {pitch:.2}; wait 0.1s; press +melee 0.1; wait 1.2s; mc mobs c_struck {kind}"
                    ),
                    "c_struck",
                    Duration::from_secs(60),
                )?;
                blows += 1;
            }
        }
        let (host, killed) = host_saw_death(pair, kind, id)?;
        if killed || start.elapsed() > timeout {
            return Ok(Blows {
                host,
                killed,
                blows,
            });
        }
        let at = Instant::now();
        pair.until("the next look", Duration::from_secs(5), |_| {
            at.elapsed() >= Duration::from_millis(300)
        })?;
    }
}

/// The client punches the mob `id` with an empty hand (hotbar `slot`, from
/// 1): it looks at the mob's `middle` and clicks, a blow every 0.7 s (past
/// the mob's half second of invulnerability), until the host sees it die
/// or `timeout` passes.
pub fn punch(
    pair: &mut Pair,
    kind: &str,
    id: u64,
    middle: f64,
    slot: u32,
    timeout: Duration,
) -> Result<Blows, String> {
    let start = Instant::now();
    pair.client
        .as_mut()
        .ok_or("no client")?
        .send(&format!("key {slot}; wait 1s"))?;
    let mut blows = 0;
    loop {
        let seen = census(pair, true, "c_punch", Some(kind))?;
        if let (Some(eye), Some(mob)) = (
            eye(&seen),
            mobs(&seen).into_iter().find(|m| m.id == id && !m.dead),
        ) {
            let (yaw, pitch) = aim(eye, [mob.at[0], mob.at[1] + middle, mob.at[2]]);
            pair.ask(
                true,
                &format!(
                    "look {yaw:.2} {pitch:.2}; wait 0.1s; key mouse1 0; wait 0.6s; mc mobs c_punched {kind}"
                ),
                "c_punched",
                Duration::from_secs(60),
            )?;
            blows += 1;
        }
        let (host, killed) = host_saw_death(pair, kind, id)?;
        if killed || start.elapsed() > timeout {
            return Ok(Blows {
                host,
                killed,
                blows,
            });
        }
    }
}

/// Asks the client (for its witches) until it has drawn a potion and has an
/// effect, or `timeout` passes; the last answer.
pub fn poll_potion(pair: &mut Pair, label: &str, timeout: Duration) -> Result<Event, String> {
    let start = Instant::now();
    loop {
        let event = census(pair, true, label, Some("witch"))?;
        if (flown(&event, "potion") > 0 && !effects(&event).is_empty()) || start.elapsed() > timeout
        {
            return Ok(event);
        }
        let at = Instant::now();
        pair.until("the next potion check", Duration::from_secs(5), |_| {
            at.elapsed() >= Duration::from_millis(700)
        })?;
    }
}

/// The client's hotbar slots (from 1) with nothing in them, from a
/// `minecraft-state:` line.
pub fn empty_slots(event: &Event) -> Vec<u32> {
    event
        .get("slots")
        .unwrap_or("")
        .split(',')
        .take(9)
        .enumerate()
        .filter(|(_, s)| *s == "-")
        .map(|(i, _)| i as u32 + 1)
        .collect()
}

/// The mob steps beyond the gun, in the arena at `far` (from the world's
/// spawn block) after the client shot its husk there: the world sounds the
/// client heard of it; the client, moved to the middle of the platform,
/// knifes a husk and punches a pig in a pen to death as the host sees; a
/// witch's potion reaches it with its effect, and the witch is shot; then
/// the host's spawn range around the client and the mobs the client sees
/// beyond 48 blocks.
pub fn beyond_the_gun(
    pair: &mut Pair,
    far: [i32; 3],
    lines: &mut std::collections::BTreeMap<String, Event>,
    notes: &mut std::collections::BTreeMap<&'static str, String>,
) -> Result<(), String> {
    let step = Duration::from_secs(120);
    let see = Duration::from_secs(SEE_SECS);
    // The middle of the platform, a gun up for the knife.
    let [x, y, z] = block_centre_map(far);
    pair.client
        .as_mut()
        .ok_or("no client")?
        .send(&format!("tp {x} {y} {z}; wait 3s"))?;
    // What it heard of the husk, measured from there: a client the husk
    // killed before its gun did (a headless server runs its mobs at full
    // speed, a game drawing 3-4 frames a second shoots slowly) stood at the
    // world's spawn until now (run 39246128209).
    let heard = pair.ask(true, "mc sounds c_sounds", "c_sounds", step)?;
    lines.insert("c_sounds".into(), heard);
    draw_gun(pair)?;
    pair.client
        .as_mut()
        .ok_or("no client")?
        .send("mc summon husk 0 0 2.5")?;
    let (seen, husk) = poll(pair, true, "c_knife_husk", Some("husk"), see, summoned)?;
    lines.insert("c_knife_husk".into(), seen);
    let husk = husk.ok_or("the client never saw the husk for the knife")?;
    let blows = knife(
        pair,
        "husk",
        husk.id,
        0.95,
        Duration::from_secs(CLOSE_SECS + 30),
    )?;
    lines.insert("h_knifed".into(), blows.host);
    notes.insert("knife_killed", blows.killed.to_string());
    notes.insert("knife_blows", blows.blows.to_string());
    // A pig in a pen beside the client, back in the middle, and its fists.
    pair.client
        .as_mut()
        .ok_or("no client")?
        .send(&format!("tp {x} {y} {z}; wait 3s"))?;
    let here = census(pair, true, "c_centre", Some("-"))?;
    let eye = eye(&here).ok_or("no eye in `c_centre`")?;
    let feet = [eye[0].floor() as i32, far[1] + 1, eye[2].floor() as i32];
    let state = pair.ask(
        true,
        &format!("{}; wait 2s; mc state c_hand", pen(feet)),
        "c_hand",
        step,
    )?;
    let slot = empty_slots(&state)
        .first()
        .copied()
        .ok_or("no empty hotbar slot on the client")?;
    lines.insert("c_hand".into(), state);
    let cell = [f64::from(feet[0] + 2) + 0.5, f64::from(feet[2]) + 0.5];
    pair.client.as_mut().ok_or("no client")?.send(&format!(
        "mc summon pig {:.2} 0 {:.2}",
        cell[0] - eye[0],
        cell[1] - eye[2]
    ))?;
    let (seen, pig) = poll(pair, true, "c_pen_pig", Some("pig"), see, |event, mob| {
        summoned(event, mob)
            && (mob.at[0] - cell[0]).abs() < 0.6
            && (mob.at[2] - cell[1]).abs() < 0.6
    })?;
    lines.insert("c_pen_pig".into(), seen);
    let pig = pig.ok_or("the client never saw the pig in its pen")?;
    let blows = punch(
        pair,
        "pig",
        pig.id,
        0.45,
        slot,
        Duration::from_secs(PUNCH_SECS),
    )?;
    lines.insert("h_punched".into(), blows.host);
    notes.insert("punch_killed", blows.killed.to_string());
    notes.insert("punch_blows", blows.blows.to_string());
    // A witch across the arena: its potion and the effect.
    summon_seen(pair, "witch", "-3 0 -3", "c_witch_seen", see)?;
    let potion = poll_potion(pair, "c_witch", Duration::from_secs(WITCH_SECS))?;
    lines.insert("c_witch".into(), potion.clone());
    draw_gun(pair)?;
    if let Some(witch) = mobs(&potion).into_iter().find(|m| !m.dead) {
        let shot = shoot(
            pair,
            "witch",
            witch.id,
            0.95,
            &potion,
            Duration::from_secs(KILL_SECS),
        )?;
        notes.insert("witch_killed", shot.killed.to_string());
    }
    // Where the host's mobs live around the client. Every mob so far was
    // summoned (the world starts with `IW4L_MINECRAFT_MOBS=off`); natural
    // spawning comes on only now, for this step. It is the world's ground
    // and chance, so each side is asked until a mob lives beyond 48 blocks
    // of the client.
    pair.host.send("mc spawning on")?;
    let range = until_one(
        pair,
        false,
        "mc mobrange s_range",
        "s_range",
        "band48_128",
        step,
    )?;
    lines.insert("s_range".into(), range);
    let far_mobs = until_one(pair, true, "mc mobs c_far_mobs", "c_far_mobs", "far", step)?;
    lines.insert("c_far_mobs".into(), far_mobs);
    Ok(())
}

/// How long `beyond_the_gun` asks for mobs far from the client.
pub const RANGE_SECS: u64 = 180;

/// Asks `script` for the line `label` until its `key` counts one or more or
/// `RANGE_SECS` pass: the last answer either way.
fn until_one(
    pair: &mut Pair,
    client: bool,
    script: &str,
    label: &str,
    key: &str,
    step: Duration,
) -> Result<Event, String> {
    let start = Instant::now();
    loop {
        let answer = pair.ask(client, script, label, step)?;
        if number(&answer, key).is_some_and(|n| n >= 1.0)
            || start.elapsed() > Duration::from_secs(RANGE_SECS)
        {
            return Ok(answer);
        }
        let pause = Instant::now();
        while pause.elapsed() < Duration::from_secs(2) {
            pair.sample();
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// What `beyond_the_gun` must show, checkpoint by checkpoint.
pub fn judge_beyond_the_gun(
    lines: &std::collections::BTreeMap<String, Event>,
    notes: &std::collections::BTreeMap<&'static str, String>,
) -> Vec<(&'static str, Result<(), String>)> {
    let line = |label: &str| lines.get(label).ok_or_else(|| format!("no `{label}` line"));
    let note = |key: &str| {
        notes
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| format!("no `{key}` answer"))
    };
    let killed = |key: &str, what: &str| -> Result<(), String> {
        match note(key)? {
            "true" => Ok(()),
            other => Err(format!("{what}: the host saw it die: {other}")),
        }
    };
    vec![
        (
            "client_hears_mob_sounds",
            (|| {
                let heard = heard(line("c_sounds")?);
                if heard
                    .iter()
                    .any(|(event, d)| event.starts_with("entity.husk.") && *d <= 16.0)
                {
                    Ok(())
                } else {
                    Err(format!(
                        "the client heard no husk within 16 blocks: {heard:?}"
                    ))
                }
            })(),
        ),
        (
            "client_knifes_mob_host_sees",
            killed("knife_killed", "the knifed husk"),
        ),
        (
            "client_punches_mob_host_sees",
            killed("punch_killed", "the punched pig"),
        ),
        (
            "client_sees_witch_potion",
            (|| {
                let n = flown(line("c_witch")?, "potion");
                if n > 0 {
                    Ok(())
                } else {
                    Err("the client drew no potion in flight".into())
                }
            })(),
        ),
        (
            "client_gets_potion_effect",
            (|| {
                let effects = effects(line("c_witch")?);
                let witchy = ["poison", "slowness", "weakness"];
                if effects
                    .iter()
                    .any(|(name, _, ticks)| witchy.contains(&name.as_str()) && *ticks > 0)
                {
                    Ok(())
                } else {
                    Err(format!("the client has no potion effect: {effects:?}"))
                }
            })(),
        ),
        (
            "mobs_live_beyond_48_blocks",
            (|| {
                let range = line("s_range")?;
                let band = number(range, "band48_128").ok_or("no band48_128")?;
                let far = number(line("c_far_mobs")?, "far").ok_or("no far")?;
                if band >= 1.0 && far >= 1.0 {
                    Ok(())
                } else {
                    Err(format!(
                        "mobs 48-128 blocks from the client on the host: {band}, beyond 48 on the client: {far}"
                    ))
                }
            })(),
        ),
        (
            "spawns_in_vanilla_range",
            (|| {
                let range = line("s_range")?;
                let spawned = number(range, "spawned").unwrap_or(0.0);
                let min = number(range, "spawn_min");
                let max = number(range, "spawn_max");
                let beyond = number(range, "spawn_beyond48").unwrap_or(0.0);
                match (min, max) {
                    (Some(min), Some(max))
                        if spawned >= 1.0 && min >= 23.0 && max <= 131.0 && beyond >= 1.0 =>
                    {
                        Ok(())
                    }
                    _ => Err(format!(
                        "natural spawns around the client: {spawned} from {min:?} to {max:?} blocks, {beyond} beyond 48 (want some, 24 to 128, some beyond 48)"
                    )),
                }
            })(),
        ),
        (
            "despawns_beyond_128",
            (|| {
                let range = line("s_range")?;
                let stray = number(range, "stray").ok_or("no stray")?;
                if stray > 0.0 {
                    return Err(format!(
                        "{stray} mobs that may despawn stand beyond 128 blocks of every player"
                    ));
                }
                match number(range, "despawn_min") {
                    Some(min) if min < 30.0 => Err(format!(
                        "a mob despawned {min} blocks from the client (vanilla: beyond 32)"
                    )),
                    _ => Ok(()),
                }
            })(),
        ),
    ]
}

/// The lines and answers of a `beyond_the_gun` that showed everything, for
/// the scenarios' judging tests.
#[cfg(test)]
pub(crate) fn passing_beyond_the_gun(
    lines: &mut std::collections::BTreeMap<String, Event>,
    notes: &mut std::collections::BTreeMap<&'static str, String>,
) {
    let event = |prefix: &str, fields: &str| {
        crate::runner::parse(&format!("{prefix} {fields}"), prefix, 0).unwrap()
    };
    lines.insert(
        "c_sounds".into(),
        event(
            "minecraft-sounds:",
            "label=c_sounds role=client total=9 events=entity.husk.ambient@4.2,entity.husk.hurt@3.1,entity.husk.death@3.0",
        ),
    );
    lines.insert(
        "c_witch".into(),
        event(
            "minecraft-mobs:",
            "label=c_witch role=client health=80 count=1 far=0 mobs=witch:77:-3.00,61.00,-3.00:26.0:alive items=- effects=poison:0:612 flown=potion:1",
        ),
    );
    lines.insert(
        "s_range".into(),
        event(
            "minecraft-mobrange:",
            "label=s_range role=dedicated player=2 spawned=14 spawn_min=24.6 spawn_max=127.1 spawn_beyond48=11 despawned=3 despawn_min=40.2 despawn_max=129.5 near48=6 band48_128=21 beyond128=4 stray=0 level_chunks=578",
        ),
    );
    lines.insert(
        "c_far_mobs".into(),
        event(
            "minecraft-mobs:",
            "label=c_far_mobs role=client far=9 mobs=- items=-",
        ),
    );
    for (key, value) in [("knife_killed", "true"), ("punch_killed", "true")] {
        notes.insert(key, value.into());
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
            "label=c role=client health=85 eye=320.50,62.62,0.50 guns=2,5 held=1 count=2 \
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
        assert_eq!(held(&e), Some(1));
        assert_eq!(items(&e), ["minecraft:porkchop*2", "minecraft:stone*1"]);
        let none = event("label=c mobs=- items=-");
        assert!(mobs(&none).is_empty());
        assert!(items(&none).is_empty());
        assert_eq!(held(&none), None);
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

    #[test]
    fn sounds_effects_and_flights_read_back() {
        let sounds = crate::runner::parse(
            "minecraft-sounds: label=s role=client total=3 events=entity.husk.step@2.5,entity.generic.explode@40.0",
            "minecraft-sounds:",
            0,
        )
        .unwrap();
        assert_eq!(
            heard(&sounds),
            [
                ("entity.husk.step".to_owned(), 2.5),
                ("entity.generic.explode".to_owned(), 40.0)
            ]
        );
        let e = event("label=w effects=poison:0:600,slowness:1:20 flown=arrow:2,potion:3 far=4");
        assert_eq!(
            effects(&e),
            [
                ("poison".to_owned(), 0, 600),
                ("slowness".to_owned(), 1, 20)
            ]
        );
        assert_eq!((flown(&e, "potion"), flown(&e, "arrow")), (3, 2));
        assert_eq!(number(&e, "far"), Some(4.0));
        let none = event("label=w effects=- flown=-");
        assert!(effects(&none).is_empty());
        assert_eq!(flown(&none, "potion"), 0);
        let state = crate::runner::parse(
            "minecraft-state: label=h slots=iw4:weapon/1*1,-,minecraft:dirt*3,-,-,-,-,-,-,-",
            "minecraft-state:",
            0,
        )
        .unwrap();
        assert_eq!(empty_slots(&state), [2, 4, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn the_pen_closes_round_one_block_beside_the_feet() {
        let lines = pen([320, 61, 0]);
        assert_eq!(
            lines,
            "mc set 321 61 -1 323 63 1 stone; mc set 322 61 0 322 62 0 air"
        );
        assert_eq!(block_centre_map([320, 60, 0]), [11520, 0, 2214]);
    }

    #[test]
    fn beyond_the_gun_passes_and_fails_as_shown() {
        let mut lines = std::collections::BTreeMap::new();
        let mut notes = std::collections::BTreeMap::new();
        passing_beyond_the_gun(&mut lines, &mut notes);
        let failed =
            |lines: &std::collections::BTreeMap<String, Event>,
             notes: &std::collections::BTreeMap<&'static str, String>| {
                judge_beyond_the_gun(lines, notes)
                    .into_iter()
                    .filter(|(_, verdict)| verdict.is_err())
                    .map(|(label, _)| label)
                    .collect::<Vec<_>>()
            };
        assert!(
            failed(&lines, &notes).is_empty(),
            "{:?}",
            judge_beyond_the_gun(&lines, &notes)
        );
        // A client that heard nothing, killed nothing, got no potion, and a
        // host whose mobs keep to 48 blocks of it.
        let mut bad = lines.clone();
        bad.insert(
            "c_sounds".into(),
            crate::runner::parse(
                "minecraft-sounds: label=c_sounds total=0 events=-",
                "minecraft-sounds:",
                0,
            )
            .unwrap(),
        );
        bad.insert("c_witch".into(), event("label=c_witch effects=- flown=-"));
        bad.insert(
            "s_range".into(),
            crate::runner::parse(
                "minecraft-mobrange: label=s_range spawned=4 spawn_min=12.0 spawn_max=45.0 spawn_beyond48=0 despawned=1 despawn_min=20.0 band48_128=0 stray=2",
                "minecraft-mobrange:",
                0,
            )
            .unwrap(),
        );
        let mut bad_notes = notes.clone();
        bad_notes.insert("knife_killed", "false".into());
        bad_notes.remove("punch_killed");
        assert_eq!(
            failed(&bad, &bad_notes),
            [
                "client_hears_mob_sounds",
                "client_knifes_mob_host_sees",
                "client_punches_mob_host_sees",
                "client_sees_witch_potion",
                "client_gets_potion_effect",
                "mobs_live_beyond_48_blocks",
                "spawns_in_vanilla_range",
                "despawns_beyond_128"
            ]
        );
    }
}
