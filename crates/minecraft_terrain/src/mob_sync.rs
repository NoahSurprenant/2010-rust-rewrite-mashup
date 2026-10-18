//! The mobs in multiplayer: what a host tells each client of the mobs near
//! it, and the client's copy of them.
//!
//! The host's level runs every mob. For each client it picks the mobs near
//! that client (the nearest `MAX_MOBS_PER_CLIENT` within `MOB_RANGE` blocks)
//! and sends:
//!
//! - on the reliable stream, a [`MobSpawn`] when a mob comes into the
//!   client's view or its looks change (kind, variant, held items, size),
//!   and a "gone" when it leaves the view or the level;
//! - with every snapshot, unreliably, the [`MobState`] of each mob it was
//!   shown: position, rotations, ground contact, hurts, swings, health and
//!   the few animation values its renderer reads (a creeper's swell, a
//!   wolf's shake). A lost snapshot costs nothing: the next one carries the
//!   whole state again;
//! - with the same snapshots, what happened near it once ([`WorldEvent`]:
//!   the mobs' sounds, potions breaking, blasts), each numbered and sent
//!   with a few snapshots in a row so that one lost snapshot loses none,
//!   the arrows and potions in flight near it ([`Projectile`]), and its own
//!   status effects ([`PlayerEffect`]).
//!
//! The client keeps an entity world of its own that never ticks, built from
//! the spawns with the host's ids and set from each state, and runs it
//! through the same client trackers and renderers the host draws its mobs
//! with.
use glam::DVec3;
use minecraftoss_entities::effects::{EffectInstance, EffectWork, MobEffect, MobEffects};
use minecraftoss_entities::health::DamageState;
use minecraftoss_entities::look::LookControl;
use minecraftoss_entities::movement::Body;
use minecraftoss_entities::world::EntityWorld;

use crate::multiplayer::{DecodeError, Reader, Writer};
use crate::scene::BlockPos;

/// How far from a client, horizontally in blocks, it is shown mobs.
pub const MOB_RANGE: f64 = 96.0;
/// The most mobs one client is shown at once (the nearest).
pub const MAX_MOBS_PER_CLIENT: usize = 48;
/// The most bytes the mob states of one snapshot take.
pub const MOB_FRAME_BYTES: usize = 4 * 1024;
/// The most bytes the spawns of one reliable row take (a row has to fit one
/// message on the master's control stream with room to spare).
pub const SPAWN_BYTES_PER_MESSAGE: usize = 3 * 1024;
/// Mob ids in one "gone" row.
pub const GONE_PER_MESSAGE: usize = 256;
/// How far from a client, horizontally in blocks, it is shown arrows and
/// potions (`clientTrackingRange`: 4 chunks).
pub const PROJECTILE_RANGE: f64 = 64.0;
/// How far from a client a potion breaking or a blast reaches it.
pub const BURST_RANGE: f64 = 64.0;
/// Snapshots in a row each event goes out with.
pub const EVENT_SENDS: u8 = 3;
/// Host ticks the host keeps an event for clients it has not sent it yet.
pub const EVENT_KEEP_TICKS: u32 = 40;
/// The most events and projectiles one snapshot carries.
pub const MAX_EVENTS_PER_FRAME: usize = 20;
pub const MAX_PROJECTILES_PER_FRAME: usize = 24;
/// Bytes of a snapshot's mob section kept for the events, projectiles and
/// effects (room for the most of each); the states take the rest.
const EXTRAS_BYTES: usize = 2048;
const FRAME_VERSION: u8 = 2;
/// Where a client's copy puts a mob to take it out of its world.
const GONE_Y: f64 = -1.0e9;

/// How a mob looks: everything its renderer reads that changes seldom.
#[derive(Clone, Debug, PartialEq)]
pub enum Look {
    Cow {
        variant: u8,
    },
    Mooshroom {
        brown: bool,
    },
    Horse {
        donkey: bool,
        variant: i32,
        chest: bool,
    },
    Sheep {
        wool: u8,
    },
    Pig {
        variant: u8,
        saddled: bool,
    },
    Chicken {
        variant: u8,
    },
    Bat,
    Zombie {
        kind: u8,
        main_hand: Option<String>,
        armor: [Option<String>; 4],
        villager: Option<(String, String)>,
    },
    Skeleton {
        kind: u8,
        holds_bow: bool,
        sheared: bool,
        armor: [Option<String>; 4],
    },
    Creeper {
        max_swell: i32,
    },
    Spider,
    Slime {
        size: i32,
    },
    Enderman {
        carried: Option<(String, Vec<(String, String)>)>,
    },
    Witch {
        drinking: Option<u8>,
    },
    IronGolem,
    Wolf {
        variant: String,
        tame: bool,
        collar: u8,
        max_health: f32,
    },
    Villager {
        kind: String,
        profession: String,
        level: i32,
        sleeping: Option<BlockPos>,
        held: Option<String>,
    },
}

impl Look {
    /// The mob's kind as `/summon` names it.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Cow { .. } => "cow",
            Self::Mooshroom { .. } => "mooshroom",
            Self::Horse { donkey: true, .. } => "donkey",
            Self::Horse { .. } => "horse",
            Self::Sheep { .. } => "sheep",
            Self::Pig { .. } => "pig",
            Self::Chicken { .. } => "chicken",
            Self::Bat => "bat",
            Self::Zombie { kind, .. } => match kind {
                1 => "drowned",
                2 => "husk",
                3 => "zombie_villager",
                _ => "zombie",
            },
            Self::Skeleton { kind, .. } => match kind {
                1 => "stray",
                2 => "bogged",
                3 => "parched",
                _ => "skeleton",
            },
            Self::Creeper { .. } => "creeper",
            Self::Spider => "spider",
            Self::Slime { .. } => "slime",
            Self::Enderman { .. } => "enderman",
            Self::Witch { .. } => "witch",
            Self::IronGolem => "iron_golem",
            Self::Wolf { .. } => "wolf",
            Self::Villager { .. } => "villager",
        }
    }
}

/// A mob coming into a client's view: its id (the host's), how it looks,
/// whether it is a baby, its box, and where it stands.
#[derive(Clone, Debug, PartialEq)]
pub struct MobSpawn {
    pub id: u64,
    pub look: Look,
    pub baby: bool,
    pub width: f32,
    pub height: f32,
    pub position: [f64; 3],
    pub yaw: f32,
}

impl MobSpawn {
    /// Whether the client's copy of `other` would look the same as this.
    pub fn same_look(&self, other: &MobSpawn) -> bool {
        self.look == other.look
            && self.baby == other.baby
            && self.width == other.width
            && self.height == other.height
    }
}

/// The few animation values a renderer reads that change every tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Extra {
    #[default]
    None,
    /// A zombie's raised arms, a skeleton's drawn bow.
    Aggressive(bool),
    Creeper {
        swell: u8,
        exploded: bool,
    },
    Enderman {
        creepy: bool,
    },
    Golem {
        attack: u8,
        flower: u16,
    },
    Wolf {
        angry: bool,
        sitting: bool,
        wet: bool,
        shake: f32,
        interested: f32,
    },
    Sheep {
        eat: u8,
    },
    Slime {
        squish: f32,
    },
    Villager {
        unhappy: u8,
    },
    Horse {
        eat: f32,
        stand: f32,
        mouth: f32,
        tail: u8,
    },
    Bat {
        resting: bool,
    },
}

/// One mob as of a host tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MobState {
    pub id: u64,
    pub position: [f64; 3],
    /// `getYRot`, `getXRot` and `getYHeadRot`.
    pub y_rot: f32,
    pub x_rot: f32,
    pub y_head_rot: f32,
    pub on_ground: bool,
    pub needs_sync: bool,
    pub burning: bool,
    pub dead: bool,
    /// Hits taken and swings made so far (they travel as their low byte).
    pub hurts: u32,
    pub swings: u32,
    pub health: f32,
    pub extra: Extra,
}

/// A mob of the host's level, as its clients are told of it.
#[derive(Clone, Debug, PartialEq)]
pub struct HostMob {
    pub spawn: MobSpawn,
    pub state: MobState,
}

/// Something that happens once in the host's level, for the clients near it
/// to hear or see.
#[derive(Clone, Debug, PartialEq)]
pub enum WorldEvent {
    /// A sound a mob made, or a player's hit on one: its event, where, its
    /// volume and pitch.
    Sound {
        event: String,
        position: [f64; 3],
        volume: f32,
        pitch: f32,
    },
    /// A splash potion broke (`LevelEvent` 2002, or 2007 for an instant
    /// one) in its colour.
    Splash {
        position: [f64; 3],
        color: u32,
        instant: bool,
    },
    /// A creeper's blast (`ClientboundExplodePacket`).
    Explosion { position: [f64; 3], radius: f32 },
}

impl WorldEvent {
    pub fn position(&self) -> [f64; 3] {
        match self {
            Self::Sound { position, .. }
            | Self::Splash { position, .. }
            | Self::Explosion { position, .. } => *position,
        }
    }

    /// How far from it a player is told of it: a sound's range
    /// (`SoundEvent.getRange`: 16 blocks, more for a louder one), or
    /// `BURST_RANGE`.
    pub fn range(&self) -> f64 {
        match self {
            Self::Sound { volume, .. } => 16.0 * f64::from(volume.max(1.0)),
            _ => BURST_RANGE,
        }
    }

    /// Whether a player at `feet` is told of it.
    pub fn reaches(&self, feet: [f64; 3]) -> bool {
        let at = self.position();
        (0..3).map(|i| (at[i] - feet[i]).powi(2)).sum::<f64>() <= self.range().powi(2)
    }
}

/// An event with the number the host gave it (one count for all clients).
#[derive(Clone, Debug, PartialEq)]
pub struct EventRow {
    pub seq: u32,
    pub event: WorldEvent,
}

/// What flies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProjectileKind {
    /// An arrow; a tipped one carries an effect.
    Arrow { tipped: bool },
    /// A thrown splash potion in its colour.
    Potion { color: u32 },
}

impl ProjectileKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Arrow { .. } => "arrow",
            Self::Potion { .. } => "potion",
        }
    }
}

/// An arrow or potion in flight (or an arrow stuck in a block) as of a host
/// tick, with its yaw and pitch (`Projectile.updateRotation`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projectile {
    pub id: u64,
    pub kind: ProjectileKind,
    pub position: [f64; 3],
    pub y_rot: f32,
    pub x_rot: f32,
}

/// A status effect on a player: the effect, its level and the ticks left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerEffect {
    pub effect: MobEffect,
    pub amplifier: u8,
    pub duration: i32,
}

/// What a snapshot carries for a client besides the mob states.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameExtras {
    pub events: Vec<EventRow>,
    pub projectiles: Vec<Projectile>,
    pub effects: Vec<PlayerEffect>,
}

/// A snapshot's mob section: the host tick, the mob states and the rest.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MobFrame {
    pub tick: u32,
    pub states: Vec<MobState>,
    pub extras: FrameExtras,
}

fn kind_of_zombie(kind: minecraftoss_entities::zombie::ZombieKind) -> u8 {
    use minecraftoss_entities::zombie::ZombieKind::*;
    match kind {
        Zombie => 0,
        Drowned => 1,
        Husk => 2,
        ZombieVillager => 3,
    }
}

fn zombie_of_kind(kind: u8) -> minecraftoss_entities::zombie::ZombieKind {
    use minecraftoss_entities::zombie::ZombieKind::*;
    match kind {
        1 => Drowned,
        2 => Husk,
        3 => ZombieVillager,
        _ => Zombie,
    }
}

fn kind_of_skeleton(kind: minecraftoss_entities::skeleton::SkeletonKind) -> u8 {
    use minecraftoss_entities::skeleton::SkeletonKind::*;
    match kind {
        Skeleton => 0,
        Stray => 1,
        Bogged => 2,
        Parched => 3,
    }
}

fn skeleton_of_kind(kind: u8) -> minecraftoss_entities::skeleton::SkeletonKind {
    use minecraftoss_entities::skeleton::SkeletonKind::*;
    match kind {
        1 => Stray,
        2 => Bogged,
        3 => Parched,
        _ => Skeleton,
    }
}

/// Cow, pig and chicken variants share their three climates.
fn climate(index: usize) -> u8 {
    index.min(2) as u8
}

fn cow_variant(v: minecraftoss_entities::cow::CowVariant) -> u8 {
    use minecraftoss_entities::cow::CowVariant::*;
    climate(match v {
        Temperate => 0,
        Warm => 1,
        Cold => 2,
    })
}

fn variant_cow(v: u8) -> minecraftoss_entities::cow::CowVariant {
    use minecraftoss_entities::cow::CowVariant::*;
    match v {
        1 => Warm,
        2 => Cold,
        _ => Temperate,
    }
}

fn pig_variant(v: minecraftoss_entities::pig::PigVariant) -> u8 {
    use minecraftoss_entities::pig::PigVariant::*;
    climate(match v {
        Temperate => 0,
        Warm => 1,
        Cold => 2,
    })
}

fn variant_pig(v: u8) -> minecraftoss_entities::pig::PigVariant {
    use minecraftoss_entities::pig::PigVariant::*;
    match v {
        1 => Warm,
        2 => Cold,
        _ => Temperate,
    }
}

fn chicken_variant(v: minecraftoss_entities::chicken::ChickenVariant) -> u8 {
    use minecraftoss_entities::chicken::ChickenVariant::*;
    climate(match v {
        Temperate => 0,
        Warm => 1,
        Cold => 2,
    })
}

fn variant_chicken(v: u8) -> minecraftoss_entities::chicken::ChickenVariant {
    use minecraftoss_entities::chicken::ChickenVariant::*;
    match v {
        1 => Warm,
        2 => Cold,
        _ => Temperate,
    }
}

fn potion_index(potion: minecraftoss_entities::potion::Potion) -> u8 {
    use minecraftoss_entities::potion::Potion;
    Potion::ALL.iter().position(|p| *p == potion).unwrap_or(0) as u8
}

fn potion_of(index: u8) -> minecraftoss_entities::potion::Potion {
    use minecraftoss_entities::potion::Potion;
    Potion::ALL[usize::from(index).min(Potion::ALL.len() - 1)]
}

/// What every mob shares, read off its parts.
#[allow(clippy::too_many_arguments)]
fn host_mob(
    id: u64,
    look: Look,
    baby: bool,
    body: &Body,
    yaw: f32,
    head: &LookControl,
    health: f32,
    damage: &DamageState,
    swings: u32,
    extra: Extra,
) -> HostMob {
    let position = body.position.to_array();
    HostMob {
        spawn: MobSpawn {
            id,
            look,
            baby,
            width: body.width,
            height: body.height,
            position,
            yaw,
        },
        state: MobState {
            id,
            position,
            y_rot: yaw,
            x_rot: head.pitch,
            y_head_rot: head.head_yaw,
            on_ground: body.on_ground,
            needs_sync: body.needs_sync,
            burning: body.fire_ticks > 0,
            dead: damage.dead,
            hurts: damage.hurts,
            swings,
            health,
            extra,
        },
    }
}

/// Every mob of the host's entity world (the mobs its players track), with
/// how it looks and its state this tick.
pub fn host_mobs(world: &EntityWorld) -> Vec<HostMob> {
    let mut out = Vec::new();
    for e in world.cows() {
        let c = &e.cow;
        let (look, extra) = match (&e.horse, &e.mooshroom) {
            (Some(h), _) => (
                Look::Horse {
                    donkey: h.kind == minecraftoss_entities::horse::HorseKind::Donkey,
                    variant: h.variant,
                    chest: h.chest,
                },
                Extra::Horse {
                    eat: h.eat_anim,
                    stand: h.stand_anim,
                    mouth: h.mouth_anim,
                    tail: h.tail_counter.clamp(0, 255) as u8,
                },
            ),
            (None, Some(m)) => (
                Look::Mooshroom {
                    brown: m.variant == minecraftoss_entities::mooshroom::MushroomVariant::Brown,
                },
                Extra::None,
            ),
            (None, None) => (
                Look::Cow {
                    variant: cow_variant(c.variant),
                },
                Extra::None,
            ),
        };
        out.push(host_mob(
            e.id,
            look,
            c.age.baby(),
            &c.body,
            c.yaw,
            &e.look_control,
            c.health,
            &c.damage,
            0,
            extra,
        ));
    }
    for e in world.sheep() {
        out.push(host_mob(
            e.id,
            Look::Sheep {
                wool: e.sheep.wool.data(),
            },
            e.sheep.age.baby(),
            &e.body,
            e.yaw,
            &e.look_control,
            e.health,
            &e.damage,
            0,
            Extra::Sheep {
                eat: e.eat_animation_ticks.clamp(0, 255) as u8,
            },
        ));
    }
    for e in world.pigs() {
        let p = &e.pig;
        out.push(host_mob(
            e.id,
            Look::Pig {
                variant: pig_variant(p.variant),
                saddled: p.saddled,
            },
            p.age.baby(),
            &p.body,
            p.yaw,
            &e.look_control,
            p.health,
            &p.damage,
            0,
            Extra::None,
        ));
    }
    for e in world.chickens() {
        let c = &e.chicken;
        out.push(host_mob(
            e.id,
            Look::Chicken {
                variant: chicken_variant(c.variant),
            },
            c.age.baby(),
            &c.body,
            c.yaw,
            &e.look_control,
            c.health,
            &c.damage,
            0,
            Extra::None,
        ));
    }
    for e in world.bats() {
        out.push(host_mob(
            e.id,
            Look::Bat,
            false,
            &e.bat.body,
            e.yaw,
            &e.look_control,
            e.bat.health,
            &e.bat.damage,
            0,
            Extra::Bat {
                resting: e.bat.resting,
            },
        ));
    }
    for e in world.zombies() {
        let z = &e.zombie;
        out.push(host_mob(
            e.id,
            Look::Zombie {
                kind: kind_of_zombie(z.kind),
                main_hand: z.main_hand.clone(),
                armor: z.armor.clone(),
                villager: z.villager.clone(),
            },
            z.baby,
            &z.body,
            e.yaw,
            &e.look_control,
            z.health,
            &z.damage,
            e.ai.as_deref().map_or(0, |ai| ai.state.melee.swings),
            Extra::Aggressive(e.aggressive),
        ));
    }
    for e in world.skeletons() {
        let s = &e.skeleton;
        out.push(host_mob(
            e.id,
            Look::Skeleton {
                kind: kind_of_skeleton(s.kind),
                holds_bow: s.holds_bow,
                sheared: s.sheared,
                armor: s.armor.clone(),
            },
            false,
            &s.body,
            e.yaw,
            &e.look_control,
            s.health,
            &s.damage,
            e.ai.as_deref().map_or(0, |ai| ai.state.melee.swings),
            Extra::Aggressive(e.bow.aggressive),
        ));
    }
    for e in world.creepers() {
        let c = &e.creeper;
        out.push(host_mob(
            e.id,
            Look::Creeper {
                max_swell: c.max_swell,
            },
            false,
            &c.body,
            c.yaw,
            &e.ai.state.look_control,
            c.health,
            &c.damage,
            0,
            Extra::Creeper {
                swell: c.swell.clamp(0, 255) as u8,
                exploded: c.exploded,
            },
        ));
    }
    for e in world.spiders() {
        let s = &e.spider;
        out.push(host_mob(
            e.id,
            Look::Spider,
            false,
            &s.body,
            s.yaw,
            &e.ai.state.look_control,
            s.health,
            &s.damage,
            0,
            Extra::None,
        ));
    }
    for e in world.slimes() {
        let s = &e.slime;
        out.push(host_mob(
            e.id,
            Look::Slime { size: s.size },
            false,
            &s.body,
            s.yaw,
            &e.ai.state.look_control,
            s.health,
            &s.damage,
            0,
            Extra::Slime { squish: s.squish },
        ));
    }
    for e in world.endermen() {
        let n = &e.enderman;
        let state = &e.ai.state;
        out.push(host_mob(
            e.id,
            Look::Enderman {
                carried: state.enderman.carried.as_ref().map(|b| {
                    (
                        b.id.clone(),
                        b.properties
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                    )
                }),
            },
            false,
            &n.body,
            n.yaw,
            &state.look_control,
            n.health,
            &n.damage,
            state.melee.swings,
            Extra::Enderman {
                creepy: state.enderman.creepy,
            },
        ));
    }
    for e in world.witches() {
        let w = &e.witch;
        out.push(host_mob(
            e.id,
            Look::Witch {
                drinking: w.drinking.map(potion_index),
            },
            false,
            &w.body,
            w.yaw,
            &e.ai.state.look_control,
            w.health,
            &w.damage,
            0,
            Extra::None,
        ));
    }
    for e in world.iron_golems() {
        let g = &e.golem;
        out.push(host_mob(
            e.id,
            Look::IronGolem,
            false,
            &g.body,
            g.yaw,
            &e.ai.state.look_control,
            g.health,
            &g.damage,
            0,
            Extra::Golem {
                attack: g.attack_animation_tick.clamp(0, 255) as u8,
                flower: e.ai.state.golem.offer_flower_tick.clamp(0, 65535) as u16,
            },
        ));
    }
    let time = world.game_time();
    for e in world.wolves() {
        let w = &e.wolf;
        out.push(host_mob(
            e.id,
            Look::Wolf {
                variant: w.variant.clone(),
                tame: w.tame,
                collar: w.collar,
                max_health: w.max_health_base,
            },
            w.baby(),
            &w.body,
            w.yaw,
            &e.ai.state.look_control,
            w.health,
            &w.damage,
            0,
            Extra::Wolf {
                angry: e.ai.state.enderman.anger_end_time > time,
                sitting: w.sitting,
                wet: w.wet,
                shake: w.shake_anim,
                interested: w.interested_angle,
            },
        ));
    }
    for e in world.villagers() {
        let v = &e.villager;
        let (yaw, head) = match e.ai.as_deref() {
            Some(ai) => (ai.yaw, ai.look_control.clone()),
            None => (e.yaw, LookControl::new(e.yaw)),
        };
        out.push(host_mob(
            e.id,
            Look::Villager {
                kind: v.kind.clone(),
                profession: v.profession.id().to_owned(),
                level: v.level,
                sleeping: e.sleeping,
                held: e.held_item.as_ref().map(|item| item.id.clone()),
            },
            v.age.baby(),
            &v.body,
            yaw,
            &head,
            v.health,
            &v.damage,
            0,
            Extra::Villager {
                unhappy: e.unhappy.clamp(0, 255) as u8,
            },
        ));
    }
    out
}

/// The mobs a client at `feet` is shown: indices into `mobs`, nearest
/// first, at most `MAX_MOBS_PER_CLIENT` within `MOB_RANGE` blocks.
pub fn interest(mobs: &[HostMob], feet: [f64; 3]) -> Vec<usize> {
    let mut near: Vec<(f64, usize)> = mobs
        .iter()
        .enumerate()
        .filter_map(|(i, mob)| {
            let p = mob.state.position;
            let d = (p[0] - feet[0]).powi(2) + (p[2] - feet[2]).powi(2);
            (d <= MOB_RANGE * MOB_RANGE).then_some((d, i))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    near.truncate(MAX_MOBS_PER_CLIENT);
    near.into_iter().map(|(_, i)| i).collect()
}

/// `Projectile.updateRotation` from a velocity: yaw and pitch in degrees.
fn rotation_of(velocity: DVec3) -> Option<(f32, f32)> {
    let horizontal = velocity.x.hypot(velocity.z);
    (velocity.length_squared() > 1.0e-7).then(|| {
        (
            velocity.x.atan2(velocity.z).to_degrees() as f32,
            velocity.y.atan2(horizontal).to_degrees() as f32,
        )
    })
}

/// The arrows and potions of the host's entity world. An arrow stuck in a
/// block has no velocity left: it keeps the rotation it flew with, which
/// `rotations` remembers by id.
pub fn host_projectiles(
    world: &EntityWorld,
    rotations: &mut std::collections::HashMap<u64, (f32, f32)>,
) -> Vec<Projectile> {
    let mut out = Vec::new();
    for e in world.arrows().iter().filter(|e| e.arrow.alive) {
        let (y_rot, x_rot) = match rotation_of(e.arrow.velocity) {
            Some(rotation) => *rotations
                .entry(e.id)
                .and_modify(|r| *r = rotation)
                .or_insert(rotation),
            None => rotations.get(&e.id).copied().unwrap_or_default(),
        };
        out.push(Projectile {
            id: e.id,
            kind: ProjectileKind::Arrow {
                tipped: e.effect.is_some(),
            },
            position: e.arrow.position.to_array(),
            y_rot,
            x_rot,
        });
    }
    for e in world.potions().iter().filter(|e| e.potion.alive) {
        out.push(Projectile {
            id: e.id,
            kind: ProjectileKind::Potion {
                color: e.potion.potion.color(),
            },
            position: e.potion.position.to_array(),
            y_rot: e.potion.yaw,
            x_rot: e.potion.pitch,
        });
    }
    rotations.retain(|id, _| out.iter().any(|p| p.id == *id));
    out
}

/// The projectiles a client at `feet` is shown: nearest first, within
/// `PROJECTILE_RANGE` blocks horizontally.
pub fn projectiles_near(all: &[Projectile], feet: [f64; 3]) -> Vec<Projectile> {
    let distance =
        |p: &Projectile| (p.position[0] - feet[0]).powi(2) + (p.position[2] - feet[2]).powi(2);
    let mut near: Vec<Projectile> = all
        .iter()
        .filter(|p| distance(p) <= PROJECTILE_RANGE * PROJECTILE_RANGE)
        .copied()
        .collect();
    near.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
    near.truncate(MAX_PROJECTILES_PER_FRAME);
    near
}

/// The host's recent events, numbered, kept `EVENT_KEEP_TICKS` host ticks.
#[derive(Default)]
pub struct EventLog {
    next: u32,
    rows: std::collections::VecDeque<(u32, EventRow)>,
}

impl EventLog {
    /// An event in host tick `tick`.
    pub fn push(&mut self, tick: u32, event: WorldEvent) {
        self.rows.push_back((
            tick,
            EventRow {
                seq: self.next,
                event,
            },
        ));
        self.next = self.next.wrapping_add(1);
    }

    /// Forgets the events too old to send as of `tick`.
    pub fn age(&mut self, tick: u32) {
        while self
            .rows
            .front()
            .is_some_and(|(at, _)| tick.wrapping_sub(*at) >= EVENT_KEEP_TICKS)
        {
            self.rows.pop_front();
        }
    }
}

/// The events going out to one client: each new one that reaches it goes
/// with the next `EVENT_SENDS` snapshots it is sent, so one lost snapshot
/// loses none, however few snapshots the host sends a second.
#[derive(Default)]
pub struct EventWindow {
    last: Option<u32>,
    pending: std::collections::VecDeque<(EventRow, u8)>,
}

impl EventWindow {
    /// The events for the next snapshot to a player at `feet`, oldest
    /// first.
    pub fn next(&mut self, log: &EventLog, feet: [f64; 3]) -> Vec<EventRow> {
        for (_, row) in &log.rows {
            if self
                .last
                .is_some_and(|last| row.seq.wrapping_sub(last) as i32 <= 0)
            {
                continue;
            }
            self.last = Some(row.seq);
            if row.event.reaches(feet) {
                self.pending.push_back((row.clone(), 0));
            }
        }
        let out: Vec<EventRow> = self
            .pending
            .iter_mut()
            .take(MAX_EVENTS_PER_FRAME)
            .map(|(row, sends)| {
                *sends += 1;
                row.clone()
            })
            .collect();
        self.pending.retain(|(_, sends)| *sends < EVENT_SENDS);
        out
    }
}

/// A client's arrows and potions in flight, as its snapshots last told:
/// each where it was the tick before (to draw between) and where it is.
#[derive(Default)]
pub struct ClientProjectiles {
    now: Vec<(Projectile, [f64; 3])>,
    /// The ids seen of each kind so far.
    seen: std::collections::BTreeMap<&'static str, std::collections::BTreeSet<u64>>,
}

impl ClientProjectiles {
    /// One snapshot's projectiles: those it does not name have landed.
    pub fn receive(&mut self, projectiles: &[Projectile]) {
        let now = projectiles
            .iter()
            .map(|p| {
                let previous = self
                    .now
                    .iter()
                    .find(|(q, _)| q.id == p.id)
                    .map_or(p.position, |(q, _)| q.position);
                self.seen.entry(p.kind.name()).or_default().insert(p.id);
                (*p, previous)
            })
            .collect();
        self.now = now;
    }

    /// Each projectile and where to draw it, `partial` of a tick on from
    /// where it was.
    pub fn drawn(&self, partial: f64) -> Vec<(Projectile, DVec3)> {
        self.now
            .iter()
            .map(|(p, previous)| {
                let (from, to) = (DVec3::from_array(*previous), DVec3::from_array(p.position));
                (*p, from.lerp(to, partial))
            })
            .collect()
    }

    /// How many of each kind were ever seen.
    pub fn seen(&self) -> Vec<(&'static str, usize)> {
        self.seen
            .iter()
            .map(|(kind, ids)| (*kind, ids.len()))
            .collect()
    }

    pub fn clear(&mut self) {
        self.now.clear();
    }
}

/// The players' status effects on a host: what potions and tipped arrows
/// gave each player (by its mob world id), ticked with the level as
/// `LivingEntity.tickEffects` ticks a player's.
#[derive(Default)]
pub struct PlayerEffects {
    players: std::collections::HashMap<u64, MobEffects>,
}

impl PlayerEffects {
    /// A splash on `player`: a lasting effect joins its effects; an instant
    /// one is the magic damage it does at once (in Minecraft health).
    pub fn splash(
        &mut self,
        player: u64,
        splash: &minecraftoss_entities::world::PlayerSplash,
    ) -> Option<f32> {
        match splash.instant {
            Some(EffectWork::HurtMagic(amount)) => Some(amount),
            Some(_) => None,
            None => {
                self.add(player, splash.effect, splash.amplifier, splash.duration);
                None
            }
        }
    }

    /// `LivingEntity.addEffect` on a player.
    pub fn add(&mut self, player: u64, effect: MobEffect, amplifier: i32, duration: i32) {
        self.players
            .entry(player)
            .or_default()
            .add(EffectInstance::new(effect, duration, amplifier));
    }

    /// A tipped arrow's effect (`Arrow.doPostHurtEffects`), as the player's
    /// side knows the kinds.
    pub fn arrow(
        &mut self,
        player: u64,
        kind: minecraftoss_player::survival::EffectKind,
        duration: u32,
    ) {
        use minecraftoss_player::survival::EffectKind as K;
        let effect = match kind {
            K::Poison => MobEffect::Poison,
            K::Slowness => MobEffect::Slowness,
            K::Weakness => MobEffect::Weakness,
            K::Regeneration => MobEffect::Regeneration,
            K::FireResistance => MobEffect::FireResistance,
            _ => return,
        };
        self.add(player, effect, 0, duration.min(i32::MAX as u32) as i32);
    }

    /// One tick of every player's effects: the magic damage each takes (in
    /// Minecraft health), given each one's health (poison stops at 1).
    pub fn tick(&mut self, health: impl Fn(u64) -> f32) -> Vec<(u64, f32)> {
        let mut hurts = Vec::new();
        for (&player, effects) in &mut self.players {
            for work in effects.tick(false) {
                if let Some(EffectWork::HurtMagic(amount)) = work.resolve(health(player), 20.0) {
                    hurts.push((player, amount));
                }
            }
        }
        self.players.retain(|_, effects| !effects.is_empty());
        hurts
    }

    /// A player died: its effects go (`LivingEntity.die` → `removeAllEffects`).
    pub fn clear(&mut self, player: u64) {
        self.players.remove(&player);
    }

    /// The effects a player has.
    pub fn of(&self, player: u64) -> Vec<PlayerEffect> {
        self.players.get(&player).map_or_else(Vec::new, |effects| {
            effects
                .iter()
                .map(|e| PlayerEffect {
                    effect: e.effect,
                    amplifier: e.amplifier.clamp(0, 255) as u8,
                    duration: e.duration,
                })
                .collect()
        })
    }

    /// What witches look at: slowed, poisoned, weakened.
    pub fn marks(&self, player: u64) -> (bool, bool, bool) {
        self.players
            .get(&player)
            .map_or((false, false, false), |e| {
                (
                    e.has(MobEffect::Slowness),
                    e.has(MobEffect::Poison),
                    e.has(MobEffect::Weakness),
                )
            })
    }

    /// The player's movement speed as a share of its own (speed and
    /// slowness).
    pub fn speed(&self, player: u64) -> f32 {
        self.players
            .get(&player)
            .map_or(1.0, |e| e.movement_speed(1.0) as f32)
    }

    /// What its effects add to its attack damage (weakness takes off).
    pub fn attack_damage(&self, player: u64) -> f64 {
        self.players
            .get(&player)
            .map_or(0.0, MobEffects::attack_damage_modifier)
    }
}

fn varint(w: &mut Writer, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            w.u8(byte);
            return;
        }
        w.u8(byte | 0x80);
    }
}

fn read_varint(r: &mut Reader<'_>) -> Result<u64, DecodeError> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = r.u8()?;
        v |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err(DecodeError::Malformed("varint"))
}

/// A coordinate in 1/4096 of a block (`VecDeltaCodec`'s unit).
fn fixed(v: f64) -> i32 {
    (v * 4096.0)
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

fn unfixed(v: i32) -> f64 {
    f64::from(v) / 4096.0
}

/// An angle as a 256th of a turn (`Mth.packDegrees`), as vanilla sends it.
fn pack(angle: f32) -> u8 {
    (angle * 256.0 / 360.0).floor() as i32 as u8
}

fn unpack(byte: u8) -> f32 {
    f32::from(byte as i8) * 360.0 / 256.0
}

fn unit(v: f32, scale: f32) -> u8 {
    (v * scale).round().clamp(0.0, 255.0) as u8
}

fn encode_extra(w: &mut Writer, extra: &Extra) {
    match *extra {
        Extra::None => w.u8(0),
        Extra::Aggressive(on) => {
            w.u8(1);
            w.u8(u8::from(on));
        }
        Extra::Creeper { swell, exploded } => {
            w.u8(2);
            w.u8(swell);
            w.u8(u8::from(exploded));
        }
        Extra::Enderman { creepy } => {
            w.u8(3);
            w.u8(u8::from(creepy));
        }
        Extra::Golem { attack, flower } => {
            w.u8(4);
            w.u8(attack);
            w.u16(flower);
        }
        Extra::Wolf {
            angry,
            sitting,
            wet,
            shake,
            interested,
        } => {
            w.u8(5);
            w.u8(u8::from(angry) | u8::from(sitting) << 1 | u8::from(wet) << 2);
            w.u8(unit(shake, 100.0));
            w.u8(unit(interested, 255.0));
        }
        Extra::Sheep { eat } => {
            w.u8(6);
            w.u8(eat);
        }
        Extra::Slime { squish } => {
            w.u8(7);
            w.u16(((squish * 1000.0).round().clamp(-32768.0, 32767.0) as i16) as u16);
        }
        Extra::Villager { unhappy } => {
            w.u8(8);
            w.u8(unhappy);
        }
        Extra::Horse {
            eat,
            stand,
            mouth,
            tail,
        } => {
            w.u8(9);
            w.u8(unit(eat, 255.0));
            w.u8(unit(stand, 255.0));
            w.u8(unit(mouth, 255.0));
            w.u8(tail);
        }
        Extra::Bat { resting } => {
            w.u8(10);
            w.u8(u8::from(resting));
        }
    }
}

fn decode_extra(r: &mut Reader<'_>) -> Result<Extra, DecodeError> {
    Ok(match r.u8()? {
        0 => Extra::None,
        1 => Extra::Aggressive(r.u8()? != 0),
        2 => Extra::Creeper {
            swell: r.u8()?,
            exploded: r.u8()? != 0,
        },
        3 => Extra::Enderman {
            creepy: r.u8()? != 0,
        },
        4 => Extra::Golem {
            attack: r.u8()?,
            flower: r.u16()?,
        },
        5 => {
            let bits = r.u8()?;
            Extra::Wolf {
                angry: bits & 1 != 0,
                sitting: bits & 2 != 0,
                wet: bits & 4 != 0,
                shake: f32::from(r.u8()?) / 100.0,
                interested: f32::from(r.u8()?) / 255.0,
            }
        }
        6 => Extra::Sheep { eat: r.u8()? },
        7 => Extra::Slime {
            squish: f32::from(r.u16()? as i16) / 1000.0,
        },
        8 => Extra::Villager { unhappy: r.u8()? },
        9 => Extra::Horse {
            eat: f32::from(r.u8()?) / 255.0,
            stand: f32::from(r.u8()?) / 255.0,
            mouth: f32::from(r.u8()?) / 255.0,
            tail: r.u8()?,
        },
        10 => Extra::Bat {
            resting: r.u8()? != 0,
        },
        _ => return Err(DecodeError::Malformed("mob extra")),
    })
}

fn encode_state(w: &mut Writer, s: &MobState) {
    varint(w, s.id);
    for v in s.position {
        w.i32(fixed(v));
    }
    w.u8(pack(s.y_rot));
    w.u8(pack(s.x_rot));
    w.u8(pack(s.y_head_rot));
    w.u8(u8::from(s.on_ground)
        | u8::from(s.needs_sync) << 1
        | u8::from(s.burning) << 2
        | u8::from(s.dead) << 3);
    w.u8(s.hurts as u8);
    w.u8(s.swings as u8);
    w.u16((s.health * 16.0).round().clamp(0.0, 65535.0) as u16);
    encode_extra(w, &s.extra);
}

fn decode_state(r: &mut Reader<'_>) -> Result<MobState, DecodeError> {
    let id = read_varint(r)?;
    let position = [unfixed(r.i32()?), unfixed(r.i32()?), unfixed(r.i32()?)];
    let (y_rot, x_rot, y_head_rot) = (unpack(r.u8()?), unpack(r.u8()?), unpack(r.u8()?));
    let flags = r.u8()?;
    Ok(MobState {
        id,
        position,
        y_rot,
        x_rot,
        y_head_rot,
        on_ground: flags & 1 != 0,
        needs_sync: flags & 2 != 0,
        burning: flags & 4 != 0,
        dead: flags & 8 != 0,
        hurts: u32::from(r.u8()?),
        swings: u32::from(r.u8()?),
        health: f32::from(r.u16()?) / 16.0,
        extra: decode_extra(r)?,
    })
}

/// The mob states of one host tick as one snapshot carries them, in the
/// order given, as many as fit `MOB_FRAME_BYTES` with the extras (the
/// nearest first, when given nearest first). The extras go in the order
/// given, as many as fit their share.
pub fn encode_frame<'a>(
    tick: u32,
    states: impl IntoIterator<Item = &'a MobState>,
    extras: &FrameExtras,
) -> Vec<u8> {
    let tail = encode_extras(extras);
    let mut w = Writer::default();
    w.u8(FRAME_VERSION);
    w.u32(tick);
    let count_at = w.0.len();
    w.u16(0);
    let mut count = 0u16;
    for state in states {
        let mut one = Writer::default();
        encode_state(&mut one, state);
        if w.0.len() + one.0.len() + tail.len() > MOB_FRAME_BYTES || count == u16::MAX {
            break;
        }
        w.0.extend_from_slice(&one.0);
        count += 1;
    }
    w.0[count_at..count_at + 2].copy_from_slice(&count.to_le_bytes());
    w.0.extend_from_slice(&tail);
    w.0
}

/// Up to `limit` items, each encoded by `one`, while the bytes stay within
/// `budget`: a count byte and the items.
fn encode_list<T>(
    out: &mut Writer,
    items: &[T],
    limit: usize,
    budget: usize,
    one: impl Fn(&mut Writer, &T),
) {
    let count_at = out.0.len();
    out.u8(0);
    let mut count = 0u8;
    for item in items.iter().take(limit.min(usize::from(u8::MAX))) {
        let mut w = Writer::default();
        one(&mut w, item);
        if out.0.len() + w.0.len() > budget {
            break;
        }
        out.0.extend_from_slice(&w.0);
        count += 1;
    }
    out.0[count_at] = count;
}

fn effect_index(effect: MobEffect) -> u8 {
    MobEffect::ALL
        .iter()
        .position(|e| *e == effect)
        .unwrap_or(0) as u8
}

fn encode_event(w: &mut Writer, base: u32, row: &EventRow) {
    varint(w, u64::from(row.seq.wrapping_sub(base)));
    match &row.event {
        WorldEvent::Sound {
            event,
            position,
            volume,
            pitch,
        } => {
            w.u8(0);
            w.str(event);
            for v in position {
                w.i32(fixed(*v));
            }
            w.f32(*volume);
            w.f32(*pitch);
        }
        WorldEvent::Splash {
            position,
            color,
            instant,
        } => {
            w.u8(1);
            for v in position {
                w.i32(fixed(*v));
            }
            w.u32(*color);
            w.u8(u8::from(*instant));
        }
        WorldEvent::Explosion { position, radius } => {
            w.u8(2);
            for v in position {
                w.i32(fixed(*v));
            }
            w.f32(*radius);
        }
    }
}

fn decode_event(r: &mut Reader<'_>, base: u32) -> Result<EventRow, DecodeError> {
    let seq = base.wrapping_add(read_varint(r)? as u32);
    let position = |r: &mut Reader<'_>| -> Result<[f64; 3], DecodeError> {
        Ok([unfixed(r.i32()?), unfixed(r.i32()?), unfixed(r.i32()?)])
    };
    let event = match r.u8()? {
        0 => WorldEvent::Sound {
            event: r.str()?,
            position: position(r)?,
            volume: r.f32()?,
            pitch: r.f32()?,
        },
        1 => WorldEvent::Splash {
            position: position(r)?,
            color: r.u32()?,
            instant: r.bool()?,
        },
        2 => WorldEvent::Explosion {
            position: position(r)?,
            radius: r.f32()?,
        },
        _ => return Err(DecodeError::Malformed("world event")),
    };
    Ok(EventRow { seq, event })
}

fn encode_projectile(w: &mut Writer, p: &Projectile) {
    varint(w, p.id);
    match p.kind {
        ProjectileKind::Arrow { tipped } => w.u8(u8::from(tipped)),
        ProjectileKind::Potion { color } => {
            w.u8(2);
            w.u32(color);
        }
    }
    for v in p.position {
        w.i32(fixed(v));
    }
    w.u8(pack(p.y_rot));
    w.u8(pack(p.x_rot));
}

fn decode_projectile(r: &mut Reader<'_>) -> Result<Projectile, DecodeError> {
    let id = read_varint(r)?;
    let kind = match r.u8()? {
        0 => ProjectileKind::Arrow { tipped: false },
        1 => ProjectileKind::Arrow { tipped: true },
        2 => ProjectileKind::Potion { color: r.u32()? },
        _ => return Err(DecodeError::Malformed("projectile")),
    };
    Ok(Projectile {
        id,
        kind,
        position: [unfixed(r.i32()?), unfixed(r.i32()?), unfixed(r.i32()?)],
        y_rot: unpack(r.u8()?),
        x_rot: unpack(r.u8()?),
    })
}

/// The events (numbered from the first one's number), projectiles and
/// effects, within `EXTRAS_BYTES`.
fn encode_extras(extras: &FrameExtras) -> Vec<u8> {
    let mut w = Writer::default();
    let base = extras.events.first().map_or(0, |row| row.seq);
    w.u32(base);
    let budget = EXTRAS_BYTES - 2 - 7 * MobEffect::ALL.len();
    encode_list(
        &mut w,
        &extras.events,
        MAX_EVENTS_PER_FRAME,
        budget,
        |w, row| encode_event(w, base, row),
    );
    encode_list(
        &mut w,
        &extras.projectiles,
        MAX_PROJECTILES_PER_FRAME,
        budget,
        encode_projectile,
    );
    encode_list(
        &mut w,
        &extras.effects,
        MobEffect::ALL.len(),
        EXTRAS_BYTES,
        |w, e| {
            w.u8(effect_index(e.effect));
            w.u8(e.amplifier);
            w.i32(e.duration);
        },
    );
    w.0
}

fn decode_extras(r: &mut Reader<'_>) -> Result<FrameExtras, DecodeError> {
    let base = r.u32()?;
    let mut extras = FrameExtras::default();
    for _ in 0..r.u8()? {
        extras.events.push(decode_event(r, base)?);
    }
    for _ in 0..r.u8()? {
        extras.projectiles.push(decode_projectile(r)?);
    }
    for _ in 0..r.u8()? {
        let effect = *MobEffect::ALL
            .get(usize::from(r.u8()?))
            .ok_or(DecodeError::Malformed("effect"))?;
        extras.effects.push(PlayerEffect {
            effect,
            amplifier: r.u8()?,
            duration: r.i32()?,
        });
    }
    Ok(extras)
}

/// How many states an encoded frame carries (none for bytes that are no
/// frame): fewer than given when the rest did not fit.
pub fn frame_len(bytes: &[u8]) -> usize {
    bytes
        .get(5..7)
        .map_or(0, |b| usize::from(u16::from_le_bytes([b[0], b[1]])))
}

/// A snapshot's mob section.
pub fn decode_frame(bytes: &[u8]) -> Result<MobFrame, DecodeError> {
    let mut r = Reader(bytes);
    if r.u8()? != FRAME_VERSION {
        return Err(DecodeError::Malformed("mob frame version"));
    }
    let tick = r.u32()?;
    let count = usize::from(r.u16()?);
    // Each state takes at least 21 bytes.
    if count > r.0.len() / 21 {
        return Err(DecodeError::Truncated);
    }
    let mut states = Vec::with_capacity(count);
    for _ in 0..count {
        states.push(decode_state(&mut r)?);
    }
    let extras = decode_extras(&mut r)?;
    r.done()?;
    Ok(MobFrame {
        tick,
        states,
        extras,
    })
}

fn encode_armor(w: &mut Writer, armor: &[Option<String>; 4]) {
    for piece in armor {
        w.opt_str(piece.as_deref());
    }
}

fn decode_armor(r: &mut Reader<'_>) -> Result<[Option<String>; 4], DecodeError> {
    Ok([r.opt_str()?, r.opt_str()?, r.opt_str()?, r.opt_str()?])
}

fn encode_look(w: &mut Writer, look: &Look) {
    match look {
        Look::Cow { variant } => {
            w.u8(0);
            w.u8(*variant);
        }
        Look::Mooshroom { brown } => {
            w.u8(1);
            w.u8(u8::from(*brown));
        }
        Look::Horse {
            donkey,
            variant,
            chest,
        } => {
            w.u8(2);
            w.u8(u8::from(*donkey));
            w.i32(*variant);
            w.u8(u8::from(*chest));
        }
        Look::Sheep { wool } => {
            w.u8(3);
            w.u8(*wool);
        }
        Look::Pig { variant, saddled } => {
            w.u8(4);
            w.u8(*variant);
            w.u8(u8::from(*saddled));
        }
        Look::Chicken { variant } => {
            w.u8(5);
            w.u8(*variant);
        }
        Look::Bat => w.u8(6),
        Look::Zombie {
            kind,
            main_hand,
            armor,
            villager,
        } => {
            w.u8(7);
            w.u8(*kind);
            w.opt_str(main_hand.as_deref());
            encode_armor(w, armor);
            match villager {
                None => w.u8(0),
                Some((kind, profession)) => {
                    w.u8(1);
                    w.str(kind);
                    w.str(profession);
                }
            }
        }
        Look::Skeleton {
            kind,
            holds_bow,
            sheared,
            armor,
        } => {
            w.u8(8);
            w.u8(*kind);
            w.u8(u8::from(*holds_bow) | u8::from(*sheared) << 1);
            encode_armor(w, armor);
        }
        Look::Creeper { max_swell } => {
            w.u8(9);
            w.i32(*max_swell);
        }
        Look::Spider => w.u8(10),
        Look::Slime { size } => {
            w.u8(11);
            w.i32(*size);
        }
        Look::Enderman { carried } => {
            w.u8(12);
            match carried {
                None => w.u8(0),
                Some((id, properties)) => {
                    w.u8(1);
                    w.str(id);
                    w.u8(properties.len().min(255) as u8);
                    for (k, v) in properties.iter().take(255) {
                        w.str(k);
                        w.str(v);
                    }
                }
            }
        }
        Look::Witch { drinking } => {
            w.u8(13);
            w.u8(drinking.map_or(0, |p| p.saturating_add(1)));
        }
        Look::IronGolem => w.u8(14),
        Look::Wolf {
            variant,
            tame,
            collar,
            max_health,
        } => {
            w.u8(15);
            w.str(variant);
            w.u8(u8::from(*tame));
            w.u8(*collar);
            w.f32(*max_health);
        }
        Look::Villager {
            kind,
            profession,
            level,
            sleeping,
            held,
        } => {
            w.u8(16);
            w.str(kind);
            w.str(profession);
            w.i32(*level);
            match sleeping {
                None => w.u8(0),
                Some(pos) => {
                    w.u8(1);
                    w.pos(*pos);
                }
            }
            w.opt_str(held.as_deref());
        }
    }
}

fn decode_look(r: &mut Reader<'_>) -> Result<Look, DecodeError> {
    Ok(match r.u8()? {
        0 => Look::Cow { variant: r.u8()? },
        1 => Look::Mooshroom { brown: r.bool()? },
        2 => Look::Horse {
            donkey: r.bool()?,
            variant: r.i32()?,
            chest: r.bool()?,
        },
        3 => Look::Sheep { wool: r.u8()? },
        4 => Look::Pig {
            variant: r.u8()?,
            saddled: r.bool()?,
        },
        5 => Look::Chicken { variant: r.u8()? },
        6 => Look::Bat,
        7 => Look::Zombie {
            kind: r.u8()?,
            main_hand: r.opt_str()?,
            armor: decode_armor(r)?,
            villager: if r.bool()? {
                Some((r.str()?, r.str()?))
            } else {
                None
            },
        },
        8 => {
            let kind = r.u8()?;
            let bits = r.u8()?;
            Look::Skeleton {
                kind,
                holds_bow: bits & 1 != 0,
                sheared: bits & 2 != 0,
                armor: decode_armor(r)?,
            }
        }
        9 => Look::Creeper {
            max_swell: r.i32()?,
        },
        10 => Look::Spider,
        11 => Look::Slime { size: r.i32()? },
        12 => Look::Enderman {
            carried: if r.bool()? {
                let id = r.str()?;
                let n = r.u8()?;
                let mut properties = Vec::with_capacity(usize::from(n));
                for _ in 0..n {
                    properties.push((r.str()?, r.str()?));
                }
                Some((id, properties))
            } else {
                None
            },
        },
        13 => Look::Witch {
            drinking: r.u8()?.checked_sub(1),
        },
        14 => Look::IronGolem,
        15 => Look::Wolf {
            variant: r.str()?,
            tame: r.bool()?,
            collar: r.u8()?,
            max_health: r.f32()?,
        },
        16 => Look::Villager {
            kind: r.str()?,
            profession: r.str()?,
            level: r.i32()?,
            sleeping: if r.bool()? { Some(r.pos()?) } else { None },
            held: r.opt_str()?,
        },
        _ => return Err(DecodeError::Malformed("mob look")),
    })
}

pub(crate) fn encode_spawn(w: &mut Writer, s: &MobSpawn) {
    varint(w, s.id);
    encode_look(w, &s.look);
    w.u8(u8::from(s.baby));
    w.f32(s.width);
    w.f32(s.height);
    for v in s.position {
        w.f64(v);
    }
    w.f32(s.yaw);
}

pub(crate) fn decode_spawn(r: &mut Reader<'_>) -> Result<MobSpawn, DecodeError> {
    Ok(MobSpawn {
        id: read_varint(r)?,
        look: decode_look(r)?,
        baby: r.bool()?,
        width: r.f32()?,
        height: r.f32()?,
        position: [r.f64()?, r.f64()?, r.f64()?],
        yaw: r.f32()?,
    })
}

pub(crate) fn encode_ids(w: &mut Writer, ids: &[u64]) {
    w.u16(ids.len().min(usize::from(u16::MAX)) as u16);
    for &id in ids.iter().take(usize::from(u16::MAX)) {
        varint(w, id);
    }
}

pub(crate) fn decode_ids(r: &mut Reader<'_>) -> Result<Vec<u64>, DecodeError> {
    let n = usize::from(r.u16()?);
    if n > r.0.len() {
        return Err(DecodeError::Truncated);
    }
    (0..n).map(|_| read_varint(r)).collect()
}

/// How many of `spawns`, from the first, one `HostMessage::Mobs` takes: at
/// most `SPAWN_BYTES_PER_MESSAGE`, and at least one.
pub fn spawns_that_fit(spawns: &[MobSpawn]) -> usize {
    let mut bytes = 0;
    let mut count = 0;
    for spawn in spawns {
        let mut w = Writer::default();
        encode_spawn(&mut w, spawn);
        bytes += w.0.len();
        if count > 0 && bytes > SPAWN_BYTES_PER_MESSAGE {
            break;
        }
        count += 1;
    }
    count
}

/// The low byte of a counter that travels as its low byte, made whole again
/// on the client.
#[derive(Clone, Copy, Debug, Default)]
struct Counter {
    wire: Option<u8>,
    total: u32,
}

impl Counter {
    fn take(&mut self, wire: u32) -> u32 {
        let wire = wire as u8;
        match self.wire {
            None => self.total = u32::from(wire),
            Some(last) => self.total = self.total.wrapping_add(u32::from(wire.wrapping_sub(last))),
        }
        self.wire = Some(wire);
        self.total
    }
}

/// A client's copy of the host's mobs near it: an entity world that never
/// ticks, with the host's ids.
#[derive(Default)]
pub struct ClientCopy {
    /// The last host tick applied.
    last_tick: Option<u32>,
    /// The number of the last event taken.
    last_event: Option<u32>,
    counters: std::collections::HashMap<u64, (Counter, Counter)>,
    /// What each mob was spawned as.
    spawns: std::collections::HashMap<u64, MobSpawn>,
}

impl ClientCopy {
    /// The mobs the client has, with what they were spawned as.
    pub fn spawns(&self) -> impl Iterator<Item = &MobSpawn> {
        self.spawns.values()
    }

    /// Takes a mob into the world, or makes it again with new looks.
    pub fn spawn(&mut self, world: &mut EntityWorld, spawn: &MobSpawn) {
        if self.spawns.contains_key(&spawn.id) {
            remove(world, spawn.id);
        }
        if spawn.id == 0 {
            return;
        }
        add(world, spawn);
        self.spawns.insert(spawn.id, spawn.clone());
    }

    /// Takes mobs out of the world.
    pub fn gone(&mut self, world: &mut EntityWorld, ids: &[u64]) {
        for &id in ids {
            if self.spawns.remove(&id).is_some() {
                remove(world, id);
            }
            self.counters.remove(&id);
        }
    }

    /// One snapshot's states; false when they are older than what the copy
    /// already has (they arrived out of order) or of mobs it does not have.
    pub fn apply(&mut self, world: &mut EntityWorld, tick: u32, states: &[MobState]) -> bool {
        if self
            .last_tick
            .is_some_and(|last| tick.wrapping_sub(last) as i32 <= 0)
        {
            return false;
        }
        self.last_tick = Some(tick);
        world.set_game_time(i64::from(tick));
        for state in states {
            if !self.spawns.contains_key(&state.id) {
                continue;
            }
            let (hurts, swings) = self.counters.entry(state.id).or_default();
            let mut state = *state;
            state.hurts = hurts.take(state.hurts);
            state.swings = swings.take(state.swings);
            set(world, &state, i64::from(tick));
        }
        true
    }

    /// The events of an applied snapshot not taken before, in order: each
    /// comes with a few snapshots in a row.
    pub fn fresh_events(&mut self, rows: &[EventRow]) -> Vec<WorldEvent> {
        let mut fresh = Vec::new();
        for row in rows {
            if self
                .last_event
                .is_some_and(|last| row.seq.wrapping_sub(last) as i32 <= 0)
            {
                continue;
            }
            self.last_event = Some(row.seq);
            fresh.push(row.event.clone());
        }
        fresh
    }

    /// Forgets every mob (a new world).
    pub fn clear(&mut self, world: &mut EntityWorld) {
        let ids: Vec<u64> = self.spawns.keys().copied().collect();
        self.gone(world, &ids);
        self.last_tick = None;
        self.last_event = None;
    }
}

fn remove(world: &mut EntityWorld, id: u64) {
    if let Some(body) = world.body_mut(id) {
        body.position = DVec3::new(0.0, GONE_Y, 0.0);
        world.remove_where(|p| p.y <= GONE_Y);
    }
}

fn age(baby: bool) -> minecraftoss_entities::age::Age {
    minecraftoss_entities::age::Age {
        ticks: if baby {
            minecraftoss_entities::age::Age::BABY_START
        } else {
            0
        },
        ..Default::default()
    }
}

/// Spawns a mob as `spawn` describes, with its id.
fn add(world: &mut EntityWorld, spawn: &MobSpawn) {
    use minecraftoss_entities as e;
    let at = DVec3::from_array(spawn.position);
    let yaw = spawn.yaw;
    world.set_next_entity_id(spawn.id);
    let id = match &spawn.look {
        Look::Cow { variant } => {
            let mut cow = e::cow::Cow::new(at);
            cow.variant = variant_cow(*variant);
            cow.age = age(spawn.baby);
            cow.yaw = yaw;
            world.spawn_cow(cow, true)
        }
        Look::Mooshroom { brown } => {
            let variant = if *brown {
                e::mooshroom::MushroomVariant::Brown
            } else {
                e::mooshroom::MushroomVariant::Red
            };
            let mut cow = e::mooshroom::MushroomCow::new(at, variant);
            cow.cow.age = age(spawn.baby);
            cow.cow.yaw = yaw;
            world.spawn_mooshroom(cow, true)
        }
        Look::Horse {
            donkey,
            variant,
            chest,
        } => {
            let kind = if *donkey {
                e::horse::HorseKind::Donkey
            } else {
                e::horse::HorseKind::Horse
            };
            let mut horse = e::horse::HorseState::new(kind);
            horse.variant = *variant;
            horse.chest = *chest;
            let mut cow = e::cow::Cow::new(at);
            cow.age = age(spawn.baby);
            cow.yaw = yaw;
            world.spawn_horse(cow, horse, true)
        }
        Look::Sheep { wool } => {
            let sheep = e::sheep::Sheep {
                wool: e::sheep::Wool::from_data(*wool),
                age: age(spawn.baby),
                ..Default::default()
            };
            world.spawn_sheep(sheep, at, true)
        }
        Look::Pig { variant, saddled } => {
            let mut pig = e::pig::Pig::new(at);
            pig.variant = variant_pig(*variant);
            pig.saddled = *saddled;
            pig.age = age(spawn.baby);
            pig.yaw = yaw;
            world.spawn_pig(pig, true)
        }
        Look::Chicken { variant } => {
            let mut chicken = e::chicken::Chicken::new(at);
            chicken.variant = variant_chicken(*variant);
            chicken.age = age(spawn.baby);
            chicken.yaw = yaw;
            world.spawn_chicken(chicken, true)
        }
        Look::Bat => world.spawn_bat(e::bat::Bat::new(at), true),
        Look::Zombie {
            kind,
            main_hand,
            armor,
            villager,
        } => {
            let mut zombie = e::zombie::Zombie::new(at);
            zombie.kind = zombie_of_kind(*kind);
            zombie.baby = spawn.baby;
            zombie.main_hand = main_hand.clone();
            zombie.armor = armor.clone();
            zombie.villager = villager.clone();
            world.spawn_zombie_active(zombie, yaw)
        }
        Look::Skeleton {
            kind,
            holds_bow,
            sheared,
            armor,
        } => {
            let mut skeleton = e::skeleton::Skeleton::new(at);
            skeleton.kind = skeleton_of_kind(*kind);
            skeleton.holds_bow = *holds_bow;
            skeleton.sheared = *sheared;
            skeleton.armor = armor.clone();
            world.spawn_skeleton_active(skeleton, yaw)
        }
        Look::Creeper { max_swell } => {
            let mut creeper = e::creeper::Creeper::new(at);
            creeper.max_swell = *max_swell;
            creeper.yaw = yaw;
            world.spawn_creeper(creeper, true)
        }
        Look::Spider => {
            let mut spider = e::spider::Spider::new(at);
            spider.yaw = yaw;
            world.spawn_spider(spider, true)
        }
        Look::Slime { size } => {
            let mut slime = e::slime::Slime::new(at, *size);
            slime.yaw = yaw;
            world.spawn_slime(slime, true)
        }
        Look::Enderman { carried } => {
            let mut enderman = e::enderman::Enderman::new(at);
            enderman.yaw = yaw;
            let id = world.spawn_enderman(enderman, true);
            if let Some(entity) = world.enderman_mut(id) {
                entity.ai.state.enderman.carried =
                    carried
                        .as_ref()
                        .map(|(block, properties)| minecraftoss_player::Block {
                            id: block.clone(),
                            properties: properties.iter().cloned().collect(),
                        });
            }
            id
        }
        Look::Witch { drinking } => {
            let mut witch = e::witch::Witch::new(at);
            witch.drinking = drinking.map(potion_of);
            witch.yaw = yaw;
            world.spawn_witch(witch, true)
        }
        Look::IronGolem => {
            let mut golem = e::iron_golem::IronGolem::new(at);
            golem.yaw = yaw;
            world.spawn_iron_golem(golem, true)
        }
        Look::Wolf {
            variant,
            tame,
            collar,
            max_health,
        } => {
            let mut wolf = e::wolf::Wolf::new(at);
            wolf.variant = variant.clone();
            wolf.tame = *tame;
            wolf.collar = *collar;
            wolf.max_health_base = *max_health;
            wolf.age = age(spawn.baby);
            wolf.yaw = yaw;
            world.spawn_wolf(wolf, true)
        }
        Look::Villager {
            kind,
            profession,
            level,
            sleeping,
            held,
        } => {
            let mut villager = e::villager::Villager::new(at);
            villager.kind = kind.clone();
            villager.profession = e::villager::Profession::from_id(profession).unwrap_or_default();
            villager.level = *level;
            villager.age = age(spawn.baby);
            let id = world.spawn_villager_active(villager, yaw);
            if let Some(entity) = world.villager_mut(id) {
                entity.sleeping = *sleeping;
                entity.held_item = held.as_ref().map(|item| e::trading::TradeItem {
                    id: item.clone(),
                    count: 1,
                    components: Default::default(),
                });
            }
            id
        }
    };
    debug_assert_eq!(id, spawn.id);
    if let Some(body) = world.body_mut(id) {
        body.width = spawn.width;
        body.height = spawn.height;
    }
}

/// What every mob shares, as a client's copy sets it.
struct Parts<'a> {
    body: &'a mut Body,
    previous: &'a mut DVec3,
    yaw: &'a mut f32,
    head: Option<&'a mut LookControl>,
    health: &'a mut f32,
    damage: &'a mut DamageState,
    swings: Option<&'a mut u32>,
}

fn set_parts(parts: Parts<'_>, s: &MobState) {
    *parts.previous = parts.body.position;
    parts.body.position = DVec3::from_array(s.position);
    parts.body.on_ground = s.on_ground;
    parts.body.needs_sync = s.needs_sync;
    parts.body.fire_ticks = i32::from(s.burning);
    *parts.yaw = s.y_rot;
    if let Some(head) = parts.head {
        head.head_yaw = s.y_head_rot;
        head.pitch = s.x_rot;
    }
    *parts.health = s.health;
    parts.damage.hurts = s.hurts;
    parts.damage.dead = s.dead;
    if let Some(swings) = parts.swings {
        *swings = s.swings;
    }
}

/// Sets a mob of a client's copy to a state from the host.
fn set(world: &mut EntityWorld, s: &MobState, tick: i64) {
    let id = s.id;
    if let Some(e) = world.cow_mut(id) {
        set_parts(
            Parts {
                body: &mut e.cow.body,
                previous: &mut e.previous_position,
                yaw: &mut e.cow.yaw,
                head: Some(&mut e.look_control),
                health: &mut e.cow.health,
                damage: &mut e.cow.damage,
                swings: None,
            },
            s,
        );
        if let (
            Some(h),
            Extra::Horse {
                eat,
                stand,
                mouth,
                tail,
            },
        ) = (e.horse.as_mut(), s.extra)
        {
            h.eat_anim_o = h.eat_anim;
            h.stand_anim_o = h.stand_anim;
            h.mouth_anim_o = h.mouth_anim;
            h.eat_anim = eat;
            h.stand_anim = stand;
            h.mouth_anim = mouth;
            h.tail_counter = i32::from(tail);
        }
    } else if let Some(e) = world.sheep_mut(id) {
        set_parts(
            Parts {
                body: &mut e.body,
                previous: &mut e.previous_position,
                yaw: &mut e.yaw,
                head: Some(&mut e.look_control),
                health: &mut e.health,
                damage: &mut e.damage,
                swings: None,
            },
            s,
        );
        if let Extra::Sheep { eat } = s.extra {
            e.eat_animation_ticks = i32::from(eat);
        }
    } else if let Some(e) = world.pig_mut(id) {
        set_parts(
            Parts {
                body: &mut e.pig.body,
                previous: &mut e.previous_position,
                yaw: &mut e.pig.yaw,
                head: Some(&mut e.look_control),
                health: &mut e.pig.health,
                damage: &mut e.pig.damage,
                swings: None,
            },
            s,
        );
    } else if let Some(e) = world.chicken_mut(id) {
        set_parts(
            Parts {
                body: &mut e.chicken.body,
                previous: &mut e.previous_position,
                yaw: &mut e.chicken.yaw,
                head: Some(&mut e.look_control),
                health: &mut e.chicken.health,
                damage: &mut e.chicken.damage,
                swings: None,
            },
            s,
        );
    } else if let Some(e) = world.bat_mut(id) {
        set_parts(
            Parts {
                body: &mut e.bat.body,
                previous: &mut e.previous_position,
                yaw: &mut e.yaw,
                head: Some(&mut e.look_control),
                health: &mut e.bat.health,
                damage: &mut e.bat.damage,
                swings: None,
            },
            s,
        );
        if let Extra::Bat { resting } = s.extra {
            e.bat.resting = resting;
        }
    } else if let Some(e) = world.zombie_mut(id) {
        let ai = e.ai.as_deref_mut();
        set_parts(
            Parts {
                body: &mut e.zombie.body,
                previous: &mut e.previous_position,
                yaw: &mut e.yaw,
                head: Some(&mut e.look_control),
                health: &mut e.zombie.health,
                damage: &mut e.zombie.damage,
                swings: ai.map(|ai| &mut ai.state.melee.swings),
            },
            s,
        );
        if let Extra::Aggressive(on) = s.extra {
            e.aggressive = on;
        }
    } else if let Some(e) = world.skeleton_mut(id) {
        let ai = e.ai.as_deref_mut();
        set_parts(
            Parts {
                body: &mut e.skeleton.body,
                previous: &mut e.previous_position,
                yaw: &mut e.yaw,
                head: Some(&mut e.look_control),
                health: &mut e.skeleton.health,
                damage: &mut e.skeleton.damage,
                swings: ai.map(|ai| &mut ai.state.melee.swings),
            },
            s,
        );
        if let Extra::Aggressive(on) = s.extra {
            e.bow.aggressive = on;
        }
    } else if let Some(e) = world.creeper_mut(id) {
        set_parts(
            Parts {
                body: &mut e.creeper.body,
                previous: &mut e.previous_position,
                yaw: &mut e.creeper.yaw,
                head: Some(&mut e.ai.state.look_control),
                health: &mut e.creeper.health,
                damage: &mut e.creeper.damage,
                swings: None,
            },
            s,
        );
        if let Extra::Creeper { swell, exploded } = s.extra {
            e.creeper.old_swell = e.creeper.swell;
            e.creeper.swell = i32::from(swell);
            e.creeper.exploded = exploded;
        }
    } else if let Some(e) = world.spider_mut(id) {
        set_parts(
            Parts {
                body: &mut e.spider.body,
                previous: &mut e.previous_position,
                yaw: &mut e.spider.yaw,
                head: Some(&mut e.ai.state.look_control),
                health: &mut e.spider.health,
                damage: &mut e.spider.damage,
                swings: None,
            },
            s,
        );
    } else if let Some(e) = world.slime_mut(id) {
        set_parts(
            Parts {
                body: &mut e.slime.body,
                previous: &mut e.previous_position,
                yaw: &mut e.slime.yaw,
                head: Some(&mut e.ai.state.look_control),
                health: &mut e.slime.health,
                damage: &mut e.slime.damage,
                swings: None,
            },
            s,
        );
        if let Extra::Slime { squish } = s.extra {
            e.slime.previous_squish = e.slime.squish;
            e.slime.squish = squish;
        }
    } else if let Some(e) = world.enderman_mut(id) {
        let state = &mut e.ai.state;
        set_parts(
            Parts {
                body: &mut e.enderman.body,
                previous: &mut e.previous_position,
                yaw: &mut e.enderman.yaw,
                head: Some(&mut state.look_control),
                health: &mut e.enderman.health,
                damage: &mut e.enderman.damage,
                swings: Some(&mut state.melee.swings),
            },
            s,
        );
        if let Extra::Enderman { creepy } = s.extra {
            state.enderman.creepy = creepy;
        }
    } else if let Some(e) = world.witch_mut(id) {
        set_parts(
            Parts {
                body: &mut e.witch.body,
                previous: &mut e.previous_position,
                yaw: &mut e.witch.yaw,
                head: Some(&mut e.ai.state.look_control),
                health: &mut e.witch.health,
                damage: &mut e.witch.damage,
                swings: None,
            },
            s,
        );
    } else if let Some(e) = world.iron_golem_mut(id) {
        let state = &mut e.ai.state;
        set_parts(
            Parts {
                body: &mut e.golem.body,
                previous: &mut e.previous_position,
                yaw: &mut e.golem.yaw,
                head: Some(&mut state.look_control),
                health: &mut e.golem.health,
                damage: &mut e.golem.damage,
                swings: None,
            },
            s,
        );
        if let Extra::Golem { attack, flower } = s.extra {
            e.golem.attack_animation_tick = i32::from(attack);
            state.golem.offer_flower_tick = i32::from(flower);
        }
    } else if let Some(e) = world.wolf_mut(id) {
        let state = &mut e.ai.state;
        set_parts(
            Parts {
                body: &mut e.wolf.body,
                previous: &mut e.previous_position,
                yaw: &mut e.wolf.yaw,
                head: Some(&mut state.look_control),
                health: &mut e.wolf.health,
                damage: &mut e.wolf.damage,
                swings: None,
            },
            s,
        );
        if let Extra::Wolf {
            angry,
            sitting,
            wet,
            shake,
            interested,
        } = s.extra
        {
            state.enderman.anger_end_time = if angry { tick + 1_000_000 } else { i64::MIN };
            let w = &mut e.wolf;
            w.sitting = sitting;
            w.wet = wet;
            w.shake_anim_o = w.shake_anim;
            w.shake_anim = shake;
            w.interested_angle_o = w.interested_angle;
            w.interested_angle = interested;
        }
    } else if let Some(e) = world.villager_mut(id) {
        let (yaw, head) = match e.ai.as_deref_mut() {
            Some(ai) => (&mut ai.yaw, Some(&mut ai.look_control)),
            None => (&mut e.yaw, None),
        };
        set_parts(
            Parts {
                body: &mut e.villager.body,
                previous: &mut e.previous_position,
                yaw,
                head,
                health: &mut e.villager.health,
                damage: &mut e.villager.damage,
                swings: None,
            },
            s,
        );
        if let Extra::Villager { unhappy } = s.extra {
            e.unhappy = i32::from(unhappy);
        }
    }
}

/// Every mob of a world: id, kind, feet, health and whether it is dead (the
/// console's `mc mobs`).
pub fn census(world: &EntityWorld) -> Vec<(u64, &'static str, [f64; 3], f32, bool)> {
    host_mobs(world)
        .into_iter()
        .map(|mob| {
            (
                mob.spawn.id,
                mob.spawn.look.kind_name(),
                mob.state.position,
                mob.state.health,
                mob.state.dead,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summoned(world: &mut EntityWorld) -> Vec<u64> {
        use minecraftoss_entities as e;
        let at = |x: f64| DVec3::new(x, 64.0, 2.0);
        let mut zombie = e::zombie::Zombie::new(at(1.0));
        zombie.kind = e::zombie::ZombieKind::Husk;
        zombie.main_hand = Some("minecraft:iron_sword".into());
        zombie.armor[0] = Some("minecraft:leather_helmet".into());
        let mut wolf = e::wolf::Wolf::new(at(9.0));
        wolf.variant = "ashen".into();
        wolf.collar = 4;
        wolf.tame = true;
        let mut villager = e::villager::Villager::new(at(10.0));
        villager.profession = e::villager::Profession::Librarian;
        villager.level = 3;
        let mut pig = e::pig::Pig::new(at(3.0));
        pig.saddled = true;
        pig.variant = e::pig::PigVariant::Cold;
        let mut horse = e::horse::HorseState::new(e::horse::HorseKind::Horse);
        horse.variant = 0x0203;
        vec![
            world.spawn_zombie_active(zombie, 30.0),
            world.spawn_skeleton_active(e::skeleton::Skeleton::new(at(2.0)), 0.0),
            world.spawn_pig(pig, true),
            world.spawn_sheep(
                e::sheep::Sheep {
                    wool: e::sheep::Wool::from_data(5 | 16),
                    ..Default::default()
                },
                at(4.0),
                true,
            ),
            world.spawn_creeper(e::creeper::Creeper::new(at(5.0)), true),
            world.spawn_slime(e::slime::Slime::new(at(6.0), 4), true),
            world.spawn_enderman(e::enderman::Enderman::new(at(7.0)), true),
            world.spawn_iron_golem(e::iron_golem::IronGolem::new(at(8.0)), true),
            world.spawn_wolf(wolf, true),
            world.spawn_villager_active(villager, 45.0),
            world.spawn_horse(e::cow::Cow::new(at(11.0)), horse, true),
            world.spawn_mooshroom(
                e::mooshroom::MushroomCow::new(at(12.0), e::mooshroom::MushroomVariant::Brown),
                true,
            ),
            world.spawn_bat(e::bat::Bat::new(at(13.0)), true),
            world.spawn_witch(e::witch::Witch::new(at(14.0)), true),
            world.spawn_spider(e::spider::Spider::new(at(15.0)), true),
            world.spawn_chicken(e::chicken::Chicken::new(at(16.0)), true),
            world.spawn_cow(e::cow::Cow::new(at(17.0)), true),
        ]
    }

    /// A host world with one of every kind, and a client copy of it made
    /// through the wire.
    fn copied() -> (EntityWorld, EntityWorld, ClientCopy) {
        let mut host = EntityWorld::default();
        summoned(&mut host);
        let mobs = host_mobs(&host);
        let mut client = EntityWorld::default();
        let mut copy = ClientCopy::default();
        for mob in &mobs {
            let mut w = Writer::default();
            encode_spawn(&mut w, &mob.spawn);
            let spawn = decode_spawn(&mut Reader(&w.0)).unwrap();
            assert_eq!(spawn, mob.spawn);
            copy.spawn(&mut client, &spawn);
        }
        let frame = encode_frame(7, mobs.iter().map(|m| &m.state), &FrameExtras::default());
        let MobFrame { tick, states, .. } = decode_frame(&frame).unwrap();
        assert!(copy.apply(&mut client, tick, &states));
        (host, client, copy)
    }

    #[test]
    fn every_kind_comes_out_the_same_on_the_client() {
        let (host, client, _) = copied();
        let theirs = host_mobs(&host);
        let ours = host_mobs(&client);
        assert_eq!(ours.len(), theirs.len());
        assert_eq!(ours.len(), 17);
        for theirs in &theirs {
            let ours = ours
                .iter()
                .find(|m| m.spawn.id == theirs.spawn.id)
                .unwrap_or_else(|| panic!("{} missing", theirs.spawn.look.kind_name()));
            assert!(
                ours.spawn.same_look(&theirs.spawn),
                "{ours:?} != {theirs:?}"
            );
            for k in 0..3 {
                assert!((ours.state.position[k] - theirs.state.position[k]).abs() < 1e-3);
            }
        }
        // The trackers see the same mobs.
        let a = crate::client_mobs::server_mobs(&host);
        let b = crate::client_mobs::server_mobs(&client);
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(&b) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.baby, b.baby);
            assert!((a.eye_height - b.eye_height).abs() < 1e-4, "{a:?} {b:?}");
            assert_eq!(a.size, b.size);
        }
    }

    #[test]
    fn states_move_hurt_and_kill_the_client_copy() {
        let (host, mut client, mut copy) = copied();
        let mobs = host_mobs(&host);
        let husk = mobs
            .iter()
            .find(|m| m.spawn.look.kind_name() == "husk")
            .unwrap();
        let mut state = husk.state;
        state.position[0] += 2.5;
        state.y_rot = 90.0;
        state.y_head_rot = 120.0;
        state.hurts = 300;
        state.swings = 2;
        state.health = 7.5;
        state.extra = Extra::Aggressive(true);
        assert!(copy.apply(&mut client, 8, &[state]));
        // Counters travel as their low byte: the copy keeps counting on.
        state.hurts = 301;
        state.dead = true;
        assert!(copy.apply(&mut client, 9, &[state]));
        let ours = host_mobs(&client);
        let husk_now = ours.iter().find(|m| m.spawn.id == husk.spawn.id).unwrap();
        assert!((husk_now.state.position[0] - state.position[0]).abs() < 1e-3);
        assert!((husk_now.state.y_rot - 90.0).abs() < 1.5);
        assert!((husk_now.state.y_head_rot - 120.0).abs() < 1.5);
        assert_eq!(husk_now.state.health, 7.5);
        assert_eq!(husk_now.state.extra, Extra::Aggressive(true));
        assert!(husk_now.state.dead);
        // 300 went as 44, then 45: one more hit than it had.
        assert_eq!(husk_now.state.hurts, 45);
        assert_eq!(husk_now.state.swings, 2);
        // An older snapshot changes nothing.
        let mut old = state;
        old.position[0] += 50.0;
        assert!(!copy.apply(&mut client, 8, &[old]));
        // Gone, it leaves the world; the others stay.
        copy.gone(&mut client, &[husk.spawn.id]);
        let left = host_mobs(&client);
        assert_eq!(left.len(), mobs.len() - 1);
        assert!(left.iter().all(|m| m.spawn.id != husk.spawn.id));
    }

    #[test]
    fn new_looks_make_the_mob_again_with_its_id() {
        let (host, mut client, mut copy) = copied();
        let sheep = host_mobs(&host)
            .into_iter()
            .find(|m| m.spawn.look.kind_name() == "sheep")
            .unwrap();
        let mut shorn = sheep.spawn.clone();
        shorn.look = Look::Sheep { wool: 14 };
        assert!(!shorn.same_look(&sheep.spawn));
        copy.spawn(&mut client, &shorn);
        let ours = host_mobs(&client);
        assert_eq!(ours.len(), 17);
        let again = ours.iter().find(|m| m.spawn.id == sheep.spawn.id).unwrap();
        assert_eq!(again.spawn.look, Look::Sheep { wool: 14 });
        assert_eq!(copy.spawns().count(), 17);
    }

    #[test]
    fn a_frame_keeps_to_its_byte_limit_nearest_first() {
        let state = |id: u64| MobState {
            id: id * 1000,
            position: [id as f64, 64.0, -3.25],
            extra: Extra::Horse {
                eat: 1.0,
                stand: 0.5,
                mouth: 0.0,
                tail: 3,
            },
            ..Default::default()
        };
        let states: Vec<MobState> = (1..=1000).map(state).collect();
        let frame = encode_frame(1, &states, &FrameExtras::default());
        assert!(frame.len() <= MOB_FRAME_BYTES, "{}", frame.len());
        let decoded = decode_frame(&frame).unwrap().states;
        assert!(decoded.len() > 100, "{}", decoded.len());
        assert_eq!(frame_len(&frame), decoded.len());
        assert_eq!(frame_len(&[1]), 0);
        for (a, b) in decoded.iter().zip(&states) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.position, b.position);
        }
        // A full client view fits with room to spare.
        let frame = encode_frame(1, &states[..MAX_MOBS_PER_CLIENT], &FrameExtras::default());
        let decoded = decode_frame(&frame).unwrap().states;
        assert_eq!(decoded.len(), MAX_MOBS_PER_CLIENT);
        assert!(frame.len() < 1500, "{}", frame.len());
        // Damaged bytes are refused, not misread.
        assert!(decode_frame(&frame[..frame.len() - 1]).is_err());
        assert!(decode_frame(&[9, 0, 0, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn interest_is_the_nearest_within_range() {
        let mob = |id: u64, x: f64, z: f64| HostMob {
            spawn: MobSpawn {
                id,
                look: Look::Bat,
                baby: false,
                width: 0.5,
                height: 0.9,
                position: [x, 64.0, z],
                yaw: 0.0,
            },
            state: MobState {
                id,
                position: [x, 64.0, z],
                ..Default::default()
            },
        };
        let mut mobs = vec![mob(1, 500.0, 0.0), mob(2, 10.0, 0.0), mob(3, 0.0, 95.0)];
        // Height does not count: a mob far above is still near.
        mobs[1].state.position[1] = 300.0;
        assert_eq!(interest(&mobs, [0.0, 64.0, 0.0]), vec![1, 2]);
        let crowd: Vec<HostMob> = (0..200)
            .map(|i| mob(i, f64::from(i as u32) * 0.3, 0.0))
            .collect();
        let near = interest(&crowd, [0.0, 64.0, 0.0]);
        assert_eq!(near.len(), MAX_MOBS_PER_CLIENT);
        assert_eq!(near[0], 0);
        assert!(near.iter().all(|&i| i < MAX_MOBS_PER_CLIENT));
    }

    #[test]
    fn spawn_rows_keep_to_their_byte_limit() {
        let (host, _, _) = copied();
        let mobs = host_mobs(&host);
        let spawns: Vec<MobSpawn> = (0..40)
            .flat_map(|_| mobs.iter().map(|m| m.spawn.clone()))
            .collect();
        let mut rest = &spawns[..];
        let mut rows = 0;
        while !rest.is_empty() {
            let n = spawns_that_fit(rest);
            assert!(n >= 1);
            let mut w = Writer::default();
            for spawn in &rest[..n] {
                encode_spawn(&mut w, spawn);
            }
            assert!(w.0.len() <= SPAWN_BYTES_PER_MESSAGE, "{}", w.0.len());
            rest = &rest[n..];
            rows += 1;
        }
        assert!(rows > 1);
        let mut w = Writer::default();
        encode_ids(&mut w, &[1, 300, u64::MAX]);
        assert_eq!(
            decode_ids(&mut Reader(&w.0)).unwrap(),
            vec![1, 300, u64::MAX]
        );
    }

    fn sound(event: &str, x: f64, volume: f32) -> WorldEvent {
        WorldEvent::Sound {
            event: event.into(),
            position: [x, 64.5, -2.25],
            volume,
            pitch: 1.25,
        }
    }

    #[test]
    fn events_projectiles_and_effects_come_through_the_frame() {
        let extras = FrameExtras {
            events: vec![
                EventRow {
                    seq: u32::MAX,
                    event: sound("entity.husk.ambient", 3.5, 1.0),
                },
                EventRow {
                    seq: 1,
                    event: WorldEvent::Splash {
                        position: [1.0, 65.0, 2.0],
                        color: 8889187,
                        instant: false,
                    },
                },
                EventRow {
                    seq: 2,
                    event: WorldEvent::Explosion {
                        position: [-4.0, 63.0, 9.5],
                        radius: 3.0,
                    },
                },
            ],
            projectiles: vec![
                Projectile {
                    id: 77,
                    kind: ProjectileKind::Arrow { tipped: true },
                    position: [0.5, 70.25, -1.0],
                    y_rot: 45.0,
                    x_rot: -11.25,
                },
                Projectile {
                    id: 78,
                    kind: ProjectileKind::Potion { color: 0x80A0C0 },
                    position: [2.0, 66.0, 3.0],
                    y_rot: 0.0,
                    x_rot: 90.0,
                },
            ],
            effects: vec![PlayerEffect {
                effect: MobEffect::Poison,
                amplifier: 1,
                duration: 337,
            }],
        };
        let state = MobState {
            id: 5,
            position: [1.0, 64.0, 1.0],
            ..Default::default()
        };
        let frame = decode_frame(&encode_frame(12, [&state], &extras)).unwrap();
        assert_eq!(frame.tick, 12);
        assert_eq!(frame.states.len(), 1);
        assert_eq!(frame.extras, extras);
        // The extras keep to their share and the states fill the rest.
        let many = FrameExtras {
            events: (0..200)
                .map(|seq| EventRow {
                    seq,
                    event: sound("entity.zombie_villager.ambient", 0.0, 1.0),
                })
                .collect(),
            projectiles: vec![extras.projectiles[1]; 100],
            effects: extras.effects.clone(),
        };
        let states: Vec<MobState> = (0..200).map(|id| MobState { id, ..state }).collect();
        let bytes = encode_frame(13, &states, &many);
        assert!(bytes.len() <= MOB_FRAME_BYTES, "{}", bytes.len());
        let frame = decode_frame(&bytes).unwrap();
        assert!(frame.extras.events.len() <= MAX_EVENTS_PER_FRAME);
        assert_eq!(
            frame.extras.events[..],
            many.events[..frame.extras.events.len()]
        );
        assert!(frame.extras.projectiles.len() <= MAX_PROJECTILES_PER_FRAME);
        assert_eq!(frame.extras.effects, extras.effects);
        assert!(frame.states.len() > 50, "{}", frame.states.len());
    }

    #[test]
    fn an_event_reaches_once_through_a_lost_snapshot() {
        let mut log = EventLog::default();
        let mut window = EventWindow::default();
        let mut copy = ClientCopy::default();
        let mut world = EntityWorld::default();
        let feet = [0.0, 64.0, 0.0];
        let mut heard = Vec::new();
        for tick in 1..=10u32 {
            log.age(tick);
            log.push(tick, sound(&format!("tick{tick}"), 2.0, 1.0));
            // Too far to hear: a quiet sound 20 blocks off.
            log.push(tick, sound("far", 20.0, 1.0));
            // A slow host sends a snapshot every other tick.
            if tick % 2 == 1 {
                continue;
            }
            let rows = window.next(&log, feet);
            assert!(rows.len() <= 2 * usize::from(EVENT_SENDS));
            // The snapshot of tick 4 is lost on the way.
            if tick == 4 {
                continue;
            }
            assert!(copy.apply(&mut world, tick, &[]));
            heard.extend(copy.fresh_events(&rows));
        }
        let names: Vec<String> = heard
            .iter()
            .map(|e| match e {
                WorldEvent::Sound { event, .. } => event.clone(),
                other => panic!("{other:?}"),
            })
            .collect();
        let want: Vec<String> = (1..=10).map(|t| format!("tick{t}")).collect();
        assert_eq!(names, want);
        // A loud sound and a blast carry further.
        assert!(sound("entity.generic.explode", 60.0, 4.0).reaches(feet));
        assert!(!sound("entity.husk.step", 17.0, 0.15).reaches(feet));
        let blast = WorldEvent::Explosion {
            position: [0.0, 64.0, 63.0],
            radius: 3.0,
        };
        assert!(blast.reaches(feet));
    }

    #[test]
    fn projectiles_fly_and_land_on_the_client() {
        use minecraftoss_entities::potion::{Potion, ThrownPotion};
        let mut host = EntityWorld::default();
        let mut random = minecraftoss_player::rng::LegacyRandom::new(5);
        let thrown = ThrownPotion::shoot(
            Potion::Poison,
            9,
            DVec3::new(0.0, 66.0, 0.0),
            DVec3::new(0.0, 0.2, 1.0),
            0.75,
            8.0,
            &mut random,
        );
        let id = host.spawn_potion(thrown);
        let mut rotations = std::collections::HashMap::new();
        let all = host_projectiles(&host, &mut rotations);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, id);
        assert_eq!(
            all[0].kind,
            ProjectileKind::Potion {
                color: Potion::Poison.color()
            }
        );
        assert!(projectiles_near(&all, [100.0, 64.0, 0.0]).is_empty());
        let mut client = ClientProjectiles::default();
        client.receive(&all);
        let mut moved = all[0];
        moved.position[2] += 1.0;
        client.receive(&[moved]);
        let drawn = client.drawn(0.5);
        assert!((drawn[0].1.z - (all[0].position[2] + 0.5)).abs() < 1e-9);
        client.receive(&[]);
        assert!(client.drawn(0.0).is_empty());
        assert_eq!(client.seen(), [("potion", 1)]);
    }

    #[test]
    fn a_players_effects_work_as_on_a_vanilla_player() {
        use minecraftoss_entities::world::PlayerSplash;
        let mut effects = PlayerEffects::default();
        let poison = PlayerSplash {
            effect: MobEffect::Poison,
            amplifier: 0,
            duration: 60,
            instant: None,
            at: DVec3::ZERO,
        };
        assert_eq!(effects.splash(3, &poison), None);
        assert_eq!(effects.marks(3), (false, true, false));
        assert_eq!(effects.of(3)[0].duration, 60);
        // Poison bites every 25 ticks, and never below 1 health.
        let mut bites = 0;
        for tick in 0..60 {
            let health = if tick < 30 { 20.0 } else { 1.0 };
            bites += effects.tick(|_| health).len();
        }
        assert_eq!(bites, 1);
        assert!(effects.of(3).is_empty());
        let harming = PlayerSplash {
            effect: MobEffect::InstantDamage,
            amplifier: 0,
            duration: 0,
            instant: Some(EffectWork::HurtMagic(5.0)),
            at: DVec3::ZERO,
        };
        assert_eq!(effects.splash(3, &harming), Some(5.0));
        effects.add(4, MobEffect::Slowness, 0, 100);
        effects.add(4, MobEffect::Weakness, 0, 100);
        assert!((effects.speed(4) - 0.85).abs() < 1e-6);
        assert_eq!(effects.attack_damage(4), -4.0);
        assert_eq!(effects.speed(3), 1.0);
        effects.arrow(5, minecraftoss_player::survival::EffectKind::Slowness, 200);
        assert_eq!(effects.marks(5), (true, false, false));
        effects.clear(4);
        assert!(effects.of(4).is_empty());
    }
}
