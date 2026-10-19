//! `minecraft_multiplayer` — owner-approved. This file is the one place its
//! steps and what each checkpoint must show live.
//!
//! A host opens a lobby on the Minecraft map through a master of the run's
//! own and a client joins it. The client must generate the host's world
//! (the same seed), see a block the host sets and one the host clears; the
//! host must see a block the client sets, one it places by hand (a right
//! click with planks the host gave it on its `mc give`) and one it breaks
//! by hand. Far from the host, in a walled arena, a husk summoned beside the
//! client must reach the client with the host's id and stand where the host
//! has it, hurt the client, and die to the client's gun as the host sees;
//! a pig shot dead there must drop its porkchop where the client sees it.
//! The client must have heard the husk (its sounds, from the host), kill a
//! husk with MW2's knife and a penned pig with its fists as the host sees,
//! draw a witch's thrown potion and get its effect; the host's mobs must
//! spawn and live out past 48 blocks around the client (24 to 128 blocks,
//! despawning beyond 128) and the client must see some of them out there.
//! Then both quit, the host and the client start again on the same
//! settings, and the host's world comes back: the same seed, its blocks,
//! the host's inventory and the client's (kept by player name).

use std::collections::BTreeMap;
use std::time::Duration;

use crate::duo::Pair;
use crate::runner::Event;
use crate::scenario::{Part, Parts};
use crate::scenarios::mob_steps;

pub const NAME: &str = "minecraft_multiplayer";
/// The blocks, the client's hand and the restart: both sessions.
pub const WORLD: &str = "world";
/// Far from the host, the mobs: the first session only.
pub const MOBS: &str = "mobs";
/// Each part's checks. Both need a listen host: they are the host's own
/// world and mobs as a player of it sees them, far from its own player;
/// `dedicated_server` has a dedicated server's.
pub const PARTS: Parts = &[
    (
        WORLD,
        &[
            "seed_shared",
            "host_block_on_client",
            "client_block_on_host",
            "host_clear_on_client",
            "host_gives_client",
            "client_place_on_host",
            "client_break_on_host",
            "client_picks_up_drop",
            "restart_same_seed",
            "restart_world_kept",
            "restart_host_inventory",
            "restart_client_inventory",
        ],
    ),
    (
        MOBS,
        &[
            "seed_shared",
            "client_ground_far_from_host",
            "client_sees_mob_far_from_host",
            "mob_tracks_host_position",
            "mob_hurts_client",
            "client_kills_mob_host_sees",
            "mob_drop_reaches_client",
            "mob_states_sent",
            "client_hears_mob_sounds",
            "client_knifes_mob_host_sees",
            "client_punches_mob_host_sees",
            "client_sees_witch_potion",
            "client_gets_potion_effect",
            "mobs_live_beyond_48_blocks",
            "spawns_in_vanilla_range",
            "despawns_beyond_128",
        ],
    ),
];
pub const MAP: &str = "minecraft:overworld";
/// The same class as `crafting_table`'s: guns fill the first hotbar slots.
pub const CLASSES: &str = crate::scenarios::crafting_table::CLASSES;
/// Loading a game and its world, on a CPU renderer.
pub const LOAD_SECS: u64 = 600;
/// One step's answer.
pub const STEP_SECS: u64 = 60;
/// A change reaching the other game.
pub const SYNC_SECS: u64 = 30;

const GOLD: [i32; 3] = [2, 5, 0];
const DIAMOND: [i32; 3] = [3, 6, 0];
/// A stone platform far out of the host's view (320 blocks east, 60 up), and
/// the map point (36 units a block) just over its middle.
const FAR: [i32; 3] = [320, 60, 0];
const FAR_MAP: [i32; 3] = [FAR[0] * 36, -FAR[2] * 36, (FAR[1] + 1) * 36 + 18];

/// Each line both games printed, by label (`h_` the host's, `c_` the
/// client's), and the facts the steps worked out.
#[derive(Default)]
pub struct Facts {
    pub lines: BTreeMap<String, Event>,
    pub notes: BTreeMap<&'static str, String>,
    /// The host's `minecraft-mobnet:` lines: the mob bytes it sent each
    /// client, every ten seconds.
    pub mobnet: Vec<Event>,
}

impl Facts {
    fn keep(&mut self, label: &str, event: Event) -> Event {
        self.lines.insert(label.to_owned(), event.clone());
        event
    }
}

fn slots(event: &Event) -> Vec<String> {
    event
        .get("slots")
        .unwrap_or("")
        .split(',')
        .map(str::to_owned)
        .collect()
}

fn feet(event: &Event) -> Option<[i32; 3]> {
    let v: Vec<i32> = event
        .get("feet")?
        .split(',')
        .filter_map(|v| v.parse().ok())
        .collect();
    (v.len() == 3).then(|| [v[0], v[1], v[2]])
}

fn add(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn set(at: [i32; 3], block: &str) -> String {
    format!(
        "mc set {0} {1} {2} {0} {1} {2} {block}",
        at[0], at[1], at[2]
    )
}

/// `mc set` lines for a closed stone room from the block `feet` along +X,
/// three blocks long, three wide and two high inside, its far wall the
/// stone the client places against: the client walks and places on no block
/// of the seed's, and no sand, gravel, water or lava gets in.
fn corridor(feet: [i32; 3]) -> String {
    let [x, y, z] = feet;
    let fill = |from: [i32; 3], to: [i32; 3], block: &str| {
        format!(
            "mc set {} {} {} {} {} {} {block}",
            from[0], from[1], from[2], to[0], to[1], to[2]
        )
    };
    [
        fill([x - 1, y - 1, z - 2], [x + 4, y - 1, z + 2], "stone"),
        fill([x - 1, y + 2, z - 2], [x + 4, y + 2, z + 2], "stone"),
        fill([x - 1, y, z - 2], [x + 4, y + 1, z - 2], "stone"),
        fill([x - 1, y, z + 2], [x + 4, y + 1, z + 2], "stone"),
        fill([x - 1, y, z - 1], [x - 1, y + 1, z + 1], "stone"),
        fill([x + 3, y, z - 1], [x + 4, y + 1, z + 1], "stone"),
        fill([x, y, z - 1], [x + 2, y + 1, z + 1], "air"),
    ]
    .join("; ")
}

/// The first session: the lobby is up and the match started; everything
/// both games are asked, in order (the part's steps, or all of them). Stops
/// at the first step that cannot go on (a missing answer); the judge says
/// what failed.
pub fn first_session(pair: &mut Pair, facts: &mut Facts, part: Part) -> Result<(), String> {
    let load = Duration::from_secs(LOAD_SECS);
    pair.host.send(&format!(
        "wait world; spawn 0; force_match_start; wait minecraft; wait 1s; mc clear; mc give torch 7; {}; wait 1s",
        set(GOLD, "gold_block")
    ))?;
    pair.client
        .as_mut()
        .ok_or("no client")?
        .send("wait world; spawn 0; wait minecraft; wait 2s")?;
    let host = pair.ask(false, "wait minecraft; mc state h_ready", "h_ready", load)?;
    facts.keep("h_ready", host);
    let client = pair.ask(true, "wait minecraft; mc state c_ready", "c_ready", load)?;
    let client = facts.keep("c_ready", client);
    if part.plays(WORLD) {
        world_steps(pair, facts, &client)?;
    }
    if part.plays(MOBS) {
        far_steps(pair, facts)?;
    }
    Ok(())
}

/// Part `world`'s first session: blocks set and cleared both ways, and the
/// client's planks placed, broken and picked up by hand.
fn world_steps(pair: &mut Pair, facts: &mut Facts, client: &Event) -> Result<(), String> {
    let step = Duration::from_secs(STEP_SECS);
    let sync = Duration::from_secs(SYNC_SECS);
    // The host's block, on the client.
    let gold = pair.poll_block(true, "c_gold", GOLD, "minecraft:gold_block", sync)?;
    facts.notes.insert("c_gold", gold);
    // The client's block, on the host.
    pair.client
        .as_mut()
        .ok_or("no client")?
        .send(&set(DIAMOND, "diamond_block"))?;
    let diamond = pair.poll_block(false, "h_diamond", DIAMOND, "minecraft:diamond_block", sync)?;
    facts.notes.insert("h_diamond", diamond);
    // The host clears its block; the client sees air.
    pair.host.send(&set(GOLD, "air"))?;
    let gone = pair.poll_block(true, "c_gold_gone", GOLD, "air", sync)?;
    facts.notes.insert("c_gold_gone", gone);

    // The client places planks by hand against a stone three blocks along
    // +X from its feet, with planks its host gave it.
    let feet = feet(client).ok_or("c_ready has no feet")?;
    let plank = add(feet, [2, 0, 0]);
    let script = format!(
        "{}; mc clear; mc give oak_planks 4; wait 2s; mc state c_given",
        corridor(feet)
    );
    let given = pair.ask(true, &script, "c_given", step)?;
    let given = facts.keep("c_given", given);
    let planks = slots(&given)
        .iter()
        .position(|s| s == "minecraft:oak_planks*4")
        .filter(|&i| i < 9)
        .ok_or("the client's planks are not on its hotbar")?;
    facts.notes.insert("planks_slot", planks.to_string());
    let placed = pair.ask(
        true,
        &format!(
            "key {}; wait 1s; look 0 23; wait 0.2s; key mouse2 0; wait 2s; mc state c_placed",
            planks + 1
        ),
        "c_placed",
        step,
    )?;
    facts.keep("c_placed", placed.clone());
    let block = pair.poll_block(false, "h_plank", plank, "minecraft:oak_planks", sync)?;
    facts.notes.insert("h_plank", block);
    // And breaks it by hand, from an empty hotbar slot.
    let empty = slots(&placed)
        .iter()
        .take(9)
        .position(|s| s == "-")
        .ok_or("no empty hotbar slot on the client")?;
    pair.client.as_mut().ok_or("no client")?.send(&format!(
        "key {}; wait 1s; look 0 23; wait 0.2s; key mouse1 4",
        empty + 1
    ))?;
    let broken = pair.poll_block(false, "h_plank_gone", plank, "air", Duration::from_secs(20))?;
    facts.notes.insert("h_plank_gone", broken);
    // It walks onto the plank the break dropped, on the host, and picks it up.
    let picked = pair.ask(
        true,
        "hold +forward; wait 0.4s; release +forward; wait 3s; mc state c_picked",
        "c_picked",
        step,
    )?;
    facts.keep("c_picked", picked);
    let last = pair.ask(true, "key 1; wait 1s; mc state c_final", "c_final", step)?;
    facts.keep("c_final", last);
    Ok(())
}

/// Part `mobs`'s first session: the client far from the host, on a
/// platform, and the mobs there.
fn far_steps(pair: &mut Pair, facts: &mut Facts) -> Result<(), String> {
    let step = Duration::from_secs(STEP_SECS);
    // Far from the host, the host still has ground under the client: it
    // stands on a platform it asked for there (the host has no terrain
    // there of its own).
    pair.client.as_mut().ok_or("no client")?.send(&format!(
        "mc set {} {} {} {} {} {} stone; wait 2s; tp {} {} {}; wait 8s",
        FAR[0] - 1,
        FAR[1],
        FAR[2] - 1,
        FAR[0] + 1,
        FAR[1],
        FAR[2] + 1,
        FAR_MAP[0],
        FAR_MAP[1],
        FAR_MAP[2]
    ))?;
    let far = pair.ask(
        true,
        "mc state c_far1",
        "c_far1",
        Duration::from_secs(STEP_SECS),
    )?;
    facts.keep("c_far1", far);
    let far = pair.ask(true, "wait 2s; mc state c_far2", "c_far2", step)?;
    facts.keep("c_far2", far);
    far_mobs(pair, facts)
}

/// Far from the host, in a walled arena around the client's platform: a
/// husk summoned beside the client reaches the client's view with the
/// host's id, stands where the host has it, hurts the client, and dies to
/// the client's gun as the host sees; then a pig shot dead there drops its
/// porkchop where the client sees it (or picks it up).
fn far_mobs(pair: &mut Pair, facts: &mut Facts) -> Result<(), String> {
    let client = pair.client.as_mut().ok_or("no client")?;
    client.send(&format!("{}; wait 2s", mob_steps::arena(FAR)))?;
    // A bullet gun up before the husk comes.
    mob_steps::draw_gun(pair)?;
    pair.client
        .as_mut()
        .ok_or("no client")?
        .send("mc summon husk 3 0 0")?;
    let see = Duration::from_secs(mob_steps::SEE_SECS);
    let (seen, husk) =
        mob_steps::poll(pair, true, "c_husk", Some("husk"), see, mob_steps::summoned)?;
    facts.keep("c_husk", seen);
    let husk = husk.ok_or("the client never saw the husk")?;
    facts.notes.insert("husk_id", husk.id.to_string());
    // Where each game has it, a moment apart.
    let at = std::time::Instant::now();
    pair.until("the husk to close in", Duration::from_secs(10), |_| {
        at.elapsed() >= Duration::from_secs(2)
    })?;
    let mine = mob_steps::census(pair, true, "c_track", Some("husk"))?;
    facts.keep("c_track", mine);
    let theirs = mob_steps::census(pair, false, "h_track", Some("husk"))?;
    facts.keep("h_track", theirs);
    let hurt = mob_steps::poll_hurt(
        pair,
        "c_hurt",
        "husk",
        100.0,
        Duration::from_secs(mob_steps::HURT_SECS),
    )?;
    facts.keep("c_hurt", hurt.clone());
    let kill = Duration::from_secs(mob_steps::KILL_SECS);
    let shot = mob_steps::shoot(pair, "husk", husk.id, 0.95, &hurt, kill)?;
    facts.keep("h_husk_shot", shot.host);
    facts.notes.insert("husk_killed", shot.killed.to_string());
    facts.notes.insert("husk_bursts", shot.bursts.to_string());
    pair.client
        .as_mut()
        .ok_or("no client")?
        .send("mc summon pig 2 0 2")?;
    let (seen, pig) = mob_steps::poll(pair, true, "c_pig", Some("pig"), see, mob_steps::summoned)?;
    facts.keep("c_pig", seen.clone());
    let pig = pig.ok_or("the client never saw the pig")?;
    let shot = mob_steps::shoot(pair, "pig", pig.id, 0.45, &seen, kill)?;
    facts.notes.insert("pig_killed", shot.killed.to_string());
    let drop = mob_steps::poll_drop(pair, "minecraft:porkchop", Duration::from_secs(SYNC_SECS))?;
    facts.notes.insert("pig_drop", drop);
    mob_steps::beyond_the_gun(pair, FAR, &mut facts.lines, &mut facts.notes)
}

/// The second session, both games started again on the same settings.
pub fn second_session(pair: &mut Pair, facts: &mut Facts) -> Result<(), String> {
    let load = Duration::from_secs(LOAD_SECS);
    pair.host.send("wait world; spawn 0; force_match_start")?;
    pair.client
        .as_mut()
        .ok_or("no client")?
        .send("wait world; spawn 0")?;
    let host = pair.ask(
        false,
        "wait minecraft; wait 2s; mc state h2_ready",
        "h2_ready",
        load,
    )?;
    facts.keep("h2_ready", host);
    let client = pair.ask(
        true,
        "wait minecraft; wait 2s; mc state c2_ready",
        "c2_ready",
        load,
    )?;
    facts.keep("c2_ready", client);
    for (client, label, at) in [
        (false, "h2_diamond", DIAMOND),
        (false, "h2_gold", GOLD),
        (true, "c2_diamond", DIAMOND),
    ] {
        let event = pair.ask(
            client,
            &format!("mc block {label} {} {} {}", at[0], at[1], at[2]),
            label,
            Duration::from_secs(STEP_SECS),
        )?;
        facts.keep(label, event);
    }
    Ok(())
}

/// One checkpoint's verdict.
pub struct Check {
    pub label: &'static str,
    pub failures: Vec<String>,
}

fn line<'a>(facts: &'a Facts, label: &str) -> Result<&'a Event, String> {
    facts
        .lines
        .get(label)
        .ok_or_else(|| format!("no `{label}` line"))
}

fn note<'a>(facts: &'a Facts, key: &str) -> Result<&'a str, String> {
    facts
        .notes
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("no `{key}` answer"))
}

/// The husk in a census line, by the id the client first saw it with.
fn mob(facts: &Facts, label: &str) -> Result<mob_steps::Mob, String> {
    let id: u64 = note(facts, "husk_id")?.parse().map_err(|_| "bad husk id")?;
    mob_steps::mobs(line(facts, label)?)
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("`{label}` lists no mob {id}"))
}

fn want(got: &str, want: &str, what: &str) -> Result<(), String> {
    if got == want {
        Ok(())
    } else {
        Err(format!("{what}: want {want}, got {got}"))
    }
}

/// Judges what both games printed.
pub fn judge(facts: &Facts) -> Vec<Check> {
    type Rule = fn(&Facts) -> Result<(), String>;
    let rules: [(&'static str, Rule); 19] = [
        ("seed_shared", |f| {
            let host = line(f, "h_ready")?.get("seed").unwrap_or("-");
            let client = line(f, "c_ready")?.get("seed").unwrap_or("-");
            if host == "-" {
                return Err("the host printed no seed".into());
            }
            want(
                line(f, "h_ready")?.get("role").unwrap_or(""),
                "host",
                "host role",
            )?;
            want(
                line(f, "c_ready")?.get("role").unwrap_or(""),
                "client",
                "client role",
            )?;
            want(client, host, "client seed")
        }),
        ("host_block_on_client", |f| {
            want(note(f, "c_gold")?, "minecraft:gold_block", "client sees")
        }),
        ("client_block_on_host", |f| {
            want(
                note(f, "h_diamond")?,
                "minecraft:diamond_block",
                "host sees",
            )
        }),
        ("host_clear_on_client", |f| {
            want(note(f, "c_gold_gone")?, "air", "client sees")
        }),
        ("host_gives_client", |f| {
            let slot: usize = note(f, "planks_slot")?.parse().map_err(|_| "bad slot")?;
            want(
                &slots(line(f, "c_given")?)[slot],
                "minecraft:oak_planks*4",
                "client slot",
            )
        }),
        ("client_place_on_host", |f| {
            let slot: usize = note(f, "planks_slot")?.parse().map_err(|_| "bad slot")?;
            want(
                &slots(line(f, "c_placed")?)[slot],
                "minecraft:oak_planks*3",
                "client slot after placing",
            )?;
            want(note(f, "h_plank")?, "minecraft:oak_planks", "host sees")
        }),
        ("client_break_on_host", |f| {
            want(note(f, "h_plank_gone")?, "air", "host sees")
        }),
        ("client_picks_up_drop", |f| {
            let slot: usize = note(f, "planks_slot")?.parse().map_err(|_| "bad slot")?;
            want(
                &slots(line(f, "c_picked")?)[slot],
                "minecraft:oak_planks*4",
                "client slot after walking onto the drop",
            )
        }),
        ("client_ground_far_from_host", |f| {
            let want = [FAR[0], FAR[1] + 1, FAR[2]];
            for label in ["c_far1", "c_far2"] {
                let at = feet(line(f, label)?).ok_or_else(|| format!("no feet in `{label}`"))?;
                if at != want {
                    return Err(format!(
                        "{label}: feet {at:?}, want {want:?} on the platform"
                    ));
                }
            }
            Ok(())
        }),
        ("client_sees_mob_far_from_host", |f| {
            let husk = mob(f, "c_husk")?;
            let platform = [f64::from(FAR[0]), f64::from(FAR[1] + 1), f64::from(FAR[2])];
            if mob_steps::apart(husk.at, platform) > 8.0 {
                return Err(format!(
                    "the client's husk at {:?}, not by the platform",
                    husk.at
                ));
            }
            mob(f, "h_track").map(|_| ())
        }),
        ("mob_tracks_host_position", |f| {
            let (mine, theirs) = (mob(f, "c_track")?, mob(f, "h_track")?);
            let apart = mob_steps::apart(mine.at, theirs.at);
            if apart > mob_steps::TRACK_BLOCKS {
                return Err(format!(
                    "the client has the husk at {:?}, the host at {:?}: {apart:.2} blocks apart",
                    mine.at, theirs.at
                ));
            }
            Ok(())
        }),
        ("mob_hurts_client", |f| {
            let health = mob_steps::health(line(f, "c_hurt")?).ok_or("no health in `c_hurt`")?;
            if health < 100.0 {
                Ok(())
            } else {
                Err(format!("the client's health stayed {health}"))
            }
        }),
        ("client_kills_mob_host_sees", |f| {
            want(note(f, "husk_killed")?, "true", "the host saw the husk die")
        }),
        ("mob_drop_reaches_client", |f| {
            want(note(f, "pig_killed")?, "true", "the host saw the pig die")?;
            match note(f, "pig_drop")? {
                "ground" | "picked" => Ok(()),
                other => Err(format!("the client saw no porkchop: {other}")),
            }
        }),
        ("mob_states_sent", |f| {
            let sent = f.mobnet.iter().any(|e| {
                e.get("frame_bytes_per_s")
                    .and_then(|v| v.parse::<f64>().ok())
                    .is_some_and(|b| b > 0.0)
            });
            if sent {
                Ok(())
            } else {
                Err("the host reported no mob states sent".into())
            }
        }),
        ("restart_same_seed", |f| {
            let seed = line(f, "h_ready")?.get("seed").unwrap_or("-");
            want(
                line(f, "h2_ready")?.get("seed").unwrap_or("-"),
                seed,
                "host seed again",
            )?;
            want(
                line(f, "c2_ready")?.get("seed").unwrap_or("-"),
                seed,
                "client seed again",
            )
        }),
        ("restart_world_kept", |f| {
            want(
                line(f, "h2_diamond")?.get("block").unwrap_or(""),
                "minecraft:diamond_block",
                "host diamond",
            )?;
            want(
                line(f, "h2_gold")?.get("block").unwrap_or(""),
                "air",
                "host cleared gold",
            )?;
            want(
                line(f, "c2_diamond")?.get("block").unwrap_or(""),
                "minecraft:diamond_block",
                "client diamond",
            )
        }),
        ("restart_host_inventory", |f| {
            let had = slots(line(f, "h_ready")?);
            let has = slots(line(f, "h2_ready")?);
            if !had.iter().any(|s| s == "minecraft:torch*7") {
                return Err("the host never had its torches".into());
            }
            if has.iter().any(|s| s == "minecraft:torch*7") {
                Ok(())
            } else {
                Err(format!("host slots {}", has.join(",")))
            }
        }),
        ("restart_client_inventory", |f| {
            // The planks it had when it left (three, or four if it picked the
            // broken one up), wherever they were.
            let had: Vec<String> = slots(line(f, "c_final")?)
                .into_iter()
                .filter(|s| s.starts_with("minecraft:oak_planks*"))
                .collect();
            let has: Vec<String> = slots(line(f, "c2_ready")?)
                .into_iter()
                .filter(|s| s.starts_with("minecraft:oak_planks*"))
                .collect();
            if had.is_empty() {
                return Err("the client had no planks when it left".into());
            }
            if has == had {
                Ok(())
            } else {
                Err(format!(
                    "client planks {} after the restart, {} before",
                    has.join(","),
                    had.join(",")
                ))
            }
        }),
    ];
    rules
        .iter()
        .map(|(label, rule)| Check {
            label,
            failures: rule(facts).err().into_iter().collect(),
        })
        .chain(
            mob_steps::judge_beyond_the_gun(&facts.lines, &facts.notes)
                .into_iter()
                .map(|(label, verdict)| Check {
                    label,
                    failures: verdict.err().into_iter().collect(),
                }),
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parts_keep_every_check() {
        let judged: Vec<&str> = judge(&Facts::default()).iter().map(|c| c.label).collect();
        crate::scenario::assert_parts_cover(PARTS, &judged);
    }

    fn event(fields: &str) -> Event {
        crate::runner::parse(&format!("minecraft-state: {fields}"), "minecraft-state:", 0).unwrap()
    }

    fn passing() -> Facts {
        let mut f = Facts::default();
        mob_steps::passing_beyond_the_gun(&mut f.lines, &mut f.notes);
        let mut slots = vec!["-"; 41];
        slots[..5].copy_from_slice(&["iw4:weapon/1*1"; 5]);
        let with = |slot: usize, stack: &'static str| {
            let mut s = slots.clone();
            s[slot] = stack;
            s.join(",")
        };
        let ready = |label: &str, role: &str, extra: String| {
            event(&format!(
                "label={label} seed=12345 role={role} feet=0,0,3 slots={extra}"
            ))
        };
        f.lines.insert(
            "h_ready".into(),
            ready("h_ready", "host", with(5, "minecraft:torch*7")),
        );
        f.lines
            .insert("c_ready".into(), ready("c_ready", "client", with(5, "-")));
        f.lines.insert(
            "c_given".into(),
            ready("c_given", "client", with(5, "minecraft:oak_planks*4")),
        );
        f.lines.insert(
            "c_placed".into(),
            ready("c_placed", "client", with(5, "minecraft:oak_planks*3")),
        );
        f.lines.insert(
            "h2_ready".into(),
            ready("h2_ready", "host", with(5, "minecraft:torch*7")),
        );
        f.lines.insert(
            "c2_ready".into(),
            ready("c2_ready", "client", with(5, "minecraft:oak_planks*3")),
        );
        f.lines.insert(
            "c_final".into(),
            ready("c_final", "client", with(5, "minecraft:oak_planks*3")),
        );
        f.lines.insert(
            "c_picked".into(),
            ready("c_picked", "client", with(5, "minecraft:oak_planks*4")),
        );
        for label in ["c_far1", "c_far2"] {
            f.lines
                .insert(label.into(), event(&format!("label={label} feet=320,61,0")));
        }
        for (label, block) in [
            ("h2_diamond", "minecraft:diamond_block"),
            ("h2_gold", "air"),
            ("c2_diamond", "minecraft:diamond_block"),
        ] {
            f.lines
                .insert(label.into(), event(&format!("label={label} block={block}")));
        }
        for (key, value) in [
            ("c_gold", "minecraft:gold_block"),
            ("h_diamond", "minecraft:diamond_block"),
            ("c_gold_gone", "air"),
            ("planks_slot", "5"),
            ("h_plank", "minecraft:oak_planks"),
            ("h_plank_gone", "air"),
        ] {
            f.notes.insert(key, value.into());
        }
        let census = |label: &str, at: &str, health: u32| {
            event(&format!(
                "label={label} health={health} mobs=husk:41:{at}:20.0:alive"
            ))
        };
        for (label, at, health) in [
            ("c_husk", "323.00,61.00,0.00", 100),
            ("c_track", "321.20,61.00,0.30", 100),
            ("h_track", "321.50,61.00,0.20", 100),
            ("c_hurt", "321.20,61.00,0.30", 85),
        ] {
            f.lines.insert(label.into(), census(label, at, health));
        }
        for (key, value) in [
            ("husk_id", "41"),
            ("husk_killed", "true"),
            ("pig_killed", "true"),
            ("pig_drop", "ground"),
        ] {
            f.notes.insert(key, value.into());
        }
        f.mobnet.push(
            crate::runner::parse(
                "minecraft-mobnet: client=1 mobs=2 frame_bytes_per_s=420 row_bytes_per_s=12",
                "minecraft-mobnet:",
                0,
            )
            .unwrap(),
        );
        f
    }

    #[test]
    fn a_run_that_did_everything_passes() {
        for check in judge(&passing()) {
            assert!(
                check.failures.is_empty(),
                "{}: {:?}",
                check.label,
                check.failures
            );
        }
    }

    #[test]
    fn a_client_on_another_seed_fails() {
        let mut f = passing();
        f.lines
            .get_mut("c_ready")
            .unwrap()
            .fields
            .insert("seed".into(), "999".into());
        let check = judge(&f)
            .into_iter()
            .find(|c| c.label == "seed_shared")
            .unwrap();
        assert_eq!(check.failures, ["client seed: want 12345, got 999"]);
    }

    #[test]
    fn a_lost_inventory_after_the_restart_fails() {
        let mut f = passing();
        f.lines.insert(
            "c2_ready".into(),
            event("label=c2_ready seed=12345 role=client slots=-,-"),
        );
        let failed: Vec<_> = judge(&f)
            .into_iter()
            .filter(|c| !c.failures.is_empty())
            .map(|c| c.label)
            .collect();
        assert_eq!(failed, ["restart_client_inventory"]);
    }

    #[test]
    fn a_missing_answer_fails_its_checkpoint() {
        let mut f = passing();
        f.notes.remove("h_plank");
        let failed: Vec<_> = judge(&f)
            .into_iter()
            .filter(|c| !c.failures.is_empty())
            .map(|c| c.label)
            .collect();
        assert_eq!(failed, ["client_place_on_host"]);
    }

    #[test]
    fn a_mob_the_two_games_place_apart_fails() {
        let mut f = passing();
        f.lines.insert(
            "h_track".into(),
            event("label=h_track mobs=husk:41:330.00,61.00,0.00:20.0:alive"),
        );
        let failed: Vec<_> = judge(&f)
            .into_iter()
            .filter(|c| !c.failures.is_empty())
            .map(|c| c.label)
            .collect();
        assert_eq!(failed, ["mob_tracks_host_position"]);
    }

    #[test]
    fn a_client_never_hurt_or_a_mob_never_killed_fails() {
        let mut f = passing();
        f.lines
            .insert("c_hurt".into(), event("label=c_hurt health=100 mobs=-"));
        f.notes.insert("husk_killed", "false".into());
        f.notes.insert("pig_drop", "none".into());
        f.mobnet.clear();
        let failed: Vec<_> = judge(&f)
            .into_iter()
            .filter(|c| !c.failures.is_empty())
            .map(|c| c.label)
            .collect();
        assert_eq!(
            failed,
            [
                "mob_hurts_client",
                "client_kills_mob_host_sees",
                "mob_drop_reaches_client",
                "mob_states_sent"
            ]
        );
    }

    #[test]
    fn feet_read_from_the_state_line() {
        assert_eq!(feet(&event("label=x feet=-1,2,30")), Some([-1, 2, 30]));
        assert_eq!(feet(&event("label=x feet=-")), None);
        assert_eq!(set([1, -2, 3], "air"), "mc set 1 -2 3 1 -2 3 air");
    }
}
