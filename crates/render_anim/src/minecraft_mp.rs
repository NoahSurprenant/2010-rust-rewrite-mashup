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
//! Mobs are the host's: its level spawns and runs them around every player
//! (`minecraft_entities`), and each client is told of the mobs near it
//! (`minecraft_terrain::mob_sync`): a `HostMessage::Mobs` row when one comes
//! into its view or changes its looks, a `MobsGone` row when it leaves, and
//! every mob's state with each snapshot, unreliably.
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use minecraft_terrain::multiplayer::{
    Click, EditLog, HostMessage, Intent, InventoryState, SavedWorld, SlotRef, WireItem, WireStack,
    edits_that_fit,
};
use minecraft_terrain::scene::BlockPos;
use minecraft_terrain::mob_sync::{GONE_PER_MESSAGE, HostMob, MobSpawn, MobState, spawns_that_fit};
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
/// How often a client is sent what changed on the maps it carries: only
/// the 16 by 16 tiles that changed go, so a map costs its 16 KiB once and
/// then the ground its holder uncovers.
const MAP_SECONDS: f64 = 0.5;
/// A map's ticks (`MapItem.update` runs each server tick).
const MAP_TICK_SECONDS: f64 = 1.0 / 20.0;
/// How often the host reports the mob bytes it sends each client.
const MOB_REPORT_SECONDS: f64 = 10.0;
/// Host ticks after which a mob that has not changed is sent again.
const MOB_RESEND_TICKS: u32 = 5;
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
    /// The player key it proved, which its inventory is kept under; none
    /// for a guest, whose inventory is not kept.
    pub key: Option<String>,
    pub inventory: Inventory,
    /// The MW2 weapons it has permanently unlocked (the starter kit, then
    /// the crafting table), seeded from its record if it has one; fed to
    /// the host's simulation every tick (`FrameWorld::set_minecraft_unlocked_weapons`).
    pub unlocked_weapons: Vec<String>,
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
    /// Each map of its inventory as it was last sent it.
    maps_sent: HashMap<u32, minecraft_terrain::filled_map::MapData>,
    map_clock: f64,
    map_bytes: u64,
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
    /// Chunks generated around it for the mobs' level, out to the
    /// simulation distance, and those of them it holds there.
    pub mob_stream: Option<minecraft_terrain::terrain::CollisionStream>,
    pub level_held: std::collections::HashSet<(i32, i32)>,
    /// The host holds it still: there is no ground where it stands yet.
    pub held_still: bool,
    /// The mobs it was shown, as it was shown them.
    mobs_sent: HashMap<u64, MobSpawn>,
    /// The state each was last sent, and the tick it was sent at.
    mob_states: HashMap<u64, (MobState, u32)>,
    /// Mob bytes sent it since the last report: snapshot states and rows.
    mob_frame_bytes: u64,
    mob_row_bytes: u64,
    mob_clock: f64,
    /// The events going out to it.
    events: minecraft_terrain::mob_sync::EventWindow,
}

/// The host's mobs this frame, for its clients: each mob as it is told,
/// the tick they are of, and whether that tick is new; the events of the
/// last few ticks, what flies, and the players' effects.
pub(crate) struct MobView<'a> {
    pub mobs: &'a [HostMob],
    pub tick: u32,
    pub fresh: bool,
    pub events: &'a minecraft_terrain::mob_sync::EventLog,
    pub projectiles: &'a [minecraft_terrain::mob_sync::Projectile],
    pub effects: &'a minecraft_terrain::mob_sync::PlayerEffects,
}

pub(crate) struct Host {
    pub seed: i64,
    pub origin: [f64; 3],
    /// The kept world's directory, when this world is kept.
    pub save: Option<PathBuf>,
    pub log: EditLog,
    /// Each player as last saved or left, by player key
    /// (`minecraft_terrain::multiplayer::Players`).
    pub players: minecraft_terrain::multiplayer::Players,
    /// The clients in the world, by client number.
    pub remotes: BTreeMap<u32, Remote>,
    /// The host's own player key and name once known (its inventory then
    /// came back); none on a dedicated server.
    pub local: Option<(String, String)>,
    since_save: f64,
    /// Something changed since the last save.
    pub dirty: bool,
    /// The world's maps by id, and the columns they read around every
    /// player (`minecraft_terrain::filled_map`).
    pub maps: minecraft_terrain::filled_map::MapStore,
    pub columns: minecraft_terrain::filled_map::Columns,
    /// Each holder's ticks with each map it held, by (holder, map).
    map_steps: HashMap<(u64, u32), u32>,
    map_clock: f64,
    /// The host's own player, without a key: given its map this session.
    pub local_map_given: bool,
    /// The host's own player, without a key: given its starter kit this
    /// session.
    pub local_starter_kit_given: bool,
    /// The host's own player's permanently unlocked MW2 weapons (the
    /// starter kit, then the crafting table): kept for a keyed player,
    /// fresh each session for a guest, same as `local_map_given`.
    pub local_unlocked_weapons: Vec<String>,
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
    /// The maps of its inventory, as the host sent them.
    pub maps: HashMap<u32, minecraft_terrain::filled_map::MapData>,
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
        let (log, players, maps) = saved.map_or_else(Default::default, |s| (s.edits, s.players, s.maps));
        Self {
            seed,
            origin,
            save,
            log,
            players,
            remotes: BTreeMap::new(),
            local: None,
            since_save: 0.0,
            dirty: false,
            maps,
            columns: Default::default(),
            map_steps: HashMap::new(),
            map_clock: 0.0,
            local_map_given: false,
            local_starter_kit_given: false,
            local_unlocked_weapons: Vec::new(),
        }
    }

    /// Logs a block change.
    pub(crate) fn record(&mut self, pos: BlockPos, block: Option<String>) {
        self.log.push(pos, block);
        self.dirty = true;
    }

    /// A player joining with the key `key` under `name`: its kept inventory,
    /// if any (an inventory kept under its name from before player keys is
    /// claimed by the first key that joins with the name).
    pub(crate) fn join(&mut self, key: &str, name: &str) -> Option<InventoryState> {
        self.dirty = true;
        self.players.join(key, name, unix_now())
    }

    /// A player's starting map, once per player and world: a keyed player
    /// owed one by its record (a new player, or one from a world saved
    /// before maps), or a guest, who is new at each join. Puts it in
    /// `inventory`; whether it did. A full inventory is given none, and its
    /// player is still owed one at its next join.
    pub(crate) fn give_map_once(&mut self, key: Option<&str>, inventory: &mut Inventory, name: &str) -> bool {
        if crate::minecraft_inventory::starting_map_slot(inventory).is_none() {
            return false;
        }
        if let Some(key) = key
            && !self.players.take_map(key)
        {
            return false;
        }
        let id = self.maps.allocate();
        let given = crate::minecraft_inventory::give_starting_map(inventory, id);
        diag::info!(World, "Minecraft: `{name}` given its map (map {id})");
        self.dirty = true;
        given
    }

    /// A player's starter pistol and ammo, once per player and world: a
    /// keyed player owed one by its record (a new player, or one from a
    /// world saved before the starter kit), or a guest, who is new at each
    /// join, same as `give_map_once`. Unlocks `crate::minecraft_inventory::STARTER_WEAPON`
    /// in `unlocked` (a keyed player's record too) and gives the ammo
    /// stack; the unlock always lands (it has no slot to run out of, only
    /// the ammo item can fail to fit), so this is only ever asked once.
    pub(crate) fn give_starter_kit_once(
        &mut self,
        key: Option<&str>,
        inventory: &mut Inventory,
        unlocked: &mut Vec<String>,
        name: &str,
    ) -> bool {
        if let Some(key) = key
            && !self.players.take_starter_kit(key)
        {
            return false;
        }
        let weapon = crate::minecraft_inventory::STARTER_WEAPON;
        if !unlocked.iter().any(|w| w == weapon) {
            unlocked.push(weapon.to_owned());
        }
        if let Some(key) = key {
            self.players.add_unlocked_weapon(key, weapon);
        }
        // A full inventory loses the ammo stack, same as a full one gets no
        // map; a brand new inventory always has room.
        let given = crate::minecraft_inventory::give_starter_ammo(inventory);
        diag::info!(
            World,
            "TEMP-GUN-DEBUG4: give_starter_ammo returned {given}, inventory now has {} ammo",
            inventory.count(crate::minecraft_inventory::AMMO_ITEM)
        );
        diag::info!(World, "Minecraft: `{name}` given its starter kit ({weapon})");
        self.dirty = true;
        true
    }

    /// Fills in the maps held by `holders` (each a holder's key, its feet in
    /// blocks and the map in its hand) at 20 ticks a second; a map held for
    /// the first time is made where its holder stands.
    pub(crate) fn tick_maps(&mut self, dt: f64, holders: &[(u64, [f64; 3], u32)]) {
        self.map_clock += dt;
        let ticks = (self.map_clock / MAP_TICK_SECONDS) as u32;
        self.map_clock -= f64::from(ticks) * MAP_TICK_SECONDS;
        if ticks == 0 {
            return;
        }
        for &(holder, feet, id) in holders {
            if id >= self.maps.next_id {
                continue;
            }
            let map = self
                .maps
                .maps
                .entry(id)
                .or_insert_with(|| minecraft_terrain::filled_map::MapData::new(feet[0], feet[2], 0));
            let step = self.map_steps.entry((holder, id)).or_default();
            for _ in 0..ticks.min(4) {
                if minecraft_terrain::filled_map::update(map, feet, step, |x, z| self.columns.get(x, z)) {
                    self.dirty = true;
                }
            }
        }
    }

    /// Keeps a player's inventory (without its guns, which a match gives
    /// anew) under its key.
    pub(crate) fn keep_inventory(&mut self, key: &str, name: &str, inventory: &Inventory, selected: usize) {
        let state =
            InventoryState::capture(inventory, selected).without(|s| s.id.starts_with("iw4:"));
        if self.players.keep(key, name, state, unix_now()) {
            self.dirty = true;
        }
    }

    /// The keys of the players in the game now: a transfer leaves them be.
    pub(crate) fn keys_in_game(&self) -> Vec<String> {
        self.local
            .iter()
            .map(|(key, _)| key.clone())
            .chain(self.remotes.values().filter_map(|remote| remote.key.clone()))
            .collect()
    }

    /// Writes the kept world, if this one is kept.
    pub(crate) fn save_now(&mut self) {
        let Some(dir) = self.save.clone() else {
            return;
        };
        let mut world = SavedWorld::new(self.seed, self.origin);
        world.edits = self.log.compacted();
        world.players = self.players.clone();
        world.maps = self.maps.clone();
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
        key: Option<String>,
        mut inventory: Inventory,
        saved: Option<&InventoryState>,
    ) -> Self {
        let selected = saved.map_or(0, |state| state.restore(&mut inventory));
        Self {
            name,
            key,
            inventory,
            unlocked_weapons: Vec::new(),
            selected: selected.min(frame::minecraft_ui::MC_HOTBAR - 1),
            table: None,
            applied: 0,
            world_sent: false,
            cursor: 0,
            corrections: Vec::new(),
            sent_inventory: None,
            items_clock: ITEMS_SECONDS,
            maps_sent: HashMap::new(),
            map_clock: MAP_SECONDS,
            map_bytes: 0,
            sent_items: None,
            placed: false,
            placed_at: None,
            life: None,
            collision: None,
            held: HashMap::new(),
            mob_stream: None,
            level_held: std::collections::HashSet::new(),
            held_still: false,
            mobs_sent: HashMap::new(),
            mob_states: HashMap::new(),
            mob_frame_bytes: 0,
            mob_row_bytes: 0,
            mob_clock: 0.0,
            events: minecraft_terrain::mob_sync::EventWindow::default(),
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

/// Queues a message on a client's reliable stream; its length.
fn push(reliable: &mut net::ReliableEventHub, client: sim::ClientId, message: &HostMessage) -> usize {
    let bytes = message.encode();
    let len = bytes.len();
    reliable.queue_mut(client).push(net::ReliableRow::Minecraft(bytes));
    len
}

/// Sends one client what it lacks, as its stream has room: the world, the
/// host's corrections, its inventory when it changed, the items near it,
/// the mobs near it, and then the edit log from its cursor.
#[allow(clippy::too_many_arguments)]
pub(crate) fn send_to(
    reliable: &mut net::ReliableEventHub,
    client: sim::ClientId,
    host: &Host,
    remote: &mut Remote,
    items: &[WireItem],
    feet: Option<[f64; 3]>,
    dt: f64,
    mobs: Option<&MobView<'_>>,
    frames: Option<&mut net::PendingMinecraftFrames>,
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
    // What is the client's or near it now (its inventory, the items, the
    // mobs) goes ahead of the edit log, which takes the room left: the log
    // can be many seconds behind (a joining player's whole history, or a
    // level that changes thousands of blocks near a client, water and lava
    // running in fresh chunks), and a mob or an item must not wait for it.
    send_inventory(reliable, client, remote);
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
    if let (Some(view), Some(feet)) = (mobs, feet) {
        send_mobs(reliable, client, remote, view, feet, frames);
    }
    remote.map_clock += dt;
    if remote.map_clock >= MAP_SECONDS {
        remote.map_clock = 0.0;
        send_maps(reliable, client, &host.maps, remote);
    }
    while remote.cursor < host.log.len() && has_room(reliable.queue(client)) {
        let (batch, next) = host.log.batch(remote.cursor);
        push(reliable, client, &HostMessage::Edits(batch));
        remote.cursor = next;
    }
    remote.mob_clock += dt;
    if remote.mob_clock >= MOB_REPORT_SECONDS {
        let per_second = |bytes: u64| (bytes as f64 / remote.mob_clock).round();
        let line = format!(
            "minecraft-mobnet: client={} mobs={} frame_bytes_per_s={} row_bytes_per_s={}",
            client.0,
            remote.mobs_sent.len(),
            per_second(remote.mob_frame_bytes),
            per_second(remote.mob_row_bytes),
        );
        diag::info!(World, "{line}");
        println!("{line}");
        if remote.map_bytes > 0 {
            let line = format!(
                "minecraft-mapnet: client={} maps={} bytes_per_s={}",
                client.0,
                remote.maps_sent.len(),
                per_second(remote.map_bytes)
            );
            diag::info!(World, "{line}");
            println!("{line}");
            remote.map_bytes = 0;
        }
        remote.mob_clock = 0.0;
        remote.mob_frame_bytes = 0;
        remote.mob_row_bytes = 0;
    }
}

/// The client's inventory, when it changed since it was last sent and its
/// stream has room.
fn send_inventory(reliable: &mut net::ReliableEventHub, client: sim::ClientId, remote: &mut Remote) {
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
}

/// The maps a client carries (anywhere in its inventory, as a server sends
/// a player's maps), each the tiles that changed since it was last sent
/// them, as its stream has room.
fn send_maps(
    reliable: &mut net::ReliableEventHub,
    client: sim::ClientId,
    maps: &minecraft_terrain::filled_map::MapStore,
    remote: &mut Remote,
) {
    use minecraft_terrain::filled_map::{MapData, SIZE, TILES_PER_MESSAGE};
    let inventory = &remote.inventory;
    let mut carried: Vec<u32> = inventory
        .slots
        .iter()
        .chain(inventory.crafting.iter())
        .chain(inventory.workbench.iter())
        .chain(std::iter::once(&inventory.cursor))
        .filter_map(|s| s.as_ref().and_then(crate::minecraft_inventory::map_of))
        .collect();
    carried.sort_unstable();
    carried.dedup();
    remote.maps_sent.retain(|id, _| carried.contains(id));
    for id in carried {
        let Some(map) = maps.maps.get(&id) else {
            continue;
        };
        let changed = map.changed_tiles(remote.maps_sent.get(&id));
        for tiles in changed.chunks(TILES_PER_MESSAGE) {
            if !has_room(reliable.queue(client)) {
                return;
            }
            let message = HostMessage::MapTiles {
                id,
                centre: [map.centre.0, map.centre.1],
                scale: map.scale,
                tiles: tiles.iter().map(|&i| (i as u8, map.tile(i).to_vec())).collect(),
            };
            remote.map_bytes += push(reliable, client, &message) as u64;
            let sent = remote.maps_sent.entry(id).or_insert_with(|| MapData {
                centre: map.centre,
                scale: map.scale,
                colours: vec![0; SIZE * SIZE],
            });
            if (sent.centre, sent.scale) != (map.centre, map.scale) {
                *sent = MapData { centre: map.centre, scale: map.scale, colours: vec![0; SIZE * SIZE] };
            }
            for &i in tiles {
                sent.set_tile(i, &map.tile(i));
            }
        }
    }
}

/// A map's tiles as a client takes them.
pub(crate) fn take_map_tiles(
    maps: &mut HashMap<u32, minecraft_terrain::filled_map::MapData>,
    id: u32,
    centre: [i32; 2],
    scale: u8,
    tiles: &[(u8, Vec<u8>)],
) {
    use minecraft_terrain::filled_map::{MapData, SIZE, TILE};
    let map = maps.entry(id).or_insert_with(|| MapData {
        centre: (centre[0], centre[1]),
        scale,
        colours: vec![0; SIZE * SIZE],
    });
    if (map.centre, map.scale) != ((centre[0], centre[1]), scale) {
        *map = MapData { centre: (centre[0], centre[1]), scale, colours: vec![0; SIZE * SIZE] };
    }
    for (index, pixels) in tiles {
        if let Ok(pixels) = <[u8; TILE * TILE]>::try_from(pixels.as_slice()) {
            map.set_tile(usize::from(*index), &pixels);
        }
    }
}

/// The mobs near a client: rows for the ones it leaves and the ones it
/// comes to see (or that change their looks), as its stream has room, then
/// the states of those it was shown with its next snapshot, once each tick.
fn send_mobs(
    reliable: &mut net::ReliableEventHub,
    client: sim::ClientId,
    remote: &mut Remote,
    view: &MobView<'_>,
    feet: [f64; 3],
    frames: Option<&mut net::PendingMinecraftFrames>,
) {
    let near = minecraft_terrain::mob_sync::interest(view.mobs, feet);
    let near_ids: std::collections::HashSet<u64> = near.iter().map(|&i| view.mobs[i].spawn.id).collect();
    let gone: Vec<u64> = remote
        .mobs_sent
        .keys()
        .copied()
        .filter(|id| !near_ids.contains(id))
        .collect();
    for ids in gone.chunks(GONE_PER_MESSAGE) {
        if !has_room(reliable.queue(client)) {
            break;
        }
        remote.mob_row_bytes += push(reliable, client, &HostMessage::MobsGone(ids.to_vec())) as u64;
        for id in ids {
            remote.mobs_sent.remove(id);
            remote.mob_states.remove(id);
        }
    }
    let spawns: Vec<MobSpawn> = near
        .iter()
        .map(|&i| &view.mobs[i].spawn)
        .filter(|spawn| remote.mobs_sent.get(&spawn.id).is_none_or(|sent| !sent.same_look(spawn)))
        .cloned()
        .collect();
    let mut rest = &spawns[..];
    while !rest.is_empty() && has_room(reliable.queue(client)) {
        let n = spawns_that_fit(rest);
        remote.mob_row_bytes += push(reliable, client, &HostMessage::Mobs(rest[..n].to_vec())) as u64;
        for spawn in &rest[..n] {
            remote.mobs_sent.insert(spawn.id, spawn.clone());
            remote.mob_states.remove(&spawn.id);
        }
        rest = &rest[n..];
    }
    if view.fresh
        && let Some(frames) = frames
    {
        // A mob that stands as it stood goes again only now and then: a lost
        // snapshot leaves it stale that long at most.
        let states: Vec<&MobState> = near
            .iter()
            .map(|&i| &view.mobs[i].state)
            .filter(|state| remote.mobs_sent.contains_key(&state.id))
            .filter(|state| {
                remote.mob_states.get(&state.id).is_none_or(|(last, at)| {
                    last != *state || view.tick.wrapping_sub(*at) >= MOB_RESEND_TICKS
                })
            })
            .collect();
        let extras = minecraft_terrain::mob_sync::FrameExtras {
            events: remote.events.next(view.events, feet),
            projectiles: minecraft_terrain::mob_sync::projectiles_near(view.projectiles, feet),
            effects: view
                .effects
                .of(crate::minecraft_entities::remote_key(client.0)),
        };
        let bytes =
            minecraft_terrain::mob_sync::encode_frame(view.tick, states.iter().copied(), &extras);
        let sent = minecraft_terrain::mob_sync::frame_len(&bytes);
        for state in &states[..sent] {
            remote.mob_states.insert(state.id, (**state, view.tick));
        }
        remote.mob_frame_bytes += bytes.len() as u64;
        frames.set(client, bytes);
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

/// Now, in Unix seconds, for a player's first and last seen.
pub(crate) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// The host's clients whose inventories are kept (not guests): key, name,
/// inventory and hotbar slot.
pub(crate) fn kept_remotes(host: &Host) -> Vec<(String, String, Inventory, usize)> {
    host.remotes
        .values()
        .filter_map(|remote| {
            let key = remote.key.clone()?;
            Some((key, remote.name.clone(), remote.inventory.clone(), remote.selected))
        })
        .collect()
}

/// An `admin transfer` (the host's console, or an admin's): the record of
/// `from` (a key, or the name a key last played under) moved to the key
/// `to`, and `from`'s place on the admin list with it. Neither key may be in
/// the game. The line it prints: `admin-transfer: label=… result=ok from=…
/// to=… name=… inventory=… admin=…` or `… result=refused reason="…"`.
pub(crate) fn admin_transfer(
    host: &mut Host,
    admins: Option<&mut net::DebugAdmins>,
    from: &str,
    to: &str,
    label: Option<&str>,
) -> String {
    let label = label.unwrap_or("-");
    let result = (|| {
        let in_game = host.keys_in_game();
        if in_game.iter().any(|key| key == to) {
            return Err("the new key is in the game: transfer once it has left".to_owned());
        }
        let old = if minecraft_terrain::multiplayer::is_fingerprint(from) {
            Some(from.to_owned())
        } else {
            match host.players.keys_named(from).as_slice() {
                [one] => Some(one.clone()),
                _ => None,
            }
        };
        if old.as_ref().is_some_and(|old| in_game.contains(old)) {
            return Err("that player is in the game: transfer once it has left".to_owned());
        }
        let done = host.players.transfer(from, to, unix_now())?;
        host.dirty = true;
        host.save_now();
        let admin = match (done.from.as_deref(), admins) {
            (Some(old), Some(admins)) => admins.replace(old, to),
            _ => false,
        };
        Ok(format!(
            "from={} to={} name=\"{}\" inventory={} admin={admin}",
            done.from.as_deref().unwrap_or("-"),
            done.to,
            done.name,
            done.inventory
        ))
    })();
    match result {
        Ok(detail) => format!("admin-transfer: label={label} result=ok {detail}"),
        Err(reason) => format!(
            "admin-transfer: label={label} result=refused reason=\"{}\"",
            reason.replace('"', "'")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minecraft_terrain::mob_sync::{EventLog, Look, PlayerEffects};

    fn rows(reliable: &net::ReliableEventHub, client: sim::ClientId) -> Vec<HostMessage> {
        reliable
            .queue(client)
            .map(|queue| queue.pending())
            .unwrap_or_default()
            .iter()
            .filter_map(|(_, row)| match row {
                net::ReliableRow::Minecraft(bytes) => HostMessage::decode(bytes).ok(),
                _ => None,
            })
            .collect()
    }

    /// A mob that comes into a client's view while the host's edit log is
    /// far behind (the level changing thousands of blocks near it) is sent
    /// as soon as the client's acks make room, not after the whole log.
    #[test]
    fn a_mob_goes_ahead_of_a_long_edit_log() {
        let mut host = Host::new(1, [0.0; 3], None, None);
        for x in 0..20_000 {
            host.record((x, 64, 0), Some("minecraft:water[level=3]".into()));
        }
        let mut remote = Remote::new("client".into(), None, Inventory::default(), None);
        let mut reliable = net::ReliableEventHub::default();
        let client = sim::ClientId(1);
        let feet = [0.5, 65.0, 0.5];
        let (events, effects) = (EventLog::default(), PlayerEffects::default());
        let mut mobs = Vec::new();
        let frame =
            |reliable: &mut net::ReliableEventHub, remote: &mut Remote, mobs: &[HostMob]| {
                let view = MobView {
                    mobs,
                    tick: 1,
                    fresh: false,
                    events: &events,
                    projectiles: &[],
                    effects: &effects,
                };
                send_to(
                    reliable,
                    client,
                    &host,
                    remote,
                    &[],
                    Some(feet),
                    0.05,
                    Some(&view),
                    None,
                );
            };
        // The first frame: the window is full of the log.
        frame(&mut reliable, &mut remote, &mobs);
        assert!(!has_room(reliable.queue(client)));
        let behind = host.log.len() - remote.cursor;
        assert!(behind > 10_000, "{behind} edits left");
        // A bat comes; the client acks all it has, and the next frame
        // sends the bat though the log is still thousands of edits behind.
        mobs.push(HostMob {
            spawn: MobSpawn {
                id: 7,
                look: Look::Bat,
                baby: false,
                width: 0.6,
                height: 1.95,
                position: [3.5, 65.0, 0.5],
                yaw: 0.0,
            },
            state: MobState {
                id: 7,
                position: [3.5, 65.0, 0.5],
                ..MobState::default()
            },
        });
        let through = reliable
            .queue(client)
            .and_then(|queue| queue.pending().last().map(|(seq, _)| *seq))
            .expect("rows queued");
        reliable.queue_mut(client).ack(through);
        frame(&mut reliable, &mut remote, &mobs);
        let sent = rows(&reliable, client);
        let bat = sent.iter().position(
            |m| matches!(m, HostMessage::Mobs(spawns) if spawns.iter().any(|s| s.id == 7)),
        );
        let first_edit = sent.iter().position(|m| matches!(m, HostMessage::Edits(_)));
        assert!(bat.is_some(), "no Mobs row for the bat");
        assert!(bat < first_edit, "the bat after the log's edits");
        assert!(host.log.len() - remote.cursor > 5_000);
    }

    fn key(n: u8) -> String {
        format!("{n:02x}").repeat(32)
    }

    fn maps_in(inventory: &Inventory) -> Vec<u32> {
        inventory.slots.iter().filter_map(|s| s.as_ref().and_then(crate::minecraft_inventory::map_of)).collect()
    }

    /// A new player is given one map, on joining; another join of the same
    /// key (a rejoin, or a spawn that asks again) in the same session or
    /// after the world is saved and loaded gives none. A guest is new at
    /// each join.
    #[test]
    fn a_player_is_given_its_map_once_per_world() {
        let dir = std::env::temp_dir().join(format!("iw4l-map-once-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut host = Host::new(12345, [90.5, 130.0, -30.5], Some(dir.clone()), None);
        let mut inventory = Inventory::default();
        host.join(&key(1), "noah");
        assert!(host.give_map_once(Some(&key(1)), &mut inventory, "noah"));
        assert_eq!(maps_in(&inventory), vec![0]);
        // A respawn or a rejoin in the same session.
        host.join(&key(1), "noah");
        assert!(!host.give_map_once(Some(&key(1)), &mut inventory, "noah"));
        assert_eq!(maps_in(&inventory), vec![0]);
        host.keep_inventory(&key(1), "noah", &inventory, 0);
        host.save_now();
        // The world again: its record says it was given.
        let saved = SavedWorld::load(&dir).unwrap().unwrap();
        let mut host = Host::new(12345, [90.5, 130.0, -30.5], Some(dir.clone()), Some(saved));
        let state = host.join(&key(1), "noah").unwrap();
        let mut back = Inventory::default();
        state.restore(&mut back);
        assert!(!host.give_map_once(Some(&key(1)), &mut back, "noah"));
        assert_eq!(maps_in(&back), vec![0]);
        // A new id for the next new player: ids are never given twice.
        host.join(&key(2), "ann");
        let mut other = Inventory::default();
        assert!(host.give_map_once(Some(&key(2)), &mut other, "ann"));
        assert_eq!(maps_in(&other), vec![1]);
        // Guests have no record: one each time they come.
        let mut guest = Inventory::default();
        assert!(host.give_map_once(None, &mut guest, "guest"));
        assert_eq!(maps_in(&guest), vec![2]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A world saved before maps (no `map_given` in players.json): each
    /// kept player is given a map on its next join, once.
    #[test]
    fn a_player_of_an_older_world_is_given_its_map_on_its_next_join() {
        let mut old = SavedWorld::new(12345, [90.5, 130.0, -30.5]);
        let mut torches = Inventory::default();
        torches.slots[2] = Some(minecraftoss_player::inventory::ItemStack::new("minecraft:torch", 7));
        old.players.keep(&key(1), "noah", InventoryState::capture(&torches, 0), 1);
        let text = serde_json::to_string(&old.players).unwrap().replace(",\"map_given\":false", "");
        assert!(!text.contains("map_given"));
        old.players = serde_json::from_str(&text).unwrap();
        let mut host = Host::new(12345, [90.5, 130.0, -30.5], None, Some(old));
        let mut inventory = Inventory::default();
        host.join(&key(1), "noah").unwrap().restore(&mut inventory);
        assert!(host.give_map_once(Some(&key(1)), &mut inventory, "noah"));
        assert_eq!(inventory.slots[2].as_ref().map(|s| s.count), Some(7));
        assert_eq!(maps_in(&inventory), vec![0]);
        assert!(!host.give_map_once(Some(&key(1)), &mut inventory, "noah"));
        assert_eq!(maps_in(&inventory), vec![0]);
    }

    /// A map held fills in from the host's columns, and its holder's client
    /// is sent its tiles: all of them once, then only those that change;
    /// it reads back the same map.
    #[test]
    fn a_held_map_fills_in_and_reaches_its_holder_tile_by_tile() {
        use minecraft_terrain::filled_map::{Column, GRASS, TILE};
        let mut host = Host::new(1, [0.0; 3], None, None);
        let mut inventory = Inventory::default();
        assert!(host.give_map_once(None, &mut inventory, "client"));
        let mut remote = Remote::new("client".into(), None, inventory, None);
        // The ground the host has: grass at 64 in the chunks within 48
        // blocks of the world's middle.
        for cx in -3..3 {
            for cz in -3..3 {
                host.columns.set_chunk((cx, cz), Box::new([Column { colour: GRASS, y: 64, water_depth: 0 }; 256]));
            }
        }
        let feet = [10.5, 65.0, 10.5];
        for _ in 0..20 {
            host.tick_maps(MAP_TICK_SECONDS, &[(1, feet, 0)]);
        }
        let map = host.maps.maps.get(&0).expect("made where first held");
        assert_eq!(map.centre, (0, 0));
        assert_eq!(map.explored(), 96 * 96, "the loaded ground's pixels");
        assert!(host.dirty);
        let mut reliable = net::ReliableEventHub::default();
        let client = sim::ClientId(1);
        let mut client_maps = HashMap::new();
        fn take(
            reliable: &mut net::ReliableEventHub,
            client: sim::ClientId,
            maps: &mut HashMap<u32, minecraft_terrain::filled_map::MapData>,
        ) -> usize {
            let mut bytes = 0;
            for message in rows(reliable, client) {
                if let HostMessage::MapTiles { id, centre, scale, tiles } = &message {
                    bytes += message.encode().len();
                    take_map_tiles(maps, *id, *centre, *scale, tiles);
                }
            }
            let through = reliable.queue(client).and_then(|q| q.pending().last().map(|(seq, _)| *seq));
            if let Some(through) = through {
                reliable.queue_mut(client).ack(through);
            }
            bytes
        }
        // Its 36 tiles with ground, 15 a row.
        send_maps(&mut reliable, client, &host.maps, &mut remote);
        let header = 1 + 4 + 8 + 1 + 1;
        assert_eq!(take(&mut reliable, client, &mut client_maps), 3 * header + 36 * (1 + TILE * TILE));
        assert_eq!(client_maps.get(&0), host.maps.maps.get(&0));
        // Nothing changed: nothing goes.
        send_maps(&mut reliable, client, &host.maps, &mut remote);
        assert_eq!(take(&mut reliable, client, &mut client_maps), 0);
        // One pixel changes: its tile goes.
        host.maps.maps.get_mut(&0).unwrap().set(3, 3, 9);
        send_maps(&mut reliable, client, &host.maps, &mut remote);
        assert_eq!(take(&mut reliable, client, &mut client_maps), header + 1 + TILE * TILE);
        assert_eq!(client_maps.get(&0), host.maps.maps.get(&0));
        // A map the client no longer carries is not sent.
        remote.inventory.slots[8] = None;
        host.maps.maps.get_mut(&0).unwrap().set(4, 4, 9);
        send_maps(&mut reliable, client, &host.maps, &mut remote);
        assert_eq!(take(&mut reliable, client, &mut client_maps), 0);
    }
}
