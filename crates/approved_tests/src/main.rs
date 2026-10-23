//! The owner-approved end-to-end scenarios; see README.md.

#[cfg(test)]
mod control_relay;
mod duo;
#[cfg(test)]
mod master_pin;
mod report;
mod runner;
mod scenario;
mod world;
mod scenarios {
    pub mod crafting_table;
    pub mod dedicated_server;
    pub mod heavy_gameplay_lifecycle;
    pub mod map_steps;
    pub mod minecraft_multiplayer;
    pub mod mob_steps;
    pub mod skate_multiplayer;
}

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use scenario::{MapB, Place, ResolvedPlace, ResolvedScene};
use scenarios::crafting_table;
use scenarios::dedicated_server;
use scenarios::heavy_gameplay_lifecycle::{self as hgl, SCENARIO};
use scenarios::minecraft_multiplayer;
use scenarios::skate_multiplayer;

#[derive(Clone, Copy, PartialEq)]
enum CacheMode {
    Cold,
    Shared,
}

struct Args {
    scenario: String,
    /// `--seed N`: the scenario seed of heavy_gameplay_lifecycle (a u64),
    /// or the Minecraft world seed of the scenarios on the Minecraft map.
    seed: Option<u64>,
    mc_seed: Option<i64>,
    /// `--spawn "x y z"`: where a Minecraft world's first player stands.
    spawn: Option<String>,
    replay: Option<PathBuf>,
    cache: CacheMode,
    bin: PathBuf,
    /// `--part <name>`: one part of a split scenario.
    part: Option<String>,
}

fn main() {
    let root = repo_root();
    let args = match parse_args(&root) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            eprintln!(
                "usage: approved_tests heavy_gameplay_lifecycle [--seed N | --replay run.json] \
                 [--cache cold|shared] [--bin path]\n       approved_tests crafting_table [WORLD] [--cache cold|shared] [--bin path]\n       approved_tests minecraft_multiplayer [--part world|mobs] [WORLD] [--cache cold|shared] [--bin path]\n       approved_tests dedicated_server [--part visits|mobs] [WORLD] [--cache cold|shared] [--bin path]\n       approved_tests skate_multiplayer [--part listen|dedicated] [WORLD] [--cache cold|shared] [--bin path]\n  WORLD: [--seed N [--spawn \"x y z\"] | --replay run.json], the Minecraft world (default: seed 12345 at its fixed spawn)"
            );
            std::process::exit(2);
        }
    };
    let run = if args.scenario == SCENARIO.name {
        run
    } else if args.scenario == crafting_table::NAME {
        run_crafting_table
    } else if args.scenario == minecraft_multiplayer::NAME {
        run_minecraft_multiplayer
    } else if args.scenario == dedicated_server::NAME {
        run_dedicated_server
    } else if args.scenario == skate_multiplayer::NAME {
        run_skate_multiplayer
    } else {
        eprintln!(
            "approved scenarios: {}, {}, {}, {}, {} (asked for {:?})",
            SCENARIO.name,
            crafting_table::NAME,
            minecraft_multiplayer::NAME,
            dedicated_server::NAME,
            skate_multiplayer::NAME,
            args.scenario
        );
        std::process::exit(2);
    };
    std::process::exit(match run(&root, &args) {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(message) => {
            eprintln!("approved_tests: {message}");
            1
        }
    });
}

/// The repository: `IW4L_REPO_ROOT`, or where this was built (a CI job that
/// runs a build made in another job checks the repository out elsewhere).
fn repo_root() -> PathBuf {
    if let Some(root) = std::env::var_os("IW4L_REPO_ROOT").filter(|r| !r.is_empty()) {
        return PathBuf::from(root);
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .unwrap_or(manifest)
        .to_path_buf()
}

fn parse_args(root: &Path) -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let scenario = it.next().ok_or("which scenario?")?;
    let mut args = Args {
        scenario,
        seed: None,
        mc_seed: None,
        spawn: None,
        replay: None,
        cache: CacheMode::Cold,
        bin: root.join("target/play/iw4l"),
        part: None,
    };
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--seed" => {
                let raw = value()?;
                args.seed = raw.parse().ok();
                args.mc_seed = raw.parse().ok();
                if args.seed.is_none() && args.mc_seed.is_none() {
                    return Err(format!("--seed {raw}: not a number"));
                }
            }
            "--spawn" => args.spawn = Some(value()?),
            "--replay" => args.replay = Some(PathBuf::from(value()?)),
            "--cache" => {
                args.cache = match value()?.as_str() {
                    "cold" => CacheMode::Cold,
                    "shared" => CacheMode::Shared,
                    other => return Err(format!("--cache {other}: cold or shared")),
                }
            }
            "--bin" => args.bin = PathBuf::from(value()?),
            "--part" => args.part = Some(value()?),
            other => return Err(format!("unknown flag {other}")),
        }
    }
    let picked = args.seed.is_some() || args.mc_seed.is_some() || args.spawn.is_some();
    if picked && args.replay.is_some() {
        return Err("--seed and --replay are exclusive: a replay keeps its own".into());
    }
    Ok(args)
}

struct Replay {
    source: PathBuf,
    seed: u64,
    map_b: String,
    landed: Vec<(String, [f32; 3], f32)>,
}

fn read_replay(path: &Path) -> Result<Replay, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let run: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let seed = run["seed"].as_u64().ok_or("replay: no seed")?;
    let map_b = run["maps"]["b"]
        .as_str()
        .ok_or("replay: no maps.b")?
        .to_owned();
    let mut landed = Vec::new();
    for scene in run["scenes"].as_array().into_iter().flatten() {
        let phase = scene["phase"].as_str().unwrap_or("").to_owned();
        let spawn = &scene["observed"]["spawn"];
        let origin = spawn["origin"].as_array().map(|a| {
            let f = |i: usize| a.get(i).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            [f(0), f(1), f(2)]
        });
        if let (Some(origin), Some(yaw)) = (origin, spawn["yaw"].as_f64()) {
            landed.push((phase, origin, yaw as f32));
        }
    }
    Ok(Replay {
        source: path.to_path_buf(),
        seed,
        map_b,
        landed,
    })
}

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn fresh_seed() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    mix(now ^ (u64::from(std::process::id()) << 32))
}

fn resolve_scenes(
    tag: &'static str,
    map: &str,
    map_index: u64,
    seed: u64,
    replay: Option<&Replay>,
) -> Vec<ResolvedScene> {
    SCENARIO
        .scenes
        .iter()
        .enumerate()
        .map(|(index, scene)| {
            let phase = format!("{tag}.scene_{}", index + 1);
            let replayed = replay
                .and_then(|r| r.landed.iter().find(|(p, ..)| *p == phase))
                .map(|(_, origin, yaw)| ResolvedPlace::At {
                    origin: *origin,
                    yaw: *yaw,
                });
            let place = replayed.unwrap_or(match scene.place {
                Place::At { origin, yaw } => ResolvedPlace::At { origin, yaw },
                Place::Random => {
                    ResolvedPlace::Seeded(mix(seed ^ mix(map_index * 16 + index as u64 + 1)))
                }
            });
            ResolvedScene {
                tag,
                map: map.to_owned(),
                index,
                name: scene.name,
                place,
                weapon: scene.weapon.as_ref(),
                commands: scenario::scene_commands(scene, place),
            }
        })
        .collect()
}

fn run(root: &Path, args: &Args) -> Result<bool, String> {
    asset_transport::load_dotenv();
    scenario::Part::of(&[], args.part.as_deref())?;
    if args.spawn.is_some() || (args.mc_seed.is_some() && args.seed.is_none()) {
        return Err(
            "heavy_gameplay_lifecycle: --seed is a u64, and it has no Minecraft spawn".into(),
        );
    }
    let replay = args.replay.as_deref().map(read_replay).transpose()?;
    let seed = replay
        .as_ref()
        .map(|r| r.seed)
        .or(args.seed)
        .unwrap_or_else(fresh_seed);

    if !args.bin.is_file() {
        return Err(format!(
            "{} is not built (make approved builds it)",
            args.bin.display()
        ));
    }

    let games = asset_transport::games_root_from_env()?;
    let installed = asset_transport::list_mp_maps(&games);
    let (map_b, map_b_rule, candidates): (Option<String>, String, Vec<String>) =
        match (&replay, &SCENARIO.map_b) {
            (Some(r), _) => (Some(r.map_b.clone()), "replay".into(), Vec::new()),
            (None, MapB::Fixed(zone)) => (Some((*zone).to_owned()), "fixed".into(), Vec::new()),
            (None, MapB::FromInstalled { game }) => {
                let prefix = format!("{game}:");
                let candidates: Vec<String> = installed
                    .iter()
                    .filter_map(|m| m.strip_prefix(&prefix))
                    .filter(|m| *m != SCENARIO.map_a)
                    .map(str::to_owned)
                    .collect();
                let pick = (!candidates.is_empty()).then(|| {
                    candidates[(mix(seed ^ 0xB) % candidates.len() as u64) as usize].clone()
                });
                (
                    pick,
                    format!(
                        "seeded pick among installed {game} mp zones, {} excluded",
                        SCENARIO.map_a
                    ),
                    candidates,
                )
            }
        };
    let map_a_installed = installed
        .iter()
        .any(|m| m.split_once(':').is_some_and(|(_, z)| z == SCENARIO.map_a));

    let run_id = format!("{}-{seed:016x}", utc_stamp());
    let run_dir = root.join("iw4l-artifacts/approved-tests").join(&run_id);
    std::fs::create_dir_all(run_dir.parent().expect("has parent"))
        .map_err(|e| format!("create run root: {e}"))?;
    std::fs::create_dir(&run_dir).map_err(|e| format!("create {}: {e}", run_dir.display()))?;

    let mut manifest = json!({
        "scenario": SCENARIO.name,
        "run_id": run_id,
        "result": "running",
        "seed": seed,
        "replay_of": replay.as_ref().map(|r| r.source.display().to_string()),
        "build": report::build_facts(root, &args.bin),
        "platform": report::platform_facts(),
        "content": { "games_root": games.0.display().to_string() },
        "gametype": SCENARIO.gametype,
        "class": SCENARIO.class,
        "maps": {
            "a": SCENARIO.map_a,
            "a_installed": map_a_installed,
            "b": map_b,
            "b_rule": map_b_rule,
            "b_candidates": candidates,
        },
        "bots": { "players_target": SCENARIO.players, "bots_requested": SCENARIO.players - 1 },
        "timeouts_secs": { "phase": SCENARIO.phase_timeout_secs, "quit": SCENARIO.quit_timeout_secs },
    });

    let Some(map_b) = map_b else {
        manifest["result"] = "blocked".into();
        manifest["failure"] = "no second installed map for the gametype: a one-map run is not the lifecycle this scenario tests".into();
        report::write(&run_dir, &manifest)?;
        println!("blocked: no map B; {}", run_dir.join("run.json").display());
        return Ok(false);
    };
    if !map_a_installed {
        manifest["result"] = "blocked".into();
        manifest["failure"] =
            format!("{} is not installed under the games root", SCENARIO.map_a).into();
        report::write(&run_dir, &manifest)?;
        return Ok(false);
    }

    let scenes_a = resolve_scenes("a", SCENARIO.map_a, 0, seed, replay.as_ref());
    let scenes_b = resolve_scenes("b", &map_b, 1, seed, replay.as_ref());
    let phases = hgl::phases(&map_b, &scenes_a, &scenes_b);
    let script = scenario::script(&phases);
    let child_args = vec![
        "map".to_owned(),
        SCENARIO.map_a.to_owned(),
        "--cmds".to_owned(),
        script.clone(),
    ];

    let cache = prepare_cache(root, &run_dir, args.cache)?;
    manifest["cache"] = cache;
    let bin = stage_binary(&args.bin, &run_dir)?;
    manifest["command_line"] = json!([bin.display().to_string(), child_args]);
    manifest["script"] = script.into();
    manifest["phases_declared"] = phases
        .iter()
        .map(|p| p.name.clone())
        .collect::<Vec<_>>()
        .into();
    manifest["scenes"] = scenes_a
        .iter()
        .chain(&scenes_b)
        .map(ResolvedScene::to_json)
        .collect::<Vec<_>>()
        .into();
    report::write(&run_dir, &manifest)?;
    println!("run: {}", run_dir.display());

    let outcome = runner::run(runner::Launch {
        bin: &bin,
        cwd: &run_dir,
        args: child_args,
        env: vec![
            ("IW4L_PERF", "1".into()),
            ("IW4L_GAMETYPE", SCENARIO.gametype.into()),
        ],
        phase_timeout: Duration::from_secs(SCENARIO.phase_timeout_secs),
        quit_timeout: Duration::from_secs(SCENARIO.quit_timeout_secs),
    });

    let passed = report::finish(&run_dir, &mut manifest, &phases, &outcome);
    report::write(&run_dir, &manifest)?;
    println!(
        "{}: {}",
        manifest["result"].as_str().unwrap_or("?"),
        run_dir.join("run.json").display()
    );
    Ok(passed)
}

/// The Minecraft world a scenario on the Minecraft map plays (`world`).
fn minecraft_world(args: &Args) -> Result<world::World, String> {
    if args.seed.is_some() && args.mc_seed.is_none() {
        return Err("--seed: a Minecraft seed is an i64".into());
    }
    world::World::from_args(args.mc_seed, args.spawn.as_deref(), args.replay.as_deref())
}

fn run_crafting_table(root: &Path, args: &Args) -> Result<bool, String> {
    asset_transport::load_dotenv();
    scenario::Part::of(&[], args.part.as_deref())?;
    let world = minecraft_world(args)?;
    if !args.bin.is_file() {
        return Err(format!(
            "{} is not built (make approved builds it)",
            args.bin.display()
        ));
    }
    let games = asset_transport::games_root_from_env()?;
    let mw2_installed = asset_transport::list_mp_maps(&games)
        .iter()
        .any(|m| m.starts_with("iw4:"));

    let run_id = format!("{}-{}", utc_stamp(), crafting_table::NAME);
    let run_dir = root.join("iw4l-artifacts/approved-tests").join(&run_id);
    std::fs::create_dir_all(run_dir.parent().expect("has parent"))
        .map_err(|e| format!("create run root: {e}"))?;
    std::fs::create_dir(&run_dir).map_err(|e| format!("create {}: {e}", run_dir.display()))?;

    let script = crafting_table::script();
    let child_args = vec![
        "map".to_owned(),
        crafting_table::MAP.to_owned(),
        "--cmds".to_owned(),
        script.clone(),
    ];
    let mut manifest = json!({
        "scenario": crafting_table::NAME,
        "run_id": run_id,
        "result": "running",
        "build": report::build_facts(root, &args.bin),
        "platform": report::platform_facts(),
        "content": { "games_root": games.0.display().to_string(), "mw2_installed": mw2_installed },
        "map": crafting_table::MAP,
        "minecraft_world": world.facts(false),
        "timeouts_secs": { "phase": crafting_table::PHASE_TIMEOUT_SECS, "quit": crafting_table::QUIT_TIMEOUT_SECS },
        "script": script,
    });
    if !mw2_installed {
        manifest["result"] = "blocked".into();
        manifest["failure"] = "no MW2 multiplayer zone under the games root".into();
        report::write(&run_dir, &manifest)?;
        return Ok(false);
    }

    let profile = run_dir.join("iw4l-artifacts/profile");
    std::fs::create_dir_all(&profile).map_err(|e| format!("create {}: {e}", profile.display()))?;
    std::fs::write(profile.join("classes.txt"), crafting_table::CLASSES)
        .map_err(|e| format!("write the class file: {e}"))?;
    manifest["cache"] = prepare_cache(root, &run_dir, args.cache)?;
    manifest["minecraft_files"] = share_minecraft_files(root, &run_dir)?.into();
    let bin = stage_binary(&args.bin, &run_dir)?;
    manifest["command_line"] = json!([bin.display().to_string(), child_args]);
    report::write(&run_dir, &manifest)?;
    println!("run: {}", run_dir.display());

    let outcome = runner::run(runner::Launch {
        bin: &bin,
        cwd: &run_dir,
        args: child_args,
        env: world.env(),
        phase_timeout: Duration::from_secs(crafting_table::PHASE_TIMEOUT_SECS),
        quit_timeout: Duration::from_secs(crafting_table::QUIT_TIMEOUT_SECS),
    });

    keep_minecraft_files(root, &run_dir);
    let mut checks = crafting_table::judge(&outcome.minecraft);
    checks.push(crafting_table::judge_player_key(&outcome.player_keys));
    for check in &checks {
        match check.failures.as_slice() {
            [] => println!("  pass  {}", check.label),
            failures => println!("  FAIL  {}: {}", check.label, failures.join("; ")),
        }
    }
    let exit = match outcome.end {
        runner::End::Exited { code: Some(0), .. } => None,
        runner::End::Exited { code, .. } => Some(format!("game exited with {code:?}")),
        runner::End::Timeout { waiting_for, .. } => {
            Some(format!("killed waiting for {waiting_for}"))
        }
        runner::End::SpawnFailed(why) => Some(why),
    };
    let failed: Vec<&str> = checks
        .iter()
        .filter(|c| !c.failures.is_empty())
        .map(|c| c.label)
        .collect();
    let passed = exit.is_none() && failed.is_empty();
    manifest["checks"] = checks
        .iter()
        .map(|c| json!({ "label": c.label, "failures": c.failures }))
        .collect::<Vec<_>>()
        .into();
    manifest["result"] = if passed { "passed" } else { "failed" }.into();
    if let Some(exit) = exit {
        manifest["failure"] = exit.into();
    } else if !failed.is_empty() {
        manifest["failure"] = format!("checkpoints failed: {}", failed.join(", ")).into();
    }
    report::write(&run_dir, &manifest)?;
    println!(
        "{}: {}",
        manifest["result"].as_str().unwrap_or("?"),
        run_dir.join("run.json").display()
    );
    Ok(passed)
}

/// A lobby on the master, the client in it, and the match started.
fn open_duo(
    bin: &Path,
    master: &duo::Master,
    dirs: &[PathBuf; 2],
    session: u32,
    displays: [duo::Display; 2],
    extra_env: &[(&str, String)],
) -> Result<duo::Pair, String> {
    use duo::field;
    let mut env = vec![
        ("IW4L_MASTER_ADDR", master.addr.clone()),
        ("IW4L_MASTER_SERVER_NAME", duo::SERVER_NAME.to_owned()),
        ("IW4L_MASTER_CA_CERT", master.ca.display().to_string()),
    ];
    env.extend(extra_env.iter().cloned());
    env.push(duo::admins_env());
    let setup = format!(
        "set ui_mapname {}; set ui_gametype dm",
        minecraft_multiplayer::MAP
    );
    let host = duo::Player::launch(bin, &dirs[0], "host", session, displays[0], &env)?;
    let mut pair = duo::Pair::new(host);
    pair.host
        .send(&format!("{setup}; ui_create_lobby; ui_lobby_privacy"))?;
    let lobby = Duration::from_secs(minecraft_multiplayer::LOAD_SECS);
    pair.until("the host's lobby", lobby, |pair| {
        field(&pair.host.status(), "state") == Some("hosting")
    })?;
    let status = pair.host.status();
    let room = field(&status, "room")
        .ok_or("no room in the host's status")?
        .to_owned();
    let client = duo::Player::launch(bin, &dirs[1], "client", session, displays[1], &env)?;
    pair.client = Some(client);
    pair.client
        .as_mut()
        .expect("launched")
        .send(&format!("{setup}; ui_join_lobby_id {room}"))?;
    pair.until("the client in the lobby", lobby, |pair| {
        pair.client
            .as_ref()
            .is_some_and(|c| field(&c.status(), "state") == Some("joined"))
            && field(&pair.host.status(), "members")
                .and_then(|n| n.parse::<usize>().ok())
                .is_some_and(|n| n >= 2)
    })?;
    pair.host.send("ui_start_match")?;
    Ok(pair)
}

/// Both games end through the normal exit: the client first, so the host
/// keeps its inventory, then the host, which saves its world.
fn close_duo(pair: &mut duo::Pair) -> Result<(), String> {
    let quit = Duration::from_secs(240);
    if let Some(client) = pair.client.as_mut() {
        client.send("finish_run")?;
    }
    pair.until("the client to exit", quit, |pair| {
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
    pair.host.send("wait 3s; finish_run")?;
    let start = std::time::Instant::now();
    while pair.host.exited().is_none() {
        pair.sample();
        if start.elapsed() > quit {
            return Err("the host did not exit".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    pair.host.pump();
    Ok(())
}

fn run_minecraft_multiplayer(root: &Path, args: &Args) -> Result<bool, String> {
    asset_transport::load_dotenv();
    let part = scenario::Part::of(minecraft_multiplayer::PARTS, args.part.as_deref())?;
    let world = minecraft_world(args)?;
    // Natural spawning only for the spawn range, in the part that checks it.
    let mobs = part.plays(minecraft_multiplayer::MOBS);
    let master_bin = args
        .bin
        .with_file_name(format!("iw4l-master{}", std::env::consts::EXE_SUFFIX));
    for bin in [&args.bin, &master_bin] {
        if !bin.is_file() {
            return Err(format!(
                "{} is not built (cargo build --profile play -p launcher --bin iw4l -p iw4l-master)",
                bin.display()
            ));
        }
    }
    let games = asset_transport::games_root_from_env()?;
    let mw2_installed = asset_transport::list_mp_maps(&games)
        .iter()
        .any(|m| m.starts_with("iw4:"));
    let run_id = format!(
        "{}-{}",
        utc_stamp(),
        part.run_name(minecraft_multiplayer::NAME)
    );
    let run_dir = root.join("iw4l-artifacts/approved-tests").join(&run_id);
    std::fs::create_dir_all(run_dir.parent().expect("has parent"))
        .map_err(|e| format!("create run root: {e}"))?;
    std::fs::create_dir(&run_dir).map_err(|e| format!("create {}: {e}", run_dir.display()))?;
    let mut manifest = json!({
        "scenario": minecraft_multiplayer::NAME,
        "part": part.0,
        "run_id": run_id,
        "result": "running",
        "build": report::build_facts(root, &args.bin),
        "platform": report::platform_facts(),
        "content": { "games_root": games.0.display().to_string(), "mw2_installed": mw2_installed },
        "map": minecraft_multiplayer::MAP,
        "minecraft_world": world.facts(mobs),
        "view_distance": std::env::var("IW4L_MINECRAFT_VIEW_DISTANCE").ok(),
    });
    if !mw2_installed {
        manifest["result"] = "blocked".into();
        manifest["failure"] = "no MW2 multiplayer zone under the games root".into();
        report::write(&run_dir, &manifest)?;
        return Ok(false);
    }
    let dirs = [run_dir.join("host"), run_dir.join("client")];
    let mut caches = Vec::new();
    for (dir, name) in dirs.iter().zip(["host", "client"]) {
        let profile = dir.join("iw4l-artifacts/profile");
        std::fs::create_dir_all(&profile)
            .map_err(|e| format!("create {}: {e}", profile.display()))?;
        std::fs::write(profile.join("classes.txt"), minecraft_multiplayer::CLASSES)
            .map_err(|e| format!("write the class file: {e}"))?;
        std::fs::write(
            dir.join("settings.cfg"),
            format!("resolution=960x540\nfullscreen=false\nvsync=true\nplayer_name={name}\n"),
        )
        .map_err(|e| format!("write the settings: {e}"))?;
        if name == "client" {
            duo::write_client_key(dir)?;
        }
        caches.push(prepare_cache(root, dir, args.cache)?);
        share_minecraft_files(root, dir)?;
    }
    manifest["cache"] = caches.into();
    let bin = stage_binary(&args.bin, &run_dir)?;
    report::write(&run_dir, &manifest)?;
    println!("run: {}", run_dir.display());

    let master = duo::Master::start(&master_bin, &run_dir.join("master"))?;
    let mut facts = minecraft_multiplayer::Facts::default();
    let mut peaks = Vec::new();
    let mut failure = None;
    // Part `mobs` needs no restart.
    let sessions: &[u32] = if part.plays(minecraft_multiplayer::WORLD) {
        &[1, 2]
    } else {
        &[1]
    };
    for &session in sessions {
        let mut pair = match open_duo(
            &bin,
            &master,
            &dirs,
            session,
            minecraft_multiplayer::DISPLAYS,
            &world.env(),
        ) {
            Ok(pair) => pair,
            Err(e) => {
                failure = Some(format!("session {session}: {e}"));
                break;
            }
        };
        let played = if session == 1 {
            minecraft_multiplayer::first_session(&mut pair, &mut facts, part)
        } else {
            minecraft_multiplayer::second_session(&mut pair, &mut facts)
        };
        let closed = close_duo(&mut pair);
        pair.host.pump();
        facts.mobnet.append(&mut pair.host.mobnet);
        peaks.push(json!({
            "session": session,
            "host_peak_rss_mib": pair.host.peak_rss_kib / 1024,
            "client_peak_rss_mib": pair.peak_client_kib / 1024,
            "combined_peak_rss_mib": pair.peak_combined_kib / 1024,
            "host_display": pair.host.display.label(),
            "client_display": minecraft_multiplayer::DISPLAYS[1].label(),
            "host_cpu_secs": (pair.host.cpu_secs * 10.0).round() / 10.0,
            "client_cpu_secs": (pair.client_cpu_secs * 10.0).round() / 10.0,
        }));
        if let Err(e) = played.and(closed) {
            failure = Some(format!("session {session}: {e}"));
            break;
        }
        if session == 1 {
            let level = dirs[0].join("minecraft-worlds/world/level.json");
            manifest["saved_world"] =
                json!({ "level": level.display().to_string(), "exists": level.is_file() });
            // The second session plays it as a world saved before maps: its
            // players are each owed a map once more.
            if sessions.len() > 1 {
                let world = dirs[0].join("minecraft-worlds/world");
                match scenarios::map_steps::age_players(&world) {
                    Ok(aged) => {
                        facts.notes.insert("aged_players", aged.to_string());
                    }
                    Err(e) => {
                        failure = Some(format!("aging the saved world: {e}"));
                        break;
                    }
                }
            }
        }
    }
    drop(master);
    for dir in &dirs {
        keep_minecraft_files(root, dir);
    }
    manifest["memory"] = peaks.into();
    let mut checks = minecraft_multiplayer::judge(&facts);
    checks.retain(|c| part.judges(minecraft_multiplayer::PARTS, c.label));
    for check in &checks {
        match check.failures.as_slice() {
            [] => println!("  pass  {}", check.label),
            failures => println!("  FAIL  {}: {}", check.label, failures.join("; ")),
        }
    }
    let failed: Vec<&str> = checks
        .iter()
        .filter(|c| !c.failures.is_empty())
        .map(|c| c.label)
        .collect();
    let passed = failure.is_none() && failed.is_empty();
    manifest["checks"] = checks
        .iter()
        .map(|c| json!({ "label": c.label, "failures": c.failures }))
        .collect::<Vec<_>>()
        .into();
    manifest["lines"] = facts
        .lines
        .iter()
        .map(|(label, event)| (label.clone(), json!(event.fields)))
        .collect::<serde_json::Map<_, _>>()
        .into();
    manifest["mob_bandwidth"] = facts
        .mobnet
        .iter()
        .map(|e| json!(e.fields))
        .collect::<Vec<_>>()
        .into();
    manifest["answers"] = json!(facts.notes);
    manifest["result"] = if passed { "passed" } else { "failed" }.into();
    if let Some(failure) = failure {
        manifest["failure"] = failure.into();
    } else if !failed.is_empty() {
        manifest["failure"] = format!("checkpoints failed: {}", failed.join(", ")).into();
    }
    report::write(&run_dir, &manifest)?;
    println!(
        "{}: {}",
        manifest["result"].as_str().unwrap_or("?"),
        run_dir.join("run.json").display()
    );
    Ok(passed)
}

/// The dedicated server's two lives, each with the client's visits, every
/// step in `dedicated_server`. Rosters and memory peaks are kept even when
/// a step fails.
/// `world`: the settings of the world the server makes.
#[allow(clippy::too_many_arguments)]
fn dedicated_sessions(
    bin: &Path,
    master_bin: &Path,
    master: &duo::Master,
    dirs: &[PathBuf; 2],
    part: scenario::Part,
    world: &[(&'static str, String)],
    facts: &mut dedicated_server::Facts,
    peaks: &mut Vec<Value>,
) -> Result<(), String> {
    use dedicated_server as ds;
    let env = [
        ("IW4L_MASTER_ADDR", master.addr.clone()),
        ("IW4L_MASTER_SERVER_NAME", duo::SERVER_NAME.to_owned()),
        ("IW4L_MASTER_CA_CERT", master.ca.display().to_string()),
    ];
    let mut server_env = env.to_vec();
    server_env.push(("IW4L_MASTER_HOST_NAME", ds::ROOM.to_owned()));
    server_env.push(duo::admins_env());
    server_env.extend(world.iter().cloned());
    server_env.push((
        "IW4L_MINECRAFT_WORLDS",
        dirs[0].join("minecraft-worlds").display().to_string(),
    ));
    // A Unix socket's path is at most 107 bytes: the run directory is too deep.
    let socket = std::env::temp_dir().join(format!("iw4l-approved-{}.sock", std::process::id()));
    server_env.push(("IW4L_CONSOLE_SOCKET", socket.display().to_string()));
    let list = || {
        let out = std::process::Command::new(master_bin)
            .args([
                "list",
                "--connect",
                &master.addr,
                "--server-name",
                duo::SERVER_NAME,
            ])
            .arg("--ca-cert")
            .arg(&master.ca)
            .output()
            .map_err(|e| format!("iw4l-master list: {e}"))?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let client =
        |session| duo::Player::launch(bin, &dirs[1], "client", session, ds::CLIENT_DISPLAY, &env);
    // Part `mobs` needs no second life.
    let lives: &[u32] = if part.plays(ds::VISITS) {
        &[1, 2]
    } else {
        &[1]
    };
    for &life in lives {
        let server = duo::Player::launch_with(
            bin,
            &dirs[0],
            "server",
            life,
            &["dedicated", ds::MAP],
            &server_env,
        )?;
        let mut pair = duo::Pair::new(server);
        let played = (|| {
            if !part.plays(ds::VISITS) {
                let room = ds::server_ready(&mut pair, facts, "s_ready")?;
                pair.client = Some(client(1)?);
                ds::mobs_visit(&mut pair, facts, &room)?;
                ds::leave(&mut pair)?;
                ds::stop(&mut pair, facts, "stop1")
            } else if life == 1 {
                let room = ds::server_ready(&mut pair, facts, "s_ready")?;
                ds::console_socket(bin, &socket, 1, facts)?;
                ds::measure(&mut pair, facts, "no_client")?;
                pair.client = Some(client(1)?);
                ds::first_visit(&mut pair, facts, &room)?;
                ds::measure(&mut pair, facts, "one_client")?;
                if part.plays(ds::MOBS) {
                    ds::mob_visit(&mut pair, facts)?;
                }
                ds::leave(&mut pair)?;
                ds::empty_room(&mut pair, facts, &list)?;
                pair.client = Some(client(2)?);
                ds::revisit(&mut pair, facts, &room, 2)?;
                ds::leave(&mut pair)?;
                ds::stop(&mut pair, facts, "stop1")
            } else {
                let room = ds::server_ready(&mut pair, facts, "s2_ready")?;
                ds::console_socket(bin, &socket, 2, facts)?;
                pair.client = Some(client(3)?);
                ds::revisit(&mut pair, facts, &room, 3)?;
                ds::leave(&mut pair)?;
                ds::stop(&mut pair, facts, "stop2")
            }
        })();
        pair.host.pump();
        facts.rosters.append(&mut pair.host.rosters);
        facts.refusals.append(&mut pair.host.refusals);
        facts.joins.append(&mut pair.host.joins);
        facts.mobnet.append(&mut pair.host.mobnet);
        peaks.push(json!({
            "server_life": life,
            "server_peak_rss_mib": pair.host.peak_rss_kib / 1024,
            "client_peak_rss_mib": pair.peak_client_kib / 1024,
            "combined_peak_rss_mib": pair.peak_combined_kib / 1024,
            "host_display": pair.host.display.label(),
            "client_display": ds::CLIENT_DISPLAY.label(),
            "host_cpu_secs": (pair.host.cpu_secs * 10.0).round() / 10.0,
            "client_cpu_secs": (pair.client_cpu_secs * 10.0).round() / 10.0,
        }));
        played.map_err(|e| format!("server life {life}: {e}"))?;
    }
    Ok(())
}

fn run_dedicated_server(root: &Path, args: &Args) -> Result<bool, String> {
    asset_transport::load_dotenv();
    let part = scenario::Part::of(dedicated_server::PARTS, args.part.as_deref())?;
    let world = minecraft_world(args)?;
    // Natural spawning only for the spawn range, in the part that checks it.
    let mobs = part.plays(dedicated_server::MOBS);
    let master_bin = args
        .bin
        .with_file_name(format!("iw4l-master{}", std::env::consts::EXE_SUFFIX));
    for bin in [&args.bin, &master_bin] {
        if !bin.is_file() {
            return Err(format!(
                "{} is not built (cargo build --profile play -p launcher --bin iw4l -p iw4l-master)",
                bin.display()
            ));
        }
    }
    let games = asset_transport::games_root_from_env()?;
    let mw2_installed = asset_transport::list_mp_maps(&games)
        .iter()
        .any(|m| m.starts_with("iw4:"));
    let run_id = format!("{}-{}", utc_stamp(), part.run_name(dedicated_server::NAME));
    let run_dir = root.join("iw4l-artifacts/approved-tests").join(&run_id);
    std::fs::create_dir_all(run_dir.parent().expect("has parent"))
        .map_err(|e| format!("create run root: {e}"))?;
    std::fs::create_dir(&run_dir).map_err(|e| format!("create {}: {e}", run_dir.display()))?;
    let mut manifest = json!({
        "scenario": dedicated_server::NAME,
        "part": part.0,
        "run_id": run_id,
        "result": "running",
        "build": report::build_facts(root, &args.bin),
        "platform": report::platform_facts(),
        "content": { "games_root": games.0.display().to_string(), "mw2_installed": mw2_installed },
        "map": dedicated_server::MAP,
        "minecraft_world": world.facts(mobs),
    });
    if !mw2_installed {
        manifest["result"] = "blocked".into();
        manifest["failure"] = "no MW2 multiplayer zone under the games root".into();
        report::write(&run_dir, &manifest)?;
        return Ok(false);
    }
    let dirs = [run_dir.join("server"), run_dir.join("client")];
    let mut caches = Vec::new();
    for (dir, name) in dirs.iter().zip(["server", dedicated_server::CLIENT]) {
        let profile = dir.join("iw4l-artifacts/profile");
        std::fs::create_dir_all(&profile)
            .map_err(|e| format!("create {}: {e}", profile.display()))?;
        std::fs::write(profile.join("classes.txt"), dedicated_server::CLASSES)
            .map_err(|e| format!("write the class file: {e}"))?;
        std::fs::write(
            dir.join("settings.cfg"),
            format!("resolution=960x540\nfullscreen=false\nvsync=true\nplayer_name={name}\n"),
        )
        .map_err(|e| format!("write the settings: {e}"))?;
        if name == "client" {
            duo::write_client_key(dir)?;
        }
        caches.push(prepare_cache(root, dir, args.cache)?);
        share_minecraft_files(root, dir)?;
    }
    manifest["cache"] = caches.into();
    let bin = stage_binary(&args.bin, &run_dir)?;
    report::write(&run_dir, &manifest)?;
    println!("run: {}", run_dir.display());

    let master = duo::Master::start(&master_bin, &run_dir.join("master"))?;
    let mut facts = dedicated_server::Facts::default();
    let mut peaks = Vec::new();
    let failure = dedicated_sessions(
        &bin,
        &master_bin,
        &master,
        &dirs,
        part,
        &world.env(),
        &mut facts,
        &mut peaks,
    )
    .err();
    drop(master);
    for dir in &dirs {
        keep_minecraft_files(root, dir);
    }
    manifest["memory"] = peaks.into();
    manifest["server_measures"] = facts
        .measures
        .iter()
        .map(|(point, m)| {
            (
                (*point).to_owned(),
                json!({
                    "rss_mib": m.rss_mib,
                    "peak_rss_mib": m.peak_rss_mib,
                    "cpu_percent_of_one_core": (m.cpu_percent * 10.0).round() / 10.0,
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>()
        .into();
    manifest["mob_bandwidth"] = facts
        .mobnet
        .iter()
        .map(|e| json!(e.fields))
        .collect::<Vec<_>>()
        .into();
    let mut checks = dedicated_server::judge(&facts);
    checks.retain(|c| part.judges(dedicated_server::PARTS, c.label));
    for check in &checks {
        match check.failures.as_slice() {
            [] => println!("  pass  {}", check.label),
            failures => println!("  FAIL  {}: {}", check.label, failures.join("; ")),
        }
    }
    let failed: Vec<&str> = checks
        .iter()
        .filter(|c| !c.failures.is_empty())
        .map(|c| c.label)
        .collect();
    let passed = failure.is_none() && failed.is_empty();
    manifest["checks"] = checks
        .iter()
        .map(|c| json!({ "label": c.label, "failures": c.failures }))
        .collect::<Vec<_>>()
        .into();
    manifest["lines"] = facts
        .lines
        .iter()
        .map(|(label, event)| (label.clone(), json!(event.fields)))
        .collect::<serde_json::Map<_, _>>()
        .into();
    manifest["rosters"] = facts
        .rosters
        .iter()
        .map(|event| json!(event.fields))
        .collect::<Vec<_>>()
        .into();
    manifest["answers"] = json!(facts.notes);
    manifest["result"] = if passed { "passed" } else { "failed" }.into();
    if let Some(failure) = failure {
        manifest["failure"] = failure.into();
    } else if !failed.is_empty() {
        manifest["failure"] = format!("checkpoints failed: {}", failed.join(", ")).into();
    }
    report::write(&run_dir, &manifest)?;
    println!(
        "{}: {}",
        manifest["result"].as_str().unwrap_or("?"),
        run_dir.join("run.json").display()
    );
    Ok(passed)
}

/// `skate_multiplayer`'s two rooms: a listen host and its client, then a
/// dedicated server and a client. Memory peaks are kept even when a step
/// fails.
/// `world`: the settings of the world each host makes.
#[allow(clippy::too_many_arguments)]
fn skate_sessions(
    bin: &Path,
    master: &duo::Master,
    dirs: &[PathBuf; 3],
    skate_assets: &str,
    part: scenario::Part,
    world: &[(&'static str, String)],
    facts: &mut skate_multiplayer::Facts,
    peaks: &mut Vec<Value>,
) -> Result<(), String> {
    use duo::field;
    use skate_multiplayer as sk;
    let skate = [("IW4L_SKATE_ASSETS", skate_assets.to_owned())];
    if part.plays(sk::LISTEN) {
        let listen_dirs = [dirs[0].clone(), dirs[1].clone()];
        let mut listen_env = skate.to_vec();
        listen_env.extend(world.iter().cloned());
        let mut pair = open_duo(
            bin,
            master,
            &listen_dirs,
            1,
            sk::LISTEN_DISPLAYS,
            &listen_env,
        )?;
        let played = sk::listen_session(&mut pair, facts);
        let closed = close_duo(&mut pair);
        peaks.push(json!({
            "room": "listen",
            "host_peak_rss_mib": pair.host.peak_rss_kib / 1024,
            "client_peak_rss_mib": pair.peak_client_kib / 1024,
            "combined_peak_rss_mib": pair.peak_combined_kib / 1024,
            "host_display": pair.host.display.label(),
            "client_display": sk::LISTEN_DISPLAYS[1].label(),
            "host_cpu_secs": (pair.host.cpu_secs * 10.0).round() / 10.0,
            "client_cpu_secs": (pair.client_cpu_secs * 10.0).round() / 10.0,
        }));
        played
            .and(closed)
            .map_err(|e| format!("listen room: {e}"))?;
    }
    if !part.plays(sk::DEDICATED) {
        return Ok(());
    }

    let env = [
        ("IW4L_MASTER_ADDR", master.addr.clone()),
        ("IW4L_MASTER_SERVER_NAME", duo::SERVER_NAME.to_owned()),
        ("IW4L_MASTER_CA_CERT", master.ca.display().to_string()),
    ];
    let mut server_env = env.to_vec();
    server_env.push(("IW4L_MASTER_HOST_NAME", sk::ROOM.to_owned()));
    server_env.push(duo::admins_env());
    server_env.extend(world.iter().cloned());
    server_env.push((
        "IW4L_MINECRAFT_WORLDS",
        dirs[2].join("minecraft-worlds").display().to_string(),
    ));
    let server = duo::Player::launch_with(
        bin,
        &dirs[2],
        "server",
        1,
        &["dedicated", sk::MAP],
        &server_env,
    )?;
    let mut pair = duo::Pair::new(server);
    let played = (|| {
        let load = Duration::from_secs(sk::LOAD_SECS);
        pair.until("the server's room in its match", load, |pair| {
            let status = pair.host.status();
            field(&status, "state") == Some("hosting") && field(&status, "in_match") == Some("true")
        })?;
        let room = field(&pair.host.status(), "room")
            .ok_or("no room in the server's status")?
            .to_owned();
        pair.ask(false, "wait minecraft; skate state s_ready", "s_ready", load)?;
        let mut client_env = env.to_vec();
        client_env.extend(skate.iter().cloned());
        pair.client = Some(duo::Player::launch(
            bin,
            &dirs[1],
            "client",
            2,
            sk::DEDICATED_CLIENT_DISPLAY,
            &client_env,
        )?);
        sk::dedicated_session(&mut pair, facts, &room)?;
        dedicated_server::leave(&mut pair)?;
        pair.host.terminate()
    })();
    let start = std::time::Instant::now();
    while pair.host.exited().is_none() && start.elapsed() < Duration::from_secs(60) {
        pair.sample();
        std::thread::sleep(Duration::from_millis(100));
    }
    pair.host.pump();
    peaks.push(json!({
        "room": "dedicated",
        "server_peak_rss_mib": pair.host.peak_rss_kib / 1024,
        "client_peak_rss_mib": pair.peak_client_kib / 1024,
        "combined_peak_rss_mib": pair.peak_combined_kib / 1024,
        "host_display": pair.host.display.label(),
        "client_display": sk::DEDICATED_CLIENT_DISPLAY.label(),
        "host_cpu_secs": (pair.host.cpu_secs * 10.0).round() / 10.0,
        "client_cpu_secs": (pair.client_cpu_secs * 10.0).round() / 10.0,
    }));
    played.map_err(|e| format!("dedicated room: {e}"))
}

fn run_skate_multiplayer(root: &Path, args: &Args) -> Result<bool, String> {
    asset_transport::load_dotenv();
    let part = scenario::Part::of(skate_multiplayer::PARTS, args.part.as_deref())?;
    let world = minecraft_world(args)?;
    let master_bin = args
        .bin
        .with_file_name(format!("iw4l-master{}", std::env::consts::EXE_SUFFIX));
    for bin in [&args.bin, &master_bin] {
        if !bin.is_file() {
            return Err(format!(
                "{} is not built (cargo build --profile play -p launcher --bin iw4l -p iw4l-master)",
                bin.display()
            ));
        }
    }
    let games = asset_transport::games_root_from_env()?;
    let mw2_installed = asset_transport::list_mp_maps(&games)
        .iter()
        .any(|m| m.starts_with("iw4:"));
    let skate_assets = std::env::var("IW4L_SKATE_ASSETS").ok().filter(|root| {
        let root = Path::new(root);
        root.join("rig.json").is_file() && root.join("board.json").is_file()
    });
    let run_id = format!("{}-{}", utc_stamp(), part.run_name(skate_multiplayer::NAME));
    let run_dir = root.join("iw4l-artifacts/approved-tests").join(&run_id);
    std::fs::create_dir_all(run_dir.parent().expect("has parent"))
        .map_err(|e| format!("create run root: {e}"))?;
    std::fs::create_dir(&run_dir).map_err(|e| format!("create {}: {e}", run_dir.display()))?;
    let mut manifest = json!({
        "scenario": skate_multiplayer::NAME,
        "part": part.0,
        "run_id": run_id,
        "result": "running",
        "build": report::build_facts(root, &args.bin),
        "platform": report::platform_facts(),
        "content": {
            "games_root": games.0.display().to_string(),
            "mw2_installed": mw2_installed,
            "skate_assets": skate_assets.is_some(),
        },
        "map": skate_multiplayer::MAP,
        "minecraft_world": world.facts(false),
    });
    let Some(skate_assets) = skate_assets.filter(|_| mw2_installed) else {
        manifest["result"] = "blocked".into();
        manifest["failure"] = "needs MW2 under the games root and the Skate 3 data (IW4L_SKATE_ASSETS with rig.json and board.json)".into();
        report::write(&run_dir, &manifest)?;
        return Ok(false);
    };
    let dirs = [
        run_dir.join("host"),
        run_dir.join("client"),
        run_dir.join("server"),
    ];
    let mut caches = Vec::new();
    for (dir, name) in dirs.iter().zip(["host", skate_multiplayer::CLIENT, "server"]) {
        let profile = dir.join("iw4l-artifacts/profile");
        std::fs::create_dir_all(&profile)
            .map_err(|e| format!("create {}: {e}", profile.display()))?;
        std::fs::write(profile.join("classes.txt"), skate_multiplayer::CLASSES)
            .map_err(|e| format!("write the class file: {e}"))?;
        std::fs::write(
            dir.join("settings.cfg"),
            format!("resolution=960x540\nfullscreen=false\nvsync=true\nplayer_name={name}\n"),
        )
        .map_err(|e| format!("write the settings: {e}"))?;
        if name == "client" {
            duo::write_client_key(dir)?;
        }
        caches.push(prepare_cache(root, dir, args.cache)?);
        share_minecraft_files(root, dir)?;
    }
    manifest["cache"] = caches.into();
    let bin = stage_binary(&args.bin, &run_dir)?;
    report::write(&run_dir, &manifest)?;
    println!("run: {}", run_dir.display());

    let master = duo::Master::start(&master_bin, &run_dir.join("master"))?;
    let mut facts = skate_multiplayer::Facts::default();
    let mut peaks = Vec::new();
    let failure = skate_sessions(
        &bin,
        &master,
        &dirs,
        &skate_assets,
        part,
        &world.env(),
        &mut facts,
        &mut peaks,
    )
    .err();
    drop(master);
    for dir in &dirs {
        keep_minecraft_files(root, dir);
    }
    manifest["memory"] = peaks.into();
    manifest["bandwidth"] = json!(skate_multiplayer::bandwidth(&facts));
    let mut checks = skate_multiplayer::judge(&facts);
    checks.retain(|c| part.judges(skate_multiplayer::PARTS, c.label));
    for check in &checks {
        match check.failures.as_slice() {
            [] => println!("  pass  {}", check.label),
            failures => println!("  FAIL  {}: {}", check.label, failures.join("; ")),
        }
    }
    let failed: Vec<&str> = checks
        .iter()
        .filter(|c| !c.failures.is_empty())
        .map(|c| c.label)
        .collect();
    let passed = failure.is_none() && failed.is_empty();
    manifest["checks"] = checks
        .iter()
        .map(|c| json!({ "label": c.label, "failures": c.failures }))
        .collect::<Vec<_>>()
        .into();
    manifest["lines"] = facts
        .lines
        .iter()
        .map(|(label, event)| (label.clone(), json!(event.fields)))
        .collect::<serde_json::Map<_, _>>()
        .into();
    manifest["answers"] = json!(facts.notes);
    manifest["result"] = if passed { "passed" } else { "failed" }.into();
    if let Some(failure) = failure {
        manifest["failure"] = failure.into();
    } else if !failed.is_empty() {
        manifest["failure"] = format!("checkpoints failed: {}", failed.join(", ")).into();
    }
    report::write(&run_dir, &manifest)?;
    println!(
        "{}: {}",
        manifest["result"].as_str().unwrap_or("?"),
        run_dir.join("run.json").display()
    );
    Ok(passed)
}

const MINECRAFT_FILES: &str = "iw4l-artifacts/minecraft-26.3";

/// The game fetches Minecraft's files into its working directory. A run
/// directory links the repository's complete copy; a link to an incomplete
/// one would be replaced by the fetch, which swaps its result into place.
fn share_minecraft_files(root: &Path, run_dir: &Path) -> Result<String, String> {
    let shared = root.join(MINECRAFT_FILES);
    if !shared.join(".complete").is_file() {
        return Ok("fetched in the run; kept for the next".into());
    }
    let child = run_dir.join("iw4l-artifacts");
    std::fs::create_dir_all(&child).map_err(|e| format!("create {}: {e}", child.display()))?;
    let link = run_dir.join(MINECRAFT_FILES);
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&shared, &link);
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_dir(&shared, &link);
    linked.map_err(|e| format!("link {}: {e}", link.display()))?;
    Ok(shared.display().to_string())
}

/// Moves files a run fetched to where the next run links them from.
fn keep_minecraft_files(root: &Path, run_dir: &Path) {
    let fetched = run_dir.join(MINECRAFT_FILES);
    let shared = root.join(MINECRAFT_FILES);
    if fetched.is_symlink()
        || !fetched.join(".complete").is_file()
        || shared.join(".complete").is_file()
    {
        return;
    }
    let _ = std::fs::remove_dir_all(&shared);
    if let Err(e) = std::fs::rename(&fetched, &shared) {
        eprintln!("keep {}: {e}", fetched.display());
    }
}

/// On Windows the launcher enters the folder holding its executable, so the binary must sit in the run directory.
fn stage_binary(bin: &Path, run_dir: &Path) -> Result<PathBuf, String> {
    if !cfg!(windows) {
        return Ok(bin.to_path_buf());
    }
    let exe = bin.with_extension("exe");
    let bin = if bin.exists() { bin } else { exe.as_path() };
    let name = bin
        .file_name()
        .ok_or_else(|| format!("--bin {} names no file", bin.display()))?;
    let staged = run_dir.join(name);
    std::fs::hard_link(bin, &staged)
        .or_else(|_| std::fs::copy(bin, &staged).map(drop))
        .map_err(|e| format!("stage {} into {}: {e}", bin.display(), run_dir.display()))?;
    Ok(staged)
}

fn prepare_cache(root: &Path, run_dir: &Path, mode: CacheMode) -> Result<Value, String> {
    let child_artifacts = run_dir.join("iw4l-artifacts");
    let child_cache = child_artifacts.join("cache");
    let uncontrolled = json!([
        "OS page cache",
        "GPU driver shader caches (e.g. ~/.cache/mesa_shader_cache)",
        "games install on disk",
    ]);
    match mode {
        CacheMode::Cold => {
            let cold = !child_cache.exists();
            Ok(json!({
                "mode": "cold",
                "cold_load": cold,
                "root": child_cache.display().to_string(),
                "reset": ["every IW4L-managed loading product: iw4l-artifacts/cache (localize, mips, nav, wgsl, xwma_pcm) starts empty in the run directory"],
                "kept": ["the repository's iw4l-artifacts/cache, untouched", ".env and the games root"],
                "not_controlled": uncontrolled,
            }))
        }
        CacheMode::Shared => {
            let shared = root.join("iw4l-artifacts/cache");
            // A fresh checkout has no cache yet, and a link to nothing would
            // leave the games unable to create theirs.
            for dir in [&shared, &child_artifacts] {
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("create {}: {e}", dir.display()))?;
            }
            #[cfg(unix)]
            let linked = std::os::unix::fs::symlink(&shared, &child_cache);
            #[cfg(windows)]
            let linked = std::os::windows::fs::symlink_dir(&shared, &child_cache);
            linked.map_err(|e| format!("link {}: {e}", child_cache.display()))?;
            Ok(json!({
                "mode": "shared",
                "cold_load": false,
                "root": shared.display().to_string(),
                "reset": [],
                "kept": ["the repository's iw4l-artifacts/cache, reused and written to"],
                "not_controlled": uncontrolled,
            }))
        }
    }
}

fn utc_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(all(test, unix))]
mod cache_tests {
    use super::*;

    #[test]
    fn a_shared_cache_is_made_on_a_fresh_checkout() {
        let root =
            std::env::temp_dir().join(format!("iw4l-shared-cache-{}", std::process::id()));
        let run_dir = root.join("iw4l-artifacts/approved-tests/run");
        let _ = std::fs::remove_dir_all(&root);
        prepare_cache(&root, &run_dir, CacheMode::Shared).expect("shared cache");
        let child_cache = run_dir.join("iw4l-artifacts/cache");
        assert!(child_cache.is_dir(), "the run's cache leads to a directory");
        std::fs::create_dir_all(child_cache.join("wgsl/ab")).expect("a game makes its cache");
        assert!(root.join("iw4l-artifacts/cache/wgsl/ab").is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }
}
