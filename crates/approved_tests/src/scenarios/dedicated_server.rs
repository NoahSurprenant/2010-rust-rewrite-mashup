//! `dedicated_server` — owner-approved. This file is the one place its
//! steps and what each checkpoint must show live.
//!
//! A dedicated server (`iw4l dedicated minecraft:overworld`) hosts a room on
//! a master of the run's own, with no player of its own. A client joins
//! that room (the server logs it by its player key); with the server's admin
//! list emptied from its console, the client's `mc set`, `mc give`, `mc
//! summon`, `tp` and `admin transfer` are refused, logged on the server and
//! told to the client, and change nothing. Its key an admin again (as
//! `IW4L_ADMIN_KEYS` made it), the client sets a block and fills its
//! inventory; far from the spawn, in
//! a walled arena, a husk summoned beside it must reach it with the
//! server's id, hurt it and die to its gun as the server sees (with no
//! player of the server's own about); it must hear the husk, knife a husk
//! and punch a penned pig to death as the server sees, draw a witch's potion
//! and get its effect, and the server's mobs must spawn and live out past 48
//! blocks around it (24 to 128, despawning beyond 128); then a dozen pigs
//! there stand for the server's measure with mobs. It leaves; the room
//! stays listed and the server keeps running with nobody in it, past the
//! master session's silence deadline, and meanwhile the server changes
//! 2048 blocks, more history than one control message holds, and its
//! console moves the client's record to another key by name and back by key
//! (`admin transfer`). The client
//! joins again and finds its block, the last of those 2048, and its
//! inventory. Then the server is stopped with SIGTERM, as a
//! supervisor stops it, and started again: the world (seed, block) and the
//! client's inventory come back. Memory and CPU of the server are recorded
//! with no client and with one.
//! Each time it is up, its console answers `iw4l dedicated-console` over
//! its Unix socket (`whoami`, `admins`, an unknown command), the second
//! time over the socket the first left behind.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::duo::{Display, Pair, Player, cpu_secs, field};
use crate::runner::Event;
use crate::scenario::Parts;
use crate::scenarios::mob_steps;

pub const NAME: &str = "dedicated_server";
/// The rooms with and without the client, its keys and admin rights, and
/// the restarts: both server lives.
pub const VISITS: &str = "visits";
/// Far from the spawn, the mobs: one visit of the first life.
pub const MOBS: &str = "mobs";
/// Each part's checks.
pub const PARTS: Parts = &[
    (
        VISITS,
        &[
            "server_has_no_player",
            "seed_shared",
            "non_admin_refused",
            "admin_allowed",
            "player_joined_by_key",
            "admin_transfer_on_server",
            "client_block_on_server",
            "client_inventory",
            "room_kept_empty",
            "rejoin_world_kept",
            "sigterm_saves_and_exits",
            "restart_world_kept",
            "second_sigterm",
            "console_socket_answers",
        ],
    ),
    (
        MOBS,
        &[
            "server_has_no_player",
            "seed_shared",
            "player_joined_by_key",
            "mob_seen_with_no_server_player",
            "mob_hurts_client",
            "client_kills_mob_server_sees",
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
/// The client: it draws nothing (the server never does). Every check reads
/// what the games print or the master lists, none a picture.
pub const CLIENT_DISPLAY: Display = Display::Headless;
/// The room's name on the master, longer than the 16 characters a master
/// keeps of a player's name: the list shows it whole.
pub const ROOM: &str = "approved dedicated server";
/// The client's player name: the server keeps inventories by name.
pub const CLIENT: &str = "client";
pub const CLASSES: &str = crate::scenarios::crafting_table::CLASSES;
/// Loading a game and its world, on a CPU renderer.
pub const LOAD_SECS: u64 = 900;
pub const STEP_SECS: u64 = 120;
pub const SYNC_SECS: u64 = 60;
/// A rejoining client's whole edit history: the level's own changes (the
/// leaves of a mangrove swamp settling) can run to a hundred thousand.
pub const REJOIN_SECS: u64 = 240;
/// The room left empty: longer than a master session's 60 s without
/// authority progress, after which a stalled host is dropped.
pub const EMPTY_SECS: u64 = 75;
/// How long the server's CPU use is sampled at each point.
pub const CPU_SECS: u64 = 20;
/// SIGTERM to exit.
pub const STOP_SECS: u64 = 60;

/// A block the client sets, from the world's spawn block.
const GOLD: [i32; 3] = [2, 5, 0];
const PLANKS: &str = "minecraft:oak_planks*5";
/// A glass roof the server sets while the room is empty, from the world's
/// spawn block: 32 x 2 x 32 blocks, some 60 KB of edit history a joining
/// client is owed (a control message holds 16 KiB).
const ROOF: ([i32; 3], [i32; 3]) = ([-16, 30, -16], [15, 31, 15]);
const GLASS: &str = "minecraft:glass";
/// A stone platform far from the world's spawn (320 blocks east, 60 up),
/// in a walled arena, and the map point (36 units a block) just over its
/// middle.
const FAR: [i32; 3] = [320, 60, 0];
const FAR_MAP: [i32; 3] = [FAR[0] * 36, -FAR[2] * 36, (FAR[1] + 1) * 36 + 18];
/// Where the pigs go, in blocks from the client's feet across the arena.
const PIGS: [[i32; 2]; 12] = [
    [-3, -3],
    [-3, 0],
    [-3, 3],
    [0, -3],
    [0, 3],
    [3, -3],
    [3, 3],
    [-2, -2],
    [-2, 2],
    [2, -2],
    [2, 2],
    [-1, 3],
];

#[derive(Default)]
pub struct Facts {
    /// Each labelled line: `s*` the server's, `c*` the client's.
    pub lines: BTreeMap<String, Event>,
    /// The server's `dedicated-players:` lines, in order.
    pub rosters: Vec<Event>,
    /// The server's `debug-refused:` lines, in order.
    pub refusals: Vec<Event>,
    /// The server's `player joined:` lines, in order.
    pub joins: Vec<Event>,
    pub notes: BTreeMap<&'static str, String>,
    /// Server memory and CPU, by point.
    pub measures: BTreeMap<&'static str, Measure>,
    /// The server's `minecraft-mobnet:` lines: the mob bytes it sent its
    /// client, every ten seconds.
    pub mobnet: Vec<Event>,
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

/// How the server names the client in a refusal: its name and its key's
/// short form.
fn client_who() -> String {
    let key = crate::duo::client_key().fingerprint();
    format!("{CLIENT} {}", player_key::short(&key))
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

/// A block a client not on the admin list asks for, from the world's spawn
/// block, and the item it asks to be given: the server must refuse both.
const REFUSED_AT: [i32; 3] = [-2, 5, 0];
const REFUSED_BLOCK: &str = "minecraft:diamond_block";
const REFUSED_ITEM: &str = "minecraft:diamond";
/// The debug commands the locked-out client tries, as the server names them.
pub const REFUSED: [&str; 5] = ["mc fill", "mc give", "mc summon", "tp", "admin transfer"];

/// The server's admin list emptied from its console: the client's `mc set`,
/// `mc give`, `mc summon`, `tp` and `admin transfer` are refused (each logged
/// on the server, each told to the client), the world and the client's
/// inventory and place stay as they were; then the client's key is an admin
/// again (as `IW4L_ADMIN_KEYS` made it), for the steps after, which need it.
pub fn locked_out(pair: &mut Pair, facts: &mut Facts) -> Result<(), String> {
    let step = Duration::from_secs(STEP_SECS);
    let none = pair.ask(false, "admins set - s_admins_none", "s_admins_none", step)?;
    facts.keep("s_admins_none", none);
    let before = pair.host.refusals.len();
    // A refused `tp` never reaches its pose: the console stops waiting for
    // it when the refusal comes, and the script goes on.
    client(pair)?.send(&format!(
        "{}; mc give diamond 3; mc summon pig 2 0 0; admin transfer {CLIENT} {}; tp {} {} {}",
        set(REFUSED_AT, "diamond_block"),
        crate::duo::other_key().fingerprint(),
        FAR_MAP[0],
        FAR_MAP[1],
        FAR_MAP[2]
    ))?;
    pair.until("the server's refusals", step, |pair| {
        pair.host.refusals.len() >= before + REFUSED.len()
            && pair
                .client
                .as_ref()
                .is_some_and(|c| c.messages.len() >= REFUSED.len())
    })?;
    // The client's guess at its own inventory is taken back.
    let after = pair.ask(true, "wait 3s; mc state c_refused", "c_refused", step)?;
    facts.keep("c_refused", after);
    let block = pair.ask(
        false,
        &format!(
            "mc block s_refused {} {} {}",
            REFUSED_AT[0], REFUSED_AT[1], REFUSED_AT[2]
        ),
        "s_refused",
        step,
    )?;
    facts
        .notes
        .insert("s_refused", block.get("block").unwrap_or("").to_owned());
    facts
        .notes
        .insert("client_messages", client(pair)?.messages.join("\n"));
    let back = pair.ask(
        false,
        &format!(
            "admins set {} s_admins",
            crate::duo::client_key().fingerprint()
        ),
        "s_admins",
        step,
    )?;
    facts.keep("s_admins", back);
    facts
        .notes
        .insert("refused_before_admin", pair.host.refusals.len().to_string());
    Ok(())
}

/// The first visit: the client sets a block and fills its inventory, and
/// the server sees the block.
pub fn first_visit(pair: &mut Pair, facts: &mut Facts, room: &str) -> Result<(), String> {
    let step = Duration::from_secs(STEP_SECS);
    let ready = join(pair, room, "c_ready")?;
    facts.keep("c_ready", ready);
    locked_out(pair, facts)?;
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

/// Part `mobs`'s visit: the client joins (an admin, as `IW4L_ADMIN_KEYS`
/// made it) and goes straight to the mobs.
pub fn mobs_visit(pair: &mut Pair, facts: &mut Facts, room: &str) -> Result<(), String> {
    let ready = join(pair, room, "c_ready")?;
    facts.keep("c_ready", ready);
    mob_visit(pair, facts)
}

/// Far from the world's spawn, in a walled arena on a platform the client
/// asks for: a husk summoned beside the client, with no player of the
/// server's own about, reaches the client with the server's id, hurts it,
/// and dies to its gun as the server sees. Then a dozen pigs there, for the
/// server's memory and CPU with mobs about and the mob bytes it sends.
pub fn mob_visit(pair: &mut Pair, facts: &mut Facts) -> Result<(), String> {
    let step = Duration::from_secs(STEP_SECS);
    client(pair)?.send(&format!(
        "{}; wait 2s; tp {} {} {}; wait 8s",
        mob_steps::arena(FAR),
        FAR_MAP[0],
        FAR_MAP[1],
        FAR_MAP[2]
    ))?;
    let far = pair.ask(true, "mc state c_far", "c_far", step)?;
    facts.keep("c_far", far);
    // A bullet gun up before the husk comes.
    mob_steps::draw_gun(pair)?;
    mob_steps::arena_floor(pair, FAR, Duration::from_secs(mob_steps::SEE_SECS))?;
    let see = Duration::from_secs(mob_steps::SEE_SECS);
    let (seen, husk) = mob_steps::summon_seen(pair, "husk", "3 0 0", "c_husk", see)?;
    facts.keep("c_husk", seen);
    let husk = husk.ok_or("the client never saw the husk")?;
    facts.notes.insert("husk_id", husk.id.to_string());
    let theirs = mob_steps::census(pair, false, "s_track", Some("husk"))?;
    facts.keep("s_track", theirs);
    let hurt = mob_steps::poll_hurt(
        pair,
        "c_hurt",
        "husk",
        100.0,
        Duration::from_secs(mob_steps::HURT_SECS),
    )?;
    facts.keep("c_hurt", hurt.clone());
    let shot = mob_steps::shoot(
        pair,
        "husk",
        husk.id,
        0.95,
        &hurt,
        Duration::from_secs(mob_steps::KILL_SECS),
    )?;
    facts.keep("s_husk_shot", shot.host);
    facts.notes.insert("husk_killed", shot.killed.to_string());
    mob_steps::beyond_the_gun(pair, FAR, &mut facts.lines, &mut facts.notes)?;
    let pigs: Vec<String> = PIGS
        .iter()
        .map(|[x, z]| format!("mc summon pig {x} 0 {z}"))
        .collect();
    client(pair)?.send(&pigs.join("; "))?;
    let (seen, _) = mob_steps::poll(pair, true, "c_pigs", Some("pig"), see, |event, _| {
        mob_steps::mobs(event).iter().filter(|m| !m.dead).count() >= PIGS.len()
    })?;
    facts.keep("c_pigs", seen);
    measure(pair, facts, "one_client_mobs")?;
    let census = mob_steps::census(pair, false, "s_mobs", None)?;
    facts.keep("s_mobs", census);
    Ok(())
}

/// The client leaves through the normal exit and the server's room stays.
pub fn leave(pair: &mut Pair) -> Result<(), String> {
    client(pair)?.send("wait 3s; finish_run")?;
    pair.until("the client to exit", Duration::from_secs(240), |pair| {
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
    // From the server's console, as its owner would after a player lost its
    // key: the client's record moved by name to another key, and back by key.
    let (mine, other) = (
        crate::duo::client_key().fingerprint(),
        crate::duo::other_key().fingerprint(),
    );
    let step = Duration::from_secs(STEP_SECS);
    for (label, from, to) in [
        ("s_xfer_away", CLIENT, other.as_str()),
        ("s_xfer_back", other.as_str(), mine.as_str()),
    ] {
        let done = pair.ask(
            false,
            &format!("admin transfer {from} {to} {label}"),
            label,
            step,
        )?;
        facts.keep(label, done);
    }
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
        Duration::from_secs(REJOIN_SECS),
    )?;
    facts.notes.insert(gold, block);
    let block = pair.poll_block(true, roof, ROOF.1, GLASS, Duration::from_secs(REJOIN_SECS))?;
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

/// The husk in a census line, by the id the client first saw it with.
fn husk(facts: &Facts, label: &str) -> Result<mob_steps::Mob, String> {
    let id: u64 = note(facts, "husk_id")?.parse().map_err(|_| "bad husk id")?;
    mob_steps::mobs(line(facts, label)?)
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("`{label}` lists no mob {id}"))
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

/// What `iw4l dedicated-console` asks the server, by the note its answer is
/// kept under, and what each answer must hold: the second life's socket is
/// made over the one the first left behind.
pub const CONSOLE_ASKS: [(&str, &str); 4] = [
    ("console_whoami", "player-key: none"),
    ("console_admins", "debug-admins: keys="),
    ("console_unknown", "unknown command"),
    ("console2_admins", "debug-admins: keys="),
];

/// The server's console asked from outside its process, as `kubectl exec`
/// asks it: `iw4l dedicated-console <command>` over the socket at `socket`,
/// each answer kept under its note. `life` 1 asks everything, 2 `admins`.
pub fn console_socket(
    bin: &std::path::Path,
    socket: &std::path::Path,
    life: u32,
    facts: &mut Facts,
) -> Result<(), String> {
    let asks: &[(&'static str, &str)] = if life == 1 {
        &[
            ("console_whoami", "whoami"),
            ("console_admins", "admins"),
            ("console_unknown", "no_such_command"),
        ]
    } else {
        &[("console2_admins", "admins")]
    };
    for (key, command) in asks {
        let out = std::process::Command::new(bin)
            .args(["dedicated-console", command])
            .env("IW4L_CONSOLE_SOCKET", socket)
            .output()
            .map_err(|e| format!("iw4l dedicated-console: {e}"))?;
        let answer = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        println!("[console] {command} -> {}", answer.trim_end());
        facts.notes.insert(key, answer);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(socket)
            .map_err(|e| format!("{}: {e}", socket.display()))?
            .permissions()
            .mode();
        facts
            .notes
            .insert("console_mode", format!("{:o}", mode & 0o777));
    }
    Ok(())
}

pub fn judge(facts: &Facts) -> Vec<Check> {
    type Rule = fn(&Facts) -> Result<(), String>;
    let rules: [(&'static str, Rule); 18] = [
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
        ("non_admin_refused", |f| {
            want(
                line(f, "s_admins_none")?.get("keys").unwrap_or(""),
                "-",
                "the server's admins",
            )?;
            for command in REFUSED {
                let refused = f.refusals.iter().any(|r| {
                    r.get("command") == Some(command) && r.get("who") == Some(client_who().as_str())
                });
                if !refused {
                    return Err(format!("the server never refused the client's `{command}`"));
                }
                let told = note(f, "client_messages")?
                    .lines()
                    .any(|m| m.contains(command) && m.contains("refused"));
                if !told {
                    return Err(format!("the client was never told `{command}` was refused"));
                }
            }
            if note(f, "s_refused")? == REFUSED_BLOCK {
                return Err("the refused `mc set` changed the server's world".into());
            }
            let after = line(f, "c_refused")?;
            if slots(after)
                .iter()
                .any(|s| s.starts_with(&format!("{REFUSED_ITEM}*")))
            {
                return Err(format!(
                    "the refused `mc give` left {REFUSED_ITEM} with the client"
                ));
            }
            let x = after
                .get("origin")
                .and_then(|o| o.split(',').next())
                .and_then(|x| x.parse::<f64>().ok())
                .ok_or("no origin in `c_refused`")?;
            if (x - f64::from(FAR_MAP[0])).abs() < 1000.0 {
                return Err(format!("the refused `tp` moved the client (x {x})"));
            }
            Ok(())
        }),
        ("admin_allowed", |f| {
            want(
                line(f, "s_admins")?.get("keys").unwrap_or(""),
                &crate::duo::client_key().fingerprint(),
                "the server's admins",
            )?;
            // The admin's `mc set`, `mc give`, `mc summon` and `tp` are the
            // steps the other checks judge; none of them was refused.
            let before: usize = note(f, "refused_before_admin")?
                .parse()
                .map_err(|_| "bad refusal count")?;
            match f.refusals.get(before) {
                Some(late) => Err(format!(
                    "refused once the client was an admin: {}",
                    late.get("command").unwrap_or("?")
                )),
                None => Ok(()),
            }
        }),
        ("player_joined_by_key", |f| {
            let key = crate::duo::client_key().fingerprint();
            let joined = f
                .joins
                .iter()
                .any(|j| j.get("name") == Some(CLIENT) && j.get("key") == Some(key.as_str()));
            if joined {
                Ok(())
            } else {
                Err(format!(
                    "no `player joined: name={CLIENT} key={key}` on the server"
                ))
            }
        }),
        ("admin_transfer_on_server", |f| {
            let (mine, other) = (
                crate::duo::client_key().fingerprint(),
                crate::duo::other_key().fingerprint(),
            );
            for (label, from, to) in [
                ("s_xfer_away", &mine, &other),
                ("s_xfer_back", &other, &mine),
            ] {
                let done = line(f, label)?;
                want(done.get("result").unwrap_or("-"), "ok", label)?;
                want(
                    done.get("from").unwrap_or("-"),
                    from,
                    &format!("{label} from"),
                )?;
                want(done.get("to").unwrap_or("-"), to, &format!("{label} to"))?;
                want(
                    done.get("name").unwrap_or("-"),
                    CLIENT,
                    &format!("{label} name"),
                )?;
                want(
                    done.get("inventory").unwrap_or("-"),
                    "true",
                    &format!("{label} inventory"),
                )?;
            }
            // The planks came back with the record (rejoin_world_kept).
            Ok(())
        }),
        ("client_block_on_server", |f| {
            want(note(f, "s_gold")?, "minecraft:gold_block", "server sees")
        }),
        ("client_inventory", |f| has_planks(f, "c_given")),
        ("mob_seen_with_no_server_player", |f| {
            let seen = husk(f, "c_husk")?;
            let platform = [f64::from(FAR[0]), f64::from(FAR[1] + 1), f64::from(FAR[2])];
            if mob_steps::apart(seen.at, platform) > 8.0 {
                return Err(format!(
                    "the client's husk at {:?}, not by the platform",
                    seen.at
                ));
            }
            husk(f, "s_track").map(|_| ())
        }),
        ("mob_hurts_client", |f| {
            let health = mob_steps::health(line(f, "c_hurt")?).ok_or("no health in `c_hurt`")?;
            if health < 100.0 {
                Ok(())
            } else {
                Err(format!("the client's health stayed {health}"))
            }
        }),
        ("client_kills_mob_server_sees", |f| {
            want(
                note(f, "husk_killed")?,
                "true",
                "the server saw the husk die",
            )
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
                Err("the server reported no mob states sent".into())
            }
        }),
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
        ("console_socket_answers", |f| {
            for (key, expected) in CONSOLE_ASKS {
                let answer = note(f, key)?;
                if !answer.contains(expected) {
                    return Err(format!("{key}: `{expected}` not in {answer:?}"));
                }
            }
            want(note(f, "console_mode")?, "600", "the console socket's mode")
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

    fn event(prefix: &str, fields: &str) -> Event {
        crate::runner::parse(&format!("{prefix} {fields}"), prefix, 0).unwrap()
    }

    fn state(fields: &str) -> Event {
        event("minecraft-state:", fields)
    }

    fn passing() -> Facts {
        let mut f = Facts::default();
        mob_steps::passing_beyond_the_gun(&mut f.lines, &mut f.notes);
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
                "generation=3 adverts=1\nab12\t1/18\tminecraft:overworld\tdm\tlocked=0\tin_match=1\tgen=3\trequires=0x01\tapproved dedicated server",
            ),
            ("stop1", "code=0 secs=0.4"),
            ("stop2", "code=0 secs=0.3"),
            (
                "console_whoami",
                "player-key: none (a dedicated server has no player; or no key could be kept)\n",
            ),
            ("console_admins", "debug-admins: keys=0123456789abcdef\n"),
            ("console_unknown", "unknown command `no_such_command`\n"),
            ("console2_admins", "debug-admins: keys=0123456789abcdef\n"),
            ("console_mode", "600"),
        ] {
            f.notes.insert(key, value.into());
        }
        for (label, health) in [("c_husk", 100), ("s_track", 100), ("c_hurt", 85)] {
            f.lines.insert(
                label.into(),
                event(
                    "minecraft-mobs:",
                    &format!(
                        "label={label} health={health} mobs=husk:9:322.50,61.00,0.20:20.0:alive"
                    ),
                ),
            );
        }
        f.lines.insert(
            "s_admins_none".into(),
            event("debug-admins:", "label=s_admins_none keys=-"),
        );
        f.lines.insert(
            "s_admins".into(),
            event(
                "debug-admins:",
                &format!(
                    "label=s_admins keys={}",
                    crate::duo::client_key().fingerprint()
                ),
            ),
        );
        f.lines.insert(
            "c_refused".into(),
            state("label=c_refused origin=36,-18,330 seed=7 role=client slots=-"),
        );
        let mut told = Vec::new();
        for command in REFUSED {
            f.refusals.push(event(
                "debug-refused:",
                &format!("client=1 who=\"{}\" command=\"{command}\"", client_who()),
            ));
            told.push(format!(
                "^1{command}^7 refused: only the host's admins may run debug commands"
            ));
        }
        f.notes.insert("client_messages", told.join("\n"));
        f.notes
            .insert("refused_before_admin", REFUSED.len().to_string());
        f.notes.insert("s_refused", "air".into());
        let (mine, other) = (
            crate::duo::client_key().fingerprint(),
            crate::duo::other_key().fingerprint(),
        );
        f.joins
            .push(event("player joined:", &format!("name=client key={mine}")));
        for (label, from, to) in [
            ("s_xfer_away", &mine, &other),
            ("s_xfer_back", &other, &mine),
        ] {
            f.lines.insert(
                label.into(),
                event(
                    "admin-transfer:",
                    &format!(
                        "label={label} result=ok from={from} to={to} name=\"client\" inventory=true admin=false"
                    ),
                ),
            );
        }
        f.notes.insert("husk_id", "9".into());
        f.notes.insert("husk_killed", "true".into());
        f.mobnet.push(event(
            "minecraft-mobnet:",
            "client=1 mobs=13 frame_bytes_per_s=5600 row_bytes_per_s=40",
        ));
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
    fn a_room_listed_by_a_cut_name_fails() {
        let mut f = passing();
        let cut: String = ROOM.chars().take(16).collect();
        let list = f.notes["empty_list"].replace(ROOM, &cut);
        f.notes.insert("empty_list", list);
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
    fn a_mob_only_the_server_has_fails() {
        let mut f = passing();
        f.lines.insert(
            "c_husk".into(),
            event("minecraft-mobs:", "label=c_husk health=100 mobs=-"),
        );
        f.notes.insert("husk_killed", "false".into());
        assert_eq!(
            failed(&f),
            [
                "mob_seen_with_no_server_player",
                "client_kills_mob_server_sees"
            ]
        );
    }

    #[test]
    fn a_console_socket_that_does_not_answer_fails() {
        let mut f = passing();
        f.notes.insert(
            "console_admins",
            "dedicated-console: /tmp/x.sock: Connection refused (os error 111)".into(),
        );
        assert_eq!(failed(&f), ["console_socket_answers"]);
        let mut f = passing();
        f.notes.remove("console2_admins");
        assert_eq!(failed(&f), ["console_socket_answers"]);
        let mut f = passing();
        f.notes.insert("console_mode", "666".into());
        assert_eq!(failed(&f), ["console_socket_answers"]);
    }

    #[test]
    fn a_server_killed_rather_than_stopped_fails() {
        let mut f = passing();
        f.notes.insert("stop1", "code=signal secs=30.0".into());
        assert_eq!(failed(&f), ["sigterm_saves_and_exits"]);
    }

    #[test]
    fn a_debug_command_a_non_admin_got_through_fails() {
        let mut f = passing();
        f.notes.insert("s_refused", REFUSED_BLOCK.into());
        assert_eq!(failed(&f), ["non_admin_refused"]);
        let mut f = passing();
        f.lines.insert(
            "c_refused".into(),
            state("label=c_refused origin=36,-18,330 seed=7 role=client slots=minecraft:diamond*3"),
        );
        assert_eq!(failed(&f), ["non_admin_refused"]);
        let mut f = passing();
        f.lines.insert(
            "c_refused".into(),
            state("label=c_refused origin=11520,0,2232 seed=7 role=client slots=-"),
        );
        assert_eq!(failed(&f), ["non_admin_refused"]);
        let mut f = passing();
        f.refusals.retain(|r| r.get("command") != Some("mc summon"));
        f.notes.insert("refused_before_admin", "4".into());
        assert_eq!(failed(&f), ["non_admin_refused"]);
    }

    #[test]
    fn a_refused_client_never_told_fails() {
        let mut f = passing();
        f.notes.insert("client_messages", String::new());
        assert_eq!(failed(&f), ["non_admin_refused"]);
    }

    #[test]
    fn an_admin_refused_fails() {
        let mut f = passing();
        f.refusals.push(event(
            "debug-refused:",
            "client=1 name=client command=\"mc fill\"",
        ));
        assert_eq!(failed(&f), ["admin_allowed"]);
    }

    #[test]
    fn a_join_not_logged_by_key_fails() {
        let mut f = passing();
        f.joins = vec![event("player joined:", "name=client key=guest")];
        assert_eq!(failed(&f), ["player_joined_by_key"]);
    }

    #[test]
    fn a_refused_transfer_on_the_server_fails() {
        let mut f = passing();
        f.lines.insert(
            "s_xfer_away".into(),
            event(
                "admin-transfer:",
                "label=s_xfer_away result=refused reason=\"2 players have played as `client`\"",
            ),
        );
        assert_eq!(failed(&f), ["admin_transfer_on_server"]);
    }
}
