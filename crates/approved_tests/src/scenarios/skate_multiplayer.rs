//! `skate_multiplayer` — owner-approved. This file is the one place its
//! steps and what each checkpoint must show live.
//!
//! Skating with other players on the Minecraft map, through a master of the
//! run's own, both ways a room is hosted:
//!
//! * A listen host and a client, both with the Skate 3 files. On a stone
//!   runway high over the world, the client (not the host) gets on its
//!   board and pushes off: the host must have the client's player moving
//!   down the runway in its simulation, on its board, and draw the client's
//!   skater pose (every bone) in place of its MW2 body. The client gets off
//!   and the host sees it stop skating. Then the host skates and the client
//!   receives the host's pose over the network.
//! * A dedicated server (`iw4l dedicated`, no player of its own and no
//!   Skate 3 files) and a client with them: the client skates and the
//!   server moves its player down the runway and has it on its board.
//!
//! Each skater's own game runs the skate simulation and sends its state; the
//! host checks it and passes it on (crates/net/src/skate.rs). The bytes
//! each skater sent and the host passed on are recorded.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::duo::Pair;
use crate::runner::Event;

pub const NAME: &str = "skate_multiplayer";
pub const MAP: &str = "minecraft:overworld";
pub const CLASSES: &str = crate::scenarios::crafting_table::CLASSES;
/// The dedicated server's room on the master (16 characters at most).
pub const ROOM: &str = "approved skate";
pub const CLIENT: &str = "client";
/// Loading a game and its world, on a CPU renderer.
pub const LOAD_SECS: u64 = 600;
pub const STEP_SECS: u64 = 60;
/// The skate session's preparation for the map, after the world loads.
pub const READY_SECS: u64 = 300;
/// How long a skater pushes before the others are asked.
pub const PUSH_SECS: u64 = 4;
/// A skater's movement reaching the host.
pub const SYNC_SECS: u64 = 20;
/// How far down the runway (map units) a skater must have gone: about 4 m,
/// half what pushing for `PUSH_SECS` covers.
pub const MOVED_UNITS: f64 = 160.0;

/// A stone runway high over the world (blocks from the world's spawn block):
/// 120 blocks along +X, 13 across, nothing above it.
const RUNWAY: ([i32; 3], [i32; 3]) = ([600, 150, -6], [719, 150, 6]);
/// Where each player stands on it, facing down it (+X, yaw 0).
const SKATER_START: [i32; 3] = [604, 150, 0];
const WATCHER_START: [i32; 3] = [600, 150, -5];

/// The map point just over block `at`'s floor (36 units a block).
fn map_over(at: [i32; 3]) -> [i32; 3] {
    [at[0] * 36, -at[2] * 36, (at[1] + 1) * 36 + 18]
}

fn tp(at: [i32; 3]) -> String {
    let [x, y, z] = map_over(at);
    format!("tp {x} {y} {z} 0 0")
}

fn runway() -> String {
    let ([x1, y1, z1], [x2, y2, z2]) = RUNWAY;
    format!("mc set {x1} {y1} {z1} {x2} {y2} {z2} stone")
}

#[derive(Default)]
pub struct Facts {
    /// Each `skate-state:` line, by label: `h_` the listen host's, `c_` its
    /// client's, `s_` the dedicated server's, `d_` the dedicated server's
    /// client's.
    pub lines: BTreeMap<String, Event>,
    pub notes: BTreeMap<&'static str, String>,
}

impl Facts {
    fn keep(&mut self, label: &str, event: Event) -> Event {
        self.lines.insert(label.to_owned(), event.clone());
        event
    }
}

/// `skate state` from one game.
fn state(pair: &mut Pair, client: bool, label: &str) -> Result<Event, String> {
    pair.ask(
        client,
        &format!("skate state {label}"),
        label,
        Duration::from_secs(STEP_SECS),
    )
}

/// Asks until `want` holds or `timeout` passes; the last answer.
fn poll(
    pair: &mut Pair,
    client: bool,
    label: &str,
    timeout: Duration,
    want: impl Fn(&Event) -> bool,
) -> Result<Event, String> {
    let start = Instant::now();
    loop {
        let event = state(pair, client, label)?;
        if want(&event) || start.elapsed() > timeout {
            return Ok(event);
        }
        let at = Instant::now();
        pair.until("the next skate state", Duration::from_secs(5), |_| {
            at.elapsed() >= Duration::from_millis(500)
        })?;
    }
}

fn triple(text: &str) -> Option<[f64; 3]> {
    let v: Vec<f64> = text.split(',').filter_map(|v| v.parse().ok()).collect();
    (v.len() == 3).then(|| [v[0], v[1], v[2]])
}

/// A player as a host's `peers=` lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Peer {
    pub client: u32,
    pub origin: [f64; 3],
    pub skating: bool,
    pub accepted: u64,
    pub rejected: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

pub fn peers(event: &Event) -> Vec<Peer> {
    event
        .get("peers")
        .unwrap_or("-")
        .split('|')
        .filter_map(|entry| {
            let p: Vec<&str> = entry.split(':').collect();
            let [
                client,
                origin,
                skating,
                accepted,
                rejected,
                bytes_in,
                bytes_out,
            ] = p.as_slice()
            else {
                return None;
            };
            Some(Peer {
                client: client.parse().ok()?,
                origin: triple(origin)?,
                skating: *skating == "1",
                accepted: accepted.parse().ok()?,
                rejected: rejected.parse().ok()?,
                bytes_in: bytes_in.parse().ok()?,
                bytes_out: bytes_out.parse().ok()?,
            })
        })
        .collect()
}

/// A skater as a game's `remote=` lists it (one it draws).
#[derive(Clone, Debug, PartialEq)]
pub struct Remote {
    pub client: u32,
    pub at: [f64; 3],
    pub bones: usize,
    pub received: u64,
    pub drawn: u64,
    pub label: String,
}

pub fn remotes(event: &Event) -> Vec<Remote> {
    event
        .get("remote")
        .unwrap_or("-")
        .split('|')
        .filter_map(|entry| {
            let p: Vec<&str> = entry.split(':').collect();
            let [client, at, bones, received, drawn, label] = p.as_slice() else {
                return None;
            };
            Some(Remote {
                client: client.parse().ok()?,
                at: triple(at)?,
                bones: bones.parse().ok()?,
                received: received.parse().ok()?,
                drawn: drawn.parse().ok()?,
                label: (*label).to_owned(),
            })
        })
        .collect()
}

fn number(event: &Event, key: &str) -> f64 {
    event
        .get(key)
        .and_then(|v| v.parse().ok())
        .unwrap_or(f64::NAN)
}

fn client_id(event: &Event) -> Option<u32> {
    event.get("client")?.parse().ok()
}

fn peer(event: &Event, client: u32) -> Option<Peer> {
    peers(event).into_iter().find(|p| p.client == client)
}

fn remote(event: &Event, client: u32) -> Option<Remote> {
    remotes(event).into_iter().find(|r| r.client == client)
}

fn active(event: &Event) -> bool {
    event.get("active") == Some("1")
}

fn ready(event: &Event) -> bool {
    event.get("ready") == Some("1")
}

/// Along the runway (+X) from where `a` was to `b`.
fn along(a: [f64; 3], b: [f64; 3]) -> f64 {
    b[0] - a[0]
}

/// One game on the runway, its skate session prepared: `prefix` names its
/// lines.
fn to_runway(
    pair: &mut Pair,
    client: bool,
    facts: &mut Facts,
    at: [i32; 3],
    label: &str,
) -> Result<Event, String> {
    let player = if client {
        pair.client.as_mut().ok_or("no client")?
    } else {
        &mut pair.host
    };
    player.send(&format!("{}; wait 6s", tp(at)))?;
    let event = poll(pair, client, label, Duration::from_secs(READY_SECS), ready)?;
    let event = facts.keep(label, event);
    if !ready(&event) {
        return Err(format!(
            "`{label}`: the skate session never got ready: {}",
            event.get("sim").unwrap_or("-")
        ));
    }
    Ok(event)
}

/// A skater gets on and pushes for `PUSH_SECS`; its line before, and once
/// pushing.
fn skate_on(
    pair: &mut Pair,
    client: bool,
    facts: &mut Facts,
    on: &str,
    pushing: &str,
) -> Result<Event, String> {
    let player = if client {
        pair.client.as_mut().ok_or("no client")?
    } else {
        &mut pair.host
    };
    player.send("skate pad push; skate on")?;
    let event = poll(pair, client, on, Duration::from_secs(STEP_SECS), active)?;
    let event = facts.keep(on, event);
    if !active(&event) {
        return Err(format!(
            "`{on}`: never got on the board: {}",
            event.get("sim").unwrap_or("-")
        ));
    }
    let at = Instant::now();
    pair.until("pushing", Duration::from_secs(PUSH_SECS + 5), |_| {
        at.elapsed() >= Duration::from_secs(PUSH_SECS)
    })?;
    let event = state(pair, client, pushing)?;
    Ok(facts.keep(pushing, event))
}

fn skate_off(pair: &mut Pair, client: bool, facts: &mut Facts, label: &str) -> Result<(), String> {
    let player = if client {
        pair.client.as_mut().ok_or("no client")?
    } else {
        &mut pair.host
    };
    player.send("skate pad stop; skate off")?;
    let event = poll(pair, client, label, Duration::from_secs(STEP_SECS), |e| {
        !active(e)
    })?;
    facts.keep(label, event);
    Ok(())
}

/// The listen host's room: the client skates while the host watches, then
/// the other way round.
pub fn listen_session(pair: &mut Pair, facts: &mut Facts) -> Result<(), String> {
    let load = Duration::from_secs(LOAD_SECS);
    let sync = Duration::from_secs(SYNC_SECS);
    pair.host
        .send("wait world; spawn 0; force_match_start; wait minecraft; wait 1s")?;
    pair.client
        .as_mut()
        .ok_or("no client")?
        .send("wait world; spawn 0; wait minecraft; wait 2s")?;
    let host = pair.ask(
        false,
        "wait minecraft; skate state h_ready",
        "h_ready",
        load,
    )?;
    facts.keep("h_ready", host);
    let client = pair.ask(true, "wait minecraft; skate state c_ready", "c_ready", load)?;
    let client = facts.keep("c_ready", client);
    let client_id = client_id(&client).ok_or("c_ready names no client")?;
    facts.notes.insert("client_id", client_id.to_string());
    pair.host.send(&format!("{}; wait 3s", runway()))?;
    to_runway(pair, true, facts, SKATER_START, "c_runway")?;
    to_runway(pair, false, facts, WATCHER_START, "h_runway")?;
    let before = state(pair, false, "h_before")?;
    facts.keep("h_before", before);

    // The client skates; the host watches.
    skate_on(pair, true, facts, "c_on", "c_pushing")?;
    let start = facts
        .lines
        .get("h_before")
        .and_then(|e| peer(e, client_id))
        .map(|p| p.origin)
        .ok_or("h_before lists no client")?;
    let seen = poll(pair, false, "h_seen", sync, |e| {
        peer(e, client_id).is_some_and(|p| p.skating && along(start, p.origin) > MOVED_UNITS)
            && remote(e, client_id).is_some_and(|r| r.drawn > 0)
    })?;
    facts.keep("h_seen", seen);
    let pushed = state(pair, true, "c_pushed")?;
    facts.keep("c_pushed", pushed);
    skate_off(pair, true, facts, "c_off")?;
    let stopped = poll(pair, false, "h_stopped", sync, |e| {
        peer(e, client_id).is_some_and(|p| !p.skating) && remote(e, client_id).is_none()
    })?;
    facts.keep("h_stopped", stopped);

    // The host skates; the client receives its pose over the network.
    let host_id = facts
        .lines
        .get("h_ready")
        .and_then(client_id_of)
        .ok_or("h_ready names no client")?;
    facts.notes.insert("host_id", host_id.to_string());
    to_runway(pair, false, facts, SKATER_START, "h_runway2")?;
    skate_on(pair, false, facts, "h_on", "h_pushing")?;
    let watched = poll(pair, true, "c_watched", sync, |e| {
        remote(e, host_id).is_some_and(|r| r.bones == 35 && r.received > 10)
    })?;
    facts.keep("c_watched", watched);
    skate_off(pair, false, facts, "h_off")?;
    let after = poll(pair, true, "c_after", sync, |e| {
        remote(e, host_id).is_none()
    })?;
    facts.keep("c_after", after);
    Ok(())
}

fn client_id_of(event: &Event) -> Option<u32> {
    client_id(event)
}

/// The dedicated server's room: the client skates with no player of the
/// server's own about.
pub fn dedicated_session(pair: &mut Pair, facts: &mut Facts, room: &str) -> Result<(), String> {
    let load = Duration::from_secs(LOAD_SECS);
    let sync = Duration::from_secs(SYNC_SECS);
    let joined = crate::scenarios::dedicated_server::join(pair, room, "d_join")?;
    facts.keep("d_join", joined);
    let client = pair.ask(true, "wait minecraft; skate state d_ready", "d_ready", load)?;
    let client = facts.keep("d_ready", client);
    let client_id = client_id(&client).ok_or("d_ready names no client")?;
    facts.notes.insert("d_client_id", client_id.to_string());
    pair.host.send(&format!("{}; wait 3s", runway()))?;
    to_runway(pair, true, facts, SKATER_START, "d_runway")?;
    let before = state(pair, false, "s_before")?;
    facts.keep("s_before", before);
    let start = facts
        .lines
        .get("s_before")
        .and_then(|e| peer(e, client_id))
        .map(|p| p.origin)
        .ok_or("s_before lists no client")?;
    skate_on(pair, true, facts, "d_on", "d_pushing")?;
    let seen = poll(pair, false, "s_seen", sync, |e| {
        peer(e, client_id).is_some_and(|p| p.skating && along(start, p.origin) > MOVED_UNITS)
    })?;
    facts.keep("s_seen", seen);
    let pushed = state(pair, true, "d_pushed")?;
    facts.keep("d_pushed", pushed);
    skate_off(pair, true, facts, "d_off")?;
    let stopped = poll(pair, false, "s_stopped", sync, |e| {
        peer(e, client_id).is_some_and(|p| !p.skating)
    })?;
    facts.keep("s_stopped", stopped);
    Ok(())
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

fn id(facts: &Facts, key: &str) -> Result<u32, String> {
    note(facts, key)?
        .parse()
        .map_err(|_| format!("bad `{key}`"))
}

/// The host's player for `client` moved down the runway from `before` to
/// `seen`, on its board, its states taken.
fn host_moved(facts: &Facts, before: &str, seen: &str, client: u32) -> Result<(), String> {
    let start = peer(line(facts, before)?, client)
        .ok_or_else(|| format!("`{before}` lists no client {client}"))?;
    let now = peer(line(facts, seen)?, client)
        .ok_or_else(|| format!("`{seen}` lists no client {client}"))?;
    if !now.skating {
        return Err(format!("`{seen}`: client {client} not on its board"));
    }
    let moved = along(start.origin, now.origin);
    if moved <= MOVED_UNITS {
        return Err(format!(
            "`{seen}`: client {client} went {moved:.0} units down the runway, under {MOVED_UNITS}"
        ));
    }
    if now.accepted < 20 {
        return Err(format!("`{seen}`: only {} states taken", now.accepted));
    }
    Ok(())
}

/// A skater's own line says it was on its board and moving, its states
/// going out with little quantizing error.
fn skater_sent(facts: &Facts, label: &str) -> Result<(), String> {
    let event = line(facts, label)?;
    if !active(event) {
        return Err(format!("`{label}`: not on the board"));
    }
    if !(number(event, "speed") > 100.0) {
        return Err(format!(
            "`{label}`: speed {} (units/s)",
            event.get("speed").unwrap_or("-")
        ));
    }
    if !(number(event, "sent") >= 20.0) {
        return Err(format!(
            "`{label}`: {} states sent",
            event.get("sent").unwrap_or("-")
        ));
    }
    let err = number(event, "pose_err_mm");
    if !(err < 2.0) {
        return Err(format!("`{label}`: pose quantizing error {err} mm"));
    }
    Ok(())
}

pub fn judge(facts: &Facts) -> Vec<Check> {
    type Rule = fn(&Facts) -> Result<(), String>;
    let rules: [(&'static str, Rule); 9] = [
        ("client_skates", |f| skater_sent(f, "c_pushing")),
        ("host_moves_skating_client", |f| {
            host_moved(f, "h_before", "h_seen", id(f, "client_id")?)
        }),
        ("host_draws_client_pose", |f| {
            let client = id(f, "client_id")?;
            let seen = remote(line(f, "h_seen")?, client)
                .ok_or("`h_seen`: the host draws no skater for the client")?;
            if seen.bones != 35 {
                return Err(format!("`h_seen`: {} bones", seen.bones));
            }
            if seen.drawn == 0 {
                return Err("`h_seen`: the skater was never drawn".into());
            }
            Ok(())
        }),
        ("client_gets_off", |f| {
            let client = id(f, "client_id")?;
            if active(line(f, "c_off")?) {
                return Err("`c_off`: still on the board".into());
            }
            let host = line(f, "h_stopped")?;
            if peer(host, client).is_none_or(|p| p.skating) {
                return Err("`h_stopped`: the host still has the client skating".into());
            }
            if remote(host, client).is_some() {
                return Err("`h_stopped`: the host still draws the client's skater".into());
            }
            Ok(())
        }),
        ("host_skates", |f| skater_sent(f, "h_pushing")),
        ("client_receives_host_pose", |f| {
            let host = id(f, "host_id")?;
            let watched = remote(line(f, "c_watched")?, host)
                .ok_or("`c_watched`: the client has no skater for the host")?;
            if watched.bones != 35 || watched.received <= 10 {
                return Err(format!(
                    "`c_watched`: {} bones, {} states",
                    watched.bones, watched.received
                ));
            }
            if !(number(line(f, "c_watched")?, "received_bytes") > 0.0) {
                return Err("`c_watched`: no skater bytes in its snapshots".into());
            }
            if remote(line(f, "c_after")?, host).is_some() {
                return Err("`c_after`: the host's skater still drawn after it got off".into());
            }
            Ok(())
        }),
        ("dedicated_client_skates", |f| skater_sent(f, "d_pushing")),
        ("dedicated_moves_skating_client", |f| {
            let event = line(f, "s_seen")?;
            if event.get("role") != Some("dedicated") {
                return Err("`s_seen` is not from a dedicated server".into());
            }
            if active(event) || ready(event) {
                return Err("the dedicated server skates or prepared a skate session".into());
            }
            host_moved(f, "s_before", "s_seen", id(f, "d_client_id")?)
        }),
        ("dedicated_client_gets_off", |f| {
            let client = id(f, "d_client_id")?;
            if peer(line(f, "s_stopped")?, client).is_none_or(|p| p.skating) {
                return Err("`s_stopped`: the server still has the client skating".into());
            }
            Ok(())
        }),
    ];
    rules
        .iter()
        .map(|(label, rule)| Check {
            label,
            failures: rule(facts).err().into_iter().collect(),
        })
        .collect()
}

/// Bytes a second each way, from the lines' counters and times.
pub fn bandwidth(facts: &Facts) -> BTreeMap<&'static str, f64> {
    let mut out = BTreeMap::new();
    let rate = |a: &Event, b: &Event, value: &dyn Fn(&Event) -> Option<f64>| {
        let secs = (b.at_ms as f64 - a.at_ms as f64) / 1000.0;
        Some((value(b)? - value(a)?) / secs).filter(|r| r.is_finite() && secs > 0.5)
    };
    let sent = |e: &Event| e.get("sent_bytes").and_then(|v| v.parse().ok());
    for (key, a, b) in [
        ("client_uplink_bytes_per_s", "c_on", "c_pushed"),
        ("host_skater_bytes_per_s", "h_on", "h_pushing"),
        ("dedicated_client_uplink_bytes_per_s", "d_on", "d_pushed"),
    ] {
        if let (Some(a), Some(b)) = (facts.lines.get(a), facts.lines.get(b))
            && let Some(r) = rate(a, b, &sent)
        {
            out.insert(key, r);
        }
    }
    let received = |e: &Event| e.get("received_bytes").and_then(|v| v.parse().ok());
    if let (Some(a), Some(b)) = (facts.lines.get("c_runway"), facts.lines.get("c_watched"))
        && let Some(r) = rate(a, b, &received)
    {
        out.insert("client_downlink_bytes_per_s_average", r);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(fields: &str) -> Event {
        crate::runner::parse(&format!("skate-state: {fields}"), "skate-state:", 0).unwrap()
    }

    fn at(fields: &str, at_ms: u128) -> Event {
        crate::runner::parse(&format!("skate-state: {fields}"), "skate-state:", at_ms).unwrap()
    }

    fn passing() -> Facts {
        let mut f = Facts::default();
        let skating = "active=1 ready=1 speed=320 sent=80 sent_bytes=32000 pose_err_mm=0.4";
        for (label, fields) in [
            (
                "h_ready",
                "role=listen client=0 active=0 ready=1 peers=0:0,0,0:0:0:0:0:0|1:10,0,0:0:0:0:0:0",
            ),
            ("c_ready", "role=client client=1 active=0 ready=0"),
            (
                "h_before",
                "role=listen client=0 peers=0:21600,180,5454:0:0:0:0:0|1:21744,0,5454:0:0:0:0:0",
            ),
            ("c_pushing", skating),
            (
                "h_seen",
                "role=listen client=0 remote=1:22400,0,5454:35:60:40:PhysicsGround peers=0:21600,180,5454:0:0:0:0:0|1:22400,0,5454:1:60:0:23000:0",
            ),
            ("c_off", "role=client client=1 active=0 ready=1"),
            (
                "h_stopped",
                "role=listen client=0 remote=- peers=1:22900,0,5454:0:90:0:35000:0",
            ),
            ("h_pushing", skating),
            (
                "c_watched",
                "role=client client=1 received_bytes=20000 remote=0:22000,0,5454:35:40:30:PhysicsGround",
            ),
            (
                "c_after",
                "role=client client=1 received_bytes=21000 remote=-",
            ),
            ("d_pushing", skating),
            (
                "s_before",
                "role=dedicated client=0 active=0 ready=0 peers=1:21744,0,5454:0:0:0:0:0",
            ),
            (
                "s_seen",
                "role=dedicated client=0 active=0 ready=0 peers=1:22300,0,5454:1:50:0:20000:0",
            ),
            (
                "s_stopped",
                "role=dedicated client=0 active=0 ready=0 peers=1:22600,0,5454:0:70:0:28000:0",
            ),
        ] {
            f.lines.insert(label.into(), event(fields));
        }
        f.notes.insert("client_id", "1".into());
        f.notes.insert("host_id", "0".into());
        f.notes.insert("d_client_id", "1".into());
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
    fn a_host_that_leaves_the_skater_standing_fails() {
        let mut f = passing();
        f.lines.insert(
            "h_seen".into(),
            event(
                "remote=1:21744,0,5454:35:60:40:PhysicsGround peers=1:21744,0,5454:1:60:0:23000:0",
            ),
        );
        assert_eq!(failed(&f), ["host_moves_skating_client"]);
    }

    #[test]
    fn a_host_that_draws_the_mw2_body_fails() {
        let mut f = passing();
        f.lines.insert(
            "h_seen".into(),
            event(
                "remote=1:22400,0,5454:35:60:0:PhysicsGround peers=1:22400,0,5454:1:60:0:23000:0",
            ),
        );
        assert_eq!(failed(&f), ["host_draws_client_pose"]);
    }

    #[test]
    fn a_skater_left_on_its_board_fails() {
        let mut f = passing();
        f.lines.insert(
            "h_stopped".into(),
            event("remote=- peers=1:22900,0,5454:1:90:0:35000:0"),
        );
        f.lines.insert(
            "s_stopped".into(),
            event("role=dedicated peers=1:22600,0,5454:1:70:0:28000:0"),
        );
        assert_eq!(failed(&f), ["client_gets_off", "dedicated_client_gets_off"]);
    }

    #[test]
    fn a_client_that_never_hears_of_the_host_skating_fails() {
        let mut f = passing();
        f.lines.insert(
            "c_watched".into(),
            event("role=client received_bytes=0 remote=-"),
        );
        assert_eq!(failed(&f), ["client_receives_host_pose"]);
    }

    #[test]
    fn a_dedicated_server_that_skates_itself_fails() {
        let mut f = passing();
        f.lines.insert(
            "s_seen".into(),
            event("role=dedicated active=0 ready=1 peers=1:22300,0,5454:1:50:0:20000:0"),
        );
        assert_eq!(failed(&f), ["dedicated_moves_skating_client"]);
    }

    #[test]
    fn a_pose_that_loses_precision_fails() {
        let mut f = passing();
        f.lines.insert(
            "d_pushing".into(),
            event("active=1 ready=1 speed=320 sent=80 sent_bytes=32000 pose_err_mm=9.5"),
        );
        assert_eq!(failed(&f), ["dedicated_client_skates"]);
    }

    #[test]
    fn lines_parse() {
        let e = event(
            "remote=1:22400,0,5454:35:60:40:PhysicsGround|3:1,2,3:35:1:0:- peers=1:22400,-5,5454:1:60:2:23000:9",
        );
        assert_eq!(remotes(&e).len(), 2);
        assert_eq!(
            peer(&e, 1),
            Some(Peer {
                client: 1,
                origin: [22400.0, -5.0, 5454.0],
                skating: true,
                accepted: 60,
                rejected: 2,
                bytes_in: 23000,
                bytes_out: 9,
            })
        );
        assert!(peers(&event("peers=-")).is_empty());
    }

    #[test]
    fn bandwidth_is_bytes_over_time() {
        let mut f = Facts::default();
        f.lines.insert("c_on".into(), at("sent_bytes=1000", 10_000));
        f.lines
            .insert("c_pushed".into(), at("sent_bytes=9000", 12_000));
        assert_eq!(bandwidth(&f)["client_uplink_bytes_per_s"], 4000.0);
    }

    #[test]
    fn the_runway_holds_every_start() {
        let ([x1, y, z1], [x2, _, z2]) = RUNWAY;
        for [x, sy, z] in [SKATER_START, WATCHER_START] {
            assert!((x1..=x2).contains(&x) && (z1..=z2).contains(&z) && sy == y);
        }
        let blocks = (x2 - x1 + 1) * (z2 - z1 + 1);
        assert!(blocks <= 4096, "one `mc set` holds at most 4096 blocks");
    }
}
