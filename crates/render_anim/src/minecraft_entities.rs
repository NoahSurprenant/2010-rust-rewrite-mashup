//! The Minecraft world's mobs, as MinecraftOSS runs them: its integrated
//! server (`engine/viewer/src/server.rs`) ticks the level at 20 Hz on its own
//! thread with natural spawning and the entity world, every passive and
//! hostile mob's AI and pathfinding. Here it is fed the streamed chunks and
//! the player, and hands back the tracked mobs, which are drawn with the
//! viewer's mob renderers. MW2 bullets hurt the mobs with their real damage
//! (a hundred MW2 health to Minecraft's twenty), and mobs hurt the player
//! the other way round.
use glam::{DVec3, Vec3};
use minecraft_terrain::lighting::SkyLight;
use minecraft_terrain::mesh::{Atlas, ChunkMesh};
use minecraft_terrain::scene::{Block, HandcraftedScene, Scene};
use minecraft_terrain::server::{PlayerEdit, ServerHandle, ServerSim, TickInput};
use minecraft_terrain::terrain::TerrainStream;
use minecraft_terrain::client_mobs::{ClientMobs, server_mobs};
use minecraft_terrain::mob_sync::{
    ClientCopy, ClientProjectiles, EventLog, HostMob, MobSpawn, PlayerEffect, PlayerEffects, Projectile, ProjectileKind, WorldEvent,
};
use minecraft_terrain::effect_particles::EffectParticles;
use minecraft_terrain::mesh::ItemVisuals;
use minecraft_terrain::server::{EntitySnapshot, ServerItem};
use minecraftoss_player::inventory::{Inventory, ItemStack};
use minecraftoss_player::items::{ItemEntity, WorldItems};
use minecraftoss_player::loot::LootBook;
use minecraftoss_player::rng::XoroshiroRandom;
use std::collections::{HashMap, HashSet};
use minecraft_terrain::pack::PackStack;
use minecraft_terrain::poof_particles::PoofParticles;
use minecraft_terrain::portal_particles::PortalParticles;
use minecraftoss_entities::tempt::PlayerCandidate;
use minecraftoss_entities::world::{EntityWorld, MobHit, PlayerHitKind};

const TICK_SECONDS: f64 = 1.0 / 20.0;
/// Minecraft health per MW2 health.
const HEALTH_SCALE: f32 = 20.0 / 100.0;
/// The player's id in the entity world.
const PLAYER: u64 = 0;
/// `Player.getEyeHeight` standing.
const EYE_HEIGHT: f32 = 1.62;
/// Chunks the level ticks, spawns and despawns in around each player.
pub(crate) const SIMULATION_DISTANCE: i32 = 8;

pub(crate) struct Entities {
    server: ServerHandle,
    /// The mobs the player tracks, as of the last server tick.
    world: EntityWorld,
    /// The client's copies of them: tracked, interpolated, animated.
    client: ClientMobs,
    poof: PoofParticles,
    portal: PortalParticles,
    items: ItemVisuals,
    clock: f64,
    ticks: u64,
    /// The player's inventory: vanilla slots, stacking and recipes.
    pub(crate) inventory: Inventory,
    /// The selected hotbar slot.
    pub(crate) selected: usize,
    /// Item entities as the client shows them: the server's, and drops the
    /// client spawned that the server has not taken yet.
    pub(crate) world_items: WorldItems,
    server_item_ids: HashSet<u32>,
    /// Client drops handed to the server: the command count that sent each.
    server_handed: HashMap<u32, (u64, ItemEntity)>,
    /// Stacks the server let the player pick up, not yet in the inventory.
    server_picked: Vec<(i32, [f64; 3], String, i32, Option<String>)>,
    server_snapshot: Option<EntitySnapshot>,
    server_handled: u64,
    loot: Option<LootBook>,
    loot_sequences: HashMap<String, XoroshiroRandom>,
    seed: i64,
    /// Sounds the mob world made since the last take: event, block point,
    /// volume, pitch.
    pub(crate) sounds: Vec<(String, DVec3, f32, f32)>,
    random: minecraftoss_player::rng::LegacyRandom,
    /// A multiplayer client's copy: the host runs the level, so no server
    /// tick, chunk or edit goes to this one, and the items on the ground are
    /// the host's.
    pub(crate) remote: bool,
    /// A host's other players who pick items up this tick: key, feet, a copy
    /// of the inventory and the selected slot.
    pub(crate) others: Vec<(u64, [f64; 3], Inventory, usize)>,
    /// What those players picked up, by key.
    pub(crate) others_picked: Vec<(u64, ItemStack)>,
    /// A host's other players as the mobs see them this tick.
    pub(crate) players: Vec<RemotePlayer>,
    /// Mob hits on those players since the last take: client, MW2 damage and
    /// where the mob was (block point).
    pub(crate) remote_hits: Vec<(u32, i32, Option<[f64; 3]>)>,
    /// A host's mobs as its clients are told of them, as of `mob_tick`;
    /// `mobs_fresh` while the host has not sent them yet.
    pub(crate) host_mobs: Vec<HostMob>,
    pub(crate) mob_tick: u32,
    pub(crate) mobs_fresh: bool,
    /// A client's copy of the host's mobs near it.
    copy: ClientCopy,
    /// A client's punches on mobs, for the host.
    pub(crate) pending_attacks: Vec<u64>,
    /// A host's events of the last few ticks, for its clients.
    pub(crate) events: EventLog,
    /// A host's arrows and potions as of `mob_tick`.
    pub(crate) projectiles: Vec<Projectile>,
    arrow_rotations: HashMap<u64, (f32, f32)>,
    /// The players' status effects, on a host.
    pub(crate) player_effects: PlayerEffects,
    /// This player's effects as its host last told, on a client.
    client_effects: Vec<PlayerEffect>,
    /// The arrows and potions this game draws.
    flying: ClientProjectiles,
    particles: EffectParticles,
    /// The world's sounds this game took (event and where, the latest
    /// last), and how many in all.
    heard: std::collections::VecDeque<(String, DVec3)>,
    heard_total: u64,
    /// Natural spawns and despawns, by the key of the player nearest each.
    range: HashMap<u64, RangeStats>,
    /// The living players at the last server tick, by key.
    tick_players: Vec<(u64, DVec3)>,
    /// Chunks the host's level holds.
    pub(crate) level_chunks: usize,
}

/// Where natural spawning put mobs around one player and where mobs went
/// from it, in blocks from that player.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RangeStats {
    pub spawned: u32,
    pub spawn_min: f64,
    pub spawn_max: f64,
    pub spawn_beyond_48: u32,
    pub despawned: u32,
    pub despawn_min: f64,
    pub despawn_max: f64,
}

impl Default for RangeStats {
    fn default() -> Self {
        Self { spawned: 0, spawn_min: f64::INFINITY, spawn_max: 0.0, spawn_beyond_48: 0, despawned: 0, despawn_min: f64::INFINITY, despawn_max: 0.0 }
    }
}

/// How many world sounds a game keeps for the console.
const HEARD_KEPT: usize = 64;

/// One of a host's clients as the mobs see it.
pub(crate) struct RemotePlayer {
    pub client: u32,
    pub view: PlayerView,
}

/// A host client's id in the mob world (the host's own player is 0).
pub(crate) fn remote_key(client: u32) -> u64 {
    u64::from(client) + 1
}

/// What bullets and blasts count as for block loot: vanilla drops nothing
/// from stone or ores broken bare-handed.
const LOOT_TOOL: &str = "minecraft:diamond_pickaxe";
/// TNT's power: a blast drops each block's loot one time in this many.
const BLAST_POWER: f32 = 4.0;

/// The mobs drawn this frame: entity models (cut out, back-face culled,
/// translucent) and their shadows, in `mesh::Vertex`s; and what goes with
/// the particles (held items, puffs, flames, potions).
#[derive(Default)]
pub(crate) struct MobMeshes {
    pub models: ChunkMesh,
    pub culled: ChunkMesh,
    pub translucent: ChunkMesh,
    pub shadows: ChunkMesh,
    pub items: ChunkMesh,
}

/// The player as the mobs see it this frame.
pub(crate) struct PlayerView {
    pub feet: [f64; 3],
    pub alive: bool,
    /// MW2 health, 0..100.
    pub health: f32,
    pub yaw: f32,
    pub pitch: f32,
}

impl Entities {
    pub(crate) fn new(stream: &TerrainStream, seed: i64) -> Self {
        let sim = ServerSim::new(stream.world_gen(), stream.states.clone(), "minecraft:overworld");
        let mut server = ServerHandle::spawn(sim);
        // Mob and block loot and the recipes (stack sizes), from the game's
        // data JAR when MinecraftOSS has one.
        let jar = data_jar();
        let mut inventory = Inventory::default();
        let mut loot = None;
        if let Some(jar) = jar {
            server.load_loot(jar.clone(), seed as u64);
            match minecraftoss_player::crafting::RecipeBook::from_jar(&jar) {
                Ok(recipes) => {
                    let recipes = std::sync::Arc::new(recipes);
                    server.set_recipe_book(recipes.clone());
                    inventory.recipes = recipes;
                }
                Err(error) => diag::warn!(World, "Minecraft recipes unavailable: {error:#}"),
            }
            match LootBook::from_jar(&jar) {
                Ok(book) => loot = Some(book),
                Err(error) => diag::warn!(World, "Minecraft block loot unavailable: {error:#}"),
            }
        } else {
            diag::warn!(World, "Minecraft data JAR not found: no loot or recipes");
        }
        Self {
            server,
            world: EntityWorld::default(),
            client: ClientMobs::default(),
            poof: PoofParticles::default(),
            portal: PortalParticles::default(),
            items: ItemVisuals::default(),
            clock: 0.0,
            ticks: 0,
            inventory,
            selected: 0,
            world_items: WorldItems::default(),
            server_item_ids: HashSet::new(),
            server_handed: HashMap::new(),
            server_picked: Vec::new(),
            server_snapshot: None,
            server_handled: 0,
            loot,
            loot_sequences: HashMap::new(),
            seed,
            sounds: Vec::new(),
            random: minecraftoss_player::rng::LegacyRandom::new((seed ^ 0x1735) as u64),
            remote: false,
            others: Vec::new(),
            others_picked: Vec::new(),
            players: Vec::new(),
            remote_hits: Vec::new(),
            host_mobs: Vec::new(),
            mob_tick: 0,
            mobs_fresh: false,
            copy: ClientCopy::default(),
            pending_attacks: Vec::new(),
            events: EventLog::default(),
            projectiles: Vec::new(),
            arrow_rotations: HashMap::new(),
            player_effects: PlayerEffects::default(),
            client_effects: Vec::new(),
            flying: ClientProjectiles::default(),
            particles: EffectParticles::default(),
            heard: std::collections::VecDeque::new(),
            heard_total: 0,
            range: HashMap::new(),
            tick_players: Vec::new(),
            level_chunks: 0,
        }
    }

    /// Something that happened in the level, heard and seen here: a sound,
    /// a potion's splash (its particles and `entity.splash_potion.break`) or
    /// a blast (its puffs and `ServerExplosion`'s loud, low sound).
    fn world_event(&mut self, event: WorldEvent) {
        match event {
            WorldEvent::Sound { event, position, volume, pitch } => {
                let at = DVec3::from_array(position);
                self.heard_total += 1;
                self.heard.push_back((event.clone(), at));
                if self.heard.len() > HEARD_KEPT {
                    self.heard.pop_front();
                }
                self.sounds.push((event, at, volume, pitch));
            }
            WorldEvent::Splash { position, color, instant } => {
                let at = DVec3::from_array(position);
                self.particles.splash(at, color, instant);
                let pitch = self.random.next_float() * 0.1 + 0.9;
                self.sounds.push(("minecraft:entity.splash_potion.break".to_owned(), at, 1.0, pitch));
            }
            WorldEvent::Explosion { position, radius } => {
                let at = DVec3::from_array(position);
                self.particles.explosion(at, radius);
                let pitch = (1.0 + (self.random.next_float() - self.random.next_float()) * 0.2) * 0.7;
                self.sounds.push(("minecraft:entity.generic.explode".to_owned(), at, 4.0, pitch));
            }
        }
    }

    /// The world sounds this game took, the latest last, and how many in all.
    pub(crate) fn heard(&self) -> (impl Iterator<Item = &(String, DVec3)>, u64) {
        (self.heard.iter(), self.heard_total)
    }

    /// The local player's status effects: its own on a host, as told on a
    /// client.
    pub(crate) fn effects(&self) -> Vec<PlayerEffect> {
        if self.remote { self.client_effects.clone() } else { self.player_effects.of(PLAYER) }
    }

    /// How many arrows and potions of each kind this game has drawn.
    pub(crate) fn projectiles_seen(&self) -> Vec<(&'static str, usize)> {
        self.flying.seen()
    }

    /// Natural spawns and despawns around the player with `key`.
    pub(crate) fn range_stats(&self, key: u64) -> RangeStats {
        self.range.get(&key).copied().unwrap_or_default()
    }

    /// Each tracked mob's feet and whether it goes far from every player
    /// (`removeWhenFarAway`, not persistent: the monsters and bats).
    pub(crate) fn tracked_census(&self) -> Vec<(DVec3, bool)> {
        const STAY: [&str; 8] = [
            "minecraft:iron_golem",
            "minecraft:wolf",
            "minecraft:villager",
            "minecraft:cow",
            "minecraft:mooshroom",
            "minecraft:sheep",
            "minecraft:pig",
            "minecraft:chicken",
        ];
        self.world.census().into_iter().map(|m| (m.position, !m.persistent && !STAY.contains(&m.kind))).collect()
    }

    /// The living players of the last server tick: key and feet.
    pub(crate) fn tick_players(&self) -> &[(u64, DVec3)] {
        &self.tick_players
    }

    /// A player's movement speed as a share of its own, by its effects.
    pub(crate) fn speed_of(&self, key: u64) -> f32 {
        self.player_effects.speed(key)
    }

    /// Magic damage (Minecraft health) on the player with `key`: the host's
    /// own into `hits`, a client's for its host to deal.
    fn hurt_player(&mut self, key: u64, amount: f32, from: Option<[f64; 3]>, hits: &mut Vec<(i32, Option<[f64; 3]>)>) {
        let damage = (amount / HEALTH_SCALE).round() as i32;
        if key == PLAYER {
            hits.push((damage, from));
        } else if let Some(client) = key.checked_sub(1).and_then(|c| u32::try_from(c).ok()) {
            self.remote_hits.push((client, damage, from));
        }
    }

    /// Spawns or despawns at these feet count for the player nearest them.
    fn note_range(&mut self, at: [f64; 3], spawned: bool) {
        let at = DVec3::from_array(at);
        let Some((key, distance)) =
            self.tick_players.iter().map(|(key, feet)| (*key, feet.distance(at))).min_by(|a, b| a.1.total_cmp(&b.1))
        else {
            return;
        };
        let stats = self.range.entry(key).or_default();
        if spawned {
            stats.spawned += 1;
            stats.spawn_min = stats.spawn_min.min(distance);
            stats.spawn_max = stats.spawn_max.max(distance);
            stats.spawn_beyond_48 += u32::from(distance > 48.0);
        } else {
            stats.despawned += 1;
            stats.despawn_min = stats.despawn_min.min(distance);
            stats.despawn_max = stats.despawn_max.max(distance);
        }
    }

    pub(crate) fn load_chunk(&mut self, chunk: &std::sync::Arc<minecraftoss_core::Chunk>) {
        if self.remote {
            return;
        }
        self.server.load_chunk(chunk);
    }

    pub(crate) fn unload_chunk(&mut self, pos: minecraftoss_core::ChunkPos) {
        if self.remote {
            return;
        }
        self.server.unload_chunk(pos);
    }

    /// Block loot for what the weapons broke, dropped as vanilla drops it
    /// (`Block.popResource`); a blast keeps each drop one time in four.
    pub(crate) fn drop_blocks(&mut self, broken: &[((i32, i32, i32), Block, bool)]) {
        if self.remote {
            return;
        }
        let Some(loot) = self.loot.as_ref() else {
            return;
        };
        let tool = ItemStack::new(LOOT_TOOL, 1);
        for (pos, block, blast) in broken {
            let block = minecraftoss_player::Block { id: block.id.key(), properties: block.properties.clone() };
            let Some(drops) = loot.roll_drops_named(&block, Some(&tool), self.seed as u64, &mut self.loot_sequences) else {
                continue;
            };
            for mut drop in drops {
                if *blast && self.random.next_float() >= 1.0 / BLAST_POWER {
                    continue;
                }
                if drop.components.is_none() {
                    drop.max = drop.max.min(self.inventory.recipes.max_stack(&drop.id));
                }
                self.world_items.spawn_block_drop(drop, *pos);
            }
        }
    }

    /// `ItemEntity`s to draw and pick up, as the viewer's
    /// `server_items_tick` does for a server-simulated world: client drops
    /// go to the server, stacks the server offered go into the inventory,
    /// and the server's items are mirrored for drawing.
    fn server_items_tick(&mut self, feet: DVec3) {
        let entities = std::mem::take(&mut self.world_items.entities);
        let to_hand: Vec<ItemEntity> = entities
            .iter()
            .filter(|e| !self.server_item_ids.contains(&e.entity_id) && !self.server_handed.contains_key(&e.entity_id))
            .cloned()
            .collect();
        for entity in &to_hand {
            let components = entity.stack.components.as_ref().map(|c| c.to_string());
            self.server.spawn_item(
                &entity.stack.id,
                i32::from(entity.stack.count),
                components.as_deref(),
                entity.position.to_array(),
                entity.velocity.to_array(),
                i32::from(entity.pickup_delay),
                entity.age as i32,
            );
            self.server_handed.insert(entity.entity_id, (self.server.sent(), entity.clone()));
        }
        let previous: HashMap<u32, ItemEntity> = entities.into_iter().map(|e| (e.entity_id, e)).collect();
        let target = feet + DVec3::Y * 0.81;
        self.world_items.tick_pickup_effects(target);
        for (id, position, item, count, components) in std::mem::take(&mut self.server_picked) {
            let recipes = self.inventory.recipes.clone();
            let make_stack = |item: &str, count: i32| {
                let mut stack = ItemStack::new(item, count.clamp(0, 255) as u8);
                stack.components = components.as_deref().and_then(|c| serde_json::from_str(c).ok());
                if stack.components.is_none() {
                    stack.max = stack.max.min(recipes.max_stack(item));
                }
                stack
            };
            let taken = match self.inventory.add_item(make_stack(&item, count), self.selected) {
                None => count,
                Some(rest) => {
                    let rest_count = i32::from(rest.count);
                    self.server.spawn_item(&item, rest_count, components.as_deref(), feet.to_array(), [0.0; 3], 0, 0);
                    count - rest_count
                }
            };
            if taken <= 0 {
                continue;
            }
            let transfer = make_stack(&item, taken);
            let snapshot = previous.get(&(id as u32)).cloned().unwrap_or_else(|| ItemEntity {
                entity_id: id as u32,
                stack: transfer.clone(),
                position: DVec3::from_array(position),
                previous_position: DVec3::from_array(position),
                velocity: DVec3::ZERO,
                age: 0,
                bob_offset: bob_offset(id),
                pickup_delay: 0,
                on_ground: true,
            });
            self.world_items.note_pickup(snapshot, target, transfer);
        }
        let Some(snapshot) = self.server_snapshot.take() else {
            self.world_items.entities = previous.into_values().collect();
            self.world_items.entities.sort_by_key(|e| e.entity_id);
            return;
        };
        let recipes = self.inventory.recipes.clone();
        self.world_items.entities = snapshot
            .items
            .into_iter()
            .map(|item: ServerItem| {
                let mut stack = ItemStack::new(&item.item, item.count.clamp(0, 255) as u8);
                stack.components = item.components.as_deref().and_then(|c| serde_json::from_str(c).ok());
                if stack.components.is_none() {
                    stack.max = stack.max.min(recipes.max_stack(&item.item));
                }
                ItemEntity {
                    entity_id: item.id as u32,
                    stack,
                    position: DVec3::from_array(item.position),
                    previous_position: DVec3::from_array(item.previous_position),
                    velocity: DVec3::from_array(item.velocity),
                    age: item.age.max(0) as u32,
                    bob_offset: bob_offset(item.id),
                    pickup_delay: item.pickup_delay.clamp(0, i32::from(u16::MAX)) as u16,
                    on_ground: item.on_ground,
                }
            })
            .collect();
        self.server_item_ids = self.world_items.entities.iter().map(|e| e.entity_id).collect();
        let handled = self.server_handled;
        self.server_handed.retain(|_, (sent, _)| *sent > handled);
        for (_, entity) in self.server_handed.values() {
            self.world_items.entities.push(entity.clone());
        }
    }

    /// A held item's model under a view-space pose, lit at `light_at`.
    pub(crate) fn held_item_mesh(
        &mut self,
        id: &str,
        pose: glam::Mat4,
        light_at: glam::Vec3,
        packs: &PackStack,
        atlas: &Atlas,
        light: &SkyLight,
    ) -> ChunkMesh {
        let mut mesh = ChunkMesh::default();
        let _ = self.items.append_posed_blocks(&mut mesh, &[(pose, light_at, id.to_owned())], packs, atlas, light);
        mesh
    }

    /// A fist on the mob the player looks at within reach (one damage, as
    /// an empty hand deals).
    pub(crate) fn punch(&mut self, eye: DVec3, look: DVec3, yaw: f32) -> bool {
        let Some((hit, _)) = self.world.mob_on_ray(eye, look, 3.0) else {
            return false;
        };
        // A client's punch is the host's to land.
        if self.remote {
            self.pending_attacks.push(encode(hit));
            return true;
        }
        let attack = minecraftoss_entities::world::PlayerAttack {
            player_id: PLAYER,
            position: eye - DVec3::Y * f64::from(EYE_HEIGHT),
            yaw,
            attack_damage: (1.0 + self.player_effects.attack_damage(PLAYER)).max(0.0),
            strength: 1.0,
            sprinting: false,
            can_critical: false,
            can_sweep: false,
        };
        self.server.mob_action(hit, Some(attack), &self.inventory.clone(), self.selected, false);
        true
    }

    /// A block the player placed, for the level the mobs walk in.
    pub(crate) fn placed(&mut self, scene: &HandcraftedScene, pos: (i32, i32, i32)) {
        if self.remote {
            return;
        }
        self.server.player_edit(scene, pos, PlayerEdit::Place);
    }

    /// Client ticks run so far.
    pub(crate) fn client_ticks(&self) -> u64 {
        self.ticks
    }

    /// Blocks the player's weapons broke, for the level the mobs walk in.
    pub(crate) fn broke(&mut self, scene: &HandcraftedScene, positions: &[(i32, i32, i32)]) {
        if self.remote {
            return;
        }
        for &pos in positions {
            self.server.player_edit(scene, pos, PlayerEdit::Break);
        }
    }

    /// A bullet on a mob: an attack with the bullet's damage from where it
    /// was fired.
    pub(crate) fn shoot(&mut self, key: u64, damage: f32, from: [f64; 3], yaw: f32, shooter: Option<u32>) {
        let Some(hit) = decode(key) else {
            return;
        };
        let attack = minecraftoss_entities::world::PlayerAttack {
            player_id: shooter.map_or(PLAYER, remote_key),
            position: DVec3::from_array(from),
            yaw,
            attack_damage: f64::from(damage * HEALTH_SCALE),
            strength: 1.0,
            sprinting: false,
            can_critical: false,
            can_sweep: false,
        };
        self.server
            .mob_action(hit, Some(attack), &minecraftoss_player::inventory::Inventory::default(), 0, false);
    }

    /// A host client's punch on the mob with `key`: an empty hand's one
    /// damage, when the mob is within its reach (with room for the time the
    /// punch took to arrive). False when it is not.
    pub(crate) fn attack(&mut self, key: u64, client: u32, feet: [f64; 3], yaw: f32) -> bool {
        let Some(hit) = decode(key) else {
            return false;
        };
        let reach = self.boxes().into_iter().any(|(k, b)| {
            let centre = [(b[0] + b[3]) * 0.5, (b[1] + b[4]) * 0.5, (b[2] + b[5]) * 0.5];
            k == key && (0..3).map(|i| (centre[i] - feet[i]).powi(2)).sum::<f64>() < 6.0 * 6.0
        });
        if !reach {
            return false;
        }
        let attack = minecraftoss_entities::world::PlayerAttack {
            player_id: remote_key(client),
            position: DVec3::from_array(feet),
            yaw,
            attack_damage: (1.0 + self.player_effects.attack_damage(remote_key(client))).max(0.0),
            strength: 1.0,
            sprinting: false,
            can_critical: false,
            can_sweep: false,
        };
        self.server.mob_action(hit, Some(attack), &Inventory::default(), 0, false);
        true
    }

    /// The console's `mc summon`: a mob of `kind` at a block point.
    pub(crate) fn summon(&mut self, kind: &str, at: [f64; 3]) {
        if self.remote {
            return;
        }
        let kind = if kind.contains(':') { kind.to_owned() } else { format!("minecraft:{kind}") };
        self.server.summon(kind, at, None);
    }

    /// Mobs that came into this client's view.
    pub(crate) fn mob_spawns(&mut self, spawns: &[MobSpawn]) {
        for spawn in spawns {
            self.copy.spawn(&mut self.world, spawn);
        }
    }

    /// Mobs that left this client's view.
    pub(crate) fn mobs_gone(&mut self, ids: &[u64]) {
        self.copy.gone(&mut self.world, ids);
    }

    /// A snapshot's mob states for this client's copy, which its trackers
    /// then take as they take a server tick's.
    pub(crate) fn mob_frame(&mut self, bytes: &[u8]) {
        match minecraft_terrain::mob_sync::decode_frame(bytes) {
            Ok(frame) => {
                if self.copy.apply(&mut self.world, frame.tick, &frame.states) {
                    for (feet, width, height) in self.client.receive(server_mobs(&self.world)) {
                        self.poof.spawn(feet, width, height);
                    }
                    for event in self.copy.fresh_events(&frame.extras.events) {
                        self.world_event(event);
                    }
                    self.flying.receive(&frame.extras.projectiles);
                    self.client_effects = frame.extras.effects;
                }
            }
            Err(error) => diag::warn!(World, "Minecraft mob states dropped: {error}"),
        }
    }

    /// Every mob this game has: id, kind, feet as drawn, health and whether
    /// it is dead (the console's `mc mobs`).
    pub(crate) fn census(&self) -> Vec<(u64, &'static str, [f64; 3], f32, bool)> {
        minecraft_terrain::mob_sync::census(&self.world)
            .into_iter()
            .map(|(id, kind, feet, health, dead)| {
                let shown = self.client.get(id).map_or(feet, |mob| mob.position.to_array());
                (id, kind, shown, health, dead)
            })
            .collect()
    }

    /// Sends a server tick when one is due and takes what came back: block
    /// changes to show, and mob hits on the player in MW2 damage.
    pub(crate) fn tick(
        &mut self,
        dt: f64,
        day_ticks: i64,
        bright_outside: bool,
        player: &PlayerView,
    ) -> (Vec<((i32, i32, i32), Option<Block>)>, Vec<(i32, Option<[f64; 3]>)>) {
        self.clock += dt;
        let ticked = self.clock >= TICK_SECONDS;
        let mut hits = Vec::new();
        if ticked {
            self.clock = (self.clock - TICK_SECONDS).min(TICK_SECONDS);
            self.ticks += 1;
            // Packets first, then the client level's entity ticks.
            self.client.tick();
            for enderman in self.world.endermen() {
                if let Some(mob) = self.client.get(enderman.id) {
                    self.portal.emit_enderman(mob.position, 0.6, 2.9);
                }
            }
            self.portal.tick();
            if self.ticks % 600 == 0 {
                diag::info!(World, "Minecraft mobs: {} tracked", self.boxes().len());
            }
            if self.remote {
                self.world_items.tick_pickup_effects(DVec3::from_array(player.feet) + DVec3::Y * 0.81);
                return (Vec::new(), Vec::new());
            }
            let feet = DVec3::from_array(player.feet);
            let center = ((feet.x.floor() as i32) >> 4, (feet.z.floor() as i32) >> 4);
            let candidate = |id: u64, view: &PlayerView| PlayerCandidate {
                id,
                position: DVec3::from_array(view.feet),
                eye_height: EYE_HEIGHT,
                main_hand_cow_food: false,
                offhand_cow_food: false,
                main_hand_pig_food: false,
                offhand_pig_food: false,
                main_hand_chicken_food: false,
                offhand_chicken_food: false,
                main_hand_carrot_on_a_stick: false,
                offhand_carrot_on_a_stick: false,
                main_hand_wolf_interest: false,
                offhand_wolf_interest: false,
                main_hand_horse_tempt: false,
                offhand_horse_tempt: false,
                alive: view.alive,
                spectator: !view.alive,
                attackable: view.alive,
            };
            let look = |view: &PlayerView| minecraftoss_entities::enderman::PlayerView {
                head_yaw: view.yaw,
                pitch: view.pitch,
                disguised: false,
            };
            // The players' effects: a death takes them, and poison bites.
            if !player.alive {
                self.player_effects.clear(PLAYER);
            }
            for remote in self.players.iter().filter(|r| !r.view.alive) {
                self.player_effects.clear(remote_key(remote.client));
            }
            let health_of = |key: u64| {
                let view = if key == PLAYER {
                    Some(player)
                } else {
                    self.players.iter().find(|r| remote_key(r.client) == key).map(|r| &r.view)
                };
                view.map_or(0.0, |view| view.health * HEALTH_SCALE)
            };
            for (key, amount) in self.player_effects.tick(health_of) {
                self.hurt_player(key, amount, None, &mut hits);
            }
            let effects = &self.player_effects;
            let vitals = |key: u64, view: &PlayerView| {
                let (slowed, poisoned, weakened) = effects.marks(key);
                minecraftoss_entities::monster_ai::PlayerVitals {
                    health: view.health * HEALTH_SCALE,
                    slowed,
                    poisoned,
                    weakened,
                    ..Default::default()
                }
            };
            // The host's own player, then its clients: the level ticks,
            // spawns and despawns around each, and its mobs hunt each.
            let mut players = if player.alive { vec![player.feet] } else { Vec::new() };
            let mut centers = vec![center];
            let mut tracking = vec![player.feet];
            let mut mob_players = vec![candidate(PLAYER, player)];
            let mut mob_views = vec![(PLAYER, look(player))];
            let mut mob_vitals = vec![(PLAYER, vitals(PLAYER, player))];
            self.tick_players.clear();
            if player.alive {
                self.tick_players.push((PLAYER, DVec3::from_array(player.feet)));
            }
            for remote in &self.players {
                let (key, view) = (remote_key(remote.client), &remote.view);
                if view.alive {
                    players.push(view.feet);
                    self.tick_players.push((key, DVec3::from_array(view.feet)));
                }
                centers.push(((view.feet[0].floor() as i32) >> 4, (view.feet[2].floor() as i32) >> 4));
                tracking.push(view.feet);
                mob_players.push(candidate(key, view));
                mob_views.push((key, look(view)));
                mob_vitals.push((key, vitals(key, view)));
            }
            self.server.tick(TickInput {
                day_ticks,
                players,
                difficulty: 2,
                simulation_centers: centers,
                simulation_distance: SIMULATION_DISTANCE,
                pickup: player
                    .alive
                    .then(|| (player.feet, Box::new(self.inventory.clone()), self.selected)),
                others: std::mem::take(&mut self.others)
                    .into_iter()
                    .map(|(key, feet, inventory, selected)| (key, feet, Box::new(inventory), selected))
                    .collect(),
                mob_players,
                mob_views,
                mob_vitals,
                bright_outside,
                tracking: (tracking, 160.0),
                player_hurts: Vec::new(),
                spawn_mobs: true,
            });
        }
        let mut changes = Vec::new();
        for output in self.server.poll() {
            self.server_handled = output.handled;
            if let Some(snapshot) = output.entities {
                self.server_snapshot = Some(snapshot);
            }
            self.server_picked.extend(output.picked);
            let recipes = self.inventory.recipes.clone();
            for (key, (_, _, item, count, components)) in output.others_picked {
                let mut stack = ItemStack::new(&item, count.clamp(0, 255) as u8);
                stack.components = components.as_deref().and_then(|c| serde_json::from_str(c).ok());
                if stack.components.is_none() {
                    stack.max = stack.max.min(recipes.max_stack(&item));
                }
                self.others_picked.push((key, stack));
            }
            changes.extend(output.changes);
            // What happened in the level: heard and seen here, and told to
            // the clients near it.
            let sounds = output.mob_sounds.iter().map(|s| (&s.event, s.position, s.volume, s.pitch));
            let results = output.mob_results.iter().flat_map(|r| r.sounds.iter()).map(|s| (&s.event, s.position, s.volume, s.pitch));
            let mut happened: Vec<WorldEvent> = sounds
                .chain(results)
                .map(|(event, position, volume, pitch)| WorldEvent::Sound {
                    event: event.clone(),
                    position: position.to_array(),
                    volume,
                    pitch,
                })
                .collect();
            happened.extend(output.potion_breaks.iter().map(|b| WorldEvent::Splash {
                position: b.position.to_array(),
                color: b.color,
                instant: b.instant,
            }));
            happened.extend(output.explosions.iter().map(|blast| WorldEvent::Explosion {
                position: blast.position.to_array(),
                radius: blast.radius,
            }));
            for event in happened {
                self.events.push(self.mob_tick, event.clone());
                self.world_event(event);
            }
            for (key, splash) in &output.player_splashes {
                if let Some(amount) = self.player_effects.splash(*key, splash) {
                    self.hurt_player(*key, amount, Some(splash.at.to_array()), &mut hits);
                }
            }
            for (_, at) in &output.natural_spawns {
                self.note_range(*at, true);
            }
            for (_, at) in &output.despawns {
                self.note_range(*at, false);
            }
            if output.level_chunks > 0 {
                self.level_chunks = output.level_chunks;
            }
            for summoned in &output.summoned {
                match summoned {
                    Ok(kind) => diag::info!(World, "Minecraft summon: {kind}"),
                    Err(error) => diag::warn!(World, "Minecraft summon failed: {error}"),
                }
            }
            if let Some(mobs) = output.mobs {
                self.world = *mobs;
                for (feet, width, height) in self.client.receive(server_mobs(&self.world)) {
                    self.poof.spawn(feet, width, height);
                }
                // What the host's clients are told of them.
                self.mob_tick = self.mob_tick.wrapping_add(1);
                self.events.age(self.mob_tick);
                self.mobs_fresh = true;
                self.host_mobs = if self.players.is_empty() {
                    Vec::new()
                } else {
                    minecraft_terrain::mob_sync::host_mobs(&self.world)
                };
                self.projectiles = minecraft_terrain::mob_sync::host_projectiles(&self.world, &mut self.arrow_rotations);
                self.flying.receive(&self.projectiles);
            }
            for hit in output.player_hits {
                let from = match hit.kind {
                    PlayerHitKind::Melee { attacker, .. } => Some(attacker.to_array()),
                    _ => None,
                };
                // A tipped arrow's effect lands with it.
                if let PlayerHitKind::Arrow { effect: Some((kind, duration)), .. } = hit.kind {
                    self.player_effects.arrow(hit.player_id, kind, duration);
                }
                let damage = (hit.damage / HEALTH_SCALE).round() as i32;
                if hit.player_id == PLAYER {
                    hits.push((damage, from));
                } else if let Some(client) = hit.player_id.checked_sub(1).and_then(|c| u32::try_from(c).ok()) {
                    self.remote_hits.push((client, damage, from));
                }
            }
        }
        if ticked {
            self.server_items_tick(DVec3::from_array(player.feet));
            // `ItemEntity.playerTouch`'s pickup pop.
            for _ in 0..self.world_items.take_pickup_sounds() {
                let pitch = ((self.random.next_float() - self.random.next_float()) * 0.7 + 1.0) * 2.0;
                self.sounds.push(("minecraft:entity.item.pickup".to_owned(), DVec3::from_array(player.feet), 0.2, pitch));
            }
        }
        (changes, hits)
    }

    /// Every living mob's key and box in blocks, for bullets.
    pub(crate) fn boxes(&self) -> Vec<(u64, [f64; 6])> {
        let w = &self.world;
        let mut out = Vec::new();
        let mut add = |hit: MobHit, body: &minecraftoss_entities::movement::Body| {
            let p = body.position;
            let half = f64::from(body.width) * 0.5;
            out.push((
                encode(hit),
                [p.x - half, p.y, p.z - half, p.x + half, p.y + f64::from(body.height), p.z + half],
            ));
        };
        for e in w.bats().iter().filter(|e| e.bat.health > 0.0) {
            add(MobHit::Bat(e.id), &e.bat.body);
        }
        for e in w.zombies().iter().filter(|e| e.zombie.health > 0.0) {
            add(MobHit::Zombie(e.id), &e.zombie.body);
        }
        for e in w.skeletons().iter().filter(|e| e.skeleton.health > 0.0) {
            add(MobHit::Skeleton(e.id), &e.skeleton.body);
        }
        for e in w.creepers().iter().filter(|e| e.creeper.health > 0.0 && !e.creeper.exploded) {
            add(MobHit::Creeper(e.id), &e.creeper.body);
        }
        for e in w.spiders().iter().filter(|e| e.spider.health > 0.0) {
            add(MobHit::Spider(e.id), &e.spider.body);
        }
        for e in w.slimes().iter().filter(|e| e.slime.health > 0.0) {
            add(MobHit::Slime(e.id), &e.slime.body);
        }
        for e in w.endermen().iter().filter(|e| e.enderman.health > 0.0) {
            add(MobHit::Enderman(e.id), &e.enderman.body);
        }
        for e in w.witches().iter().filter(|e| e.witch.health > 0.0) {
            add(MobHit::Witch(e.id), &e.witch.body);
        }
        for e in w.iron_golems().iter().filter(|e| e.golem.health > 0.0) {
            add(MobHit::IronGolem(e.id), &e.golem.body);
        }
        for e in w.wolves().iter().filter(|e| e.wolf.health > 0.0) {
            add(MobHit::Wolf(e.id), &e.wolf.body);
        }
        for e in w.villagers().iter().filter(|e| e.villager.health > 0.0) {
            add(MobHit::Villager(e.id), &e.villager.body);
        }
        for e in w.cows().iter().filter(|e| e.cow.health > 0.0) {
            let hit = if e.mooshroom.is_some() { MobHit::Mooshroom(e.id) } else { MobHit::Cow(e.id) };
            add(hit, &e.cow.body);
        }
        for e in w.sheep().iter().filter(|e| e.health > 0.0) {
            add(MobHit::Sheep(e.id), &e.body);
        }
        for e in w.pigs().iter().filter(|e| e.pig.health > 0.0) {
            add(MobHit::Pig(e.id), &e.pig.body);
        }
        for e in w.chickens().iter().filter(|e| e.chicken.health > 0.0) {
            add(MobHit::Chicken(e.id), &e.chicken.body);
        }
        out
    }

    /// Steps the client-side puffs, which settle on the scene's blocks.
    pub(crate) fn tick_scene(&mut self, scene: &HandcraftedScene, ticks: u32) {
        for _ in 0..ticks {
            self.poof.tick(scene);
            self.particles.tick();
        }
    }

    /// The mobs this frame, drawn with the viewer's renderers.
    pub(crate) fn meshes(
        &mut self,
        scene: &HandcraftedScene,
        packs: &PackStack,
        atlas: &Atlas,
        light: &SkyLight,
        forward: Vec3,
        camera: DVec3,
        sky_darken: u8,
    ) -> MobMeshes {
        use minecraft_terrain::*;
        // Mobs appear on the client once their trackers start.
        self.client.spawn_missing(&server_mobs(&self.world));
        let w = &self.world;
        let poses = &self.client;
        let partial = (self.clock / TICK_SECONDS).clamp(0.0, 1.0) as f32;
        let mut out = MobMeshes::default();
        cow_render::append_cows(&mut out.models, w.cows().iter(), poses, atlas, light, partial);
        sheep_render::append_sheep(&mut out.models, w.sheep().iter(), poses, atlas, light, partial);
        pig_render::append_pigs(&mut out.models, w.pigs().iter(), poses, atlas, light, partial);
        chicken_render::append_chickens(&mut out.models, w.chickens().iter(), poses, atlas, light, partial);
        bat_render::append_bats(&mut out.culled, w.bats().iter(), poses, atlas, light, partial);
        let zombie_items = zombie_render::append_zombies(&mut out.models, w.zombies().iter(), poses, atlas, light, partial);
        creeper_render::append_creepers(&mut out.models, w.creepers().iter(), poses, atlas, light, partial);
        spider_render::append_spiders(&mut out.models, w.spiders().iter(), poses, atlas, light, partial);
        let skeleton_items =
            skeleton_render::append_skeletons(&mut out.models, w.skeletons().iter(), poses, atlas, light, partial);
        let villager_items = villager_render::append_villagers(
            &mut out.models,
            w.villagers().iter(),
            poses,
            &|pos| Scene::block(scene, pos).and_then(|b| b.properties.get("facing").cloned()),
            atlas,
            light,
            partial,
        );
        horse_render::append_horses(&mut out.models, &mut out.translucent, w.cows().iter(), poses, atlas, light, partial);
        slime_render::append_slimes(&mut out.models, &mut out.translucent, w.slimes().iter(), poses, atlas, light, partial);
        let carried = enderman_render::append_endermen(
            &mut out.models,
            w.endermen().iter(),
            poses,
            atlas,
            light,
            partial,
            self.ticks.rotate_left(32) ^ (partial.to_bits() as u64),
        );
        let witch_items = witch_render::append_witches(&mut out.models, w.witches().iter(), poses, atlas, light, partial);
        let poppies = golem_render::append_iron_golems(&mut out.models, w.iron_golems().iter(), poses, atlas, light, partial);
        wolf_render::append_wolves(&mut out.models, w.wolves().iter(), poses, atlas, light, partial, w.game_time());
        flame_render::append_flames(
            &mut out.items,
            w.burning()
                .into_iter()
                .map(|(previous, now, width, height)| (previous.lerp(now, f64::from(partial)), width, height)),
            atlas,
            forward,
            light,
        );
        let flying = self.flying.drawn(f64::from(partial));
        witch_render::append_potions(
            &mut out.items,
            flying.iter().filter_map(|(p, at)| match p.kind {
                ProjectileKind::Potion { color } => Some((color, *at)),
                ProjectileKind::Arrow { .. } => None,
            }),
            atlas,
            forward,
            light,
        );
        arrow_render::append_arrows(
            &mut out.models,
            flying.iter().filter_map(|(p, at)| match p.kind {
                ProjectileKind::Arrow { tipped } => Some((*at, p.y_rot, p.x_rot, tipped)),
                ProjectileKind::Potion { .. } => None,
            }),
            atlas,
            light,
        );
        self.particles.append_mesh(&mut out.items, atlas, forward, partial, light);
        if let Ok(items) = self.items.mesh(&self.world_items, packs, atlas, light, partial) {
            let base = out.items.vertices.len() as u32;
            out.items.vertices.extend(items.vertices);
            out.items.indices.extend(items.indices.iter().map(|i| i + base));
        }
        self.poof.append_mesh(&mut out.items, atlas, forward, partial, light);
        self.portal.append_mesh(&mut out.items, atlas, forward, partial, light);
        let held: Vec<_> =
            skeleton_items.into_iter().chain(zombie_items).chain(villager_items).chain(witch_items).collect();
        let _ = self.items.append_held_items(&mut out.items, &held, packs, atlas, light);
        let carried: Vec<_> = carried.into_iter().chain(poppies).collect();
        let _ = self.items.append_posed_blocks(&mut out.items, &carried, packs, atlas, light);
        // Their shadows, at `getMaxLocalRawBrightness`.
        let casters = client_mobs::shadow_casters(w, poses, camera, partial);
        let raw = |pos: (i32, i32, i32)| light.get(pos).saturating_sub(sky_darken).max(light.get_block(pos));
        if let Ok(shadows) = mesh::entity_shadows(&casters, scene, atlas, &raw, 0.0) {
            out.shadows = shadows;
        }
        out
    }
}

/// The viewer's `server_bob_offset`: a server item's bob phase from its id.
fn bob_offset(id: i32) -> f32 {
    let hash = (id as u32).wrapping_mul(0x9e37_79b9).rotate_left(13).wrapping_mul(0x85eb_ca6b);
    hash as f32 / u32::MAX as f32 * std::f32::consts::TAU
}

/// The game's JAR with its loot tables and recipes.
pub(crate) fn data_jar() -> Option<std::path::PathBuf> {
    let root = assets::minecraft_map::root()?;
    // The client JAR fetched from Mojang, or the one a checkout's harness got.
    let fetched = root.join(assets::minecraft_setup::CLIENT_JAR);
    if fetched.is_file() {
        return Some(fetched);
    }
    let root = root.join("harness/.gradle/loom-cache/minecraftMaven/net/minecraft");
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("minecraft-common-") {
            continue;
        }
        for jar in std::fs::read_dir(entry.path().join("26.3")).ok()?.flatten() {
            if jar.path().extension().is_some_and(|ext| ext == "jar") {
                return Some(jar.path());
            }
        }
    }
    None
}

/// A mob hit as one number: the kind in the top byte, the id below.
fn encode(hit: MobHit) -> u64 {
    let kind: u64 = match hit {
        MobHit::Bat(_) => 0,
        MobHit::Zombie(_) => 1,
        MobHit::Skeleton(_) => 2,
        MobHit::Creeper(_) => 3,
        MobHit::Spider(_) => 4,
        MobHit::Slime(_) => 5,
        MobHit::Enderman(_) => 6,
        MobHit::Witch(_) => 7,
        MobHit::IronGolem(_) => 8,
        MobHit::Wolf(_) => 9,
        MobHit::Villager(_) => 10,
        MobHit::Cow(_) => 11,
        MobHit::Mooshroom(_) => 12,
        MobHit::Sheep(_) => 13,
        MobHit::Pig(_) => 14,
        MobHit::Chicken(_) => 15,
    };
    (kind << 56) | (hit.id() & ((1 << 56) - 1))
}

fn decode(key: u64) -> Option<MobHit> {
    let id = key & ((1 << 56) - 1);
    Some(match key >> 56 {
        0 => MobHit::Bat(id),
        1 => MobHit::Zombie(id),
        2 => MobHit::Skeleton(id),
        3 => MobHit::Creeper(id),
        4 => MobHit::Spider(id),
        5 => MobHit::Slime(id),
        6 => MobHit::Enderman(id),
        7 => MobHit::Witch(id),
        8 => MobHit::IronGolem(id),
        9 => MobHit::Wolf(id),
        10 => MobHit::Villager(id),
        11 => MobHit::Cow(id),
        12 => MobHit::Mooshroom(id),
        13 => MobHit::Sheep(id),
        14 => MobHit::Pig(id),
        15 => MobHit::Chicken(id),
        _ => return None,
    })
}
