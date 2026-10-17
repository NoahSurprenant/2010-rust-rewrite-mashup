//! `crafting_table` — owner-approved. This file is the one place its data
//! lives: the arena, the inputs, and what each checkpoint must show.
//!
//! On the Minecraft map, at a fixed world seed, the player builds a small
//! arena (stone floor, cleared air, a crafting table three blocks along +X,
//! which is where MW2 yaw 0 looks), then: opens the table with E, crafts a
//! chest on its 3x3 grid, leaves a plank in the grid and closes with Escape
//! (the plank comes back, no MW2 menu opens), opens it with E holding a gun,
//! finds that a right click with a gun aims instead, opens it with a right
//! click holding an item, places a plank against it crouched (and opens it
//! crouched with an empty hand), opens it with
//! a key rebound to Use, and sees the screen close when the table is broken
//! and when the player is moved away.

use crate::runner::Event;

pub const NAME: &str = "crafting_table";
pub const MAP: &str = "minecraft:overworld";
pub const SEED: &str = "12345";
pub const PHASE_TIMEOUT_SECS: u64 = 300;
pub const QUIT_TIMEOUT_SECS: u64 = 30;

/// The class the player spawns with, written as the run's class file: a
/// fresh profile draws its classes at random, and an akimbo gun fires on a
/// right click rather than aiming.
pub const CLASSES: &str = "iw4l-classes 1\nmg4_bling\tiw4:weapon/mg4_mp\tgrip,heartbeat\tiw4:weapon/at4_mp\t\tiw4:weapon/frag_grenade_mp\tiw4:weapon/smoke_grenade_mp\tspecialty_bling\tspecialty_bulletdamage\tspecialty_detectexplosive\tspecialty_copycat\n";

/// The planks' slot once the inventory is cleared: after MW2's five guns.
const PLANKS: usize = 5;

pub struct Step {
    pub commands: &'static str,
    /// The checkpoint's `mc state` label; empty for a step with none.
    pub label: &'static str,
    pub expect: &'static [Expect],
}

pub enum Expect {
    Screen(Screen),
    Menu(bool),
    Stance(&'static str),
    Result(&'static str),
    Cursor(&'static str),
    Slot(usize, &'static str),
    /// How many grid slots hold this stack (`-` for empty).
    Grid(&'static str, usize),
    /// Whether the crosshair is on a crafting table in reach.
    TableInView(bool),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Screen {
    None,
    Inventory,
    Table,
}

pub const STEPS: &[Step] = &[
    Step {
        commands: "wait world; spawn 0; force_match_start; wait minecraft; wait 1s; \
                   mc fill -1 0 -2 4 3 2 air; mc fill -1 -1 -2 4 -1 2 stone; \
                   mc fill 3 0 0 3 0 0 crafting_table; wait 2s; \
                   mc clear; mc give oak_planks 16; wait 1s; look 0 23; wait 1s",
        label: "arena",
        expect: &[
            Expect::Screen(Screen::None),
            Expect::TableInView(true),
            Expect::Slot(PLANKS, "minecraft:oak_planks*16"),
        ],
    },
    Step {
        commands: "screenshot approved/00_arena; key e; wait 1s",
        label: "e_opens_table",
        expect: &[
            Expect::Screen(Screen::Table),
            Expect::Grid("-", 9),
            Expect::Result("-"),
        ],
    },
    Step {
        commands: "mc click inv 5; wait 2t; \
                   mc click grid 0 right; mc click grid 1 right; mc click grid 2 right; \
                   mc click grid 3 right; mc click grid 5 right; mc click grid 6 right; \
                   mc click grid 7 right; mc click grid 8 right; wait 2t; \
                   mc click inv 5; wait 2t",
        label: "chest_recipe",
        expect: &[
            Expect::Screen(Screen::Table),
            Expect::Grid("minecraft:oak_planks*1", 8),
            Expect::Grid("-", 1),
            Expect::Result("minecraft:chest*1"),
            Expect::Slot(PLANKS, "minecraft:oak_planks*8"),
        ],
    },
    Step {
        commands: "screenshot approved/01_chest_recipe; mc click result; wait 2t; mc click inv 20; wait 2t",
        label: "chest_crafted",
        expect: &[
            Expect::Grid("-", 9),
            Expect::Result("-"),
            Expect::Cursor("-"),
            Expect::Slot(20, "minecraft:chest*1"),
            Expect::Slot(PLANKS, "minecraft:oak_planks*8"),
        ],
    },
    Step {
        commands: "mc click inv 5; wait 2t; mc click grid 4; wait 2t",
        label: "plank_left_in_grid",
        expect: &[
            Expect::Grid("minecraft:oak_planks*8", 1),
            Expect::Slot(PLANKS, "-"),
        ],
    },
    Step {
        commands: "key escape; wait 1s",
        label: "escape_closes",
        expect: &[
            Expect::Screen(Screen::None),
            Expect::Menu(false),
            Expect::Grid("-", 4),
            Expect::Slot(PLANKS, "minecraft:oak_planks*8"),
        ],
    },
    Step {
        commands: "screenshot approved/02_closed; key 1; wait 1s; look 0 23; wait 2t; key e; wait 1s",
        label: "e_opens_table_with_gun",
        expect: &[Expect::Screen(Screen::Table)],
    },
    Step {
        commands: "key e; wait 1s; look 0 23; wait 2t; key mouse2 0; wait 1s",
        label: "right_click_with_gun_aims",
        expect: &[Expect::Screen(Screen::None)],
    },
    Step {
        commands: "key 6; wait 1s; look 0 23; wait 2t; key mouse2 0; wait 1s; screenshot approved/03_right_click_with_item",
        label: "right_click_with_item_opens",
        expect: &[Expect::Screen(Screen::Table)],
    },
    Step {
        commands: "key e; wait 1s",
        label: "e_closes",
        expect: &[Expect::Screen(Screen::None)],
    },
    Step {
        commands: "hold +movedown; wait 1s; look 0 23; wait 2t; key mouse2 0; wait 1s",
        label: "crouched_right_click_places",
        expect: &[
            Expect::Stance("crouch"),
            Expect::Screen(Screen::None),
            Expect::Slot(PLANKS, "minecraft:oak_planks*7"),
        ],
    },
    Step {
        commands: "mc fill 2 0 0 2 0 0 air; wait 1s; key 7; wait 1s; look 0 23; wait 2t",
        label: "crouched_empty_hand",
        expect: &[Expect::Stance("crouch"), Expect::Screen(Screen::None), Expect::Slot(PLANKS + 1, "-")],
    },
    // An open screen lets go of held MW2 inputs, so the stance is checked
    // before the click.
    Step {
        commands: "key mouse2 0; wait 1s",
        label: "crouched_empty_hand_opens",
        expect: &[Expect::Screen(Screen::Table)],
    },
    Step {
        commands: "key e; wait 1s; release +movedown; wait 1s; \
                   bind g +activate; key 1; wait 1s; look 0 23; wait 2t; key g; wait 1s",
        label: "rebound_use_opens",
        expect: &[Expect::Stance("stand"), Expect::Screen(Screen::Table)],
    },
    Step {
        commands: "mc fill 3 0 0 3 0 0 air; wait 1s",
        label: "broken_table_closes",
        expect: &[Expect::Screen(Screen::None)],
    },
    Step {
        commands: "mc fill 3 0 0 3 0 0 crafting_table; wait 1s; look 0 23; wait 2t; key e; wait 1s",
        label: "reopened",
        expect: &[Expect::Screen(Screen::Table)],
    },
    Step {
        commands: "tp 4000 4000 3000; wait 1s",
        label: "walking_away_closes",
        expect: &[Expect::Screen(Screen::None)],
    },
    Step {
        commands: "finish_run",
        label: "",
        expect: &[],
    },
];

pub fn script() -> String {
    STEPS
        .iter()
        .map(|step| match step.label {
            "" => step.commands.to_owned(),
            label => format!("{}; mc state {label}", step.commands),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// One checkpoint's verdict: its label and every failed expectation.
pub struct Check {
    pub label: &'static str,
    pub failures: Vec<String>,
}

/// Judges the `minecraft-state` lines the game printed against the steps.
pub fn judge(states: &[Event]) -> Vec<Check> {
    STEPS
        .iter()
        .filter(|step| !step.label.is_empty())
        .map(|step| {
            let failures = match states.iter().find(|e| e.get("label") == Some(step.label)) {
                None => vec!["no state printed".to_owned()],
                Some(state) => step
                    .expect
                    .iter()
                    .filter_map(|expect| failure(state, expect))
                    .collect(),
            };
            Check {
                label: step.label,
                failures,
            }
        })
        .collect()
}

fn list(state: &Event, key: &str) -> Vec<String> {
    state
        .get(key)
        .unwrap_or("")
        .split(',')
        .map(str::to_owned)
        .collect()
}

fn failure(state: &Event, expect: &Expect) -> Option<String> {
    let field = |key: &str| state.get(key).unwrap_or("").to_owned();
    let (held, want, got) = match expect {
        Expect::Screen(screen) => {
            let got = match field("screen").as_str() {
                "none" => Some(Screen::None),
                "inventory" => Some(Screen::Inventory),
                s if s.starts_with("table@") => Some(Screen::Table),
                _ => None,
            };
            (
                got == Some(*screen),
                format!("screen {screen:?}"),
                format!("{got:?}"),
            )
        }
        Expect::Menu(open) => (
            field("menu") == open.to_string(),
            format!("menu={open}"),
            field("menu"),
        ),
        Expect::Stance(stance) => (
            field("stance") == *stance,
            format!("stance={stance}"),
            field("stance"),
        ),
        Expect::Result(stack) => (
            field("result") == *stack,
            format!("result={stack}"),
            field("result"),
        ),
        Expect::Cursor(stack) => (
            field("cursor") == *stack,
            format!("cursor={stack}"),
            field("cursor"),
        ),
        Expect::Slot(slot, stack) => {
            let got = list(state, "slots").get(*slot).cloned().unwrap_or_else(|| "(no slot)".to_owned());
            (got == *stack, format!("slot {slot} = {stack}"), got)
        }
        Expect::Grid(stack, count) => {
            let got = list(state, "grid")
                .iter()
                .filter(|s| s.as_str() == *stack)
                .count();
            (
                got == *count,
                format!("{count} grid slots of {stack}"),
                format!("{got} in {}", field("grid")),
            )
        }
        Expect::TableInView(in_view) => (
            (field("table_in_view") != "-") == *in_view,
            format!("table_in_view={in_view}"),
            field("table_in_view"),
        ),
    };
    (!held).then(|| format!("want {want}, got {got}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(fields: &str) -> Event {
        crate::runner::parse(&format!("minecraft-state: {fields}"), "minecraft-state:", 0).unwrap()
    }

    fn passing_states() -> Vec<Event> {
        let guns = "iw4:weapon/1*1,iw4:weapon/2*1,iw4:weapon/3*1,iw4:weapon/4*1,iw4:weapon/5*1";
        let slots = |p: &str, extra: &str| {
            let mut s: Vec<String> = vec!["-".into(); 41];
            s[PLANKS] = p.into();
            if !extra.is_empty() {
                s[20] = extra.into();
            }
            format!("{guns},{}", s[5..].join(","))
        };
        let grid9 = |cells: &[&str]| cells.join(",");
        let empty9 = grid9(&["-"; 9]);
        let empty4 = grid9(&["-"; 4]);
        let planks = "minecraft:oak_planks*1";
        let ring = grid9(&[
            planks, planks, planks, planks, "-", planks, planks, planks, planks,
        ]);
        let mut centre = vec!["-"; 9];
        centre[4] = "minecraft:oak_planks*8";
        let table = "table@1,2,3";
        let line = |label: &str,
                    stance: &str,
                    screen: &str,
                    grid: &str,
                    result: &str,
                    slots: String| {
            state(&format!(
                "label={label} stance={stance} menu=false table_in_view=1,2,3 screen={screen} grid={grid} result={result} cursor=- slots={slots}"
            ))
        };
        vec![
            line(
                "arena",
                "stand",
                "none",
                &empty4,
                "-",
                slots("minecraft:oak_planks*16", ""),
            ),
            line(
                "e_opens_table",
                "stand",
                table,
                &empty9,
                "-",
                slots("minecraft:oak_planks*16", ""),
            ),
            line(
                "chest_recipe",
                "stand",
                table,
                &ring,
                "minecraft:chest*1",
                slots("minecraft:oak_planks*8", ""),
            ),
            line(
                "chest_crafted",
                "stand",
                table,
                &empty9,
                "-",
                slots("minecraft:oak_planks*8", "minecraft:chest*1"),
            ),
            line(
                "plank_left_in_grid",
                "stand",
                table,
                &grid9(&centre),
                "minecraft:oak_button*1",
                slots("-", "minecraft:chest*1"),
            ),
            line(
                "escape_closes",
                "stand",
                "none",
                &empty4,
                "-",
                slots("minecraft:oak_planks*8", "minecraft:chest*1"),
            ),
            line(
                "e_opens_table_with_gun",
                "stand",
                table,
                &empty9,
                "-",
                slots("minecraft:oak_planks*8", ""),
            ),
            line(
                "right_click_with_gun_aims",
                "stand",
                "none",
                &empty4,
                "-",
                slots("minecraft:oak_planks*8", ""),
            ),
            line(
                "right_click_with_item_opens",
                "stand",
                table,
                &empty9,
                "-",
                slots("minecraft:oak_planks*8", ""),
            ),
            line(
                "e_closes",
                "stand",
                "none",
                &empty4,
                "-",
                slots("minecraft:oak_planks*8", ""),
            ),
            line(
                "crouched_right_click_places",
                "crouch",
                "none",
                &empty4,
                "-",
                slots("minecraft:oak_planks*7", ""),
            ),
            line(
                "crouched_empty_hand",
                "crouch",
                "none",
                &empty4,
                "-",
                slots("minecraft:oak_planks*7", ""),
            ),
            line(
                "crouched_empty_hand_opens",
                "crouch",
                table,
                &empty9,
                "-",
                slots("minecraft:oak_planks*7", ""),
            ),
            line(
                "rebound_use_opens",
                "stand",
                table,
                &empty9,
                "-",
                slots("minecraft:oak_planks*7", ""),
            ),
            line(
                "broken_table_closes",
                "stand",
                "none",
                &empty4,
                "-",
                slots("minecraft:oak_planks*7", ""),
            ),
            line(
                "reopened",
                "stand",
                table,
                &empty9,
                "-",
                slots("minecraft:oak_planks*7", ""),
            ),
            line(
                "walking_away_closes",
                "stand",
                "none",
                &empty4,
                "-",
                slots("minecraft:oak_planks*7", ""),
            ),
        ]
    }

    #[test]
    fn the_class_has_no_akimbo_gun() {
        assert!(CLASSES.starts_with("iw4l-classes 1\n"));
        assert!(!CLASSES.contains("akimbo"));
    }

    #[test]
    fn every_checkpoint_is_in_the_script() {
        let script = script();
        for step in STEPS.iter().filter(|s| !s.label.is_empty()) {
            assert!(
                script.contains(&format!("mc state {}", step.label)),
                "{}",
                step.label
            );
        }
        assert!(script.ends_with("finish_run"));
    }

    #[test]
    fn a_run_that_did_everything_passes() {
        let checks = judge(&passing_states());
        for check in &checks {
            assert!(
                check.failures.is_empty(),
                "{}: {:?}",
                check.label,
                check.failures
            );
        }
        assert_eq!(checks.len(), STEPS.len() - 1);
    }

    #[test]
    fn a_missing_checkpoint_fails() {
        let mut states = passing_states();
        states.retain(|s| s.get("label") != Some("walking_away_closes"));
        let checks = judge(&states);
        let failed: Vec<_> = checks
            .iter()
            .filter(|c| !c.failures.is_empty())
            .map(|c| c.label)
            .collect();
        assert_eq!(failed, ["walking_away_closes"]);
    }

    #[test]
    fn an_open_menu_after_escape_fails() {
        let mut states = passing_states();
        let i = states
            .iter()
            .position(|s| s.get("label") == Some("escape_closes"))
            .unwrap();
        states[i].fields.insert("menu".into(), "true".into());
        let check = judge(&states)
            .into_iter()
            .find(|c| c.label == "escape_closes")
            .unwrap();
        assert_eq!(check.failures, ["want menu=false, got true"]);
    }

    #[test]
    fn a_table_that_does_not_open_fails() {
        let mut states = passing_states();
        let i = states
            .iter()
            .position(|s| s.get("label") == Some("rebound_use_opens"))
            .unwrap();
        states[i].fields.insert("screen".into(), "none".into());
        let check = judge(&states)
            .into_iter()
            .find(|c| c.label == "rebound_use_opens")
            .unwrap();
        assert_eq!(check.failures, ["want screen Table, got Some(None)"]);
    }

    #[test]
    fn a_wrong_recipe_result_fails() {
        let mut states = passing_states();
        let i = states
            .iter()
            .position(|s| s.get("label") == Some("chest_recipe"))
            .unwrap();
        states[i].fields.insert("result".into(), "-".into());
        let check = judge(&states)
            .into_iter()
            .find(|c| c.label == "chest_recipe")
            .unwrap();
        assert_eq!(check.failures, ["want result=minecraft:chest*1, got -"]);
    }

    #[test]
    fn an_arena_away_from_the_player_fails() {
        let mut states = passing_states();
        states[0].fields.insert("table_in_view".into(), "-".into());
        let check = judge(&states)
            .into_iter()
            .find(|c| c.label == "arena")
            .unwrap();
        assert_eq!(check.failures, ["want table_in_view=true, got -"]);
    }
}
