use serde_json::{Value, json};

pub struct Scenario {
    pub name: &'static str,
    pub gametype: &'static str,
    pub players: u32,
    pub class: &'static str,
    pub map_a: &'static str,
    pub map_b: MapB,
    pub scenes: &'static [Scene],
    pub phase_timeout_secs: u64,
    pub quit_timeout_secs: u64,
}

pub enum MapB {
    #[allow(dead_code)]
    Fixed(&'static str),
    FromInstalled {
        game: &'static str,
    },
}

pub struct Scene {
    pub name: &'static str,
    pub place: Place,
    pub weapon: Option<Weapon>,
    pub hold: &'static [&'static str],
    pub pulse: Option<(&'static str, u32)>,
    pub ticks: u32,
}

#[derive(Clone, Copy)]
pub enum Place {
    Random,
    #[allow(dead_code)]
    At {
        origin: [f32; 3],
        yaw: f32,
    },
}

pub struct Weapon {
    pub give: &'static str,
    pub fallback: &'static str,
}

pub struct ResolvedScene {
    pub tag: &'static str,
    pub map: String,
    pub index: usize,
    pub name: &'static str,
    pub place: ResolvedPlace,
    pub weapon: Option<&'static Weapon>,
    pub commands: Vec<String>,
}

#[derive(Clone, Copy)]
pub enum ResolvedPlace {
    Seeded(u64),
    At { origin: [f32; 3], yaw: f32 },
}

impl ResolvedScene {
    pub fn phase(&self) -> String {
        format!("{}.scene_{}", self.tag, self.index + 1)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "map": self.map,
            "phase": self.phase(),
            "name": self.name,
            "place": match self.place {
                ResolvedPlace::Seeded(seed) => json!({ "authored_spawn_seed": seed }),
                ResolvedPlace::At { origin, yaw } => json!({ "at": origin, "yaw": yaw }),
            },
            "weapon": self.weapon.map(|w| json!({ "give": w.give, "fallback": w.fallback })),
            "commands": self.commands,
        })
    }
}

pub fn scene_commands(scene: &Scene, place: ResolvedPlace) -> Vec<String> {
    let mut out = vec![match place {
        ResolvedPlace::Seeded(seed) => format!("force_spawn random {seed}"),
        ResolvedPlace::At { origin, yaw } => format!(
            "force_spawn at {} {} {} {}",
            origin[0], origin[1], origin[2], yaw
        ),
    }];
    out.push("wait 2t".into());
    out.push("mark {phase}.spawned".into());
    if let Some(weapon) = &scene.weapon {
        out.push(format!("give weapon/{}", weapon.give));
        out.push("wait 2t".into());
    }
    for input in scene.hold {
        out.push(format!("hold {input}"));
    }
    match scene.pulse {
        Some((input, every)) if every > 0 => {
            let mut left = scene.ticks;
            while left > 0 {
                let step = every.min(left);
                out.push(format!("press {input}"));
                out.push(format!("wait {step}t"));
                left -= step;
            }
        }
        _ => out.push(format!("wait {}t", scene.ticks)),
    }
    out.push("mark {phase}.inputs_done".into());
    out.push("release all".into());
    out
}

pub struct Phase {
    pub name: String,
    pub commands: Vec<String>,
    pub capture: Option<&'static str>,
}

impl Phase {
    pub fn new(name: impl Into<String>, commands: Vec<String>) -> Self {
        Self {
            name: name.into(),
            commands,
            capture: None,
        }
    }
}

pub fn capture_commands(capture: &str) -> Vec<String> {
    vec![
        format!("screenshot approved/{capture}"),
        format!("dump {capture}"),
        "wait 1s".into(),
    ]
}

pub fn script(phases: &[Phase]) -> String {
    let mut out = Vec::new();
    for phase in phases {
        out.push(format!("mark {}.begin", phase.name));
        for command in &phase.commands {
            out.push(command.replace("{phase}", &phase.name));
        }
        if phase.name != "quit" {
            out.push(format!("mark {}.end", phase.name));
        }
    }
    out.join("; ")
}

/// A scenario's parts: each part's name and the checks it judges. A run
/// plays one part (`--part <name>`, a CI job of its own, shorter and with
/// fewer games at once) or, with no `--part`, all of them in one go.
pub type Parts = &'static [(&'static str, &'static [&'static str])];

/// The part a run plays, or all of the scenario.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Part(pub Option<&'static str>);

impl Part {
    /// The part named `arg` of a scenario with `parts`.
    pub fn of(parts: Parts, arg: Option<&str>) -> Result<Self, String> {
        let Some(arg) = arg else {
            return Ok(Self(None));
        };
        parts
            .iter()
            .find(|(name, _)| *name == arg)
            .map(|(name, _)| Self(Some(*name)))
            .ok_or_else(|| {
                let names: Vec<&str> = parts.iter().map(|(name, _)| *name).collect();
                if names.is_empty() {
                    format!("--part {arg}: this scenario has no parts")
                } else {
                    format!("--part {arg}: one of {}", names.join(", "))
                }
            })
    }

    /// Whether the run plays the steps of part `name`.
    pub fn plays(self, name: &str) -> bool {
        self.0.is_none_or(|part| part == name)
    }

    /// Whether the run judges the check `label`.
    pub fn judges(self, parts: Parts, label: &str) -> bool {
        parts
            .iter()
            .any(|(name, labels)| self.plays(name) && labels.contains(&label))
    }

    /// The run's name for its directory and report: the scenario's, with
    /// the part's after a dot.
    pub fn run_name(self, scenario: &str) -> String {
        match self.0 {
            Some(part) => format!("{scenario}.{part}"),
            None => scenario.to_owned(),
        }
    }
}

/// Every check of a judge is in some part, and every part's check is one
/// the judge gives: a split scenario keeps all its checks.
#[cfg(test)]
pub fn assert_parts_cover(parts: Parts, judged: &[&str]) {
    for label in judged {
        assert!(
            parts.iter().any(|(_, labels)| labels.contains(label)),
            "check {label} is in no part"
        );
    }
    for (name, labels) in parts {
        for label in *labels {
            assert!(
                judged.contains(label),
                "part {name}: {label} is no check of the judge"
            );
        }
    }
}
