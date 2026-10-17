//! The Minecraft map in multiplayer, beside the world it shares
//! (`minecraft_world`). The host's game owns the world: it generates it
//! from its seed, applies its clients' intents (block edits, inventory
//! clicks) with the rules it applies its own, logs every block change, and
//! sends each client the seed and map origin, the log from where that
//! client stands in it, and the client's own inventory. Each goes as a
//! `ReliableRow::Minecraft` on the client's reliable stream, a few at a time,
//! so the stream never overflows. A client predicts its own clicks and
//! placements and adopts the host's inventory once the host has applied
//! every intent it sent.
//!
//! A host that plays through a master keeps its world: the seed, the map
//! origin, the edit log and every player's inventory by player name in a
//! world directory (with the region files its chunks save to), so hosting
//! again continues it. Clients never write the host's world.
//!
//! Mobs are the host's alone: its level spawns and runs them around the
//! host's player, and clients neither see them nor are hunted by them.
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use minecraft_terrain::multiplayer::{
    Click, EditLog, HostMessage, Intent, InventoryState, SavedWorld, SlotRef, WireItem, WireStack,
    edits_that_fit,
};
use minecraft_terrain::scene::BlockPos;
use minecraftoss_player::inventory::Inventory;

/// Reliable rows a client may still owe acks for before more Minecraft rows
/// are queued (of `net::MAX_PENDING_RELIABLE`, which must never be reached).
const ROWS_IN_FLIGHT: usize = 32;
/// Minecraft bytes in flight to one client: every unacked row rides in
/// every packet.
const BYTES_IN_FLIGHT: usize = 16 * 1024;
/// How often the host saves a kept world.
const AUTOSAVE_SECONDS: f64 = 10.0;
/// How often a client is told the items near it.
const ITEMS_SECONDS: f64 = 0.25;
/// How far from a client its items are sent, in blocks.
const ITEMS_RANGE: f64 = 48.0;
/// The default world a host keeps.
pub(crate) const DEFAULT_WORLD: &str = "world";

/// What this game is to the Minecraft world.
#[derive(Default)]
pub(crate) enum NetRole {
    /// No world.
    #[default]
    None,
    Host(Box<Host>),
    Client(Box<Client>),
}

impl NetRole {
    pub(crate) fn label(&self, hosting: bool) -> &'static str {
        match self {
            Self::None => "none",
            Self::Host(_) if hosting => "host",
            Self::Host(_) => "solo",
            Self::Client(_) => "client",
        }
    }
}

/// One of the host's clients.
pub(crate) struct Remote {
    pub name: String,
    pub inventory: Inventory,
    pub selected: usize,
    /// The crafting table it has open.
    pub table: Option<BlockPos>,
    /// The last intent applied.
    pub applied: u32,
    pub world_sent: bool,
    /// Where it stands in the edit log.
    pub cursor: usize,
    /// Single blocks to send it ahead of the log: the host's answer to an
    /// edit it predicted and the host refused.
    pub corrections: Vec<(BlockPos, Option<String>)>,
    sent_inventory: Option<(u32, Option<BlockPos>, InventoryState)>,
    items_clock: f64,
    sent_items: Option<Vec<WireItem>>,
    /// Alive and moved onto the world's spawn.
    pub placed: bool,
    /// The life it was placed in.
    pub life: Option<sim::LifeSequence>,
    /// Where it was moved, not yet told.
    pub placed_at: Option<[f32; 3]>,
    /// Chunks generated around it for the host's collision, and those of
    /// them it holds in the block world.
    pub collision: Option<minecraft_terrain::terrain::CollisionStream>,
    pub held: HashMap<(i32, i32), std::sync::Arc<minecraftoss_core::Chunk>>,
    /// The host holds it still: there is no ground where it stands yet.
    pub held_still: bool,
}

pub(crate) struct Host {
    pub seed: i64,
    pub origin: [f64; 3],
    /// The kept world's directory, when this world is kept.
    pub save: Option<PathBuf>,
    pub log: EditLog,
    /// Each player's inventory as last saved or left, by name.
    pub players: BTreeMap<String, InventoryState>,
    /// The clients in the world, by client number.
    pub remotes: BTreeMap<u32, Remote>,
    /// The host's own name once known, and whether its inventory came back.
    pub local_name: Option<String>,
    since_save: f64,
    /// Something changed since the last save.
    pub dirty: bool,
}

#[derive(Default)]
pub(crate) struct Client {
    /// What the host sent before the world was ready to take it.
    pub inbox: Vec<HostMessage>,
    pub seed: Option<i64>,
    /// The last intent this client sent.
    pub last_sent: u32,
    /// The crafting table and hotbar slot the host was last told of.
    pub table: Option<BlockPos>,
    pub selected: usize,
    /// Where the host moved this player on arrival.
    pub spawn_at: Option<[f32; 3]>,
}

/// Where kept worlds live: `IW4L_MINECRAFT_WORLDS`, else beside the game's
/// settings (`minecraft-worlds/` next to `settings.cfg`: `IW4L_SETTINGS_PATH`'s
/// folder, `$XDG_CONFIG_HOME/iw4l` or `~/.config/iw4l`, and the game's
/// artifacts folder on Windows).
pub(crate) fn worlds_root(artifacts: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("IW4L_MINECRAFT_WORLDS").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let settings_dir = std::env::var_os("IW4L_SETTINGS_PATH")
        .filter(|p| !p.is_empty())
        .and_then(|p| PathBuf::from(p).parent().map(Path::to_path_buf));
    let settings_dir = settings_dir.or_else(|| {
        if cfg!(windows) {
            return artifacts.map(Path::to_path_buf);
        }
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|config| config.join("iw4l"))
    });
    settings_dir
        .or_else(|| artifacts.map(Path::to_path_buf))
        .map(|dir| dir.join("minecraft-worlds"))
}

/// The world this host keeps, if it keeps one: a host playing through a
/// master keeps `IW4L_MINECRAFT_WORLD` (or the console's `mc world`, or
/// `world`); a game on its own keeps one only when one is named.
pub(crate) fn kept_world(
    hosting: bool,
    console_name: Option<&str>,
    artifacts: Option<&Path>,
) -> Option<(String, PathBuf)> {
    let env_name = std::env::var("IW4L_MINECRAFT_WORLD")
        .ok()
        .filter(|n| !n.trim().is_empty());
    let name = console_name.map(str::to_owned).or(env_name);
    if !hosting && name.is_none() {
        return None;
    }
    let name = minecraft_terrain::multiplayer::sanitize_world_name(
        name.as_deref().unwrap_or(DEFAULT_WORLD),
    );
    let dir = minecraft_terrain::multiplayer::world_dir(&worlds_root(artifacts)?, &name);
    Some((name, dir))
}

impl Host {
    pub(crate) fn new(
        seed: i64,
        origin: [f64; 3],
        save: Option<PathBuf>,
        saved: Option<SavedWorld>,
    ) -> Self {
        let (log, players) = saved.map_or_else(Default::default, |s| (s.edits, s.players));
        Self {
            seed,
            origin,
            save,
            log,
            players,
            remotes: BTreeMap::new(),
            local_name: None,
            since_save: 0.0,
            dirty: false,
        }
    }

    /// Logs a block change.
    pub(crate) fn record(&mut self, pos: BlockPos, block: Option<String>) {
        self.log.push(pos, block);
        self.dirty = true;
    }

    /// The saved inventory of the player called `name`, if any.
    pub(crate) fn inventory_of(&self, name: &str) -> Option<&InventoryState> {
        self.players.get(name)
    }

    /// Keeps a player's inventory (without its guns, which a match gives
    /// anew) under its name.
    pub(crate) fn keep_inventory(&mut self, name: &str, inventory: &Inventory, selected: usize) {
        if name.is_empty() {
            return;
        }
        let state =
            InventoryState::capture(inventory, selected).without(|s| s.id.starts_with("iw4:"));
        if self.players.get(name) != Some(&state) {
            self.players.insert(name.to_owned(), state);
            self.dirty = true;
        }
    }

    /// Writes the kept world, if this one is kept.
    pub(crate) fn save_now(&mut self) {
        let Some(dir) = self.save.clone() else {
            return;
        };
        let mut world = SavedWorld::new(self.seed, self.origin);
        world.edits = self.log.compacted();
        world.players = self.players.clone();
        match world.save(&dir) {
            Ok(()) => {
                self.dirty = false;
                self.since_save = 0.0;
                diag::info!(
                    World,
                    "Minecraft world saved: {} ({} blocks changed, {} players)",
                    dir.display(),
                    world.edits.len(),
                    world.players.len()
                );
            }
            Err(error) => diag::warn!(
                World,
                "Minecraft world save to {} failed: {error}",
                dir.display()
            ),
        }
    }

    /// Saves when the autosave is due and something changed.
    pub(crate) fn autosave(&mut self, dt: f64) {
        self.since_save += dt;
        if self.since_save >= AUTOSAVE_SECONDS {
            self.since_save = 0.0;
            if self.dirty {
                self.save_now();
            }
        }
    }
}

impl Remote {
    pub(crate) fn new(
        name: String,
        mut inventory: Inventory,
        saved: Option<&InventoryState>,
    ) -> Self {
        let selected = saved.map_or(0, |state| state.restore(&mut inventory));
        Self {
            name,
            inventory,
            selected: selected.min(frame::minecraft_ui::MC_HOTBAR - 1),
            table: None,
            applied: 0,
            world_sent: false,
            cursor: 0,
            corrections: Vec::new(),
            sent_inventory: None,
            items_clock: ITEMS_SECONDS,
            sent_items: None,
            placed: false,
            placed_at: None,
            life: None,
            collision: None,
            held: HashMap::new(),
            held_still: false,
        }
    }
}

/// Whether a client's stream has room for another Minecraft row.
fn has_room(queue: Option<&net::ReliableEventQueue>) -> bool {
    let Some(queue) = queue else {
        return true;
    };
    let bytes: usize = queue
        .pending()
        .iter()
        .map(|(_, row)| match row {
            net::ReliableRow::Minecraft(bytes) => bytes.len(),
            _ => 0,
        })
        .sum();
    queue.pending_len() < ROWS_IN_FLIGHT && bytes < BYTES_IN_FLIGHT
}

fn push(reliable: &mut net::ReliableEventHub, client: sim::ClientId, message: &HostMessage) {
    reliable
        .queue_mut(client)
        .push(net::ReliableRow::Minecraft(message.encode()));
}

/// Sends one client what it lacks, as its stream has room: the world, the
/// host's corrections, the edit log from its cursor, its inventory when it
/// changed, and the items near it.
pub(crate) fn send_to(
    reliable: &mut net::ReliableEventHub,
    client: sim::ClientId,
    host: &Host,
    remote: &mut Remote,
    items: &[WireItem],
    feet: Option<[f64; 3]>,
    dt: f64,
) {
    if !remote.world_sent {
        push(
            reliable,
            client,
            &HostMessage::World {
                seed: host.seed,
                origin: host.origin,
            },
        );
        remote.world_sent = true;
    }
    if let Some(at) = remote.placed_at.take() {
        push(reliable, client, &HostMessage::Placed { at });
    }
    while !remote.corrections.is_empty() && has_room(reliable.queue(client)) {
        let fit = edits_that_fit(&remote.corrections);
        let corrections: Vec<_> = remote.corrections.drain(..fit).collect();
        push(reliable, client, &HostMessage::Edits(corrections));
    }
    while remote.cursor < host.log.len() && has_room(reliable.queue(client)) {
        let (batch, next) = host.log.batch(remote.cursor);
        push(reliable, client, &HostMessage::Edits(batch));
        remote.cursor = next;
    }
    let state = InventoryState::capture(&remote.inventory, remote.selected);
    let current = (remote.applied, remote.table, state);
    if remote.sent_inventory.as_ref() != Some(&current) && has_room(reliable.queue(client)) {
        push(
            reliable,
            client,
            &HostMessage::Inventory {
                applied: current.0,
                table: current.1,
                state: current.2.clone(),
            },
        );
        remote.sent_inventory = Some(current);
    }
    remote.items_clock += dt;
    if remote.items_clock >= ITEMS_SECONDS
        && let Some(feet) = feet
    {
        let near: Vec<WireItem> = items
            .iter()
            .filter(|item| {
                (0..3)
                    .map(|i| (item.position[i] - feet[i]).powi(2))
                    .sum::<f64>()
                    < ITEMS_RANGE * ITEMS_RANGE
            })
            .take(minecraft_terrain::multiplayer::MAX_ITEMS)
            .cloned()
            .collect();
        let changed = remote.sent_items.as_ref().is_none_or(|sent| sent != &near);
        if changed && has_room(reliable.queue(client)) {
            remote.items_clock = 0.0;
            push(reliable, client, &HostMessage::Items(near.clone()));
            remote.sent_items = Some(near);
        }
    }
}

/// The host's items on the ground, as clients draw them.
pub(crate) fn wire_items(items: &minecraftoss_player::items::WorldItems) -> Vec<WireItem> {
    items
        .entities
        .iter()
        .map(|e| WireItem {
            id: e.entity_id,
            stack: WireStack::from_stack(&e.stack),
            position: e.position.to_array(),
            velocity: e.velocity.as_vec3().to_array(),
            age: e.age,
            on_ground: e.on_ground,
        })
        .collect()
}

/// The host's items as a client's world items.
pub(crate) fn client_items(items: &[WireItem]) -> Vec<minecraftoss_player::items::ItemEntity> {
    items
        .iter()
        .map(|item| minecraftoss_player::items::ItemEntity {
            entity_id: item.id,
            stack: item.stack.to_stack(),
            position: glam::DVec3::from_array(item.position),
            previous_position: glam::DVec3::from_array(item.position),
            velocity: glam::Vec3::from_array(item.velocity).as_dvec3(),
            age: item.age,
            bob_offset: (item.id as f32 * 0.37) % std::f32::consts::TAU,
            pickup_delay: 0,
            on_ground: item.on_ground,
        })
        .collect()
}

impl Client {
    /// Sends an intent to the host; false when the action queue refused it.
    pub(crate) fn send(
        &mut self,
        inbox: Option<&mut net::ClientActionInbox>,
        ids: Option<&mut net::ActionRequestIds>,
        local: sim::ClientId,
        intent: &Intent,
    ) -> bool {
        let (Some(inbox), Some(ids)) = (inbox, ids) else {
            return false;
        };
        let Some(bytes) = intent
            .encode()
            .and_then(|bytes| sim::MinecraftIntent::new(&bytes))
        else {
            diag::warn!(World, "Minecraft intent too large to send: {intent:?}");
            return false;
        };
        let request_id = ids.allocate();
        match inbox.push(
            local,
            sim::ClientAction::Minecraft {
                request_id,
                intent: bytes,
            },
        ) {
            Ok(()) => {
                self.last_sent = request_id;
                true
            }
            Err(error) => {
                diag::warn!(World, "Minecraft intent not sent: {error}");
                false
            }
        }
    }
}

fn ui_slot(slot: SlotRef) -> frame::McSlot {
    match slot {
        SlotRef::Inventory(n) => frame::McSlot::Inventory(usize::from(n)),
        SlotRef::Crafting(n) => frame::McSlot::Crafting(usize::from(n)),
        SlotRef::Result => frame::McSlot::Result,
    }
}

fn wire_slot(slot: frame::McSlot) -> Option<SlotRef> {
    Some(match slot {
        frame::McSlot::Inventory(n) => SlotRef::Inventory(u8::try_from(n).ok()?),
        frame::McSlot::Crafting(n) => SlotRef::Crafting(u8::try_from(n).ok()?),
        frame::McSlot::Result => SlotRef::Result,
    })
}

/// A client's click as the inventory screen took it.
pub(crate) fn ui_click(click: Click) -> frame::McClick {
    match click {
        Click::Slot { slot, right, shift } => frame::McClick::Slot {
            slot: ui_slot(slot),
            right,
            shift,
        },
        Click::Outside { right } => frame::McClick::Outside { right },
        Click::Gather { right } => frame::McClick::Gather { right },
        Click::Swap { slot, hotbar } => frame::McClick::Swap {
            slot: ui_slot(slot),
            hotbar: usize::from(hotbar),
        },
        Click::Spread { slots, right } => frame::McClick::Spread {
            slots: slots.into_iter().map(usize::from).collect(),
            right,
        },
        Click::Close => frame::McClick::Close,
    }
}

/// The inventory screen's click as a client sends it.
pub(crate) fn wire_click(click: &frame::McClick) -> Option<Click> {
    Some(match click {
        frame::McClick::Slot { slot, right, shift } => Click::Slot {
            slot: wire_slot(*slot)?,
            right: *right,
            shift: *shift,
        },
        frame::McClick::Outside { right } => Click::Outside { right: *right },
        frame::McClick::Gather { right } => Click::Gather { right: *right },
        frame::McClick::Swap { slot, hotbar } => Click::Swap {
            slot: wire_slot(*slot)?,
            hotbar: u8::try_from(*hotbar).ok()?,
        },
        frame::McClick::Spread { slots, right } => Click::Spread {
            slots: slots
                .iter()
                .map(|&s| u8::try_from(s).ok())
                .collect::<Option<_>>()?,
            right: *right,
        },
        frame::McClick::Close => Click::Close,
    })
}
