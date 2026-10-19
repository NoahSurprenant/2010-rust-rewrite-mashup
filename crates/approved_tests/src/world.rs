//! The Minecraft world of the scenarios on the Minecraft map
//! (`crafting_table`, `minecraft_multiplayer`, `dedicated_server`,
//! `skate_multiplayer`): one fixed seed and spawn, so that every run plays
//! the same ground from the same spot, and no mobs but summoned ones until
//! the one step that checks natural spawning turns it on (`mc spawning on`). `--seed N` (with `--spawn "x y z"`) plays another world on purpose;
//! `--replay run.json` plays a run's again.

use std::path::Path;

use serde_json::{Value, json};

/// The world seed of every scenario on the Minecraft map.
pub const SEED: i64 = 12345;

/// Where the world's first player stands at `SEED` (feet, Minecraft block
/// coordinates): a spot vanilla's spawn finder picked on that seed's ground,
/// within ten blocks of its world spawn. The game would pick another at
/// random each run; `IW4L_MINECRAFT_SPAWN` sets this one. Clients stand
/// three blocks apart from it along +Z.
pub const SPAWN: &str = "90.5 130 -30.5";

#[derive(Clone, Debug, PartialEq)]
pub struct World {
    pub seed: i64,
    /// `None`: the game's own spawn, random near the world spawn (another
    /// seed, given without `--spawn`).
    pub spawn: Option<String>,
}

impl World {
    /// The world a run plays: a replayed run's, else `--seed`'s (at
    /// `--spawn`, or at `SPAWN` when it is `SEED`), else the fixed one.
    pub fn choose(
        seed: Option<i64>,
        spawn: Option<&str>,
        replay: Option<&Value>,
    ) -> Result<World, String> {
        if let Some(spawn) = spawn {
            check_spawn(spawn)?;
        }
        if let Some(run) = replay {
            if seed.is_some() || spawn.is_some() {
                return Err("--replay keeps its own world: no --seed or --spawn".into());
            }
            let world = &run["minecraft_world"];
            let seed = world["seed"]
                .as_i64()
                .ok_or("replay: no minecraft_world.seed")?;
            let spawn = match &world["spawn"] {
                Value::Null => None,
                Value::String(spawn) => Some(check_spawn(spawn)?.to_owned()),
                other => return Err(format!("replay: minecraft_world.spawn {other}")),
            };
            return Ok(World { seed, spawn });
        }
        let seed = seed.unwrap_or(SEED);
        let spawn = match spawn {
            Some(spawn) => Some(spawn.to_owned()),
            None if seed == SEED => Some(SPAWN.to_owned()),
            None => None,
        };
        Ok(World { seed, spawn })
    }

    /// `choose` with a replay read from its run.json.
    pub fn from_args(
        seed: Option<i64>,
        spawn: Option<&str>,
        replay: Option<&Path>,
    ) -> Result<World, String> {
        let run = replay
            .map(|path| {
                let text = std::fs::read_to_string(path)
                    .map_err(|e| format!("replay {}: {e}", path.display()))?;
                serde_json::from_str::<Value>(&text)
                    .map_err(|e| format!("replay {}: {e}", path.display()))
            })
            .transpose()?;
        Self::choose(seed, spawn, run.as_ref())
    }

    /// The settings a game that makes this world starts with: no mob comes
    /// of itself.
    pub fn env(&self) -> Vec<(&'static str, String)> {
        let mut env = vec![
            ("IW4L_MINECRAFT_SEED", self.seed.to_string()),
            ("IW4L_MINECRAFT_MOBS", "off".into()),
        ];
        if let Some(spawn) = &self.spawn {
            env.push(("IW4L_MINECRAFT_SPAWN", spawn.clone()));
        }
        env
    }

    /// What run.json keeps of it (and `--replay` reads back); `spawning`
    /// for a part whose spawn-range step turns natural spawning on.
    pub fn facts(&self, spawning: bool) -> Value {
        let mobs = if spawning {
            "off: only summoned ones, until the spawn-range step turns natural spawning on"
        } else {
            "off: only summoned ones"
        };
        json!({ "seed": self.seed, "spawn": self.spawn, "mobs": mobs })
    }
}

fn check_spawn(spawn: &str) -> Result<&str, String> {
    let numbers = spawn
        .split_whitespace()
        .filter(|n| n.parse::<f64>().is_ok_and(f64::is_finite))
        .count();
    if numbers == 3 && spawn.split_whitespace().count() == 3 {
        Ok(spawn)
    } else {
        Err(format!("spawn `{spawn}`: want three numbers, \"x y z\""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_plays_the_fixed_world_by_default() {
        let world = World::choose(None, None, None).unwrap();
        assert_eq!(
            world,
            World {
                seed: SEED,
                spawn: Some(SPAWN.into())
            }
        );
        assert_eq!(
            world.env(),
            [
                ("IW4L_MINECRAFT_SEED", "12345".to_owned()),
                ("IW4L_MINECRAFT_MOBS", "off".to_owned()),
                ("IW4L_MINECRAFT_SPAWN", SPAWN.to_owned()),
            ]
        );
    }

    #[test]
    fn another_seed_is_played_on_purpose() {
        let other = World::choose(Some(-7), None, None).unwrap();
        assert_eq!(other, World { seed: -7, spawn: None });
        assert!(other.env().iter().all(|(k, _)| *k != "IW4L_MINECRAFT_SPAWN"));
        let placed = World::choose(Some(-7), Some("1 70 2"), None).unwrap();
        assert_eq!(placed.spawn.as_deref(), Some("1 70 2"));
        assert_eq!(
            World::choose(Some(SEED), None, None).unwrap().spawn.as_deref(),
            Some(SPAWN)
        );
        assert!(World::choose(None, Some("1 2"), None).is_err());
    }

    #[test]
    fn a_replay_plays_its_run_s_world_again() {
        for world in [
            World::choose(None, None, None).unwrap(),
            World::choose(Some(99), None, None).unwrap(),
        ] {
            let run = json!({ "minecraft_world": world.facts(false) });
            assert_eq!(World::choose(None, None, Some(&run)).unwrap(), world);
        }
        let world = World::choose(None, None, None).unwrap();
        let run = json!({ "minecraft_world": world.facts(true) });
        assert!(World::choose(Some(1), None, Some(&run)).is_err());
        assert!(World::choose(None, None, Some(&json!({}))).is_err());
    }
}
