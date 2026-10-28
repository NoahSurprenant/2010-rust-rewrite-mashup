//! The Minecraft map at run time: MinecraftOSS generates and streams a seeded
//! world around the local player, whose chunks become block collision and
//! whose section meshes go to the renderer. The world sits with its player
//! spawn at map origin, a block to `sim::voxel::BLOCK` map units.
use std::collections::HashMap;
use std::sync::{Arc, mpsc};

use bevy::input::gamepad::GamepadButton;
use bevy::prelude::*;
use minecraft_terrain::clouds::CloudMask;
use minecraft_terrain::day_cycle::{DayCycle, Skybox};
use minecraft_terrain::environment::{DimensionEnvironment, View};
use minecraft_terrain::lighting::SkyLight;
use minecraft_terrain::mesh::{Atlas, SectionMesh, Vertex};
use minecraft_terrain::pack::PackStack;
use minecraft_terrain::scene::HandcraftedScene;
use minecraft_terrain::sections::{CullCamera, SectionPos};
use minecraft_terrain::terrain::{Dimension, TerrainStream};
use minecraftoss_core::BlockStateId;
use minecraftoss_core::registries::{DataPaths, Registries};

const VIEW_DISTANCE: i32 = 8;
const TICK_SECONDS: f64 = 1.0 / 20.0;
/// Blocks on a side of the light volume MW2 models are lit from.
pub const LIGHT_VOLUME: i32 = 64;
/// Chunk sections fade in over this long, as the viewer's default option.
const FADE_MILLIS: u64 = 750;

/// What the renderer takes from the world each frame.
#[derive(Resource, Default)]
pub struct MinecraftWorldView {
    /// A Minecraft map is loaded: the stand-in map's world is not drawn.
    pub active: bool,
    /// Block point at map origin.
    pub origin: [f64; 3],
    pub atlas: Option<Arc<Atlas>>,
    pub uploads: Vec<(SectionPos, SectionMesh)>,
    pub removed: Vec<SectionPos>,
    pub visible: Vec<(SectionPos, f32)>,
    /// Bumped when the world is replaced, so stale sections are dropped.
    pub generation: u64,
    /// The environment uniform of MinecraftOSS for this frame, in block space.
    pub environment: [[f32; 4]; 16],
    /// Sun and the eight moon phases, 32 pixels each, side by side.
    pub celestial: Option<Arc<image::RgbaImage>>,
    pub clouds: Option<Arc<(Vec<Vertex>, Vec<u32>)>>,
    /// Sky and block light around the player: origin block, then
    /// `LIGHT_VOLUME` cubed pairs, x fastest then z then y.
    pub light_volume: Option<Arc<([i32; 3], Vec<u8>)>>,
    /// Sky and block light at the eye, for the view model.
    pub eye_light: [f32; 2],
    /// Break particles as section vertices and indices, rebuilt each frame.
    pub particles: (Vec<u8>, Vec<u32>),
    /// Destroy stage cubes over blocks being mined: position, strip uv.
    pub cracks: (Vec<[f32; 5]>, Vec<u32>),
    /// The ten destroy stages side by side.
    pub crack_texture: Option<Arc<image::RgbaImage>>,
    /// Mob models (cut out, back-face culled, translucent) and entity
    /// shadows, as `mesh::Vertex` bytes and indices.
    pub entity_meshes: [(Vec<u8>, Vec<u32>); 4],
    /// The black card behind the inventory's character, in blocks.
    pub backdrop: Option<[[f32; 3]; 4]>,
    /// The first-person hand or held item: section vertex bytes in view
    /// space (x right, y up, z back) and indices, with the projection that
    /// draws them (vanilla's fixed 70 degree hand field of view).
    pub hand: (Vec<u8>, Vec<u32>),
    pub hand_clip: [f32; 16],
}

struct Loaded {
    stream: TerrainStream,
    scene: HandcraftedScene,
    packs: PackStack,
    atlas: Arc<Atlas>,
    registries: Arc<Registries>,
    seed: i64,
    /// The block at map origin: the stream's player spawn, or the host's.
    origin: [f64; 3],
    environment: DimensionEnvironment,
    celestial: Arc<image::RgbaImage>,
    cloud_mask: Option<CloudMask>,
    crack_texture: Arc<image::RgbaImage>,
    /// The mobs and items, and the block sounds: built with the world, off
    /// the main thread (a client builds them mid-match).
    entities: Option<crate::minecraft_entities::Entities>,
    sounds: Option<crate::minecraft_sounds::Sounds>,
}

/// The hand's swing and the timers of mining and placing by hand.
#[derive(Default)]
struct HandState {
    /// Ticks into a swing, while one runs.
    swing: Option<f32>,
    /// Seconds towards the next hand-mining and placing tick.
    clock: f64,
    /// Ticks until another placement while the button is held
    /// (`rightClickDelay`).
    place_delay: u32,
}

/// The player's walk, for vanilla's step and fall sounds.
#[derive(Default)]
struct StepState {
    last: Option<[f64; 3]>,
    /// `Entity.moveDist` and `nextStep`.
    move_dist: f32,
    next_step: f32,
    /// The highest point since leaving the ground.
    air_peak: Option<f64>,
}

#[derive(Default)]
struct Runtime {
    loading: Option<mpsc::Receiver<Result<Loaded, String>>>,
    world: Option<Loaded>,
    day: DayCycle,
    environment_accumulator: f64,
    environment_primed: bool,
    light: Option<SkyLight>,
    light_volume_at: Option<[i32; 3]>,
    light_volume_age: u32,
    cloud_center: Option<(i32, i32)>,
    mining: crate::minecraft_mining::Mining,
    minimap: crate::minecraft_minimap::Minimap,
    hand: HandState,
    steps: StepState,
    sounds: Option<crate::minecraft_sounds::Sounds>,
    inventory_ui: crate::minecraft_inventory::InventoryUi,
    entities: Option<crate::minecraft_entities::Entities>,
    /// Shape id of each block state already seen.
    shapes: HashMap<BlockStateId, u16>,
    /// Boxes of each shape id, to reuse an id for a repeated shape.
    shape_ids: HashMap<Vec<[u32; 6]>, u16>,
    was_alive: bool,
    /// Frames in a row the player has stood still on the spawn since the move there.
    settled: u32,
    /// Host, client, or neither.
    net: crate::minecraft_mp::NetRole,
    /// Edits for chunks not generated yet, applied when each arrives.
    deferred: HashMap<(i32, i32), Vec<((i32, i32, i32), Option<minecraft_terrain::scene::Block>)>>,
    /// How many streams hold each chunk of the block world (the player's,
    /// and on a host each client's collision stream).
    voxel_holders: HashMap<(i32, i32), u32>,
    /// How many streams hold each chunk of the mobs' level (the player's,
    /// and on a host each client's mob stream).
    level_holders: HashMap<(i32, i32), u32>,
    /// The movement speed share each player's effects last set, by client.
    speed_applied: HashMap<u32, f32>,
}

/// Frames standing still on the spawn before the player counts as in the world.
const SETTLE_FRAMES: u32 = 10;

/// The player's MW2 body stands in the inventory's character window, on a
/// black card: placed from this frame's camera and projection where the
/// window shows, facing the camera, turned and aiming towards the mouse,
/// and drawn with the card in the view model's depth band so no wall comes
/// between.
pub(crate) fn place_inventory_puppet(
    ui: Res<frame::MinecraftUi>,
    mut puppet: ResMut<frame::InventoryPuppet>,
    mut view: ResMut<MinecraftWorldView>,
    local: Res<net::LocalPresentClient>,
    presented: Res<net::PresentedSnapshot>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    cameras: Query<&Transform, With<render_scene::FlyCamera>>,
    lenses: Query<&Projection, With<render_scene::FpvLens>>,
) {
    use std::sync::atomic::Ordering::Relaxed;
    puppet.active = false;
    view.backdrop = None;
    render_frame::DEPTH_HACK_SCENE_ENTNUM.store(u32::MAX, Relaxed);
    let alive = presented.player(local.0).is_some_and(|ps| ps.pm_type == 0);
    if !(ui.active && ui.inventory_open && alive) {
        return;
    }
    let (Some([cx, cy, box_w, box_h]), Ok(window), Ok(camera), Ok(Projection::Perspective(lens))) =
        (ui.character_box, windows.single(), cameras.single(), lenses.single())
    else {
        return;
    };
    let (w, h) = (window.width().max(1.0), window.height().max(1.0));
    let tan_v = (lens.fov * 0.5).tan();
    let tan_h = tan_v * if lens.aspect_ratio > 1e-3 { lens.aspect_ratio } else { w / h };
    let (eye, fwd, right, up) = (camera.translation, *camera.forward(), *camera.right(), *camera.up());
    // A point `distance` along the view at a window pixel.
    let at = |px: f32, py: f32, distance: f32| {
        let (nx, ny) = (px / w * 2.0 - 1.0, 1.0 - py / h * 2.0);
        eye + (fwd + right * (nx * tan_h) + up * (ny * tan_v)) * distance
    };
    let distance = 10.0;
    let world_h = box_h / h * 2.0 * tan_v * distance;
    // A standing MW2 player is about 72 units: most of the window.
    let scale = world_h * 0.82 / 72.0;
    let feet = at(cx, cy + box_h * 0.5, distance) + up * (world_h * 0.07);
    // Turned by the mouse as vanilla's
    // `InventoryScreen.renderEntityInInventoryFollowsMouse` turns its body.
    let turn = (ui.gaze[0] * 1.2).atan() * 0.7;
    let x_axis = (-fwd * turn.cos() + right * turn.sin()).normalize_or(-fwd);
    let y_axis = up.cross(x_axis);
    puppet.root = Mat4::from_cols(
        (x_axis * scale).extend(0.0),
        (y_axis * scale).extend(0.0),
        (up * scale).extend(0.0),
        feet.extend(1.0),
    );
    puppet.pitch = (ui.gaze[1] * 1.2).atan().to_degrees() * 0.6;
    puppet.client = local.0.0;
    puppet.active = true;
    render_frame::DEPTH_HACK_SCENE_ENTNUM.store(local.0.0, Relaxed);
    // The card: the window and a margin the panel covers, behind it.
    let corner = |dx: f32, dy: f32| {
        let p = at(cx + dx * box_w * 0.6, cy + dy * box_h * 0.6, distance * 1.4);
        let b = sim::voxel::to_block(view.origin, p.to_array());
        [b[0] as f32, b[1] as f32, b[2] as f32]
    };
    view.backdrop = Some([corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)]);
}

pub(crate) fn register(app: &mut App) {
    app.add_systems(
        Update,
        place_inventory_puppet
            .after(crate::sync_camera_from_presented)
            .after(frame::PresentedPublished)
            .in_set(frame::ClientSet::Present),
    );
    app.init_resource::<MinecraftWorldView>()
        .init_resource::<frame::MinecraftUi>()
        .init_resource::<frame::InventoryPuppet>()
        .insert_non_send(Runtime::default())
        .add_systems(
            Update,
            update
                .after(frame::PresentedPublished)
                .in_set(frame::ClientSet::Present),
        );
}

/// The world at `seed`; with `world`, its chunks load from and save to that
/// directory, and with `origin` that block sits at map origin rather than
/// the spawn the stream finds; a world that never `draws` only generates,
/// edits and saves.
fn load(seed: i64, world: Option<std::path::PathBuf>, origin: Option<[f64; 3]>, draws: bool) -> Result<Loaded, String> {
    let root = assets::minecraft_map::root().ok_or_else(assets::minecraft_setup::status)?;
    let paths = DataPaths::under(&root);
    let registries = Arc::new(Registries::load(&paths)?);
    let packs = PackStack::open(vec![root.join("resourcepacks/local/minecraft-26.3")])
        .map_err(|e| e.to_string())?;
    if let Some(dir) = &world {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut stream = TerrainStream::for_dimension(
        registries.clone(),
        seed,
        view_distance(),
        Dimension::Overworld,
        world.as_deref(),
    )
    .map_err(|e| e.to_string())?;
    if !draws {
        stream.without_drawing();
    }
    let origin = origin.unwrap_or_else(|| {
        if let Some(spawn) = crate::minecraft_settings::spawn() {
            diag::info!(World, "Minecraft spawn: {spawn:?}, set by {}", crate::minecraft_settings::SPAWN_ENV);
            return spawn;
        }
        let (x, y, z) = stream.player_spawn;
        [x, y, z]
    });
    let build = minecraft_terrain::mesh::build(&HandcraftedScene::default(), &packs)
        .map_err(|e| e.to_string())?;
    let scene = HandcraftedScene::streamed(stream.states.clone());
    let environment =
        DimensionEnvironment::load(&registries, Dimension::Overworld.dimension_type())?;
    let celestial = Arc::new(celestial_image(&packs).map_err(|e| e.to_string())?);
    let cloud_mask = CloudMask::from_pack(&packs).ok();
    let crack_texture = Arc::new(crate::minecraft_mining::crack_strip(&packs).map_err(|e| e.to_string())?);
    let entities = Some(crate::minecraft_entities::Entities::new(&stream, seed));
    let sounds = draws.then(|| crate::minecraft_sounds::Sounds::load(&packs));
    Ok(Loaded {
        stream,
        scene,
        packs,
        atlas: build.atlas,
        registries,
        seed,
        origin,
        environment,
        celestial,
        cloud_mask,
        crack_texture,
        entities,
        sounds,
    })
}

/// The sky's sun and moon phases, laid out as MinecraftOSS lays them out.
fn celestial_image(packs: &PackStack) -> anyhow::Result<image::RgbaImage> {
    let mut celestial = image::RgbaImage::new(32 * 9, 32);
    let names = [
        "environment/celestial/sun",
        "environment/celestial/moon/full_moon",
        "environment/celestial/moon/waning_gibbous",
        "environment/celestial/moon/third_quarter",
        "environment/celestial/moon/waning_crescent",
        "environment/celestial/moon/new_moon",
        "environment/celestial/moon/waxing_crescent",
        "environment/celestial/moon/first_quarter",
        "environment/celestial/moon/waxing_gibbous",
    ];
    for (index, path) in names.iter().enumerate() {
        let id = minecraft_terrain::pack::ResourceId::parse(&format!("minecraft:{path}"))?;
        if let Some(bytes) = packs.texture(&id)? {
            let img =
                image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)?.to_rgba8();
            let tile =
                image::imageops::resize(&img, 32, 32, image::imageops::FilterType::Nearest);
            image::imageops::replace(&mut celestial, &tile, (index as i64) * 32, 0);
        }
    }
    Ok(celestial)
}

/// A controller trigger, held or just pressed: with a block or an empty hand
/// the right trigger mines and the left places, as the mouse buttons do.
fn is_crafting_table(scene: &HandcraftedScene, pos: (i32, i32, i32)) -> bool {
    minecraft_terrain::scene::Scene::block(scene, pos)
        .is_some_and(|block| block.id.namespace == "minecraft" && block.id.path == "crafting_table")
}

/// The crafting table the player looks at, within survival reach.
fn table_in_reach(scene: &HandcraftedScene, feet: [f64; 3], yaw: f32, pitch: f32) -> Option<(i32, i32, i32)> {
    let mut player = minecraftoss_player::Player::new(glam::DVec3::from_array(feet));
    player.yaw = f64::from(yaw);
    player.pitch = f64::from(pitch);
    player.target(scene, 4.5).map(|hit| hit.pos).filter(|&pos| is_crafting_table(scene, pos))
}

fn open_table(ui: &mut frame::MinecraftUi, hand: &mut HandState, pos: (i32, i32, i32)) {
    hand.swing = Some(0.0);
    ui.inventory_open = true;
    ui.workbench = Some(pos);
}

fn pad_trigger(pad: Option<&bevy::input::gamepad::Gamepad>, button: GamepadButton, just: bool) -> bool {
    pad.is_some_and(|pad| if just { pad.just_pressed(button) } else { pad.pressed(button) })
}

fn seed() -> i64 {
    if let Some(seed) = std::env::var("IW4L_MINECRAFT_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        return seed;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    (nanos as i64) ^ 0x5DEE_CE66_D1CE_4E5B
}

/// How far a client may reach to break a block: survival's 4.5 blocks, and
/// room for the host seeing it a little late.
const REMOTE_REACH: f64 = 6.5;
/// The most blocks one console box sets, as the console allows.
const FILL_MAX: i64 = 4096;
/// Chunks a host generates around each client for its collision.
const CLIENT_COLLISION_DISTANCE: i32 = 3;
/// Chunks a host's mobs live in around each client: the simulation
/// distance, as around the host.
const CLIENT_MOB_DISTANCE: i32 = crate::minecraft_entities::SIMULATION_DISTANCE;

/// Whether a block at `pos` would stand in the box of a player at `feet`.
fn inside_player([fx, fy, fz]: [f64; 3], pos: (i32, i32, i32)) -> bool {
    (fx - 0.3) < f64::from(pos.0 + 1)
        && (fx + 0.3) > f64::from(pos.0)
        && fy < f64::from(pos.1 + 1)
        && (fy + 1.8) > f64::from(pos.1)
        && (fz - 0.3) < f64::from(pos.2 + 1)
        && (fz + 0.3) > f64::from(pos.2)
}

/// Where a host's `nth` client stands on arrival, in map units: three
/// blocks apart along +Z from the world spawn (so no two players share the
/// spot), on the ground of that column near the spawn's height.
fn client_spawn(scene: &HandcraftedScene, origin: [f64; 3], nth: usize) -> [f32; 3] {
    use minecraft_terrain::scene::Scene;
    let (x, z) = (origin[0].floor() as i32, origin[2].floor() as i32 + 3 * nth as i32);
    let base = origin[1].floor() as i32;
    let ground = (base - 8..=base + 8).rev().find(|&y| {
        scene.block((x, y - 1, z)).is_some() && scene.block((x, y, z)).is_none() && scene.block((x, y + 1, z)).is_none()
    });
    let feet = ground.map_or(origin[1], f64::from);
    sim::voxel::to_map(origin, [origin[0], feet, origin[2] + 3.0 * nth as f64])
}

/// The lowest feet height at or above `feet` where a player fits (two
/// blocks without collision), within sixteen blocks; `feet` itself if it
/// fits.
fn clear_feet(world: &Loaded, feet: [f64; 3]) -> Option<f64> {
    use minecraft_terrain::scene::Scene;
    let (x, y, z) = (feet[0].floor() as i32, feet[1].floor() as i32, feet[2].floor() as i32);
    let solid = |y: i32| {
        world.scene.block((x, y, z)).and_then(|b| world.stream.states.state_of(b)).is_some_and(|state| !world.registries.blocks.collision_boxes(state).is_empty())
    };
    let fits = |y: i32| !solid(y) && !solid(y + 1);
    if fits(y) {
        return Some(feet[1]);
    }
    (y + 1..=y + 16).find(|&y| fits(y)).map(f64::from)
}

/// Chunks streamed around the player: `IW4L_MINECRAFT_VIEW_DISTANCE` (2 to
/// 32), else `VIEW_DISTANCE`.
fn view_distance() -> i32 {
    std::env::var("IW4L_MINECRAFT_VIEW_DISTANCE")
        .ok()
        .and_then(|v| v.trim().parse::<i32>().ok())
        .map_or(VIEW_DISTANCE, |v| v.clamp(2, 32))
}

/// The collision shape id of a block state, added to the block world's
/// table the first time a shape is seen.
fn shape_of(
    shapes: &mut HashMap<BlockStateId, u16>,
    shape_ids: &mut HashMap<Vec<[u32; 6]>, u16>,
    registries: &Registries,
    state: BlockStateId,
) -> u16 {
    *shapes.entry(state).or_insert_with(|| {
        let boxes = registries.blocks.collision_boxes(state);
        if boxes.is_empty() {
            return 0;
        }
        let key: Vec<[u32; 6]> = boxes.iter().map(|b| b.map(|v| (v as f32).to_bits())).collect();
        if let Some(&id) = shape_ids.get(&key) {
            return id;
        }
        let boxes32 = boxes.iter().map(|b| b.map(|v| v as f32)).collect();
        let id = sim::voxel::add_shapes(vec![boxes32]).unwrap_or(0);
        shape_ids.insert(key, id);
        id
    })
}

/// A generated chunk column's collision shape ids, x fastest then z then y.
fn chunk_shapes(
    chunk: &minecraftoss_core::Chunk,
    shapes: &mut HashMap<BlockStateId, u16>,
    shape_ids: &mut HashMap<Vec<[u32; 6]>, u16>,
    registries: &Registries,
) -> Vec<u16> {
    let (min_y, height) = (chunk.min_y(), chunk.height());
    let mut ids = vec![0u16; (height * 256) as usize];
    // Runs of one state (air, stone) skip the map.
    let mut last = None;
    for y in 0..height {
        for z in 0..16usize {
            for x in 0..16usize {
                let state = chunk.block(x, min_y + y, z);
                if let Some((last_state, id)) = last
                    && last_state == state
                {
                    ids[((y as usize * 16) + z) * 16 + x] = id;
                    continue;
                }
                let id = shape_of(shapes, shape_ids, registries, state);
                last = Some((state, id));
                ids[((y as usize * 16) + z) * 16 + x] = id;
            }
        }
    }
    ids
}

/// A chunk column's collision shapes: its generated blocks, with the
/// scene's edits over them.
fn chunk_collision(
    chunk: &minecraftoss_core::Chunk,
    world: &Loaded,
    shapes: &mut HashMap<BlockStateId, u16>,
    shape_ids: &mut HashMap<Vec<[u32; 6]>, u16>,
) -> Vec<u16> {
    let (min_y, height) = (chunk.min_y(), chunk.height());
    let mut ids = chunk_shapes(chunk, shapes, shape_ids, &world.registries);
    let (placed, cleared) = world.scene.chunk_edits((chunk.pos.x, chunk.pos.z));
    let index = |(x, y, z): (i32, i32, i32)| {
        let ly = y - min_y;
        (0..height).contains(&ly).then(|| ((ly * 16 + (z & 15)) * 16 + (x & 15)) as usize)
    };
    for (pos, block) in placed.iter().flat_map(|placed| placed.iter()) {
        if let Some(i) = index(*pos) {
            let state = world.stream.states.state_of(block);
            ids[i] = state.map_or(0, |state| shape_of(shapes, shape_ids, &world.registries, state));
        }
    }
    for pos in cleared.iter().flat_map(|cleared| cleared.iter()) {
        if let Some(i) = index(*pos) {
            ids[i] = 0;
        }
    }
    ids
}

/// The hotbar slot (from 1) that holds the weapon `weapon`
/// (`iw4:weapon/<weapon>`), for the console's mob census.
fn held_slot(hotbar: &[Option<minecraftoss_player::inventory::ItemStack>], weapon: u32) -> Option<usize> {
    hotbar
        .iter()
        .position(|stack| {
            stack
                .as_ref()
                .and_then(|stack| stack.id.strip_prefix("iw4:weapon/"))
                .and_then(|id| id.parse::<u32>().ok())
                == Some(weapon)
        })
        .map(|index| index + 1)
}

/// The positions the host's log changed in chunk `pos`, each once.
fn logged_in(
    log: &minecraft_terrain::multiplayer::EditLog,
    pos: (i32, i32),
) -> std::collections::BTreeSet<(i32, i32, i32)> {
    log.entries().iter().map(|(at, _)| *at).filter(|at| (at.0 >> 4, at.2 >> 4) == pos).collect()
}

/// One more stream holds a chunk; true for the first.
fn hold(holders: &mut HashMap<(i32, i32), u32>, pos: (i32, i32)) -> bool {
    let count = holders.entry(pos).or_default();
    *count += 1;
    *count == 1
}

/// One stream lets a chunk of the mobs' level go; true for the last.
fn release_level(holders: &mut HashMap<(i32, i32), u32>, pos: (i32, i32)) -> bool {
    let Some(count) = holders.get_mut(&pos) else {
        return false;
    };
    *count = count.saturating_sub(1);
    if *count > 0 {
        return false;
    }
    holders.remove(&pos);
    true
}

/// A chunk no player has about it any more: the mobs' level lets it go,
/// and a host's maps no longer read it.
fn forget_level_chunk(
    net_role: &mut crate::minecraft_mp::NetRole,
    entities: Option<&mut crate::minecraft_entities::Entities>,
    pos: (i32, i32),
) {
    if let crate::minecraft_mp::NetRole::Host(host) = net_role {
        host.columns.remove(pos);
    }
    if let Some(entities) = entities {
        entities.unload_chunk(minecraftoss_core::ChunkPos::new(pos.0, pos.1));
    }
}

/// One stream lets a chunk of the block world go; the last one to removes
/// it, and says so.
fn release_chunk(holders: &mut HashMap<(i32, i32), u32>, pos: (i32, i32)) -> bool {
    let Some(count) = holders.get_mut(&pos) else {
        return false;
    };
    *count = count.saturating_sub(1);
    if *count > 0 {
        return false;
    }
    holders.remove(&pos);
    sim::voxel::remove_chunk(pos.0, pos.1);
    true
}

/// Blocks set from the network or the host's clients: the scene, collision
/// and the chunk map take each; one in a chunk not generated yet is also
/// kept to set again when that chunk arrives (an air block only clears
/// generated terrain). Returns the positions set.
fn set_blocks(
    world: &mut Loaded,
    shapes: &mut HashMap<BlockStateId, u16>,
    shape_ids: &mut HashMap<Vec<[u32; 6]>, u16>,
    deferred: &mut HashMap<(i32, i32), Vec<((i32, i32, i32), Option<minecraft_terrain::scene::Block>)>>,
    changes: Vec<((i32, i32, i32), Option<minecraft_terrain::scene::Block>)>,
) -> Vec<(i32, i32, i32)> {
    let mut positions = Vec::with_capacity(changes.len());
    for (pos, block) in changes {
        let chunk = (pos.0 >> 4, pos.2 >> 4);
        if world.scene.generated_chunk(chunk).is_none() {
            deferred.entry(chunk).or_default().push((pos, block.clone()));
        }
        let state = block.as_ref().and_then(|b| world.stream.states.state_of(b));
        let shape = state.map_or(0, |state| shape_of(shapes, shape_ids, &world.registries, state));
        sim::voxel::set_block_shape(pos.0, pos.1, pos.2, shape);
        world.scene.set(pos, block);
        positions.push(pos);
    }
    if !positions.is_empty() {
        world.stream.record_edits(&world.scene, &positions);
        world.stream.mark_edited(&world.scene, &positions);
    }
    positions
}

/// The block world's networking pieces.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct MinecraftNet<'w, 's> {
    exit: MessageReader<'w, 's, bevy::app::AppExit>,
    role: Res<'w, frame::RuntimeRole>,
    prediction: Option<Res<'w, net::ClientPredictionState>>,
    downlink: Option<ResMut<'w, net::MinecraftDownlink>>,
    frames: Option<ResMut<'w, net::PendingMinecraftFrames>>,
    reliable: Option<ResMut<'w, net::ReliableEventHub>>,
    hub: Option<Res<'w, net::UdpAuthorityHub>>,
    inbox: Option<ResMut<'w, net::ClientActionInbox>>,
    ids: Option<ResMut<'w, net::ActionRequestIds>>,
    bridge: Option<Res<'w, net::MasterBridge>>,
    identity: Option<Res<'w, frame::LaunchIdentity>>,
    dedicated: Option<Res<'w, frame::DedicatedServer>>,
    headless: Option<Res<'w, frame::Headless>>,
    admins: Option<ResMut<'w, net::DebugAdmins>>,
    notify: Option<ResMut<'w, net::PendingGameNotify>>,
    identities: Option<Res<'w, net::PlayerIdentities>>,
    local_key: Option<Res<'w, net::LocalPlayerKey>>,
}

/// A console block box as world blocks: corners relative to `base`.
fn fill_box(base: (i32, i32, i32), a: [i32; 3], b: [i32; 3]) -> ((i32, i32, i32), (i32, i32, i32)) {
    (
        (base.0 + a[0], base.1 + a[1], base.2 + a[2]),
        (base.0 + b[0], base.1 + b[1], base.2 + b[2]),
    )
}

/// The blocks of a box, each set to `block`.
fn box_changes(
    a: (i32, i32, i32),
    b: (i32, i32, i32),
    block: Option<&minecraft_terrain::scene::Block>,
) -> Vec<((i32, i32, i32), Option<minecraft_terrain::scene::Block>)> {
    let mut out = Vec::new();
    for x in a.0.min(b.0)..=a.0.max(b.0) {
        for y in a.1.min(b.1)..=a.1.max(b.1) {
            for z in a.2.min(b.2)..=a.2.max(b.2) {
                out.push(((x, y, z), block.cloned()));
            }
        }
    }
    out
}

/// A console block id as a block in its default state; none for air.
fn console_block(id: Option<&str>) -> Result<Option<minecraft_terrain::scene::Block>, String> {
    match id.map(minecraft_terrain::pack::ResourceId::parse) {
        None => Ok(None),
        Some(Ok(id)) => Ok(Some(minecraft_terrain::scene::Block { id, properties: Default::default() })),
        Some(Err(error)) => Err(error.to_string()),
    }
}

/// Minecraft yaw from an MW2 view yaw: 0 facing +Z (map -Y), turning
/// towards -X.
fn mc_yaw_of(view_yaw: f32) -> f32 {
    let yaw_rad = view_yaw.to_radians();
    (-yaw_rad.cos()).atan2(-yaw_rad.sin()).to_degrees()
}

#[allow(clippy::too_many_arguments)]
fn update(
    time: Res<Time>,
    mut installed: MessageReader<frame::MatchInstalled>,
    mut torn_down: MessageReader<frame::MatchTornDown>,
    local: Res<net::LocalPresentClient>,
    presented: Res<net::PresentedSnapshot>,
    mut authority: Option<ResMut<net::AuthorityWorld>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    (mut ui, mut puppet, mut images, mut sound_queue, buttons): (
        ResMut<frame::MinecraftUi>,
        ResMut<frame::InventoryPuppet>,
        ResMut<Assets<Image>>,
        ResMut<audio::McSoundQueue>,
        Res<ButtonInput<MouseButton>>,
    ),
    mut view: ResMut<MinecraftWorldView>,
    mut runtime: NonSendMut<Runtime>,
    (skate, cameras, gamepads, active_pad): (
        Res<frame::SkateMode>,
        Query<&Transform, With<render_scene::FlyCamera>>,
        Query<&bevy::input::gamepad::Gamepad>,
        Option<Res<frame::ActivePad>>,
    ),
    mut net: MinecraftNet,
) {
    use crate::minecraft_mp::NetRole;
    use minecraft_terrain::multiplayer::{HostMessage, Intent, block_text, parse_block};
    let runtime: &mut Runtime = &mut runtime;
    let pad = active_pad.and_then(|active| active.0).and_then(|entity| gamepads.get(entity).ok());
    for _ in torn_down.read() {
        stop(runtime, &mut view);
    }
    // The game is closing: a host's kept world is saved first.
    if net.exit.read().next().is_some() {
        keep_world(runtime);
    }
    let is_client = *net.role == frame::RuntimeRole::Client;
    let dedicated = net.dedicated.is_some();
    // A game that draws nothing (a dedicated server, or a headless player's
    // game) builds no meshes, light, icons or sounds: its world is blocks,
    // mobs and items, and its player plays in it all the same.
    let draws = net.headless.is_none() && !dedicated;
    let hosting = net.bridge.is_some() || dedicated;
    // A client's world arrives with the host's seed, mid-match: the loading
    // screen never waits on it.
    ui.loading_world = runtime.loading.is_some() && !is_client;
    for match_ in installed.read() {
        stop(runtime, &mut view);
        if let Some(downlink) = net.downlink.as_mut() {
            downlink.rows.clear();
            downlink.frames.clear();
        }
        ui.seed = None;
        if !assets::minecraft_map::is_minecraft(&match_.zone) {
            continue;
        }
        view.active = true;
        if is_client {
            diag::info!(World, "Minecraft world: waiting for the host's seed");
            runtime.net = NetRole::Client(Box::default());
            continue;
        }
        let artifacts = net.identity.as_ref().map(|identity| identity.artifacts.clone());
        let kept = crate::minecraft_mp::kept_world(hosting, ui.world_name.as_deref(), artifacts.as_deref());
        let saved = kept.as_ref().and_then(|(_, dir)| match minecraft_terrain::multiplayer::SavedWorld::load(dir) {
            Ok(saved) => saved,
            Err(error) => {
                diag::warn!(World, "Minecraft world {} unreadable, not loaded: {error}", dir.display());
                None
            }
        });
        let seed = saved.as_ref().map_or_else(seed, |saved| saved.level.seed);
        let origin = saved.as_ref().map(|saved| saved.level.origin);
        match &kept {
            Some((name, dir)) if saved.is_some() => {
                diag::info!(World, "Minecraft world: continuing `{name}` in {} (seed {seed})", dir.display())
            }
            Some((name, dir)) => diag::info!(World, "Minecraft world: new `{name}` in {} (seed {seed})", dir.display()),
            None => diag::info!(World, "Minecraft world: seed {seed}"),
        }
        let dir = kept.map(|(_, dir)| dir);
        runtime.net = NetRole::Host(Box::new(crate::minecraft_mp::Host::new(seed, origin.unwrap_or_default(), dir.clone(), saved)));
        let (send, receive) = mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("minecraft-world-load".into())
            .spawn(move || {
                let _ = send.send(load(seed, dir, origin, draws));
            });
        runtime.loading = Some(receive);
    }
    ui.net_role = match runtime.net {
        NetRole::Host(_) if dedicated => "dedicated",
        _ => runtime.net.label(hosting),
    };

    // A client takes the host's messages; the first is the world to make.
    if let NetRole::Client(client) = &mut runtime.net {
        // Mob states before the world exists have no mobs to move.
        if runtime.world.is_none()
            && let Some(downlink) = net.downlink.as_mut()
        {
            downlink.frames.clear();
        }
        for bytes in net.downlink.as_mut().map(|d| std::mem::take(&mut d.rows)).unwrap_or_default() {
            match HostMessage::decode(&bytes) {
                Ok(HostMessage::World { seed, origin }) => {
                    if client.seed.is_none() && runtime.loading.is_none() && runtime.world.is_none() {
                        client.seed = Some(seed);
                        diag::info!(World, "Minecraft world: the host's seed {seed}, origin {origin:?}");
                        let (send, receive) = mpsc::channel();
                        let _ = std::thread::Builder::new()
                            .name("minecraft-world-load".into())
                            .spawn(move || {
                                let _ = send.send(load(seed, None, Some(origin), draws));
                            });
                        runtime.loading = Some(receive);
                    }
                }
                Ok(message) => client.inbox.push(message),
                Err(error) => diag::warn!(World, "Minecraft host message dropped: {error}"),
            }
        }
    }
    let content = authority
        .as_ref()
        .map(|authority| authority.0.content())
        .or_else(|| net.prediction.as_ref().map(|prediction| prediction.0.world().content()));
    let Some(content) = content else {
        return;
    };

    if let Some(receive) = &runtime.loading
        && let Ok(result) = receive.try_recv()
    {
        runtime.loading = None;
        match result {
            Ok(mut world) => {
                view.origin = world.origin;
                view.atlas = Some(world.atlas.clone());
                view.celestial = Some(world.celestial.clone());
                view.crack_texture = Some(world.crack_texture.clone());
                runtime.mining = Default::default();
                runtime.sounds = world.sounds.take();
                runtime.entities = world.entities.take();
                if let Some(entities) = runtime.entities.as_mut() {
                    entities.remote = is_client;
                }
                runtime.day = DayCycle::default();
                // Game ticks since sunrise to start at: 6000 noon, 13000
                // dusk, 18000 midnight.
                if let Some(ticks) = std::env::var("IW4L_MINECRAFT_TIME")
                    .ok()
                    .and_then(|t| t.trim().parse::<f64>().ok())
                {
                    runtime.day.set(ticks);
                }
                runtime.environment_accumulator = 0.0;
                runtime.environment_primed = false;
                runtime.light = Some(SkyLight::streamed());
                runtime.light_volume_at = None;
                runtime.cloud_center = None;
                view.generation += 1;
                sim::voxel::activate(
                    content.clip_brushes(),
                    view.origin,
                    vec![Vec::new()],
                );
                diag::info!(
                    World,
                    "Minecraft world ready: seed {} spawn {:?} origin {:?}",
                    world.seed,
                    world.stream.player_spawn,
                    world.origin
                );
                runtime.shapes.clear();
                runtime.shape_ids.clear();
                runtime.deferred.clear();
                // A kept world's edits go back over its terrain.
                if let NetRole::Host(host) = &mut runtime.net {
                    host.origin = world.origin;
                    let changes: Vec<_> = host
                        .log
                        .compacted()
                        .entries()
                        .iter()
                        .map(|(pos, block)| (*pos, block.as_deref().and_then(parse_block)))
                        .collect();
                    if !changes.is_empty() {
                        diag::info!(World, "Minecraft world: {} saved block changes restored", changes.len());
                    }
                    set_blocks(&mut world, &mut runtime.shapes, &mut runtime.shape_ids, &mut runtime.deferred, changes);
                }
                ui.seed = Some(world.seed);
                runtime.world = Some(world);
                runtime.was_alive = false;
                runtime.settled = 0;
            }
            Err(error) => {
                diag::warn!(World, "Minecraft world failed to load: {error}");
                view.active = false;
            }
        }
    }

    let origin = view.origin;
    let Runtime {
        world,
        shapes,
        shape_ids,
        was_alive,
        settled,
        day,
        environment_accumulator,
        environment_primed,
        light,
        light_volume_at,
        light_volume_age,
        cloud_center,
        mining,
        entities,
        inventory_ui,
        sounds,
        hand,
        minimap,
        steps,
        net: net_role,
        deferred,
        voxel_holders,
        level_holders,
        speed_applied,
        loading: _,
    } = &mut *runtime;
    let Some(world) = world.as_mut() else {
        ui.active = false;
        ui.inventory_open = false;
        ui.workbench = None;
        ui.table_in_view = None;
        ui.in_world = false;
        puppet.active = false;
        return;
    };
    // A dedicated server has no player: the world keeps its spawn loaded, or
    // the ground of its first client, seen from a spectator that stands there.
    let spectator;
    let ps = match presented.player(local.0) {
        Some(ps) if !dedicated => ps,
        _ if dedicated => {
            let first = match &*net_role {
                NetRole::Host(host) => host.remotes.keys().find_map(|&id| {
                    let authority = authority.as_ref()?;
                    authority.0.player(sim::ClientId(id)).filter(|ps| ps.pm_type == 0).map(|ps| ps.origin)
                }),
                _ => None,
            };
            let mut viewer = sim::blank_player_state();
            viewer.pm_type = playerstate_iw4::PM_TYPE_SPECTATOR;
            viewer.origin = first.unwrap_or([0.0; 3]);
            spectator = viewer;
            &spectator
        }
        _ => {
            ui.active = false;
            ui.in_world = false;
            puppet.active = false;
            return;
        }
    };

    // Every spawn lands on the Minecraft spawn once its ground exists. Alive is the snapshot's
    // lifecycle, not only the pose's type: before MW2's script spawns the player its pose is
    // already a normal one, and a move made then is undone by the spawn that follows.
    let alive = ps.pm_type == 0 && !dedicated && presented.alive_player(local.0).is_some();
    let spawn_chunk = ((origin[0].floor() as i32) >> 4, (origin[2].floor() as i32) >> 4);
    if alive && !*was_alive && world.scene.generated_chunk(spawn_chunk).is_some() {
        // Retried each frame until the authority has the player to move; a
        // client's host moves it.
        if authority.as_mut().is_none_or(|authority| authority.0.teleport(local.0, [0.0, 0.0, 0.0])) {
            diag::info!(World, "Minecraft spawn: moved to the world spawn");
            *was_alive = true;
            *settled = 0;
        }
    } else if !alive {
        *was_alive = false;
    }
    let feet = sim::voxel::to_block(origin, ps.origin);
    let block = (
        feet[0].floor() as i32,
        feet[1].floor() as i32,
        feet[2].floor() as i32,
    );
    // Standing: a block under some corner of the player's box. Right after the move to the
    // spawn the player can still be above the ground with no speed yet.
    let below = (feet[1] - 0.05).floor() as i32;
    let grounded = [(-0.3, -0.3), (-0.3, 0.3), (0.3, -0.3), (0.3, 0.3)].iter().any(|(dx, dz)| {
        let pos = ((feet[0] + dx).floor() as i32, below, (feet[2] + dz).floor() as i32);
        minecraft_terrain::scene::Scene::block(&world.scene, pos).is_some()
    });
    // The presented pose lags the move, and MW2's own spawn can land anywhere (under the map
    // when its ground trace finds no terrain yet): the player is in the world once a pose over
    // the spawn has stood still for a few frames, and stays so until it dies.
    // A client stands where its host moved it, beside the spawn.
    let spot = match &*net_role {
        NetRole::Client(client) => client.spawn_at,
        _ => Some([0.0; 3]),
    };
    let over_spawn = spot.is_some_and(|at| (ps.origin[0] - at[0]).hypot(ps.origin[1] - at[1]) < sim::voxel::BLOCK * 0.5);
    if !(alive && *was_alive) {
        *settled = 0;
    } else if *settled < SETTLE_FRAMES {
        let still = grounded && ps.velocity[2].abs() < 1.0;
        *settled = if over_spawn && still { *settled + 1 } else { 0 };
    }
    ui.in_world = *settled >= SETTLE_FRAMES;
    ui.feet = Some([
        block.0 - origin[0].floor() as i32,
        block.1 - origin[1].floor() as i32,
        block.2 - origin[2].floor() as i32,
    ]);
    let (loaded, forgotten) = world.stream.server_tick(block, &mut world.scene);
    for chunk in loaded {
        // Edits that came before their chunk: an air block clears terrain only
        // once there is terrain to clear.
        let mut early = Vec::new();
        if let Some(edits) = deferred.remove(&(chunk.pos.x, chunk.pos.z)) {
            let positions: Vec<_> = edits.iter().map(|(pos, _)| *pos).collect();
            for (pos, block) in edits {
                world.scene.set(pos, block);
            }
            world.stream.record_edits(&world.scene, &positions);
            world.stream.mark_edited(&world.scene, &positions);
            early = positions;
        }
        let (min_y, height) = (chunk.min_y(), chunk.height());
        let ids = chunk_collision(&chunk, world, shapes, shape_ids);
        *voxel_holders.entry((chunk.pos.x, chunk.pos.z)).or_default() += 1;
        // The mobs' level holds it once, however many players stand near,
        // with the edits that came before it.
        if hold(level_holders, (chunk.pos.x, chunk.pos.z))
            && let Some(entities) = entities.as_mut()
        {
            entities.load_chunk(&chunk);
            for pos in early {
                entities.placed(&world.scene, pos);
            }
            // And a host's logged ones: a client's collision may have laid
            // the edits that waited for this chunk into the scene already.
            if let NetRole::Host(host) = &*net_role {
                for at in logged_in(&host.log, (chunk.pos.x, chunk.pos.z)) {
                    entities.placed(&world.scene, at);
                }
            }
        }
        sim::voxel::set_chunk(
            chunk.pos.x,
            chunk.pos.z,
            sim::voxel::VoxelChunk {
                min_y,
                height,
                shapes: ids,
            },
        );
        // What a map reads of it, its edits included.
        if let NetRole::Host(host) = &mut *net_role {
            host.columns.sample_scene(&world.scene, (chunk.pos.x, chunk.pos.z));
        }
    }
    for pos in forgotten {
        // The stream dropped it from the scene; a client still standing there
        // keeps it.
        let released = release_chunk(voxel_holders, (pos.x, pos.z));
        if !released
            && let NetRole::Host(host) = &*net_role
            && let Some(chunk) = host.remotes.values().find_map(|remote| remote.held.get(&(pos.x, pos.z)))
        {
            world.scene.insert_chunk(chunk.clone());
        }
        if release_level(level_holders, (pos.x, pos.z)) {
            forget_level_chunk(net_role, entities.as_mut(), (pos.x, pos.z));
        }
    }

    // Blocks this frame changed, for the host's log.
    let mut changed: Vec<(i32, i32, i32)> = Vec::new();
    let centre_of = |pos: (i32, i32, i32)| {
        Vec3::from_array(sim::voxel::to_map(origin, [pos.0 as f64 + 0.5, pos.1 as f64 + 0.5, pos.2 as f64 + 0.5]))
    };
    // A client takes the host's blocks, its own inventory once the host has
    // applied all it asked, and the items on the ground.
    if let NetRole::Client(client) = net_role {
        for message in std::mem::take(&mut client.inbox) {
            match message {
                HostMessage::World { .. } => {}
                HostMessage::Placed { at } => client.spawn_at = Some(at),
                HostMessage::Edits(edits) => {
                    let mut changes = Vec::with_capacity(edits.len());
                    for (pos, text) in edits {
                        let block = text.as_deref().and_then(parse_block);
                        let before = minecraft_terrain::scene::Scene::block(&world.scene, pos).cloned();
                        if before == block {
                            continue;
                        }
                        // Another player's edit sounds as the host's own do.
                        let kind = match (&before, &block) {
                            (Some(old), None) => world.scene.sound_type(old).map(|k| (k.break_sound.clone(), k.volume, k.pitch)),
                            (_, Some(new)) => world.scene.sound_type(new).map(|k| (k.place.clone(), k.volume, k.pitch)),
                            (None, None) => None,
                        };
                        if let (Some(sounds), Some((event, volume, pitch))) = (sounds.as_mut(), kind) {
                            sounds.play(&world.packs, &event, Some(centre_of(pos)), (volume + 1.0) / 2.0, pitch * 0.8);
                        }
                        changes.push((pos, block));
                    }
                    set_blocks(world, shapes, shape_ids, deferred, changes);
                }
                HostMessage::Inventory { applied, state, .. } => {
                    if applied == client.last_sent
                        && let Some(entities) = entities.as_mut()
                    {
                        let selected = state.restore(&mut entities.inventory);
                        entities.selected = selected.min(frame::minecraft_ui::MC_HOTBAR - 1);
                    }
                }
                HostMessage::Items(items) => {
                    if let Some(entities) = entities.as_mut() {
                        entities.world_items.entities = crate::minecraft_mp::client_items(&items);
                    }
                }
                HostMessage::Mobs(spawns) => {
                    if let Some(entities) = entities.as_mut() {
                        entities.mob_spawns(&spawns);
                    }
                }
                HostMessage::MobsGone(ids) => {
                    if let Some(entities) = entities.as_mut() {
                        entities.mobs_gone(&ids);
                    }
                }
                HostMessage::MapTiles { id, centre, scale, tiles } => {
                    crate::minecraft_mp::take_map_tiles(&mut client.maps, id, centre, scale, &tiles);
                }
            }
        }
        // The mobs near it, as its snapshots last told.
        let frames = net.downlink.as_mut().map(|d| std::mem::take(&mut d.frames)).unwrap_or_default();
        if let Some(entities) = entities.as_mut() {
            for bytes in frames {
                entities.mob_frame(&bytes);
            }
        }
    }
    // The host takes its clients in and out, onto the spawn, and applies
    // what they ask.
    if let (NetRole::Host(host), Some(authority)) = (&mut *net_role, authority.as_mut()) {
        let authority = &mut authority.0;
        // The host's own player, by its own key (a dedicated server has
        // neither).
        if host.local.is_none()
            && !dedicated
            && let Some(key) = net.local_key.as_ref().map(|key| key.0.fingerprint())
        {
            let name = net::client_name_string(authority, local.0);
            if !name.is_empty() {
                if let (Some(state), Some(entities)) = (host.join(&key, &name), entities.as_mut()) {
                    let selected = state.restore(&mut entities.inventory);
                    entities.selected = selected.min(frame::minecraft_ui::MC_HOTBAR - 1);
                    diag::info!(World, "Minecraft: the inventory of `{name}` restored");
                }
                let mut unlocked = host.players.unlocked_weapons(&key).to_vec();
                // Its starting map and starter kit, if this world owes it them.
                if let Some(entities) = entities.as_mut() {
                    host.give_map_once(Some(&key), &mut entities.inventory, &name);
                    host.give_starter_kit_once(Some(&key), &mut entities.inventory, &mut unlocked, &name);
                }
                host.local_unlocked_weapons = unlocked;
                host.local = Some((key, name));
            }
        }
        // A player without a key keeps nothing: its map and starter kit,
        // once a session.
        if !host.local_map_given
            && host.local.is_none()
            && !dedicated
            && net.local_key.is_none()
            && let Some(entities) = entities.as_mut()
        {
            let name = net::client_name_string(authority, local.0);
            if !name.is_empty() {
                host.local_map_given = true;
                host.give_map_once(None, &mut entities.inventory, &name);
            }
        }
        if !host.local_starter_kit_given
            && host.local.is_none()
            && !dedicated
            && net.local_key.is_none()
            && let Some(entities) = entities.as_mut()
        {
            let name = net::client_name_string(authority, local.0);
            if !name.is_empty() {
                host.local_starter_kit_given = true;
                let mut unlocked = std::mem::take(&mut host.local_unlocked_weapons);
                host.give_starter_kit_once(None, &mut entities.inventory, &mut unlocked, &name);
                host.local_unlocked_weapons = unlocked;
            }
        }
        let clients: Vec<u32> = net
            .hub
            .as_ref()
            .map(|hub| hub.admitted_clients())
            .unwrap_or_default()
            .into_iter()
            .map(|client| client.0)
            .filter(|&client| client != local.0.0)
            .collect();
        let gone: Vec<u32> = host.remotes.keys().copied().filter(|id| !clients.contains(id)).collect();
        for id in gone {
            if let Some(remote) = host.remotes.remove(&id) {
                for pos in remote.held.keys() {
                    if release_chunk(voxel_holders, *pos) {
                        world.scene.remove_chunk(*pos);
                    }
                }
                for pos in &remote.level_held {
                    if release_level(level_holders, *pos) {
                        host.columns.remove(*pos);
                        if let Some(entities) = entities.as_mut() {
                            entities.unload_chunk(minecraftoss_core::ChunkPos::new(pos.0, pos.1));
                        }
                    }
                }
                match remote.key.as_deref() {
                    Some(key) => {
                        host.keep_inventory(key, &remote.name, &remote.inventory, remote.selected);
                        diag::info!(World, "Minecraft: `{}` left; inventory kept", remote.name);
                    }
                    None => diag::info!(World, "Minecraft: `{}` (a guest) left; nothing kept", remote.name),
                }
                host.save_now();
            }
        }
        if let Some(entities) = entities.as_ref() {
            for &id in &clients {
                if host.remotes.contains_key(&id) {
                    continue;
                }
                let name = net::client_name_string(authority, sim::ClientId(id));
                if name.is_empty() {
                    continue;
                }
                // Its key decides its inventory: none until its proof is in.
                let key = match net.identities.as_ref().map(|ids| ids.of(sim::ClientId(id))) {
                    Some(net::PlayerIdentity::Key(key)) => Some(key),
                    Some(net::PlayerIdentity::Guest) => None,
                    Some(net::PlayerIdentity::Pending) | None => continue,
                };
                let saved = key.as_deref().and_then(|key| host.join(key, &name));
                let mut inventory = minecraftoss_player::inventory::Inventory::default();
                inventory.recipes = entities.inventory.recipes.clone();
                let mut remote = crate::minecraft_mp::Remote::new(name.clone(), key, inventory, saved.as_ref());
                remote.unlocked_weapons = remote
                    .key
                    .as_deref()
                    .map_or_else(Vec::new, |key| host.players.unlocked_weapons(key).to_vec());
                // Its starting map and starter kit, if this world owes it them.
                host.give_map_once(remote.key.as_deref(), &mut remote.inventory, &name);
                let mut unlocked = std::mem::take(&mut remote.unlocked_weapons);
                host.give_starter_kit_once(remote.key.as_deref(), &mut remote.inventory, &mut unlocked, &name);
                remote.unlocked_weapons = unlocked;
                diag::info!(
                    World,
                    "Minecraft: `{name}` joined as client {id}; {} block changes to send",
                    host.log.len()
                );
                host.remotes.insert(id, remote);
            }
        }
        let spawn_ready = world.scene.generated_chunk(spawn_chunk).is_some();
        for (index, (&id, remote)) in host.remotes.iter_mut().enumerate() {
            let Some(weapons) = authority.player(sim::ClientId(id)).map(|ps| ps.weapons) else {
                remote.placed = false;
                continue;
            };
            // Each life is placed once: MW2's own spawn puts every player on
            // the world spawn, whatever came before.
            let meta = authority.client_meta(sim::ClientId(id));
            let alive = meta.is_some_and(|meta| meta.lifecycle == sim::ClientLifecycle::Alive);
            let life = meta.map(|meta| meta.life_sequence);
            if remote.life != life {
                remote.life = life;
                remote.placed = false;
            }
            if !alive {
                remote.placed = false;
            } else if !remote.placed
                && spawn_ready
                && let spot = client_spawn(&world.scene, origin, index + 1)
                && authority.teleport(sim::ClientId(id), spot)
            {
                diag::info!(World, "Minecraft spawn: moved client {id} beside the world spawn, {spot:?}");
                remote.placed = true;
                remote.placed_at = Some(spot);
            }
            let owned: Vec<u32> = weapons
                .iter()
                .filter(|&&w| w > 0)
                .map(|&w| w as u32)
                .filter(|&w| authority.weapon_combat_row(w).is_some_and(|facts| facts.inventory_type == 0))
                .collect();
            inventory_ui.sync_weapons(&mut remote.inventory, &owned);
            // Its permanent gun unlocks and universal ammo: the crafting
            // table's output just taken (host authoritative, this client's
            // own inventory), and what a reload just drew.
            crate::minecraft_arsenal::drain_ammo(authority, sim::ClientId(id), &mut remote.inventory);
            let fingerprint = remote.key.clone();
            let name = remote.name.clone();
            let mut crafted_dirty = false;
            crate::minecraft_arsenal::feed_arsenal(
                authority,
                sim::ClientId(id),
                &mut remote.inventory,
                &mut remote.unlocked_weapons,
                |weapon| {
                    if let Some(key) = fingerprint.as_deref() {
                        host.players.add_unlocked_weapon(key, weapon);
                    }
                    crafted_dirty = true;
                    diag::info!(World, "Minecraft: `{name}` crafted {weapon}");
                },
            );
            host.dirty |= crafted_dirty;
        }
        // A dedicated server's own stream follows its first client, out to
        // the mobs' distance: that client needs no mob stream of its own.
        let followed = host
            .remotes
            .keys()
            .copied()
            .find(|&id| authority.player(sim::ClientId(id)).is_some_and(|ps| ps.pm_type == 0))
            .filter(|_| dedicated && world.stream.view_distance() >= CLIENT_MOB_DISTANCE);
        // The ground under each client, wherever it goes: chunks generated
        // around it, the log's changes laid over them.
        for (&id, remote) in host.remotes.iter_mut() {
            let Some(feet) = authority
                .player(sim::ClientId(id))
                .filter(|ps| ps.pm_type == 0)
                .map(|ps| sim::voxel::to_block(origin, ps.origin))
            else {
                continue;
            };
            let at = (feet[0].floor() as i32, feet[1].floor() as i32, feet[2].floor() as i32);
            // The mobs' level around it, out to the simulation distance: mobs
            // spawn, walk and despawn around this client as around the host
            // (24 to 128 blocks), with the log's changes in their ground.
            if followed == Some(id) {
                remote.mob_stream = None;
                for pos in std::mem::take(&mut remote.level_held) {
                    if release_level(level_holders, pos) {
                        host.columns.remove(pos);
                        if let Some(entities) = entities.as_mut() {
                            entities.unload_chunk(minecraftoss_core::ChunkPos::new(pos.0, pos.1));
                        }
                    }
                }
            } else {
                let stream = remote.mob_stream.get_or_insert_with(|| {
                    minecraft_terrain::terrain::CollisionStream::new(world.stream.world_gen(), CLIENT_MOB_DISTANCE)
                });
                let (loaded, forgotten) = stream.tick(at);
                for chunk in loaded {
                    let pos = (chunk.pos.x, chunk.pos.z);
                    if !remote.level_held.insert(pos) || !hold(level_holders, pos) {
                        continue;
                    }
                    // What a map reads of it: as the scene has it, else as
                    // generated (the scene takes it, with its edits, when
                    // the client comes near).
                    if !host.columns.sample_scene(&world.scene, pos)
                        && let Some(states) = world.scene.states()
                    {
                        host.columns.sample_generated(&chunk, states);
                    }
                    if let Some(entities) = entities.as_mut() {
                        entities.load_chunk(&chunk);
                        // The log's changes, as the scene has them: only once the
                        // scene has the chunk (read before, they would be air);
                        // until then they come with it, below.
                        if world.scene.generated_chunk(pos).is_some() {
                            for at in logged_in(&host.log, pos) {
                                entities.placed(&world.scene, at);
                            }
                        }
                    }
                }
                for pos in forgotten {
                    if remote.level_held.remove(&pos) && release_level(level_holders, pos) {
                        host.columns.remove(pos);
                        if let Some(entities) = entities.as_mut() {
                            entities.unload_chunk(minecraftoss_core::ChunkPos::new(pos.0, pos.1));
                        }
                    }
                }
            }
            let collision = remote.collision.get_or_insert_with(|| {
                minecraft_terrain::terrain::CollisionStream::new(world.stream.world_gen(), CLIENT_COLLISION_DISTANCE)
            });
            let (loaded, forgotten) = collision.tick(at);
            for chunk in loaded {
                let pos = (chunk.pos.x, chunk.pos.z);
                if remote.held.insert(pos, chunk.clone()).is_some() {
                    continue;
                }
                if !hold(voxel_holders, pos) {
                    continue;
                }
                // The host's scene takes it too, so its edits, mining and placing
                // work there; nothing draws it.
                if world.scene.generated_chunk(pos).is_none() {
                    world.scene.insert_chunk(chunk.clone());
                    for (at, block) in deferred.remove(&pos).unwrap_or_default() {
                        world.scene.set(at, block);
                    }
                    if level_holders.contains_key(&pos) {
                        host.columns.sample_scene(&world.scene, pos);
                    }
                    // A mobs' level that took this chunk before the scene had it
                    // has the log's changes now: its mobs stood on air where a
                    // client's platform was, and walked through a pen's walls.
                    if level_holders.contains_key(&pos)
                        && let Some(entities) = entities.as_mut()
                    {
                        for at in logged_in(&host.log, pos) {
                            entities.placed(&world.scene, at);
                        }
                    }
                }
                let (min_y, height) = (chunk.min_y(), chunk.height());
                let ids = chunk_collision(&chunk, world, shapes, shape_ids);
                sim::voxel::set_chunk(pos.0, pos.1, sim::voxel::VoxelChunk { min_y, height, shapes: ids });
            }
            for pos in forgotten {
                if remote.held.remove(&pos).is_some() && release_chunk(voxel_holders, pos) {
                    world.scene.remove_chunk(pos);
                }
            }
            // Until there is ground where it stands, the host holds it still
            // rather than let it fall through.
            let column = ((feet[0].floor() as i32) >> 4, (feet[2].floor() as i32) >> 4);
            let waiting = !voxel_holders.contains_key(&column);
            if waiting != remote.held_still {
                remote.held_still = waiting;
                authority.set_external_motion(sim::ClientId(id), waiting);
                // It may have sunk a little before the hold began: once there is
                // ground, out of any block it stands in.
                if !waiting
                    && let Some(clear) = clear_feet(world, feet)
                    && clear > feet[1]
                {
                    let mut at = sim::voxel::to_map(origin, [feet[0], clear, feet[2]]);
                    at[2] += 1.0;
                    authority.teleport(sim::ClientId(id), at);
                }
            }
        }
        let cheats = authority.cheats_enabled();
        let mut transfers = Vec::new();
        for (client, request_id, bytes) in sim::voxel::take_intents() {
            let Some(remote) = host.remotes.get_mut(&client) else {
                continue;
            };
            remote.applied = request_id;
            let intent = match Intent::decode(&bytes) {
                Ok(intent) => intent,
                Err(error) => {
                    diag::warn!(World, "Minecraft intent from client {client} dropped: {error}");
                    continue;
                }
            };
            // The console's cheats only from the host's admins. A refused
            // one still counts as applied, so the client takes back the
            // inventory it predicted.
            if let Some(command) = intent.debug_name() {
                if !cheats {
                    continue;
                }
                let allowed = match (net.admins.as_ref(), net.identities.as_ref()) {
                    (Some(admins), Some(identities)) => admins.allows(identities, sim::ClientId(client)),
                    _ => false,
                };
                if !allowed {
                    let name = net::client_name_string(authority, sim::ClientId(client));
                    let who = net.identities.as_ref().map_or_else(
                        || name.clone(),
                        |identities| net::authority::admins::who(identities, &name, sim::ClientId(client)),
                    );
                    if let Some(reliable) = net.reliable.as_mut() {
                        net::refuse_debug(reliable, net.notify.as_deref_mut(), sim::ClientId(client), &who, command);
                    }
                    continue;
                }
            }
            let Some(their) = authority.player(sim::ClientId(client)).copied() else {
                continue;
            };
            let alive = their.pm_type == 0;
            let their_feet = sim::voxel::to_block(origin, their.origin);
            let their_eye = sim::voxel::to_block(
                origin,
                [their.origin[0], their.origin[1], their.origin[2] + their.view_height_current],
            );
            let near = |pos: (i32, i32, i32), reach: f64| {
                let centre = [pos.0 as f64 + 0.5, pos.1 as f64 + 0.5, pos.2 as f64 + 0.5];
                (0..3).map(|i| (their_eye[i] - centre[i]).powi(2)).sum::<f64>() <= reach * reach
            };
            let thrower = crate::minecraft_inventory::Thrower {
                eye: glam::DVec3::from_array(their_eye),
                yaw: mc_yaw_of(their.viewangles[1]),
                pitch: their.viewangles[0],
            };
            let mut thrown = Vec::new();
            match intent {
                Intent::Break { pos } => {
                    let block = minecraft_terrain::scene::Scene::block(&world.scene, pos).cloned();
                    match block {
                        Some(block) if alive && near(pos, REMOTE_REACH) => {
                            set_blocks(world, shapes, shape_ids, deferred, vec![(pos, None)]);
                            changed.push(pos);
                            if let Some(entities) = entities.as_mut() {
                                entities.broke(&world.scene, &[pos]);
                                // As the client's hand: what the host has it hold.
                                let held = remote.inventory.slots[remote.selected].clone();
                                entities.drop_blocks(&[(pos, block.clone(), crate::minecraft_tools::Breaker::Hand(held))]);
                            }
                            if let (Some(sounds), Some(kind)) = (sounds.as_mut(), world.scene.sound_type(&block)) {
                                sounds.play(&world.packs, &kind.break_sound, Some(centre_of(pos)), (kind.volume + 1.0) / 2.0, kind.pitch * 0.8);
                            }
                        }
                        block => remote.corrections.push((pos, block.as_ref().map(block_text))),
                    }
                }
                Intent::Place { feet: at, yaw, pitch, selected } => {
                    remote.selected = usize::from(selected).min(frame::minecraft_ui::MC_HOTBAR - 1);
                    let close = (0..3).all(|i| (at[i] - their_feet[i]).abs() <= 3.0);
                    if !(alive && close) {
                        continue;
                    }
                    let mut player = minecraftoss_player::Player::new(glam::DVec3::from_array(at));
                    player.yaw = f64::from(yaw);
                    player.pitch = f64::from(pitch);
                    player.selected = remote.selected;
                    let Some(pos) = player.place_selected(
                        &mut world.scene,
                        &mut remote.inventory,
                        minecraftoss_player::GameMode::Survival,
                    ) else {
                        continue;
                    };
                    let block = minecraft_terrain::scene::Scene::block(&world.scene, pos).cloned();
                    if inside_player(at, pos) {
                        // As the client did: not into the player's own box.
                        world.scene.set(pos, None);
                        if let Some(block) = block {
                            let stack = minecraftoss_player::inventory::ItemStack::new(block.id.key(), 1);
                            let _ = remote.inventory.add_item(stack, remote.selected);
                        }
                        continue;
                    }
                    set_blocks(world, shapes, shape_ids, deferred, vec![(pos, block.clone())]);
                    changed.push(pos);
                    if let Some(entities) = entities.as_mut() {
                        entities.placed(&world.scene, pos);
                    }
                    if let (Some(sounds), Some(kind)) = (sounds.as_mut(), block.as_ref().and_then(|b| world.scene.sound_type(b))) {
                        sounds.play(&world.packs, &kind.place, Some(centre_of(pos)), (kind.volume + 1.0) / 2.0, kind.pitch * 0.8);
                    }
                }
                Intent::Click(click) => {
                    let click = crate::minecraft_mp::ui_click(click);
                    if click == frame::McClick::Close {
                        remote.table = None;
                    }
                    let workbench = remote.table.is_some();
                    thrown.extend(crate::minecraft_inventory::apply_click(&mut remote.inventory, click, workbench));
                }
                Intent::Select { slot } => {
                    remote.selected = usize::from(slot).min(frame::minecraft_ui::MC_HOTBAR - 1);
                }
                Intent::Drop { whole } => thrown.extend(remote.inventory.drop_selected(remote.selected, whole)),
                Intent::OpenTable { pos } => {
                    if is_crafting_table(&world.scene, pos) && near(pos, 8.0) {
                        remote.table = Some(pos);
                    }
                }
                Intent::Give { id, count } if cheats => {
                    thrown.extend(crate::minecraft_inventory::give(&mut remote.inventory, &id, count, remote.selected));
                }
                Intent::Clear if cheats => crate::minecraft_inventory::clear_items(&mut remote.inventory),
                Intent::Fill { a, b, block } if cheats => {
                    let volume = (0..3)
                        .map(|i| {
                            let (a, b) = ([a.0, a.1, a.2][i], [b.0, b.1, b.2][i]);
                            i64::from((a - b).abs()) + 1
                        })
                        .product::<i64>();
                    if volume > FILL_MAX {
                        continue;
                    }
                    match console_block(block.as_deref()) {
                        Ok(block) => {
                            let positions = set_blocks(world, shapes, shape_ids, deferred, box_changes(a, b, block.as_ref()));
                            if let Some(entities) = entities.as_mut() {
                                for &pos in &positions {
                                    entities.placed(&world.scene, pos);
                                }
                            }
                            changed.extend(positions);
                        }
                        Err(error) => diag::warn!(World, "Minecraft fill from client {client}: {error}"),
                    }
                }
                Intent::Attack { key } => {
                    if alive
                        && let Some(entities) = entities.as_mut()
                        && !entities.attack(key, client, their_feet, mc_yaw_of(their.viewangles[1]))
                    {
                        diag::info!(World, "Minecraft: client {client}'s punch out of reach");
                    }
                }
                Intent::Summon { kind, at } if cheats => {
                    if let Some(entities) = entities.as_mut() {
                        entities.summon(&kind, at);
                    }
                }
                // An admin's `admin transfer`, done once the intents are.
                Intent::Transfer { from, to } => transfers.push((client, from, to)),
                Intent::Give { .. } | Intent::Clear | Intent::Fill { .. } | Intent::Summon { .. } => {}
            }
            thrown.extend(remote.inventory.take_pending_drops());
            let thrown = crate::minecraft_inventory::keep_guns(&mut remote.inventory, thrown, remote.selected);
            if let Some(entities) = entities.as_mut() {
                crate::minecraft_inventory::throw(&mut entities.world_items, thrown, &thrower);
            }
        }
        for (client, from, to) in transfers {
            let from = match from {
                minecraft_terrain::multiplayer::TransferFrom::Key(key) => minecraft_terrain::multiplayer::fingerprint_hex(&key),
                minecraft_terrain::multiplayer::TransferFrom::Name(name) => name,
            };
            let to = minecraft_terrain::multiplayer::fingerprint_hex(&to);
            let line = crate::minecraft_mp::admin_transfer(host, net.admins.as_deref_mut(), &from, &to, None);
            diag::info!(World, "{line} (asked by client {client})");
            println!("{line}");
            if let (Some(reliable), Some(notify)) = (net.reliable.as_mut(), net.notify.as_mut()) {
                let row = notify.plain_print(format!("{}{line}", sim::HUD_STRING_PLAIN));
                reliable.queue_mut(sim::ClientId(client)).push(net::ReliableRow::Notify(row));
            }
        }
        // Who picks items up this tick besides the host's own player.
        if let Some(entities) = entities.as_mut() {
            entities.others = host
                .remotes
                .iter()
                .filter_map(|(&id, remote)| {
                    let ps = authority.player(sim::ClientId(id)).filter(|ps| ps.pm_type == 0)?;
                    let feet = sim::voxel::to_block(origin, ps.origin);
                    Some((u64::from(id), feet, remote.inventory.clone(), remote.selected))
                })
                .collect();
            // And every client as the mobs see it.
            entities.players = host
                .remotes
                .keys()
                .filter_map(|&id| {
                    let ps = authority.player(sim::ClientId(id))?;
                    let alive = ps.pm_type == 0
                        && authority
                            .client_meta(sim::ClientId(id))
                            .is_some_and(|meta| meta.lifecycle == sim::ClientLifecycle::Alive);
                    Some(crate::minecraft_entities::RemotePlayer {
                        client: id,
                        view: crate::minecraft_entities::PlayerView {
                            feet: sim::voxel::to_block(origin, ps.origin),
                            alive,
                            health: ps.health as f32,
                            yaw: mc_yaw_of(ps.viewangles[1]),
                            pitch: ps.viewangles[0],
                        },
                    })
                })
                .collect();
        }
    }

    // Minecraft's yaw: 0 facing +Z (map -Y), turning towards -X.
    let yaw_rad = ps.viewangles[1].to_radians();
    let mc_yaw = (-yaw_rad.cos()).atan2(-yaw_rad.sin()).to_degrees();
    let mut all_events = sim::voxel::take_events();

    // The hand, when no gun is selected: vanilla's left click mines by hand
    // (with the hand's break speed) or punches, its right click places the
    // held block (`Player.place_selected`, vanilla's placement states).
    // Footsteps (`Entity.applyMovementEmissionAndPlaySound`): the walked
    // distance grows by 0.6 of each horizontal move on the ground, and past
    // the next step the block under the feet sounds its step at 0.15 of its
    // volume. A fall of more than three blocks lands with the block's fall
    // sound and the player's (`LivingEntity.causeFallDamage`).
    let on_ground = alive && ps.ground_entity_num != playerstate_iw4::ENTITYNUM_NONE;
    if let Some(last) = steps.last.filter(|_| alive) {
        let horizontal = ((feet[0] - last[0]).hypot(feet[2] - last[2]) * 0.6) as f32;
        let block_at = |dy: f64| {
            let pos = (feet[0].floor() as i32, (feet[1] - dy).floor() as i32, feet[2].floor() as i32);
            minecraft_terrain::scene::Scene::block(&world.scene, pos).cloned().map(|b| (pos, b))
        };
        let under = block_at(0.2);
        let centre = |pos: (i32, i32, i32)| {
            Vec3::from_array(sim::voxel::to_map(origin, [pos.0 as f64 + 0.5, pos.1 as f64 + 1.0, pos.2 as f64 + 0.5]))
        };
        if on_ground && horizontal < 2.0 {
            steps.move_dist += horizontal;
            if steps.move_dist > steps.next_step
                && let Some((pos, block)) = under.as_ref()
            {
                steps.next_step = steps.move_dist as i32 as f32 + 1.0;
                // Snow layers and carpets sound instead of what they lie on.
                let inside = block_at(-0.01).filter(|(_, b)| {
                    let p = b.id.path.as_str();
                    p == "snow" || p.ends_with("_carpet") || p == "moss_carpet"
                });
                let (pos, block) = inside.as_ref().map_or((*pos, block), |(p, b)| (*p, b));
                if let (Some(sounds), Some(kind)) = (sounds.as_mut(), world.scene.sound_type(block)) {
                    sounds.play(&world.packs, &kind.step, Some(centre(pos)), kind.volume * 0.15, kind.pitch);
                }
            }
        }
        if on_ground {
            if let Some(peak) = steps.air_peak.take() {
                let fall = peak - feet[1];
                if fall > 3.0
                    && let Some(sounds) = sounds.as_mut()
                {
                    let event = if fall > 7.0 { "minecraft:entity.player.big_fall" } else { "minecraft:entity.player.small_fall" };
                    sounds.play(&world.packs, event, None, 1.0, 1.0);
                    if let Some((pos, block)) = under.as_ref()
                        && let Some(kind) = world.scene.sound_type(block)
                    {
                        sounds.play(&world.packs, &kind.fall, Some(centre(*pos)), kind.volume * 0.5, kind.pitch * 0.75);
                    }
                }
            }
        } else {
            steps.air_peak = Some(steps.air_peak.map_or(feet[1], |p| p.max(feet[1])));
        }
    } else {
        steps.air_peak = None;
    }
    steps.last = alive.then_some(feet);

    let dt_hand = time.delta_secs_f64();
    if let Some(ticks) = hand.swing.as_mut() {
        *ticks += (dt_hand * 20.0) as f32;
        if *ticks >= crate::minecraft_hand::SWING_TICKS {
            hand.swing = None;
        }
    }
    let holding = entities.as_ref().is_some_and(|e| {
        e.inventory.slots[e.selected].as_ref().is_none_or(|s| crate::minecraft_inventory::weapon_of(s).is_none())
    });
    ui.holding_item = alive && holding;
    ui.empty_hand = ui.holding_item
        && entities.as_ref().is_some_and(|e| e.inventory.slots[e.selected].is_none());
    hand.clock += dt_hand;
    let hand_ticks = (hand.clock / TICK_SECONDS) as u32;
    hand.clock -= f64::from(hand_ticks) * TICK_SECONDS;
    // A crafting table in reach opens on MW2's Use whatever the hand holds,
    // and on a right click with an item or an empty hand, unless crouched with
    // something in hand: crouching stands in for vanilla's sneak, which then
    // places against it.
    ui.table_in_view = (alive && !ui.inventory_open)
        .then(|| table_in_reach(&world.scene, feet, mc_yaw, ps.viewangles[0]))
        .flatten();
    let use_pressed = std::mem::take(&mut ui.use_pressed);
    let crouched = ps.pm_flags & (playerstate_iw4::pm_flags::CROUCH | playerstate_iw4::pm_flags::PRONE) != 0;
    if use_pressed
        && entities.is_some()
        && let Some(pos) = ui.table_in_view
    {
        open_table(&mut ui, hand, pos);
    }
    if let Some(entities) = entities.as_mut()
        && ui.holding_item
        && !ui.inventory_open
    {
        let mut player = minecraftoss_player::Player::new(glam::DVec3::from_array(feet));
        player.yaw = f64::from(mc_yaw);
        player.pitch = f64::from(ps.viewangles[0]);
        player.selected = entities.selected;
        let eye_block = glam::DVec3::from_array(feet) + glam::DVec3::Y * 1.62;
        let look = {
            let (yaw, pitch) = (f64::from(mc_yaw).to_radians(), f64::from(ps.viewangles[0]).to_radians());
            glam::DVec3::new(-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos())
        };
        if buttons.just_pressed(MouseButton::Left) || pad_trigger(pad, GamepadButton::RightTrigger2, true) {
            hand.swing = Some(0.0);
            entities.punch(eye_block, look, mc_yaw);
        }
        if buttons.pressed(MouseButton::Left) || pad_trigger(pad, GamepadButton::RightTrigger2, false) {
            for _ in 0..hand_ticks {
                if let Some(hit) = player.target(&world.scene, 4.5) {
                    // A hand mines as vanilla's `getDestroyProgress`: by the
                    // held tool's speed for the block, over the hardness and
                    // over 30 (100 without the tool the block's drops need).
                    let progress = minecraft_terrain::scene::Scene::block(&world.scene, hit.pos)
                        .map_or(1.0 / 30.0, |block| entities.hand_progress(block));
                    all_events.push(sim::voxel::VoxelEvent::Shot {
                        block: [hit.pos.0, hit.pos.1, hit.pos.2],
                        damage: crate::minecraft_mining::DAMAGE_PER_HARDNESS * progress,
                        by: sim::voxel::ShotBy::Hand,
                    });
                    if hand.swing.is_none_or(|t| t >= crate::minecraft_hand::SWING_TICKS * 0.5) {
                        hand.swing = Some(0.0);
                    }
                }
            }
        }
        hand.place_delay = hand.place_delay.saturating_sub(hand_ticks);
        let place = (buttons.just_pressed(MouseButton::Right) || pad_trigger(pad, GamepadButton::LeftTrigger2, true))
            || ((buttons.pressed(MouseButton::Right) || pad_trigger(pad, GamepadButton::LeftTrigger2, false)) && hand.place_delay == 0);
        if let Some(pos) = ui.table_in_view.filter(|_| place && (!crouched || ui.empty_hand)) {
            hand.place_delay = 4;
            open_table(&mut ui, hand, pos);
        } else if place {
            hand.place_delay = 4;
            if let Some(pos) = player.place_selected(
                &mut world.scene,
                &mut entities.inventory,
                minecraftoss_player::GameMode::Survival,
            ) {
                // Not into the player's own box.
                let inside = inside_player(feet, pos);
                let block = minecraft_terrain::scene::Scene::block(&world.scene, pos).cloned();
                if inside {
                    world.scene.set(pos, None);
                    if let Some(block) = block {
                        let _ = entities.inventory.add_item(
                            minecraftoss_player::inventory::ItemStack::new(block.id.key(), 1),
                            entities.selected,
                        );
                    }
                } else if let Some(block) = block {
                    let state = world.stream.states.state_of(&block);
                    let blocks = &world.registries.blocks;
                    let shape = state.map_or(0, |state| {
                        *shapes.entry(state).or_insert_with(|| {
                            let boxes = blocks.collision_boxes(state);
                            if boxes.is_empty() {
                                return 0;
                            }
                            let key: Vec<[u32; 6]> = boxes.iter().map(|b| b.map(|v| (v as f32).to_bits())).collect();
                            if let Some(&id) = shape_ids.get(&key) {
                                return id;
                            }
                            let boxes32 = boxes.iter().map(|b| b.map(|v| v as f32)).collect();
                            let id = sim::voxel::add_shapes(vec![boxes32]).unwrap_or(0);
                            shape_ids.insert(key, id);
                            id
                        })
                    });
                    sim::voxel::set_block_shape(pos.0, pos.1, pos.2, shape);
                    world.stream.record_edits(&world.scene, &[pos]);
                    world.stream.mark_edited(&world.scene, &[pos]);
                    entities.placed(&world.scene, pos);
                    hand.swing = Some(0.0);
                    changed.push(pos);
                    // A client placed it ahead of its host, which places it the same way.
                    if let NetRole::Client(client) = net_role {
                        let intent = Intent::Place { feet, yaw: mc_yaw, pitch: ps.viewangles[0], selected: entities.selected as u8 };
                        client.send(net.inbox.as_deref_mut(), net.ids.as_deref_mut(), local.0, &intent);
                    }
                    if let (Some(sounds), Some(kind)) = (sounds.as_mut(), world.scene.sound_type(&block)) {
                        let centre = [pos.0 as f64 + 0.5, pos.1 as f64 + 0.5, pos.2 as f64 + 0.5];
                        let at = Vec3::from_array(sim::voxel::to_map(origin, centre));
                        sounds.play(&world.packs, &kind.place, Some(at), (kind.volume + 1.0) / 2.0, kind.pitch * 0.8);
                    }
                }
            }
        }
    }

    // Shots and explosions from the authoritative game: bullets that met a
    // mob hurt it, the rest mine.
    let (mob_shots, events): (Vec<_>, Vec<_>) = all_events
        .into_iter()
        .partition(|event| matches!(event, sim::voxel::VoxelEvent::MobShot { .. }));
    if let Some(entities) = entities.as_mut() {
        for shot in mob_shots {
            if let sim::voxel::VoxelEvent::MobShot { key, damage, from, shooter } = shot {
                // The host's own shot, or one of its clients', which the mob
                // then turns on.
                if shooter == local.0.0 {
                    entities.shoot(key, damage, from, mc_yaw, None);
                } else {
                    let yaw = authority
                        .as_ref()
                        .and_then(|authority| authority.0.player(sim::ClientId(shooter)))
                        .map_or(mc_yaw, |ps| mc_yaw_of(ps.viewangles[1]));
                    entities.shoot(key, damage, from, yaw, Some(shooter));
                }
            }
        }
    }
    // Vanilla's block sounds: a hit for each bullet into a block, then the
    // break of each block broken (`SoundType` volume and pitch as
    // `MultiPlayerGameMode` and `LevelRenderer` scale them), and blasts.
    let at = |b: [f64; 3]| Vec3::from_array(sim::voxel::to_map(origin, b));
    if let Some(sounds) = sounds.as_mut() {
        for event in &events {
            match *event {
                sim::voxel::VoxelEvent::Shot { block, .. } => {
                    let pos = (block[0], block[1], block[2]);
                    if let Some(kind) = minecraft_terrain::scene::Scene::block(&world.scene, pos)
                        .and_then(|b| world.scene.sound_type(b))
                    {
                        let centre = [block[0] as f64 + 0.5, block[1] as f64 + 0.5, block[2] as f64 + 0.5];
                        sounds.play(&world.packs, &kind.hit, Some(at(centre)), (kind.volume + 1.0) / 4.0, kind.pitch * 0.5);
                    }
                }
                sim::voxel::VoxelEvent::Explosion { center } => {
                    let pitch = (1.0 + (sounds.random() - sounds.random()) * 0.2) * 0.7;
                    sounds.play(&world.packs, "minecraft:entity.generic.explode", Some(at(center)), 4.0, pitch);
                }
                sim::voxel::VoxelEvent::MobShot { .. } | sim::voxel::VoxelEvent::Ray { .. } => {}
            }
        }
    }
    let broken = mining.apply(
        events,
        &mut crate::minecraft_mining::WorldRefs {
            stream: &mut world.stream,
            scene: &mut world.scene,
            packs: &world.packs,
            atlas: &world.atlas,
            registries: &world.registries,
        },
        time.elapsed_secs_f64(),
    );
    if let Some(sounds) = sounds.as_mut() {
        for (pos, block, cause) in &broken {
            if *cause == crate::minecraft_mining::Cause::Blast {
                continue;
            }
            if let Some(kind) = world.scene.sound_type(block) {
                let centre = [pos.0 as f64 + 0.5, pos.1 as f64 + 0.5, pos.2 as f64 + 0.5];
                sounds.play(&world.packs, &kind.break_sound, Some(at(centre)), (kind.volume + 1.0) / 2.0, kind.pitch * 0.8);
            }
        }
    }
    if let Some(entities) = entities.as_mut() {
        let positions: Vec<_> = broken.iter().map(|(pos, ..)| *pos).collect();
        entities.broke(&world.scene, &positions);
        // What broke each, for its loot: this game's own player mines with
        // what it holds; a knife is an empty hand; bullets and blasts are
        // anyone's (`block_drops`).
        let hand = entities.hand();
        let broken: Vec<_> = broken
            .iter()
            .map(|(pos, block, cause)| (*pos, block.clone(), crate::minecraft_tools::Breaker::of(*cause, &hand)))
            .collect();
        entities.drop_blocks(&broken);
    }
    changed.extend(broken.iter().map(|(pos, ..)| *pos));
    // A client mines only by hand (its host's game fires the guns): what it
    // broke, the host breaks.
    if let NetRole::Client(client) = net_role {
        for (pos, _, _) in &broken {
            client.send(net.inbox.as_deref_mut(), net.ids.as_deref_mut(), local.0, &Intent::Break { pos: *pos });
        }
    }

    let eye = sim::voxel::to_block(origin, [ps.origin[0], ps.origin[1], ps.origin[2] + ps.view_height_current]);
    let (pitch, yaw) = (ps.viewangles[0].to_radians(), ps.viewangles[1].to_radians());
    let map_forward = [pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), -pitch.sin()];
    let forward = glam::Vec3::new(map_forward[0], map_forward[2], -map_forward[1]);
    let aspect = windows
        .single()
        .map(|w| w.width() / w.height().max(1.0))
        .unwrap_or(16.0 / 9.0);
    // Culled from the camera that draws: the player's eye, or while
    // skating the Skate camera (a frame behind, so with room to spare).
    let skate_camera = cameras.iter().next().filter(|_| skate.active).map(|t| {
        let at = sim::voxel::to_block(origin, t.translation.to_array());
        let ahead = t.rotation * Vec3::NEG_Z;
        (
            glam::DVec3::new(at[0], at[1], at[2]),
            glam::Vec3::new(ahead.x, ahead.z, -ahead.y).normalize_or(forward),
        )
    });
    let (cull_at, cull_forward) = skate_camera.unwrap_or((glam::DVec3::new(eye[0], eye[1], eye[2]), forward));
    let camera = CullCamera {
        position: cull_at,
        forward: cull_forward,
        fov_degrees: if skate_camera.is_some() { 120.0 } else { 90.0 },
        aspect,
        yaw_degrees: (-cull_forward.x).atan2(cull_forward.z).to_degrees(),
        pitch_degrees: (-cull_forward.y).asin().to_degrees(),
    };
    let Some(light) = light.as_mut() else {
        return;
    };
    if draws {
        let update = world
            .stream
            .frame(&world.scene, &camera, FADE_MILLIS, &world.atlas, &world.packs);
        view.uploads.extend(update.uploads);
        view.removed.extend(update.removed);
        view.visible = update.visible;
        for (chunk, column) in update.lights {
            light.set_chunk_column(chunk, column);
        }
    }

    if let Some(sounds) = sounds.as_mut() {
        sound_queue.0.append(&mut sounds.queued);
    }
    // The minimap's picture, and its corners on the map.
    ui.minimap = draws
        .then(|| minimap.update(time.delta_secs_f64(), feet, &world.scene, &world.packs, &world.atlas, &mut images))
        .flatten()
        .map(|(image, [bx, bz])| {
            let corner = |x: i32, z: i32| {
                let p = sim::voxel::to_map(origin, [f64::from(x), feet[1], f64::from(z)]);
                [p[0], p[1]]
            };
            (image, corner(bx, bz), corner(bx + 256, bz + 256))
        });
    // The Overworld clock and the environment attributes of MinecraftOSS.
    let dt = time.delta_secs_f64();
    let partial = mining.tick(&world.scene, dt);
    if draws {
        view.particles = mining.particle_mesh(&world.atlas, forward, partial, light);
    }

    // The mobs: a server tick when due, the blocks it changed, its hits on
    // the player, the mobs' boxes for bullets and their meshes.
    if let Some(entities) = entities.as_mut() {
        let player = crate::minecraft_entities::PlayerView {
            feet,
            alive,
            health: ps.health as f32,
            yaw: mc_yaw,
            pitch: ps.viewangles[0],
        };
        let bright_outside = world.environment.sky_light_level() > 11.0;
        let ticks_before = entities.client_ticks();
        let (mut changes, hits) = entities.tick(dt, day.ticks as i64, bright_outside, &player);
        let mob_ticks = (entities.client_ticks() - ticks_before) as u32;
        // What the host's other players picked up goes into their inventories.
        if let NetRole::Host(host) = &mut *net_role {
            for (key, stack) in std::mem::take(&mut entities.others_picked) {
                if let Some(remote) = host.remotes.get_mut(&(key as u32)) {
                    let _ = remote.inventory.add_item(stack, remote.selected);
                }
            }
            // Speed and slowness move each player as they move a vanilla
            // one; a spawn starts its speed over.
            if let Some(authority) = authority.as_mut() {
                let own = (!dedicated).then_some((local.0.0, 0));
                let players: Vec<(u32, u64)> =
                    own.into_iter().chain(host.remotes.keys().map(|&id| (id, crate::minecraft_entities::remote_key(id)))).collect();
                for (id, key) in players {
                    let alive = authority
                        .0
                        .client_meta(sim::ClientId(id))
                        .is_some_and(|meta| meta.lifecycle == sim::ClientLifecycle::Alive);
                    if !alive {
                        speed_applied.remove(&id);
                        continue;
                    }
                    let want = entities.speed_of(key);
                    let had = speed_applied.get(&id).copied().unwrap_or(1.0);
                    if (want - had).abs() > 1.0e-4 && authority.0.scale_move_speed(sim::ClientId(id), want / had) {
                        speed_applied.insert(id, want);
                    }
                }
            }
        }
        // The console's boxes: `mc fill` from the feet, `mc set` from the
        // world's spawn block. A client asks its host for them.
        let spawn_block = (origin[0].floor() as i32, origin[1].floor() as i32, origin[2].floor() as i32);
        let mut boxes: Vec<_> = std::mem::take(&mut ui.fill).into_iter().map(|(a, b, id)| (block, a, b, id)).collect();
        boxes.extend(std::mem::take(&mut ui.set).into_iter().map(|(a, b, id)| (spawn_block, a, b, id)));
        if let NetRole::Client(client) = net_role {
            for (base, a, b, id) in std::mem::take(&mut boxes) {
                let (a, b) = fill_box(base, a, b);
                client.send(net.inbox.as_deref_mut(), net.ids.as_deref_mut(), local.0, &Intent::Fill { a, b, block: id });
            }
        }
        // The console's summons, from the feet, and a client's punches on
        // mobs: a client asks its host for both.
        for (kind, rel) in std::mem::take(&mut ui.summon) {
            let at = [feet[0] + rel[0], feet[1] + rel[1], feet[2] + rel[2]];
            if let NetRole::Client(client) = &mut *net_role {
                client.send(net.inbox.as_deref_mut(), net.ids.as_deref_mut(), local.0, &Intent::Summon { kind, at });
            } else {
                entities.summon(&kind, at);
            }
        }
        if let NetRole::Client(client) = &mut *net_role {
            for key in std::mem::take(&mut entities.pending_attacks) {
                client.send(net.inbox.as_deref_mut(), net.ids.as_deref_mut(), local.0, &Intent::Attack { key });
            }
        }
        let mut filled = Vec::new();
        for (base, a, b, id) in boxes {
            let block_of = match id
                .as_deref()
                .map(minecraft_terrain::pack::ResourceId::parse)
            {
                None => None,
                Some(Ok(id)) => Some(minecraft_terrain::scene::Block {
                    id,
                    properties: Default::default(),
                }),
                Some(Err(error)) => {
                    diag::warn!(World, "Minecraft fill: {error}");
                    continue;
                }
            };
            let span = |i: usize| a[i].min(b[i])..=a[i].max(b[i]);
            for x in span(0) {
                for y in span(1) {
                    for z in span(2) {
                        let pos = (base.0 + x, base.1 + y, base.2 + z);
                        changes.push((pos, block_of.clone()));
                        filled.push(pos);
                    }
                }
            }
        }
        if !changes.is_empty() {
            let blocks = &world.registries.blocks;
            let mut positions = Vec::with_capacity(changes.len());
            for (pos, block) in changes {
                // The level also ticks where only a client's mobs live: the
                // scene takes such a change once the chunk comes.
                let chunk = (pos.0 >> 4, pos.2 >> 4);
                if world.scene.generated_chunk(chunk).is_none() {
                    deferred.entry(chunk).or_default().push((pos, block.clone()));
                }
                let state = block.as_ref().and_then(|b| world.stream.states.state_of(b));
                let shape = state.map_or(0, |state| {
                    *shapes.entry(state).or_insert_with(|| {
                        let boxes = blocks.collision_boxes(state);
                        if boxes.is_empty() {
                            return 0;
                        }
                        let key: Vec<[u32; 6]> = boxes.iter().map(|b| b.map(|v| (v as f32).to_bits())).collect();
                        if let Some(&id) = shape_ids.get(&key) {
                            return id;
                        }
                        let boxes32 = boxes.iter().map(|b| b.map(|v| v as f32)).collect();
                        let id = sim::voxel::add_shapes(vec![boxes32]).unwrap_or(0);
                        shape_ids.insert(key, id);
                        id
                    })
                });
                sim::voxel::set_block_shape(pos.0, pos.1, pos.2, shape);
                world.scene.set(pos, block);
                positions.push(pos);
            }
            changed.extend(positions.iter().copied());
            world.stream.mark_edited(&world.scene, &positions);
        }
        world.stream.record_edits(&world.scene, &filled);
        for pos in filled {
            entities.placed(&world.scene, pos);
        }
        for (amount, from) in hits {
            sim::voxel::push_player_damage(local.0.0, amount, from.map(|b| sim::voxel::to_map(origin, b)));
        }
        // And on the host's clients.
        for (client, amount, from) in std::mem::take(&mut entities.remote_hits) {
            sim::voxel::push_player_damage(client, amount, from.map(|b| sim::voxel::to_map(origin, b)));
        }
        // The inventory: MW2 guns as items, the HUD's clicks, the hotbar's
        // gun, and what the HUD shows.
        let owned: Vec<u32> = ps
            .weapons
            .iter()
            .filter(|&&w| w > 0)
            .map(|&w| w as u32)
            .filter(|&w| {
                let sim_world = authority.as_ref().map(|authority| &authority.0).or(net.prediction.as_ref().map(|prediction| prediction.0.world()));
                sim_world.is_some_and(|sim_world| sim_world.weapon_combat_row(w).is_some_and(|facts| facts.inventory_type == 0))
            })
            .collect();
        ui.active = alive;
        // A crafting table's screen holds while its table stands within
        // eight blocks of the player, as vanilla's `stillValid`.
        let table_lost = ui.workbench.is_some_and(|pos| {
            let centre = [pos.0 as f64 + 0.5, pos.1 as f64 + 0.5, pos.2 as f64 + 0.5];
            let far = (0..3).map(|i| (feet[i] - centre[i]).powi(2)).sum::<f64>() > 64.0;
            far || !is_crafting_table(&world.scene, pos)
        });
        if ui.inventory_open && (!alive || table_lost) {
            ui.inventory_open = false;
            ui.clicks.push(frame::McClick::Close);
        }
        inventory_ui.sync_weapons(&mut entities.inventory, &owned);
        // The local player's permanent gun unlocks and universal ammo, host
        // authoritative only (a pure client waits for the host's state); a
        // dedicated server has no local player of its own.
        diag::info!(
            World,
            "TEMP-GUN-DEBUG3: dedicated={dedicated} is_host={} ammo_in_inventory={}",
            matches!(net_role, NetRole::Host(_)),
            entities.inventory.count(crate::minecraft_inventory::AMMO_ITEM)
        );
        if !dedicated
            && let NetRole::Host(host) = net_role
            && let Some(authority) = authority.as_mut()
        {
            let authority = &mut authority.0;
            let fingerprint = host.local.as_ref().map(|(key, _)| key.clone());
            let mut unlocked = std::mem::take(&mut host.local_unlocked_weapons);
            crate::minecraft_arsenal::drain_ammo(authority, local.0, &mut entities.inventory);
            let mut crafted = false;
            crate::minecraft_arsenal::feed_arsenal(authority, local.0, &mut entities.inventory, &mut unlocked, |weapon| {
                if let Some(key) = fingerprint.as_deref() {
                    host.players.add_unlocked_weapon(key, weapon);
                }
                crafted = true;
                diag::info!(World, "Minecraft: crafted {weapon}");
            });
            host.local_unlocked_weapons = unlocked;
            if crafted {
                host.dirty = true;
            }
        }
        let mut selected = entities.selected;
        // A client applies its clicks itself and asks its host to apply them
        // too, in the order it made them.
        if let NetRole::Client(client) = net_role {
            let mut intents = Vec::new();
            if ui.workbench != client.table {
                client.table = ui.workbench;
                if let Some(pos) = ui.workbench {
                    intents.push(Intent::OpenTable { pos });
                }
            }
            if ui.clear_items {
                intents.push(Intent::Clear);
            }
            for (id, count) in &ui.give {
                intents.push(Intent::Give { id: id.clone(), count: *count });
            }
            for click in &ui.clicks {
                if let Some(click) = crate::minecraft_mp::wire_click(click) {
                    intents.push(Intent::Click(click));
                }
            }
            let next = ui.select.map_or(selected, |slot| slot.min(frame::minecraft_ui::MC_HOTBAR - 1));
            if next != client.selected {
                client.selected = next;
                intents.push(Intent::Select { slot: next as u8 });
            }
            if let Some(whole) = ui.drop_selected {
                intents.push(Intent::Drop { whole });
            }
            for intent in intents {
                client.send(net.inbox.as_deref_mut(), net.ids.as_deref_mut(), local.0, &intent);
            }
        }
        let thrown = inventory_ui.apply_input(&mut ui, &mut entities.inventory, &mut selected);
        let thrower = crate::minecraft_inventory::Thrower {
            eye: glam::DVec3::from_array(eye),
            yaw: mc_yaw,
            pitch: ps.viewangles[0],
        };
        // A client's thrown stacks land where its host throws them.
        if !is_client {
            crate::minecraft_inventory::throw(&mut entities.world_items, thrown, &thrower);
        }
        ui.weapon_request = inventory_ui.weapon_request(&entities.inventory, &mut selected, ps.weapon as u32);
        if let NetRole::Client(client) = net_role
            && selected != client.selected
        {
            client.selected = selected;
            client.send(net.inbox.as_deref_mut(), net.ids.as_deref_mut(), local.0, &Intent::Select { slot: selected as u8 });
        }
        entities.selected = selected;
        if !dedicated {
            inventory_ui.publish(&mut ui, &entities.inventory, selected, &world.packs, &mut images, draws);
        }

        // A dedicated server plays none.
        let mob_sounds = std::mem::take(&mut entities.sounds);
        if let Some(sounds) = sounds.as_mut() {
            for (event, position, volume, pitch) in mob_sounds {
                sounds.play(&world.packs, &event, Some(at(position.to_array())), volume, pitch);
            }
        }
        // The held item in view; an empty hand is MW2's own hands.
        view.hand = Default::default();
        ui.hand = None;
        let swing = hand.swing.map_or(0.0, |t| (t / crate::minecraft_hand::SWING_TICKS).clamp(0.0, 1.0));
        ui.hand_swing = swing;
        // The map in hand, as this game has it: the host's own, or what the
        // host sent a client.
        let held_map_id = entities.inventory.slots[entities.selected].as_ref().and_then(crate::minecraft_inventory::map_of);
        let held_map = held_map_id.and_then(|id| match &*net_role {
            NetRole::Host(host) => host.maps.maps.get(&id),
            NetRole::Client(client) => client.maps.get(&id),
            NetRole::None => None,
        });
        ui.held_map = held_map_id.map(|id| match held_map {
            Some(map) => format!("{id}@{},{}:{}", map.centre.0, map.centre.1, map.explored()),
            None => format!("{id}@-:0"),
        });
        if draws
            && ui.holding_item
            && !puppet.active
            && let Some(stack) = entities.inventory.slots[entities.selected].clone()
        {
            let eye_light_at = glam::Vec3::new(eye[0] as f32, eye[1] as f32, eye[2] as f32);
            let (vertices, indices, faces) = if stack.id == minecraft_terrain::filled_map::ITEM {
                let region = |id: &str| {
                    minecraft_terrain::pack::ResourceId::parse(id).map_or([0.0; 4], |id| world.atlas.region(&id))
                };
                let sprites = crate::minecraft_hand::MapSprites {
                    white: region(minecraft_terrain::mesh::WHITE_TEXTURE),
                    background: region(minecraft_terrain::filled_map::BACKGROUND),
                    checkerboard: region(minecraft_terrain::filled_map::BACKGROUND_CHECKERBOARD),
                    player: region(minecraft_terrain::filled_map::PLAYER_MARKER),
                    off_map: region(minecraft_terrain::filled_map::PLAYER_OFF_MAP),
                };
                let at = (eye[0].floor() as i32, eye[1].floor() as i32, eye[2].floor() as i32);
                let lit = [f32::from(light.get(at)), f32::from(light.get_block(at))];
                let marker = held_map.and_then(|map| crate::minecraft_hand::map_marker(map, feet[0], feet[2], mc_yaw));
                let pose = crate::minecraft_hand::map_pose(ps.viewangles[0], swing);
                let (vertices, indices) = crate::minecraft_hand::map_mesh(held_map, marker, pose, &sprites, lit);
                let faces = indices.len() / 6;
                (vertices, indices, faces)
            } else {
                let display = minecraft_terrain::pack::ResourceId::parse(&stack.id)
                    .ok()
                    .and_then(|id| minecraft_terrain::model::item_first_person_transform(&world.packs, &id).ok())
                    .unwrap_or(glam::Mat4::IDENTITY);
                let pose = crate::minecraft_hand::item_pose(display, swing, 0.0);
                let mesh = entities.held_item_mesh(&stack.id, pose, eye_light_at, &world.packs, &world.atlas, light);
                (mesh.vertices, mesh.indices, mesh.faces)
            };
            ui.hand = Some((stack.id.clone(), faces));
            let vertices: Vec<minecraft_terrain::mesh::SectionVertex> =
                vertices.iter().map(minecraft_terrain::mesh::SectionVertex::from_vertex).collect();
            view.hand = (bytemuck::cast_slice(&vertices).to_vec(), indices);
            // Reverse-Z with no far plane, as the scene's; 70 degrees up.
            let f = 1.0 / (35.0f32.to_radians()).tan();
            let near = 0.05;
            view.hand_clip = Mat4::from_cols(
                Vec4::new(f / aspect, 0.0, 0.0, 0.0),
                Vec4::new(0.0, f, 0.0, 0.0),
                Vec4::new(0.0, 0.0, 0.0, -1.0),
                Vec4::new(0.0, 0.0, near, 0.0),
            )
            .to_cols_array();
        }

        sim::voxel::set_mob_boxes(entities.boxes());
        entities.tick_scene(&world.scene, mob_ticks);
        if !dedicated {
            ui.ground_items = entities
                .world_items
                .entities
                .iter()
                .map(|entity| entity.stack.id.clone())
                .collect();
        }
        if draws {
            let sky_darken = (15.0 - world.environment.sky_light_level()).clamp(0.0, 15.0) as u8;
            let meshes = entities.meshes(
                &world.scene,
                &world.packs,
                &world.atlas,
                light,
                forward,
                glam::DVec3::from_array(eye),
                sky_darken,
            );
            let raw = |mesh: &minecraft_terrain::mesh::ChunkMesh| {
                (bytemuck::cast_slice::<_, u8>(&mesh.vertices).to_vec(), mesh.indices.clone())
            };
            view.entity_meshes = [
                raw(&meshes.models),
                raw(&meshes.culled),
                raw(&meshes.translucent),
                raw(&meshes.shadows),
            ];
            ui.no_model = meshes.no_model;
            let mesh = meshes.items;
            let (bytes, indices) = &mut view.particles;
            let base = (bytes.len() / std::mem::size_of::<minecraft_terrain::mesh::SectionVertex>()) as u32;
            let vertices: Vec<minecraft_terrain::mesh::SectionVertex> =
                mesh.vertices.iter().map(minecraft_terrain::mesh::SectionVertex::from_vertex).collect();
            bytes.extend_from_slice(bytemuck::cast_slice(&vertices));
            indices.extend(mesh.indices.iter().map(|i| i + base));
        }
    }
    // The host logs what changed and sends each client what it lacks; the
    // world it keeps is saved now and then.
    if let NetRole::Host(host) = &mut *net_role {
        changed.sort_unstable();
        changed.dedup();
        host.columns.resample(&world.scene, &changed);
        // The maps in players' hands fill in where they stand: the host's
        // own player's, and each client's, as the host has them.
        let mut holders = Vec::new();
        if alive
            && let Some(entities) = entities.as_ref()
            && let Some(id) = entities.inventory.slots[entities.selected].as_ref().and_then(crate::minecraft_inventory::map_of)
        {
            holders.push((0, feet, id));
        }
        if let Some(authority) = authority.as_ref() {
            for (&id, remote) in &host.remotes {
                let Some(map) = remote.inventory.slots[remote.selected].as_ref().and_then(crate::minecraft_inventory::map_of) else {
                    continue;
                };
                if let Some(ps) = authority.0.player(sim::ClientId(id)).filter(|ps| ps.pm_type == 0) {
                    holders.push((crate::minecraft_entities::remote_key(id), sim::voxel::to_block(origin, ps.origin), map));
                }
            }
        }
        host.tick_maps(dt, &holders);
        for pos in changed {
            // A block set in a chunk not generated yet waits in `deferred`;
            // the scene would answer air for it.
            let chunk = (pos.0 >> 4, pos.2 >> 4);
            let waiting = deferred
                .get(&chunk)
                .filter(|_| world.scene.generated_chunk(chunk).is_none())
                .and_then(|edits| edits.iter().rev().find(|(at, _)| *at == pos));
            let block = match waiting {
                Some((_, block)) => block.as_ref().map(block_text),
                None => minecraft_terrain::scene::Scene::block(&world.scene, pos).map(block_text),
            };
            host.record(pos, block);
        }
        if let (Some(entities), Some((key, name))) = (entities.as_ref(), host.local.clone()) {
            host.keep_inventory(&key, &name, &entities.inventory, entities.selected);
        }
        for (key, name, inventory, selected) in crate::minecraft_mp::kept_remotes(host) {
            host.keep_inventory(&key, &name, &inventory, selected);
        }
        if let Some(reliable) = net.reliable.as_mut() {
            let items = entities
                .as_ref()
                .map(|entities| crate::minecraft_mp::wire_items(&entities.world_items))
                .unwrap_or_default();
            let mobs = entities.as_ref().map(|entities| crate::minecraft_mp::MobView {
                mobs: &entities.host_mobs,
                tick: entities.mob_tick,
                fresh: entities.mobs_fresh,
                events: &entities.events,
                projectiles: &entities.projectiles,
                effects: &entities.player_effects,
            });
            let mut remotes = std::mem::take(&mut host.remotes);
            for (&id, remote) in remotes.iter_mut() {
                let feet = authority
                    .as_ref()
                    .and_then(|authority| authority.0.player(sim::ClientId(id)))
                    .map(|ps| sim::voxel::to_block(origin, ps.origin));
                crate::minecraft_mp::send_to(
                    reliable,
                    sim::ClientId(id),
                    host,
                    remote,
                    &items,
                    feet,
                    dt,
                    mobs.as_ref(),
                    net.frames.as_deref_mut(),
                );
            }
            host.remotes = remotes;
            if let Some(entities) = entities.as_mut() {
                entities.mobs_fresh = false;
            }
        }
        host.autosave(dt);
    }
    // The console's `admin transfer`: done here on a host, asked of the host
    // on a client (which it does for an admin only).
    for (from, to, label) in std::mem::take(&mut ui.transfers) {
        let label_text = label.clone().unwrap_or_else(|| "-".to_owned());
        let line = match &mut *net_role {
            NetRole::Host(host) => crate::minecraft_mp::admin_transfer(host, net.admins.as_deref_mut(), &from, &to, label.as_deref()),
            NetRole::Client(client) => {
                let from_wire = match minecraft_terrain::multiplayer::fingerprint_bytes(&from) {
                    Some(key) => minecraft_terrain::multiplayer::TransferFrom::Key(key),
                    None => minecraft_terrain::multiplayer::TransferFrom::Name(from.clone()),
                };
                match minecraft_terrain::multiplayer::fingerprint_bytes(&to) {
                    Some(to) if client.send(net.inbox.as_deref_mut(), net.ids.as_deref_mut(), local.0, &Intent::Transfer { from: from_wire, to }) => {
                        format!("admin-transfer: label={label_text} result=asked")
                    }
                    _ => format!("admin-transfer: label={label_text} result=refused reason=\"not sent\""),
                }
            }
            NetRole::None => format!("admin-transfer: label={label_text} result=refused reason=\"no Minecraft world here\""),
        };
        diag::info!(Console, "{line}");
        println!("{line}");
    }
    // The console's block probes, from the world's spawn block.
    for (label, at) in std::mem::take(&mut ui.probes) {
        let pos = (origin[0].floor() as i32 + at[0], origin[1].floor() as i32 + at[1], origin[2].floor() as i32 + at[2]);
        let block = minecraft_terrain::scene::Scene::block(&world.scene, pos).map_or_else(|| "air".to_owned(), block_text);
        let line = format!("minecraft-block: label={label} at={},{},{} block={block}", at[0], at[1], at[2]);
        diag::info!(Console, "{line}");
        println!("{line}");
    }
    // The console's mob census: the mobs this game shows, nearest first, and
    // the player's eye, all from the world's spawn block.
    for (label, kind) in std::mem::take(&mut ui.mob_probes) {
        let base = [origin[0].floor(), origin[1].floor(), origin[2].floor()];
        let eye = sim::voxel::to_block(origin, [ps.origin[0], ps.origin[1], ps.origin[2] + ps.view_height_current]);
        let mut mobs = entities.as_ref().map(|entities| entities.census()).unwrap_or_default();
        let distance = |at: &[f64; 3]| (0..3).map(|i| (at[i] - feet[i]).powi(2)).sum::<f64>();
        // Of every kind, the living ones more than 48 blocks away.
        let far = mobs.iter().filter(|mob| !mob.4 && distance(&mob.2) > 48.0 * 48.0).count();
        mobs.retain(|mob| kind.as_deref().is_none_or(|kind| kind == mob.1));
        mobs.sort_by(|a, b| distance(&a.2).total_cmp(&distance(&b.2)));
        let list: Vec<String> = mobs
            .iter()
            .take(12)
            .map(|(id, kind, at, health, dead)| {
                format!(
                    "{kind}:{id}:{:.2},{:.2},{:.2}:{health:.1}:{}",
                    at[0] - base[0],
                    at[1] - base[1],
                    at[2] - base[2],
                    if *dead { "dead" } else { "alive" }
                )
            })
            .collect();
        // And the items on the ground within eight blocks.
        let items: Vec<String> = entities
            .as_ref()
            .map(|entities| {
                entities
                    .world_items
                    .entities
                    .iter()
                    .filter(|item| (0..3).map(|i| (item.position[i] - feet[i]).powi(2)).sum::<f64>() < 64.0)
                    .map(|item| format!("{}*{}", item.stack.id, item.stack.count))
                    .collect()
            })
            .unwrap_or_default();
        // And the hotbar slots (from 1) that hold a bullet gun, not a
        // launcher or a grenade.
        let sim_world = authority
            .as_ref()
            .map(|authority| &authority.0)
            .or(net.prediction.as_ref().map(|prediction| prediction.0.world()));
        let guns: Vec<String> = entities
            .as_ref()
            .map(|entities| {
                entities.inventory.slots[..frame::minecraft_ui::MC_HOTBAR]
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, stack)| {
                        let weapon = stack.as_ref()?.id.strip_prefix("iw4:weapon/")?.parse::<u32>().ok()?;
                        let facts = sim_world?.weapon_combat_row(weapon)?;
                        (facts.weap_type == 0).then(|| (slot + 1).to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();
        // And the hotbar slot (from 1) of the weapon the player holds.
        let held = entities
            .as_ref()
            .and_then(|entities| held_slot(&entities.inventory.slots[..frame::minecraft_ui::MC_HOTBAR], ps.weapon as u32))
            .map_or_else(|| "-".to_owned(), |slot| slot.to_string());
        // And the selected hotbar slot (from 1), a gun or not.
        let selected_slot = entities
            .as_ref()
            .map_or_else(|| "-".to_owned(), |entities| (entities.selected + 1).to_string());
        // The player's status effects (effect:level:ticks left), and the
        // arrows and potions this game has drawn so far.
        let effects: Vec<String> = entities
            .as_ref()
            .map(|entities| {
                entities
                    .effects()
                    .iter()
                    .map(|e| format!("{}:{}:{}", e.effect.id().trim_start_matches("minecraft:"), e.amplifier, e.duration))
                    .collect()
            })
            .unwrap_or_default();
        let seen: Vec<String> = entities
            .as_ref()
            .map(|entities| entities.projectiles_seen().iter().map(|(kind, n)| format!("{kind}:{n}")).collect())
            .unwrap_or_default();
        let line = format!(
            "minecraft-mobs: label={label} role={} health={} eye={:.2},{:.2},{:.2} guns={} held={held} selected={selected_slot} count={} far={far} mobs={} items={} effects={} flown={}",
            ui.net_role,
            ps.health,
            eye[0] - base[0],
            eye[1] - base[1],
            eye[2] - base[2],
            if guns.is_empty() { "-".to_owned() } else { guns.join(",") },
            mobs.len(),
            if list.is_empty() { "-".to_owned() } else { list.join(";") },
            if items.is_empty() { "-".to_owned() } else { items.join(",") },
            if effects.is_empty() { "-".to_owned() } else { effects.join(",") },
            if seen.is_empty() { "-".to_owned() } else { seen.join(",") }
        );
        diag::info!(Console, "{line}");
        println!("{line}");
    }
    // The console's sound log: the world sounds this game took, the latest
    // last, each with how far from the player it was.
    for label in std::mem::take(&mut ui.sound_probes) {
        let here = glam::DVec3::from_array(feet);
        let (heard, total) = entities.as_ref().map_or((Vec::new(), 0), |entities| {
            let (heard, total) = entities.heard();
            let heard: Vec<String> =
                heard.map(|(event, at)| format!("{}@{:.1}", event.trim_start_matches("minecraft:"), at.distance(here))).collect();
            (heard, total)
        });
        let line = format!(
            "minecraft-sounds: label={label} role={} total={total} events={}",
            ui.net_role,
            if heard.is_empty() { "-".to_owned() } else { heard.join(",") }
        );
        diag::info!(Console, "{line}");
        println!("{line}");
    }
    // The console's spawn range: on a host, where natural spawning put mobs
    // and where mobs despawned around its first client (or its own player),
    // and how far the mobs it tracks stand from that player now.
    for label in std::mem::take(&mut ui.range_probes) {
        let Some(entities) = entities.as_ref() else { continue };
        let key = match &*net_role {
            NetRole::Host(host) => host.remotes.keys().next().map_or(0, |&id| crate::minecraft_entities::remote_key(id)),
            _ => 0,
        };
        let players = entities.tick_players();
        let target = players.iter().find(|(k, _)| *k == key).map(|(_, at)| *at);
        let (mut near, mut band, mut beyond, mut stray) = (0, 0, 0, 0);
        for (at, goes) in entities.tracked_census() {
            if let Some(target) = target {
                let d = target.distance(at);
                if d <= 48.0 {
                    near += 1;
                } else if d <= 128.0 {
                    band += 1;
                } else {
                    beyond += 1;
                }
            }
            // `Mob.checkDespawn` takes one beyond 128 blocks of every player
            // at once; give it a few blocks for the tick it took to walk there.
            let nearest = players.iter().map(|(_, p)| p.distance(at)).fold(f64::INFINITY, f64::min);
            stray += usize::from(goes && nearest > 132.0);
        }
        let stats = entities.range_stats(key);
        let blocks = |v: f64| if v.is_finite() { format!("{v:.1}") } else { "-".to_owned() };
        let line = format!(
            "minecraft-mobrange: label={label} role={} player={key} spawned={} spawn_min={} spawn_max={} spawn_beyond48={} despawned={} despawn_min={} despawn_max={} near48={near} band48_128={band} beyond128={beyond} stray={stray} level_chunks={}",
            ui.net_role,
            stats.spawned,
            blocks(stats.spawn_min),
            blocks(stats.spawn_max),
            stats.spawn_beyond_48,
            stats.despawned,
            blocks(stats.despawn_min),
            blocks(stats.despawn_max),
            entities.level_chunks,
        );
        diag::info!(Console, "{line}");
        println!("{line}");
    }
    // The console's natural spawning, on a host (`mc spawning on|off`).
    if let Some(on) = ui.natural_spawning.take() {
        let line = match (&*net_role, entities.as_mut()) {
            (NetRole::Client(_), _) => "refused: a client's host spawns its mobs".to_owned(),
            (_, Some(entities)) => {
                entities.set_natural_spawning(on);
                format!("natural spawning {}", if on { "on" } else { "off" })
            }
            (_, None) => "no world".to_owned(),
        };
        let line = format!("minecraft-spawning: {line}");
        diag::info!(Console, "{line}");
        println!("{line}");
    }
    if !draws {
        day.advance(dt);
        return;
    }
    view.cracks = mining.crack_mesh();
    day.advance(dt);
    let eye_block = (eye[0].floor() as i32, eye[1].floor() as i32, eye[2].floor() as i32);
    view.eye_light = [
        f32::from(light.get(eye_block)),
        f32::from(light.get_block(eye_block)),
    ];
    world
        .environment
        .update_rain_fog(0.0, light.get(eye_block), false, (dt * 20.0) as f32);
    *environment_accumulator += dt;
    if !*environment_primed || *environment_accumulator >= TICK_SECONDS {
        *environment_accumulator = (*environment_accumulator % TICK_SECONDS).min(TICK_SECONDS);
        let scene = &world.scene;
        world.environment.tick(
            day.ticks.floor() as i64,
            0.0,
            0.0,
            eye,
            |x, y, z| scene.noise_biome((x, y, z)).map_or(0, |id| id.0),
            !*environment_primed,
        );
        *environment_primed = true;
    }
    let partial_tick = (*environment_accumulator / TICK_SECONDS).clamp(0.0, 1.0) as f32;
    let sky = world.environment.sky_state(&View {
        partial_tick,
        forward,
        camera_y: eye[1] as f32,
        render_distance: world.stream.view_distance() as u32,
        rain_level: 0.0,
        thunder_level: 0.0,
    });
    let render_distance = world.stream.view_distance() as f32 * 16.0;
    let right = forward.cross(glam::Vec3::Y).normalize_or(glam::Vec3::X);
    let up = right.cross(forward).normalize_or(glam::Vec3::Y);
    let put = |v: glam::Vec3| [v.x, v.y, v.z, 0.0];
    let game_time = day.ticks;
    view.environment = [
        put(forward),
        put(right),
        put(up),
        [eye[0] as f32, eye[1] as f32, eye[2] as f32, 0.0],
        put(sky.sky),
        [sky.fog.x, sky.fog.y, sky.fog.z, render_distance.min(sky.sky_fog_end)],
        [
            sky.sky_light_color.x,
            sky.sky_light_color.y,
            sky.sky_light_color.z,
            sky.sky_light_factor,
        ],
        sky.sunset,
        [sky.sun_direction.x, sky.sun_direction.y, sky.sun_direction.z, sky.rain_brightness],
        [sky.moon_direction.x, sky.moon_direction.y, sky.moon_direction.z, sky.rain_brightness],
        // The brightness option at its default.
        [sky.cloud.x, sky.cloud.y, sky.cloud.z, 0.5],
        [aspect, 0.0, sky.star_brightness, sky.star_angle],
        [sky.moon_phase as f32, (game_time as f32) * 0.03, 96.0, 160.0],
        [
            sky.fog_start,
            sky.fog_end,
            render_distance - (render_distance / 10.0).clamp(4.0, 64.0),
            render_distance,
        ],
        [
            sky.ambient.x,
            sky.ambient.y,
            sky.ambient.z,
            match sky.skybox {
                Skybox::Overworld => 0.0,
                Skybox::End => 1.0,
                _ => 2.0,
            },
        ],
        [
            sky.block_light_tint.x,
            sky.block_light_tint.y,
            sky.block_light_tint.z,
            sky.block_factor,
        ],
    ];

    // Clouds, rebuilt when the camera crosses a cloud cell.
    if let Some(mask) = &world.cloud_mask {
        let center = mask.center(eye[0] as f32, eye[2] as f32, game_time);
        if *cloud_center != Some(center) {
            *cloud_center = Some(center);
            let mesh = mask.build(center, eye[1] as f32);
            view.clouds = Some(Arc::new((mesh.vertices, mesh.indices)));
        }
    }

    // The light MW2 models stand in, around the player.
    *light_volume_age += 1;
    let half = LIGHT_VOLUME / 2;
    let corner = [block.0 - half, block.1 - half, block.2 - half];
    let moved =
        light_volume_at.is_none_or(|at| (0..3).any(|k| (at[k] - corner[k]).abs() >= 4));
    if moved || *light_volume_age >= 20 {
        *light_volume_age = 0;
        *light_volume_at = Some(corner);
        let n = LIGHT_VOLUME;
        let index = |x: i32, y: i32, z: i32| (((y * n + z) * n + x) * 2) as usize;
        let mut raw = vec![0u8; (n * n * n * 2) as usize];
        for z in 0..n {
            for x in 0..n {
                let (bx, bz) = (corner[0] + x, corner[2] + z);
                // One column lookup for the whole stack, not two per cell.
                let column = light.chunk_column((bx >> 4, bz >> 4));
                for y in 0..n {
                    let pos = (bx, corner[1] + y, bz);
                    let at = index(x, y, z);
                    let (sky, block) = match column {
                        Some(column) => (column.get(pos), column.get_block(pos)),
                        None => (light.get(pos), light.get_block(pos)),
                    };
                    raw[at] = sky;
                    raw[at + 1] = block;
                }
            }
        }
        // Unlit cells (inside blocks) take their brightest neighbour, so a
        // model beside a block is not darkened by filtering into it. Levels
        // become unorm bytes.
        let mut data = vec![0u8; raw.len()];
        for y in 0..n {
            for z in 0..n {
                for x in 0..n {
                    let at = index(x, y, z);
                    let (mut sky, mut block) = (raw[at], raw[at + 1]);
                    if sky == 0 && block == 0 {
                        for (dx, dy, dz) in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
                            let (nx, ny, nz) = (x + dx, y + dy, z + dz);
                            if (0..n).contains(&nx) && (0..n).contains(&ny) && (0..n).contains(&nz) {
                                let near = index(nx, ny, nz);
                                sky = sky.max(raw[near]);
                                block = block.max(raw[near + 1]);
                            }
                        }
                    }
                    data[at] = sky.min(15) * 17;
                    data[at + 1] = block.min(15) * 17;
                }
            }
        }
        view.light_volume = Some(Arc::new((corner, data)));
    }
}

/// A host keeps every player's inventory under its name and saves the world
/// it keeps, once the world is in.
fn keep_world(runtime: &mut Runtime) {
    let crate::minecraft_mp::NetRole::Host(host) = &mut runtime.net else {
        return;
    };
    if runtime.world.is_none() {
        return;
    }
    if let (Some(entities), Some((key, name))) = (runtime.entities.as_ref(), host.local.clone()) {
        host.keep_inventory(&key, &name, &entities.inventory, entities.selected);
    }
    for (key, name, inventory, selected) in crate::minecraft_mp::kept_remotes(host) {
        host.keep_inventory(&key, &name, &inventory, selected);
    }
    host.save_now();
}

fn stop(runtime: &mut Runtime, view: &mut MinecraftWorldView) {
    keep_world(runtime);
    runtime.net = crate::minecraft_mp::NetRole::None;
    runtime.deferred.clear();
    runtime.voxel_holders.clear();
    runtime.level_holders.clear();
    runtime.speed_applied.clear();
    if runtime.world.take().is_some() || runtime.loading.take().is_some() || view.active {
        sim::voxel::deactivate();
        view.active = false;
        view.atlas = None;
        view.uploads.clear();
        view.removed.clear();
        view.visible.clear();
        view.celestial = None;
        view.crack_texture = None;
        view.particles = Default::default();
        view.entity_meshes = Default::default();
        view.hand = Default::default();
        view.cracks = Default::default();
        view.clouds = None;
        view.light_volume = None;
        view.generation += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_held_slot_is_the_one_with_the_held_weapon() {
        use minecraftoss_player::inventory::ItemStack;
        let hotbar = [
            Some(ItemStack::new("iw4:weapon/79", 1)),
            Some(ItemStack::new("iw4:weapon/713", 1)),
            None,
            Some(ItemStack::new("minecraft:oak_planks", 4)),
        ];
        assert_eq!(held_slot(&hotbar, 713), Some(2));
        assert_eq!(held_slot(&hotbar, 79), Some(1));
        assert_eq!(held_slot(&hotbar, 7130), None);
        assert_eq!(held_slot(&[], 79), None);
    }

    #[test]
    fn a_chunk_s_logged_changes_are_each_position_once() {
        let mut log = minecraft_terrain::multiplayer::EditLog::default();
        log.push((322, 61, 0), Some("minecraft:stone".into()));
        log.push((322, 61, 0), None);
        log.push((321, 62, -1), Some("minecraft:stone".into()));
        log.push((336, 61, 0), Some("minecraft:stone".into()));
        let chunk20: Vec<_> = logged_in(&log, (20, 0)).into_iter().collect();
        assert_eq!(chunk20, [(322, 61, 0)]);
        let chunk20_back: Vec<_> = logged_in(&log, (20, -1)).into_iter().collect();
        assert_eq!(chunk20_back, [(321, 62, -1)]);
        assert_eq!(logged_in(&log, (21, 0)).len(), 1);
    }
}
