//! The starting map's steps and checks, shared by `minecraft_multiplayer`
//! and `dedicated_server`: each player is given one filled map once per
//! world (on its hotbar's last slot), and the map in its hand fills in from
//! the host's world and reaches it.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::duo::Pair;
use crate::runner::Event;

pub const ITEM: &str = "minecraft:filled_map*1";
/// The starting map's slot: the hotbar's last, clear of the guns.
pub const SLOT: usize = 8;

/// The slots of a state line holding a filled map.
pub fn map_slots(event: &Event) -> Vec<usize> {
    event
        .get("slots")
        .unwrap_or("")
        .split(',')
        .enumerate()
        .filter(|(_, s)| *s == ITEM)
        .map(|(i, _)| i)
        .collect()
}

/// The map in hand as the game has it (`map=<id>@<x>,<z>:<pixels>`): its
/// id, centre (none before its data comes) and pixels shown.
pub fn held(event: &Event) -> Option<(u32, Option<(i32, i32)>, usize)> {
    let text = event.get("map").filter(|m| *m != "-")?;
    let (id, rest) = text.split_once('@')?;
    let (centre, shown) = rest.split_once(':')?;
    let centre = centre
        .split_once(',')
        .and_then(|(x, z)| Some((x.parse().ok()?, z.parse().ok()?)));
    Some((id.parse().ok()?, centre, shown.parse().ok()?))
}

/// Exactly one starting map, on its slot.
pub fn one_map(event: &Event, who: &str) -> Result<(), String> {
    match map_slots(event).as_slice() {
        [SLOT] => Ok(()),
        slots => Err(format!("{who}: filled maps in slots {slots:?}, want one in slot {SLOT}")),
    }
}

/// No filled map at all.
pub fn no_map(event: &Event, who: &str) -> Result<(), String> {
    match map_slots(event).as_slice() {
        [] => Ok(()),
        slots => Err(format!("{who}: filled maps in slots {slots:?}, want none")),
    }
}

/// The map in hand shows ground.
pub fn filled(event: &Event, who: &str) -> Result<(u32, Option<(i32, i32)>, usize), String> {
    match held(event) {
        Some(map @ (_, Some(_), shown)) if shown > 0 => Ok(map),
        Some((id, _, _)) => Err(format!("{who}: map {id} in hand shows nothing")),
        None => Err(format!("{who}: no map in hand ({})", event.get("map").unwrap_or("?"))),
    }
}

/// Takes the starting map in hand (hotbar key 9) until it shows ground or
/// `timeout` passes, then the first gun again; the last state line.
pub fn hold_map(pair: &mut Pair, client: bool, label: &str, timeout: Duration) -> Result<Event, String> {
    let step = Duration::from_secs(120);
    let start = Instant::now();
    let mut event = pair.ask(client, &format!("key 9; wait 3s; mc state {label}"), label, step)?;
    while filled(&event, label).is_err() && start.elapsed() < timeout {
        event = pair.ask(client, &format!("wait 1s; mc state {label}"), label, step)?;
    }
    let player = if client { pair.client.as_mut().ok_or("no client")? } else { &mut pair.host };
    player.send("key 1; wait 1s")?;
    Ok(event)
}

/// Makes a kept world's players.json one from before maps: every record
/// without `map_given`, as an older game saved it. How many records it took
/// the flag from.
pub fn age_players(world: &Path) -> Result<usize, String> {
    let path = world.join("players.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut players: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
    let mut aged = 0;
    if let Some(entries) = players.as_object_mut() {
        for record in entries.values_mut() {
            if let Some(record) = record.as_object_mut()
                && record.remove("map_given").is_some()
            {
                aged += 1;
            }
        }
    }
    let text = serde_json::to_string_pretty(&players).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(aged)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(fields: &str) -> Event {
        crate::runner::parse(&format!("minecraft-state: {fields}"), "minecraft-state:", 0).unwrap()
    }

    #[test]
    fn the_map_in_hand_reads_from_the_state_line() {
        assert_eq!(held(&event("label=x map=3@128,0:9216")), Some((3, Some((128, 0)), 9216)));
        assert_eq!(held(&event("label=x map=0@-:0")), Some((0, None, 0)));
        assert_eq!(held(&event("label=x map=-")), None);
        assert_eq!(held(&event("label=x")), None);
        assert!(filled(&event("label=x map=3@128,0:1"), "x").is_ok());
        assert!(filled(&event("label=x map=3@-:0"), "x").is_err());
        assert!(filled(&event("label=x map=3@128,0:0"), "x").is_err());
    }

    #[test]
    fn one_map_means_one_on_the_last_hotbar_slot() {
        let mut slots = vec!["-"; 41];
        slots[..5].copy_from_slice(&["iw4:weapon/1*1"; 5]);
        let line = |slots: &[&str]| event(&format!("label=x slots={}", slots.join(",")));
        assert!(no_map(&line(&slots), "x").is_ok());
        assert!(one_map(&line(&slots), "x").is_err());
        slots[8] = ITEM;
        assert!(one_map(&line(&slots), "x").is_ok());
        assert!(no_map(&line(&slots), "x").is_err());
        slots[20] = ITEM;
        assert_eq!(
            one_map(&line(&slots), "x").unwrap_err(),
            "x: filled maps in slots [8, 20], want one in slot 8"
        );
    }

    #[test]
    fn an_aged_world_forgets_who_was_given_a_map() {
        let dir = std::env::temp_dir().join(format!("approved-age-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("players.json"),
            r#"{"key:aa": {"name": "host", "first_seen": 1, "last_seen": 2, "inventory": null, "map_given": true},
                "key:bb": {"name": "client", "first_seen": 1, "last_seen": 2, "map_given": true},
                "old": {"selected": 0, "slots": [], "crafting": [], "workbench": []}}"#,
        )
        .unwrap();
        assert_eq!(age_players(&dir).unwrap(), 2);
        let text = std::fs::read_to_string(dir.join("players.json")).unwrap();
        assert!(!text.contains("map_given"));
        assert!(text.contains("\"old\""));
        assert_eq!(age_players(&dir).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
