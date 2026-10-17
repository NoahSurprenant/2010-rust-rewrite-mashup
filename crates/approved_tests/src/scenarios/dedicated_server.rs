//! `dedicated_server` — owner-approved. This file is the one place its
//! steps and what each checkpoint must show live.
//!
//! A dedicated server (`iw4l dedicated minecraft:overworld`) hosts a room on
//! a master of the run's own, with no player of its own. A client joins
//! that room, sets a block and fills its inventory, and leaves; the room
//! stays listed and the server keeps running with nobody in it, past the
//! master session's silence deadline, and meanwhile the server changes
//! 2048 blocks, more history than one control message holds. The client
//! joins again and finds its block, the last of those 2048, and its
//! inventory. Then the server is stopped with SIGTERM, as a
//! supervisor stops it, and started again: the world (seed, block) and the
//! client's inventory come back. Memory and CPU of the server are recorded
//! with no client and with one.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::duo::{Pair, Player, cpu_secs, field};
use crate::runner::Event;

pub const NAME: &str = "dedicated_server";
pub const MAP: &str = "minecraft:overworld";
/// The room's name on the master. The browser shows a room by its host
/// member's name, which the master cuts to 16 characters.
pub const ROOM: &str = "approved server";
/// The client's player name: the server keeps inventories by name.
pub const CLIENT: &str = "client";
pub const CLASSES: &str = crate::scenarios::crafting_table::CLASSES;
/// Loading a game and its world, on a CPU renderer.
pub const LOAD_SECS: u64 = 600;
pub const STEP_SECS: u64 = 60;
pub const SYNC_SECS: u64 = 30;
/// The room left empty: longer than a master session's 60 s without
/// authority progress, after which a stalled host is dropped.
pub const EMPTY_SECS: u64 = 75;
/// How long the server's CPU use is sampled at each point.
pub const CPU_SECS: u64 = 20;
/// SIGTERM to exit.
pub const STOP_SECS: u64 = 30;

/// A block the client sets, from the world's spawn block.
const GOLD: [i32; 3] = [2, 5, 0];
const PLANKS: &str = "minecraft:oak_planks*5";
/// A glass roof the server sets while the room is empty, from the world's
/// spawn block: 32 x 2 x 32 blocks, some 60 KB of edit history a joining
/// client is owed (a control message holds 16 KiB).
const ROOF: ([i32; 3], [i32; 3]) = ([-16, 30, -16], [15, 31, 15]);
const GLASS: &str = "minecraft:glass";

#[derive(Default)]
pub struct Facts {
    /// Each labelled line: `s*` the server's, `c*` the client's.
    pub lines: BTreeMap<String, Event>,
    /// The server's `dedicated-players:` lines, in order.
    pub rosters: Vec<Event>,
    pub notes: BTreeMap<&'static str, String>,
    /// Server memory and CPU, by point.
    pub measures: BTreeMap<&'static str, Measure>,
}

#[derive(Clone, Copy, Default)]
pub struct Measure {
    pub rss_mib: u64,
    pub peak_rss_mib: u64,
    /// Share of one core over the sample.
    pub cpu_percent: f64,
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

fn set(at: [i32; 3], block: &str) -> String {
    format!(
        "mc set {0} {1} {2} {0} {1} {2} {block}",
        at[0], at[1], at[2]
    )
}

fn client(pair: &mut Pair) -> Result<&mut Player, String> {
    pair.client.as_mut().ok_or_else(|| "no client".to_owned())
}

/// The server's memory now and its CPU use over `CPU_SECS`, the games
/// pumped meanwhile.
pub fn measure(pair: &mut Pair, facts: &mut Facts, point: &'static str) -> Result<(), String> {
    let pid = pair.host.pid();
    let (cpu, at) = (cpu_secs(pid), Instant::now());
    pair.until(
        &format!("{point} sample"),
        Duration::from_secs(CPU_SECS + 5),
        |_| at.elapsed() >= Duration::from_secs(CPU_SECS),
    )?;
    let used = cpu_secs(pid) - cpu;
    facts.measures.insert(
        point,
        Measure {
            rss_mib: crate::duo::rss_kib(pid) / 1024,
            peak_rss_mib: pair.host.peak_rss_kib / 1024,
            cpu_percent: used / at.elapsed().as_secs_f64() * 100.0,
        },
    );
    Ok(())
}

/// The server's room: up, in its match, and its world answering.
pub fn server_ready(pair: &mut Pair, facts: &mut Facts, label: &str) -> Result<String, String> {
    let load = Duration::from_secs(LOAD_SECS);
    pair.until("the server's room in its match", load, |pair| {
        let status = pair.host.status();
        field(&status, "state") == Some("hosting") && field(&status, "in_match") == Some("true")
    })?;
    let status = pair.host.status();
    let room = field(&status, "room")
        .ok_or("no room in the server's status")?
        .to_owned();
    let ready = pair.ask(
        false,
        &format!("wait minecraft; mc state {label}"),
        label,
        load,
    )?;
    facts.keep(label, ready);
    facts.notes.insert(
        match label {
            "s_ready" => "room1",
            _ => "room2",
        },
        room.clone(),
    );
    Ok(room)
}

/// The client in the server's room and on the world's spawn.
pub fn join(pair: &mut Pair, room: &str, label: &str) -> Result<Event, String> {
    let load = Duration::from_secs(LOAD_SECS);
    client(pair)?.send(&format!(
        "set ui_mapname {MAP}; set ui_gametype dm; ui_join_lobby_id {room}"
    ))?;
    pair.until("the client in the room", load, |pair| {
        pair.client
            .as_ref()
            .is_some_and(|c| field(&c.status(), "state") == Some("joined"))
    })?;
    client(pair)?.send("wait world; spawn 0; wait minecraft; wait 2s")?;
    pair.ask(
        true,
        &format!("wait minecraft; mc state {label}"),
        label,
        load,
    )
}

/// The first visit: the client sets a block and fills its inventory, and
/// the server sees the block.
pub fn first_visit(pair: &mut Pair, facts: &mut Facts, room: &str) -> Result<(), String> {
    let step = Duration::from_secs(STEP_SECS);
    let ready = join(pair, room, "c_ready")?;
    facts.keep("c_ready", ready);
    let given = pair.ask(
        true,
        &format!(
            "{}; mc clear; mc give oak_planks 5; wait 2s; mc state c_given",
            set(GOLD, "gold_block")
        ),
        "c_given",
        step,
    )?;
    facts.keep("c_given", given);
    let gold = pair.poll_block(
        false,
        "s_gold",
        GOLD,
        "minecraft:gold_block",
        Duration::from_secs(SYNC_SECS),
    )?;
    facts.notes.insert("s_gold", gold);
    Ok(())
}

/// The client leaves through the normal exit and the server's room stays.
pub fn leave(pair: &mut Pair) -> Result<(), String> {
    client(pair)?.send("wait 3s; finish_run")?;
    pair.until("the client to exit", Duration::from_secs(120), |pair| {
        pair.client.as_mut().is_none_or(|c| c.exited().is_some())
    })
    .or_else(|e| {
        if e.starts_with("client exited") {
            Ok(())
        } else {
            Err(e)
        }
    })?;
    pair.client = None;
    Ok(())
}

/// The room with nobody in it, for `EMPTY_SECS`: still hosting, still
/// listed on the master, and its world still answering.
pub fn empty_room(
    pair: &mut Pair,
    facts: &mut Facts,
    list: impl Fn() -> Result<String, String>,
) -> Result<(), String> {
    let start = Instant::now();
    measure(pair, facts, "empty_after_leave")?;
    let rest = Duration::from_secs(EMPTY_SECS).saturating_sub(start.elapsed());
    pair.until(
        "the empty room's time",
        rest + Duration::from_secs(5),
        |_| start.elapsed() >= Duration::from_secs(EMPTY_SECS),
    )?;
    let status = pair.host.status();
    for key in ["state", "room", "members", "in_match"] {
        facts.notes.insert(
            match key {
                "state" => "empty_state",
                "room" => "empty_room",
                "members" => "empty_members",
                _ => "empty_in_match",
            },
            field(&status, key).unwrap_or("-").to_owned(),
        );
    }
    facts.notes.insert("empty_list", list()?);
    let answer = pair.ask(
        false,
        "mc state s_empty",
        "s_empty",
        Duration::from_secs(STEP_SECS),
    )?;
    facts.keep("s_empty", answer);
    let ([x1, y1, z1], [x2, y2, z2]) = ROOF;
    pair.host
        .send(&format!("mc set {x1} {y1} {z1} {x2} {y2} {z2} glass"))?;
    let roof = pair.poll_block(
        false,
        "s_roof",
        ROOF.1,
        GLASS,
        Duration::from_secs(SYNC_SECS),
    )?;
    facts.notes.insert("s_roof", roof);
    Ok(())
}

/// Back in the room: the client sees its block and has its inventory.
pub fn revisit(pair: &mut Pair, facts: &mut Facts, room: &str, n: u32) -> Result<(), String> {
    let (ready, gold, roof) = match n {
        2 => ("c2_ready", "c2_gold", "c2_roof"),
        _ => ("c3_ready", "c3_gold", "c3_roof"),
    };
    let event = join(pair, room, ready)?;
    facts.keep(ready, event);
    let block = pair.poll_block(
        true,
        gold,
        GOLD,
        "minecraft:gold_block",
        Duration::from_secs(SYNC_SECS),
    )?;
    facts.notes.insert(gold, block);
    let block = pair.poll_block(true, roof, ROOF.1, GLASS, Duration::from_secs(SYNC_SECS))?;
    facts.notes.insert(roof, block);
    Ok(())
}

/// SIGTERM, and the server's own exit within `STOP_SECS`.
pub fn stop(pair: &mut Pair, facts: &mut Facts, label: &'static str) -> Result<(), String> {
    pair.host.terminate()?;
    let start = Instant::now();
    loop {
        pair.host.pump();
        if let Some(status) = pair.host.exited() {
            pair.host.pump();
            facts.notes.insert(
                label,
                format!(
                    "code={} secs={:.1}",
                    status
                        .code()
                        .map_or_else(|| "signal".to_owned(), |c| c.to_string()),
                    start.elapsed().as_secs_f64()
                ),
            );
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(STOP_SECS) {
            return Err(format!(
                "the server did not exit {STOP_SECS}s after SIGTERM"
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

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

fn want(got: &str, want: &str, what: &str) -> Result<(), String> {
    if got == want {
        Ok(())
    } else {
        Err(format!("{what}: want {want}, got {got}"))
    }
}

fn has_planks(facts: &Facts, label: &str) -> Result<(), String> {
    let slots = slots(line(facts, label)?);
    if slots.iter().any(|s| s == PLANKS) {
        Ok(())
    } else {
        Err(format!("{label}: no {PLANKS} in {}", slots.join(",")))
    }
}

/// A `stop` note: exit code 0, in time.
fn clean_stop(facts: &Facts, key: &str) -> Result<(), String> {
    let stopped = note(facts, key)?;
    if stopped.starts_with("code=0 ") {
        Ok(())
    } else {
        Err(format!("{key}: {stopped}"))
    }
}

pub fn judge(facts: &Facts) -> Vec<Check> {
    type Rule = fn(&Facts) -> Result<(), String>;
    let rules: [(&'static str, Rule); 9] = [
        ("server_has_no_player", |f| {
            let first = f.rosters.first().ok_or("no roster line from the server")?;
            want(
                first.get("count").unwrap_or("-"),
                "0",
                "players before anyone joined",
            )?;
            for roster in &f.rosters {
                let clients = roster.get("clients").unwrap_or("");
                if clients.split(',').any(|id| id == "0") {
                    return Err(format!(
                        "client 0 (the server's own) in the match: {clients}"
                    ));
                }
                let names = roster.get("names").unwrap_or("");
                if names
                    .split(',')
                    .any(|name| !name.is_empty() && name != CLIENT)
                {
                    return Err(format!("a player other than `{CLIENT}`: {names}"));
                }
            }
            if !f.rosters.iter().any(|r| r.get("names") == Some(CLIENT)) {
                return Err("the client never showed in the match".into());
            }
            want(
                f.rosters.last().and_then(|r| r.get("count")).unwrap_or("-"),
                "0",
                "players after the client left",
            )
        }),
        ("seed_shared", |f| {
            let server = line(f, "s_ready")?;
            want(server.get("role").unwrap_or(""), "dedicated", "server role")?;
            let client = line(f, "c_ready")?;
            want(client.get("role").unwrap_or(""), "client", "client role")?;
            want(
                client.get("seed").unwrap_or("-"),
                server.get("seed").unwrap_or("-"),
                "client seed",
            )
        }),
        ("client_block_on_server", |f| {
            want(note(f, "s_gold")?, "minecraft:gold_block", "server sees")
        }),
        ("client_inventory", |f| has_planks(f, "c_given")),
        ("room_kept_empty", |f| {
            want(note(f, "empty_state")?, "hosting", "state")?;
            want(note(f, "empty_room")?, note(f, "room1")?, "room")?;
            want(note(f, "empty_members")?, "1", "members (the server alone)")?;
            want(note(f, "empty_in_match")?, "true", "in a match")?;
            let room = note(f, "room1")?;
            let listed = note(f, "empty_list")?
                .lines()
                .any(|row| row.starts_with(room) && row.ends_with(ROOM));
            if !listed {
                return Err(format!("room {room} `{ROOM}` not in the master's list"));
            }
            want(
                line(f, "s_empty")?.get("role").unwrap_or(""),
                "dedicated",
                "server answers",
            )?;
            want(note(f, "s_roof")?, GLASS, "server roof")
        }),
        ("rejoin_world_kept", |f| {
            want(note(f, "c2_gold")?, "minecraft:gold_block", "client sees")?;
            want(note(f, "c2_roof")?, GLASS, "client sees the roof")?;
            has_planks(f, "c2_ready")
        }),
        ("sigterm_saves_and_exits", |f| clean_stop(f, "stop1")),
        ("restart_world_kept", |f| {
            want(
                line(f, "s2_ready")?.get("seed").unwrap_or("-"),
                line(f, "s_ready")?.get("seed").unwrap_or("-"),
                "server seed after the restart",
            )?;
            want(
                line(f, "c3_ready")?.get("seed").unwrap_or("-"),
                line(f, "s_ready")?.get("seed").unwrap_or("-"),
                "client seed after the restart",
            )?;
            want(note(f, "c3_gold")?, "minecraft:gold_block", "client sees")?;
            want(note(f, "c3_roof")?, GLASS, "client sees the roof")?;
            has_planks(f, "c3_ready")
        }),
        ("second_sigterm", |f| clean_stop(f, "stop2")),
    ];
    rules
        .iter()
        .map(|(label, rule)| Check {
            label,
            failures: rule(facts).err().into_iter().collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(prefix: &str, fields: &str) -> Event {
        crate::runner::parse(&format!("{prefix} {fields}"), prefix, 0).unwrap()
    }

    fn state(fields: &str) -> Event {
        event("minecraft-state:", fields)
    }

    fn passing() -> Facts {
        let mut f = Facts::default();
        let mut slots = vec!["-"; 41];
        slots[..5].copy_from_slice(&["iw4:weapon/1*1"; 5]);
        slots[5] = PLANKS;
        let slots = slots.join(",");
        f.lines.insert(
            "s_ready".into(),
            state("label=s_ready seed=7 role=dedicated slots=-"),
        );
        f.lines.insert(
            "s2_ready".into(),
            state("label=s2_ready seed=7 role=dedicated slots=-"),
        );
        f.lines.insert(
            "s_empty".into(),
            state("label=s_empty seed=7 role=dedicated slots=-"),
        );
        f.lines.insert(
            "c_ready".into(),
            state("label=c_ready seed=7 role=client slots=-"),
        );
        for label in ["c_given", "c2_ready", "c3_ready"] {
            f.lines.insert(
                label.into(),
                state(&format!("label={label} seed=7 role=client slots={slots}")),
            );
        }
        for roster in [
            "count=0 clients= names=",
            "count=1 clients=1 names=client",
            "count=0 clients= names=",
        ] {
            f.rosters.push(event("dedicated-players:", roster));
        }
        for (key, value) in [
            ("room1", "ab12"),
            ("room2", "cd34"),
            ("s_gold", "minecraft:gold_block"),
            ("c2_gold", "minecraft:gold_block"),
            ("c3_gold", "minecraft:gold_block"),
            ("s_roof", GLASS),
            ("c2_roof", GLASS),
            ("c3_roof", GLASS),
            ("empty_state", "hosting"),
            ("empty_room", "ab12"),
            ("empty_members", "1"),
            ("empty_in_match", "true"),
            (
                "empty_list",
                "generation=3 adverts=1\nab12\t1/18\tminecraft:overworld\tdm\tlocked=0\tin_match=1\tgen=3\trequires=0x01\tapproved server",
            ),
            ("stop1", "code=0 secs=0.4"),
            ("stop2", "code=0 secs=0.3"),
        ] {
            f.notes.insert(key, value.into());
        }
        f
    }

    fn failed(f: &Facts) -> Vec<&'static str> {
        judge(f)
            .into_iter()
            .filter(|c| !c.failures.is_empty())
            .map(|c| c.label)
            .collect()
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
    fn the_server_as_a_player_fails() {
        let mut f = passing();
        f.rosters[1] = event(
            "dedicated-players:",
            "count=2 clients=0,1 names=server,client",
        );
        assert_eq!(failed(&f), ["server_has_no_player"]);
    }

    #[test]
    fn a_room_gone_from_the_list_fails() {
        let mut f = passing();
        f.notes
            .insert("empty_list", "generation=4 adverts=0".into());
        assert_eq!(failed(&f), ["room_kept_empty"]);
    }

    #[test]
    fn an_inventory_lost_on_restart_fails() {
        let mut f = passing();
        f.lines.insert(
            "c3_ready".into(),
            state("label=c3_ready seed=7 role=client slots=-,-"),
        );
        assert_eq!(failed(&f), ["restart_world_kept"]);
    }

    #[test]
    fn a_rejoin_short_of_the_long_history_fails() {
        let mut f = passing();
        f.notes.insert("c2_roof", "air".into());
        assert_eq!(failed(&f), ["rejoin_world_kept"]);
    }

    #[test]
    fn a_server_killed_rather_than_stopped_fails() {
        let mut f = passing();
        f.notes.insert("stop1", "code=signal secs=30.0".into());
        assert_eq!(failed(&f), ["sigterm_saves_and_exits"]);
    }
}
