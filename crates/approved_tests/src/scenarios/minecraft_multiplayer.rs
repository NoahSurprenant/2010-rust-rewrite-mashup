//! `minecraft_multiplayer` — owner-approved. This file is the one place its
//! steps and what each checkpoint must show live.
//!
//! A host opens a lobby on the Minecraft map through a master of the run's
//! own and a client joins it. The client must generate the host's world
//! (the same seed), see a block the host sets and one the host clears; the
//! host must see a block the client sets, one it places by hand (a right
//! click with planks the host gave it on its `mc give`) and one it breaks
//! by hand. Then both quit, the host and the client start again on the same
//! settings, and the host's world comes back: the same seed, its blocks,
//! the host's inventory and the client's (kept by player name).

use std::collections::BTreeMap;
use std::time::Duration;

use crate::duo::Pair;
use crate::runner::Event;

pub const NAME: &str = "minecraft_multiplayer";
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

/// Each line both games printed, by label (`h_` the host's, `c_` the
/// client's), and the facts the steps worked out.
#[derive(Default)]
pub struct Facts {
    pub lines: BTreeMap<String, Event>,
    pub notes: BTreeMap<&'static str, String>,
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

/// The first session: the lobby is up and the match started; everything
/// both games are asked, in order. Stops at the first step that cannot go
/// on (a missing answer); the judge says what failed.
pub fn first_session(pair: &mut Pair, facts: &mut Facts) -> Result<(), String> {
    let load = Duration::from_secs(LOAD_SECS);
    let step = Duration::from_secs(STEP_SECS);
    let sync = Duration::from_secs(SYNC_SECS);
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
    let feet = feet(&client).ok_or("c_ready has no feet")?;
    let stone = add(feet, [3, 0, 0]);
    let plank = add(feet, [2, 0, 0]);
    let script = format!(
        "{}; mc set {} {} {} {} {} {} air; mc clear; mc give oak_planks 4; wait 2s; mc state c_given",
        set(stone, "stone"),
        feet[0] + 1,
        feet[1],
        feet[2],
        feet[0] + 2,
        feet[1] + 1,
        feet[2]
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
    let last = pair.ask(true, "key 1; wait 1s; mc state c_final", "c_final", step)?;
    facts.keep("c_final", last);
    Ok(())
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
    let rules: [(&'static str, Rule); 11] = [
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
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(fields: &str) -> Event {
        crate::runner::parse(&format!("minecraft-state: {fields}"), "minecraft-state:", 0).unwrap()
    }

    fn passing() -> Facts {
        let mut f = Facts::default();
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
    fn feet_read_from_the_state_line() {
        assert_eq!(feet(&event("label=x feet=-1,2,30")), Some([-1, 2, 30]));
        assert_eq!(feet(&event("label=x feet=-")), None);
        assert_eq!(set([1, -2, 3], "air"), "mc set 1 -2 3 1 -2 3 air");
    }
}
