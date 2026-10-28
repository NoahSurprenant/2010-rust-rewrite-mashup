//! `crafting_table` — owner-approved. This file is the one place its data
//! lives: the arena, the inputs, and what each checkpoint must show.
//!
//! On the Minecraft map, in the fixed world (`crate::world`: seed and
//! spawn, no mobs), the player builds a small arena 40 blocks over the
//! spawn (stone floor, cleared air, a crafting table three blocks along +X,
//! which is where MW2 yaw 0 looks), then: opens the table with E, crafts a
//! chest on its 3x3 grid, leaves a plank in the grid and closes with Escape
//! (the plank comes back, no MW2 menu opens), opens it with E holding a gun,
//! finds that a right click with a gun aims instead, opens it with a right
//! click holding an item, places a plank against it crouched (and opens it
//! crouched with an empty hand), opens it with
//! a key rebound to Use, and sees the screen close when the table is broken,
//! holds the crafted chest (drawn in the hand as its model) and throws it
//! (drawn on the ground), and sees the screen close when the player is moved
//! away. Then the multiplayer options page shows the player key and its
//! button copies the whole key. Last, a controller the console drives
//! (`pad`) opens the pause menu and still walks after it, and works the
//! screens as the mouse does: opens the table and the inventory, walks the
//! cursor, crafts a chest and takes it, quick-moves it and closes.

use crate::runner::Event;

pub const NAME: &str = "crafting_table";
pub const MAP: &str = "minecraft:overworld";
pub const PHASE_TIMEOUT_SECS: u64 = 600;
pub const QUIT_TIMEOUT_SECS: u64 = 60;

/// The class the player spawns with, written as the run's class file: a
/// fresh profile draws its classes at random, and an akimbo gun fires on a
/// right click rather than aiming.
pub const CLASSES: &str = "iw4l-classes 1\nmg4_bling\tiw4:weapon/mg4_mp\tgrip,heartbeat\tiw4:weapon/at4_mp\t\tiw4:weapon/frag_grenade_mp\tiw4:weapon/smoke_grenade_mp\tspecialty_bling\tspecialty_bulletdamage\tspecialty_detectexplosive\tspecialty_copycat\n";

/// The planks' slot once the inventory is cleared: after the starter
/// pistol, the only gun a Minecraft player now starts with (class loadouts
/// hand out none there; this arena's own `mc give` never touches the
/// reserved ammo or map slots near the end of the hotbar).
const PLANKS: usize = 1;

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
    /// The inventory draws this item's icon (it is not in `no_icon`).
    Icon(&'static str),
    /// The hand in view holds this item, drawn as a model with faces.
    Hand(&'static str),
    /// An item of this id lies in the world, drawn as a model (it is not
    /// in `no_model`).
    Dropped(&'static str),
    /// The player stands at least this far (MW2 units, along the ground)
    /// from where it stood at that checkpoint.
    MovedFrom(&'static str, f32),
    /// The controller's cursor on the open screen (`inv5`, `grid3`,
    /// `result`, `-`).
    Pad(&'static str),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Screen {
    None,
    Inventory,
    Table,
}

pub const STEPS: &[Step] = &[
    Step {
        // The arena stands on a stone floor 40 blocks over the spawn block,
        // clear of the ground: at the fixed spawn, clearing air in the ground
        // dropped leaf litter that the player picked up into its empty hand
        // (runs 39228628595, 39235178267). `tp` is in map units: the middle of
        // the block over the floor's, as `mob_steps::block_centre_map` gives.
        commands: "wait world; spawn 0; force_match_start; wait minecraft; wait 1s; \
                   mc set -1 40 -2 4 40 2 stone; wait 2s; tp 0 0 1494; wait 2s; \
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
        commands: "mc click inv 1; wait 2t; \
                   mc click grid 0 right; mc click grid 1 right; mc click grid 2 right; \
                   mc click grid 3 right; mc click grid 5 right; mc click grid 6 right; \
                   mc click grid 7 right; mc click grid 8 right; wait 2t; \
                   mc click inv 1; wait 2t",
        label: "chest_recipe",
        expect: &[
            Expect::Screen(Screen::Table),
            Expect::Grid("minecraft:oak_planks*1", 8),
            Expect::Grid("-", 1),
            Expect::Result("minecraft:chest*1"),
            Expect::Slot(PLANKS, "minecraft:oak_planks*8"),
            // A chest's item model is `minecraft:special` (a block
            // entity's renderer): its icon is drawn from the chest's boxes.
            Expect::Icon("minecraft:chest"),
            Expect::Icon("minecraft:oak_planks"),
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
        commands: "mc click inv 1; wait 2t; mc click grid 4; wait 2t",
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
        // Digit 2 (slot 1, the planks), not 6: the planks sit right of the
        // one starter gun now, not after five class-loadout guns. This
        // selection also carries into the next (crouched) step, which
        // needs a real item in hand to place rather than open the table.
        commands: "key 2; wait 1s; look 0 23; wait 2t; key mouse2 0; wait 1s; screenshot approved/03_right_click_with_item",
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
    // The crafted chest, moved to the hotbar and held, shows in the hand
    // as its box model (a `minecraft:special` item: a block entity's
    // renderer), then thrown, on the ground.
    Step {
        // Slot 6, not 7: the starter kit's ammo now reserves 7 (one before
        // the map's slot 8), so this scratch move needs a hotbar slot
        // neither a gun, planks, ammo nor the map ever touch.
        commands: "mc click inv 20; wait 2t; mc click inv 6; wait 2t; key e; wait 1s; key 7; wait 1s",
        label: "chest_held",
        expect: &[
            Expect::Screen(Screen::None),
            Expect::Slot(6, "minecraft:chest*1"),
            Expect::Hand("minecraft:chest"),
        ],
    },
    Step {
        commands: "screenshot approved/04_chest_held; key q; wait 10t",
        label: "chest_dropped",
        expect: &[Expect::Slot(6, "-"), Expect::Dropped("minecraft:chest")],
    },
    Step {
        commands: "screenshot approved/05_chest_dropped; key 1; wait 1s; look 0 23; wait 2t; key e; wait 1s",
        label: "reopened_after_drop",
        expect: &[Expect::Screen(Screen::Table)],
    },
    Step {
        commands: "tp 4000 4000 3000; wait 1s",
        label: "walking_away_closes",
        expect: &[Expect::Screen(Screen::None)],
    },
    // The multiplayer options page shows the player key; its button copies
    // the whole key (`player-key: copied`), and `whoami` names what the page
    // shows (`shown=`).
    Step {
        commands: "menu open options_multi; wait 1s; menu focus options_multi player_key; \
                   wait 0.5s; menu; menu accept; wait 1s; whoami; menu back; wait 1s",
        label: "options_closed",
        expect: &[Expect::Menu(false), Expect::Screen(Screen::None)],
    },
    // The controller walks again after its Start button opened the pause
    // menu and its B button closed it, with the left stick resting a little
    // off centre (0.1, 0.1: outside the 0.12 deadzone, as a worn stick
    // rests). A stick had to read exactly zero after a menu before it moved
    // the player again, and this one never did: the owner's controller
    // stopped walking while it still turned the view. Back over the arena,
    // on a wider floor, the player strafes left with the stick.
    Step {
        commands: "tp 0 0 1494; wait 1s; mc fill -8 -1 -8 8 -1 8 stone; mc fill -8 0 -8 8 3 8 air; \
                   wait 1s; pad stick left 0.1 0.1; wait 0.5s; pad start; wait 1s",
        label: "pad_pause_opened",
        expect: &[Expect::Menu(true)],
    },
    Step {
        commands: "pad b; wait 1s",
        label: "pad_pause_closed",
        expect: &[Expect::Menu(false), Expect::Screen(Screen::None)],
    },
    Step {
        commands: "pad stick left -1 0; wait 1s; pad stick left 0.1 0.1; wait 0.5s",
        label: "pad_walks_after_pause",
        expect: &[Expect::MovedFrom("pad_pause_closed", 64.0)],
    },
    // The controller works the screens as the mouse does (the owner could
    // not open the inventory or a crafting table with one): D-pad down opens
    // the table in view with the cursor on the selected hotbar slot, the
    // D-pad walks it, A picks up and puts down, X puts one down, A on the
    // result takes the chest, Y quick-moves it, B closes. Away from the
    // table D-pad down opens the inventory; inside it D-pad down moves the
    // cursor, and START closes it without the pause menu.
    Step {
        commands: "pad stick left 0 0; tp 0 0 1494; wait 2s; mc fill 3 0 0 3 0 0 crafting_table; \
                   mc clear; mc give oak_planks 16; wait 1s; look 0 23; wait 2t; pad d-down; wait 1s",
        label: "pad_opens_table",
        expect: &[
            Expect::Screen(Screen::Table),
            Expect::Menu(false),
            Expect::Pad("inv0"),
            Expect::Slot(PLANKS, "minecraft:oak_planks*16"),
        ],
    },
    Step {
        // Planks land one slot right of the gun now (PLANKS = 1, not 5):
        // one d-right, not five.
        commands: "pad d-right; wait 0.3s; pad a; wait 0.5s",
        label: "pad_picks_planks",
        expect: &[
            Expect::Pad("inv1"),
            Expect::Cursor("minecraft:oak_planks*16"),
            Expect::Slot(PLANKS, "-"),
        ],
    },
    // Up the backpack to the grid's corner, then one plank on each slot of
    // its ring. Column 1's nearest corner is the grid's bottom-left
    // (`Crafting(6)`, not column 5's bottom-right): the ring walks the
    // mirror image, right/right/up/up/left/left/down, ending at grid 3.
    Step {
        commands: "pad d-up; wait 0.3s; pad d-up; wait 0.3s; pad d-up; wait 0.3s; pad d-up; wait 0.3s; \
                   pad x; wait 0.3s; pad d-right; wait 0.3s; pad x; wait 0.3s; pad d-right; wait 0.3s; \
                   pad x; wait 0.3s; pad d-up; wait 0.3s; pad x; wait 0.3s; pad d-up; wait 0.3s; \
                   pad x; wait 0.3s; pad d-left; wait 0.3s; pad x; wait 0.3s; pad d-left; wait 0.3s; \
                   pad x; wait 0.3s; pad d-down; wait 0.3s; pad x; wait 0.5s",
        label: "pad_chest_recipe",
        expect: &[
            Expect::Pad("grid3"),
            Expect::Grid("minecraft:oak_planks*1", 8),
            Expect::Grid("-", 1),
            Expect::Result("minecraft:chest*1"),
            Expect::Cursor("minecraft:oak_planks*8"),
        ],
    },
    Step {
        // From grid 3, two d-downs land on backpack slot 11 (column 0's
        // nearest backpack column), not 13 (column 2's, where the old
        // grid 5 landed).
        commands: "pad d-down; wait 0.3s; pad d-down; wait 0.3s; pad a; wait 0.5s",
        label: "pad_planks_put_back",
        expect: &[
            Expect::Pad("inv11"),
            Expect::Cursor("-"),
            Expect::Slot(11, "minecraft:oak_planks*8"),
        ],
    },
    Step {
        // Grid 6 (column 0) is not next to the result slot the way grid 8
        // (column 2) was: two extra d-rights walk the grid's row first.
        commands: "pad d-up; wait 0.3s; pad d-right; wait 0.3s; pad d-right; wait 0.3s; pad d-right; wait 0.3s; pad a; wait 0.5s",
        label: "pad_takes_chest",
        expect: &[
            Expect::Pad("result"),
            Expect::Cursor("minecraft:chest*1"),
            Expect::Result("-"),
            Expect::Grid("-", 9),
        ],
    },
    Step {
        // The result slot's own position does not depend on how the
        // cursor got there, so this still lands on inv15.
        commands: "pad d-down; wait 0.3s; pad a; wait 0.5s",
        label: "pad_places_chest",
        expect: &[
            Expect::Pad("inv15"),
            Expect::Cursor("-"),
            Expect::Slot(15, "minecraft:chest*1"),
        ],
    },
    // Y: a shift click, from the backpack to the hotbar's first free slot.
    Step {
        commands: "pad y; wait 0.5s",
        label: "pad_quick_moves",
        expect: &[
            Expect::Slot(15, "-"),
            Expect::Slot(PLANKS, "minecraft:chest*1"),
        ],
    },
    Step {
        commands: "pad b; wait 1s",
        label: "pad_b_closes",
        expect: &[
            Expect::Screen(Screen::None),
            Expect::Menu(false),
            Expect::Pad("-"),
            Expect::Slot(11, "minecraft:oak_planks*8"),
            Expect::Slot(PLANKS, "minecraft:chest*1"),
        ],
    },
    Step {
        commands: "look 180 23; wait 2t; pad d-down; wait 1s; pad d-up; wait 0.3s; pad d-down; wait 0.5s",
        label: "pad_opens_inventory",
        expect: &[
            Expect::TableInView(false),
            Expect::Screen(Screen::Inventory),
            Expect::Pad("inv0"),
        ],
    },
    Step {
        commands: "pad start; wait 1s",
        label: "pad_start_closes",
        expect: &[Expect::Screen(Screen::None), Expect::Menu(false)],
    },
    Step {
        commands: "pad stick left 0 0; finish_run",
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
                    .filter_map(|expect| failure(state, expect, states))
                    .collect(),
            };
            Check {
                label: step.label,
                failures,
            }
        })
        .collect()
}

/// The player key on the options page: copied whole by its button, and
/// shown short, the same key `whoami` prints.
pub fn judge_player_key(lines: &[Event]) -> Check {
    let failures = (|| -> Result<(), String> {
        let copied = lines
            .iter()
            .find(|e| e.name == "copied")
            .ok_or("the options page's key button copied nothing (no `player-key: copied` line)")?;
        let key = copied.get("fingerprint").unwrap_or("");
        if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("copied `{key}`, not a whole fingerprint"));
        }
        let whoami = lines
            .iter()
            .find(|e| e.get("shown").is_some())
            .ok_or("no `whoami` line")?;
        if whoami.get("fingerprint") != Some(key) {
            return Err(format!(
                "copied {key}, whoami says {}",
                whoami.get("fingerprint").unwrap_or("-")
            ));
        }
        let shown = whoami.get("shown").unwrap_or("-");
        if shown != &key[..16] {
            return Err(format!("the page shows `{shown}`, want `{}`", &key[..16]));
        }
        Ok(())
    })()
    .err()
    .into_iter()
    .collect();
    Check {
        label: "player_key_copied_from_options",
        failures,
    }
}

fn list(state: &Event, key: &str) -> Vec<String> {
    state
        .get(key)
        .unwrap_or("")
        .split(',')
        .map(str::to_owned)
        .collect()
}

/// Where the player stood (`origin=x,y,z`, MW2 units).
fn origin(state: &Event) -> Option<[f32; 3]> {
    let mut parts = state.get("origin")?.split(',').map(|v| v.parse::<f32>().ok());
    Some([parts.next()??, parts.next()??, parts.next()??])
}

fn failure(state: &Event, expect: &Expect, states: &[Event]) -> Option<String> {
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
        Expect::Icon(id) => (
            !list(state, "no_icon").iter().any(|missing| missing == id),
            format!("an icon for {id}"),
            format!("no_icon={}", field("no_icon")),
        ),
        Expect::Hand(id) => (
            field("hand").rsplit_once(':').is_some_and(|(held, faces)| {
                held == *id && faces.parse::<usize>().is_ok_and(|n| n > 0)
            }),
            format!("{id} in hand with faces"),
            format!("hand={}", field("hand")),
        ),
        Expect::Dropped(id) => (
            list(state, "ground").iter().any(|item| item == id)
                && !list(state, "no_model").iter().any(|missing| missing == id),
            format!("{id} on the ground with a model"),
            format!("ground={} no_model={}", field("ground"), field("no_model")),
        ),
        Expect::Pad(slot) => (field("pad") == *slot, format!("pad={slot}"), field("pad")),
        Expect::MovedFrom(label, min) => {
            let then = states
                .iter()
                .find(|e| e.get("label") == Some(*label))
                .and_then(origin);
            let moved = then
                .zip(origin(state))
                .map(|(a, b)| (a[0] - b[0]).hypot(a[1] - b[1]));
            (
                moved.is_some_and(|moved| moved >= *min),
                format!("moved at least {min} units from {label}"),
                moved.map_or_else(|| "no origin".to_owned(), |moved| format!("{moved:.0}")),
            )
        }
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
        // Only the starter pistol now (the owner's decision): class
        // loadouts hand out no guns on the Minecraft map, and the other
        // four slots this fixture used to reserve for them are free.
        let guns = "iw4:weapon/1*1";
        let slots_at = |p: &str, at: usize, extra: &str| {
            let mut s: Vec<String> = vec!["-".into(); 41];
            s[PLANKS] = p.into();
            if !extra.is_empty() {
                s[at] = extra.into();
            }
            format!("{guns},{}", s[1..].join(","))
        };
        let slots = |p: &str, extra: &str| slots_at(p, 20, extra);
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
        // A controller step's line: the screen, its cursor, and the slots
        // that hold something besides the guns.
        let pad_line = |label: &str, screen: &str, grid: &str, result: &str, cursor: &str, pad: &str, held: &[(usize, &str)]| {
            let mut s: Vec<String> = vec!["-".into(); 41];
            for (at, stack) in held {
                s[*at] = (*stack).into();
            }
            state(&format!(
                "label={label} stance=stand menu=false table_in_view=1,2,3 screen={screen} grid={grid} result={result} cursor={cursor} pad={pad} slots={guns},{}",
                s[1..].join(",")
            ))
        };
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
            {
                let mut held = line(
                    "chest_held",
                    "stand",
                    "none",
                    &empty4,
                    "-",
                    slots_at("minecraft:oak_planks*7", 6, "minecraft:chest*1"),
                );
                held.fields
                    .insert("hand".into(), "minecraft:chest:18".into());
                held
            },
            {
                let mut dropped = line(
                    "chest_dropped",
                    "stand",
                    "none",
                    &empty4,
                    "-",
                    slots("minecraft:oak_planks*7", ""),
                );
                dropped
                    .fields
                    .insert("ground".into(), "minecraft:chest".into());
                dropped.fields.insert("no_model".into(), "-".into());
                dropped
            },
            line(
                "reopened_after_drop",
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
            line(
                "options_closed",
                "stand",
                "none",
                &empty4,
                "-",
                slots("minecraft:oak_planks*7", ""),
            ),
            state(&format!(
                "label=pad_pause_opened stance=stand menu=true origin=37,0,1494 screen=none grid={empty4} result=- cursor=- slots={}",
                slots("minecraft:oak_planks*7", "")
            )),
            state(&format!(
                "label=pad_pause_closed stance=stand menu=false origin=37,0,1494 screen=none grid={empty4} result=- cursor=- slots={}",
                slots("minecraft:oak_planks*7", "")
            )),
            state(&format!(
                "label=pad_walks_after_pause stance=stand menu=false origin=37,180,1494 screen=none grid={empty4} result=- cursor=- slots={}",
                slots("minecraft:oak_planks*7", "")
            )),
            pad_line("pad_opens_table", table, &empty9, "-", "-", "inv0", &[(PLANKS, "minecraft:oak_planks*16")]),
            pad_line("pad_picks_planks", table, &empty9, "-", "minecraft:oak_planks*16", "inv1", &[]),
            pad_line("pad_chest_recipe", table, &ring, "minecraft:chest*1", "minecraft:oak_planks*8", "grid3", &[]),
            pad_line("pad_planks_put_back", table, &ring, "minecraft:chest*1", "-", "inv11", &[(11, "minecraft:oak_planks*8")]),
            pad_line("pad_takes_chest", table, &empty9, "-", "minecraft:chest*1", "result", &[(11, "minecraft:oak_planks*8")]),
            pad_line("pad_places_chest", table, &empty9, "-", "-", "inv15", &[(11, "minecraft:oak_planks*8"), (15, "minecraft:chest*1")]),
            pad_line("pad_quick_moves", table, &empty9, "-", "-", "inv15", &[(11, "minecraft:oak_planks*8"), (PLANKS, "minecraft:chest*1")]),
            pad_line("pad_b_closes", "none", &empty4, "-", "-", "-", &[(11, "minecraft:oak_planks*8"), (PLANKS, "minecraft:chest*1")]),
            {
                let mut away = pad_line("pad_opens_inventory", "inventory", &empty4, "-", "-", "inv0", &[(11, "minecraft:oak_planks*8"), (PLANKS, "minecraft:chest*1")]);
                away.fields.insert("table_in_view".into(), "-".into());
                away
            },
            pad_line("pad_start_closes", "none", &empty4, "-", "-", "-", &[(11, "minecraft:oak_planks*8"), (PLANKS, "minecraft:chest*1")]),
        ]
    }

    fn with_origin(mut states: Vec<Event>, label: &str, origin: &str) -> Vec<Event> {
        let at = states
            .iter()
            .position(|s| s.get("label") == Some(label))
            .unwrap();
        states[at].fields.insert("origin".into(), origin.into());
        states
    }

    /// The owner's bug: the stick that never read zero after the pause
    /// menu left the player where it was.
    #[test]
    fn a_controller_that_does_not_walk_after_the_pause_menu_fails() {
        let states = with_origin(passing_states(), "pad_walks_after_pause", "37,10,1494");
        let failed: Vec<_> = judge(&states)
            .into_iter()
            .filter(|c| !c.failures.is_empty())
            .map(|c| c.label)
            .collect();
        assert_eq!(failed, ["pad_walks_after_pause"]);
        let states = with_origin(passing_states(), "pad_walks_after_pause", "-");
        assert!(judge(&states).iter().any(|c| !c.failures.is_empty()));
    }

    #[test]
    fn a_pause_menu_the_controller_did_not_open_fails() {
        let mut states = passing_states();
        let at = states
            .iter()
            .position(|s| s.get("label") == Some("pad_pause_opened"))
            .unwrap();
        states[at].fields.insert("menu".into(), "false".into());
        let failed: Vec<_> = judge(&states)
            .into_iter()
            .filter(|c| !c.failures.is_empty())
            .map(|c| c.label)
            .collect();
        assert_eq!(failed, ["pad_pause_opened"]);
    }

    /// The owner's bug: the controller could neither open the screens nor
    /// work them.
    #[test]
    fn a_controller_that_does_not_open_or_work_the_table_fails() {
        let failed = |label: &str, field: &str, value: &str| {
            let mut states = passing_states();
            let at = states.iter().position(|s| s.get("label") == Some(label)).unwrap();
            states[at].fields.insert(field.into(), value.into());
            judge(&states)
                .into_iter()
                .filter(|c| !c.failures.is_empty())
                .map(|c| c.label)
                .collect::<Vec<_>>()
        };
        assert_eq!(failed("pad_opens_table", "screen", "none"), ["pad_opens_table"]);
        assert_eq!(failed("pad_picks_planks", "pad", "inv4"), ["pad_picks_planks"]);
        assert_eq!(failed("pad_chest_recipe", "result", "-"), ["pad_chest_recipe"]);
        assert_eq!(failed("pad_takes_chest", "cursor", "-"), ["pad_takes_chest"]);
        assert_eq!(failed("pad_opens_inventory", "screen", "none"), ["pad_opens_inventory"]);
        assert_eq!(failed("pad_start_closes", "menu", "true"), ["pad_start_closes"]);
        let mut states = passing_states();
        states.iter_mut().for_each(|s| {
            s.fields.remove("pad");
        });
        assert!(judge(&states).iter().any(|c| c.label == "pad_opens_table" && !c.failures.is_empty()));
    }

    fn key_lines(copied: &str, whoami: &str, shown: &str) -> Vec<Event> {
        [
            format!("player-key: copied fingerprint={copied}"),
            format!(
                "player-key: fingerprint={whoami} short={} file=\"k\" shown={shown}",
                &whoami[..16]
            ),
        ]
        .iter()
        .map(|line| crate::runner::parse(line, "player-key:", 0).unwrap())
        .collect()
    }

    #[test]
    fn a_key_copied_from_the_options_page_and_shown_short_passes() {
        let key = "ab".repeat(32);
        let check = judge_player_key(&key_lines(&key, &key, &key[..16]));
        assert!(check.failures.is_empty(), "{:?}", check.failures);
    }

    #[test]
    fn a_key_never_copied_or_shown_wrong_fails() {
        let key = "ab".repeat(32);
        let other = "cd".repeat(32);
        let fails = |lines: &[Event]| !judge_player_key(lines).failures.is_empty();
        assert!(fails(&key_lines(&key, &key, &key[..16])[1..]));
        assert!(fails(&key_lines(&key, &other, &other[..16])));
        assert!(fails(&key_lines(&key, &key, "-")));
        assert!(fails(&key_lines(&key[..16], &key, &key[..16])));
    }

    #[test]
    fn the_arena_s_tp_lands_over_its_floor() {
        let z = crate::scenarios::mob_steps::block_centre_map([0, 40, 0])[2];
        assert!(STEPS[0].commands.contains("mc set -1 40 -2 4 40 2 stone"));
        assert!(STEPS[0].commands.contains(&format!("tp 0 0 {z};")));
        let pause = STEPS.iter().find(|s| s.label == "pad_pause_opened").unwrap();
        assert!(pause.commands.starts_with(&format!("tp 0 0 {z};")));
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
    fn a_chest_with_no_icon_fails() {
        let mut states = passing_states();
        let i = states
            .iter()
            .position(|s| s.get("label") == Some("chest_recipe"))
            .unwrap();
        states[i]
            .fields
            .insert("no_icon".into(), "minecraft:chest".into());
        let check = judge(&states)
            .into_iter()
            .find(|c| c.label == "chest_recipe")
            .unwrap();
        assert_eq!(
            check.failures,
            ["want an icon for minecraft:chest, got no_icon=minecraft:chest"]
        );
    }

    #[test]
    fn a_chest_with_no_model_in_hand_fails() {
        let mut states = passing_states();
        let i = states
            .iter()
            .position(|s| s.get("label") == Some("chest_held"))
            .unwrap();
        states[i]
            .fields
            .insert("hand".into(), "minecraft:chest:0".into());
        let check = judge(&states)
            .into_iter()
            .find(|c| c.label == "chest_held")
            .unwrap();
        assert_eq!(
            check.failures,
            ["want minecraft:chest in hand with faces, got hand=minecraft:chest:0"]
        );
    }

    #[test]
    fn a_dropped_chest_with_no_model_fails() {
        let mut states = passing_states();
        let i = states
            .iter()
            .position(|s| s.get("label") == Some("chest_dropped"))
            .unwrap();
        states[i]
            .fields
            .insert("no_model".into(), "minecraft:chest".into());
        let check = judge(&states)
            .into_iter()
            .find(|c| c.label == "chest_dropped")
            .unwrap();
        assert_eq!(
            check.failures,
            [
                "want minecraft:chest on the ground with a model, got ground=minecraft:chest no_model=minecraft:chest"
            ]
        );
    }

    #[test]
    fn a_chest_that_never_landed_fails() {
        let mut states = passing_states();
        let i = states
            .iter()
            .position(|s| s.get("label") == Some("chest_dropped"))
            .unwrap();
        states[i].fields.insert("ground".into(), "-".into());
        let check = judge(&states)
            .into_iter()
            .find(|c| c.label == "chest_dropped")
            .unwrap();
        assert_eq!(check.failures.len(), 1);
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
