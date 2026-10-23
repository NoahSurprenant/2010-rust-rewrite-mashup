//! Vanilla's filled map (`minecraft:filled_map`): a 128 by 128 picture of
//! the blocks around its centre, one pixel a block at scale 0, each pixel a
//! map colour (the top block's `MapColor`) shaded by its height against the
//! pixel north of it. The map fills in as a player holding it walks
//! (`MapItem.update`), and its data is kept by its map id
//! (`MapItemSavedData`), separately from the item, which only names the id.
//!
//! Everything here is plain data, independent of the engine, so it is
//! tested on its own.
use std::collections::{BTreeMap, HashMap};

use minecraftoss_core::Chunk;
use minecraftoss_core::chunk::HeightmapKind;

use crate::scene::{Block, HandcraftedScene, Scene};

/// The item.
pub const ITEM: &str = "minecraft:filled_map";
/// The component naming the map's data.
pub const MAP_ID: &str = "minecraft:map_id";
/// Pixels on a side.
pub const SIZE: usize = 128;
/// Pixels on a side of one tile, the unit a map travels in.
pub const TILE: usize = 16;
/// Tiles on a side.
pub const TILES: usize = SIZE / TILE;
/// Tiles in one `HostMessage::MapTiles`: a row stays under 4 KiB.
pub const TILES_PER_MESSAGE: usize = 15;
/// The map's paper (with and without data) and its holder's markers.
pub const BACKGROUND: &str = "minecraft:map/map_background";
pub const BACKGROUND_CHECKERBOARD: &str = "minecraft:map/map_background_checkerboard";
pub const PLAYER_MARKER: &str = "minecraft:map/decorations/player";
pub const PLAYER_OFF_MAP: &str = "minecraft:map/decorations/player_off_map";
pub const TEXTURES: [&str; 4] = [BACKGROUND, BACKGROUND_CHECKERBOARD, PLAYER_MARKER, PLAYER_OFF_MAP];

/// `MapColor`'s 62 base colours, by id (0 is `NONE`, drawn transparent).
pub const BASE_COLOURS: [[u8; 3]; 62] = [
    [0, 0, 0],
    [127, 178, 56],
    [247, 233, 163],
    [199, 199, 199],
    [255, 0, 0],
    [160, 160, 255],
    [167, 167, 167],
    [0, 124, 0],
    [255, 255, 255],
    [164, 168, 184],
    [151, 109, 77],
    [112, 112, 112],
    [64, 64, 255],
    [143, 119, 72],
    [255, 252, 245],
    [216, 127, 51],
    [178, 76, 216],
    [102, 153, 216],
    [229, 229, 51],
    [127, 204, 25],
    [242, 127, 165],
    [76, 76, 76],
    [153, 153, 153],
    [76, 127, 153],
    [127, 63, 178],
    [51, 76, 178],
    [102, 76, 51],
    [102, 127, 51],
    [153, 51, 51],
    [25, 25, 25],
    [250, 238, 77],
    [92, 219, 213],
    [74, 128, 255],
    [0, 217, 58],
    [129, 86, 49],
    [112, 2, 0],
    [209, 177, 161],
    [159, 82, 36],
    [149, 87, 108],
    [112, 108, 138],
    [186, 133, 36],
    [103, 117, 53],
    [160, 77, 78],
    [57, 41, 35],
    [135, 107, 98],
    [87, 92, 92],
    [122, 73, 88],
    [76, 62, 92],
    [76, 50, 35],
    [76, 82, 42],
    [142, 60, 46],
    [37, 22, 16],
    [189, 48, 49],
    [148, 63, 97],
    [92, 25, 29],
    [22, 126, 134],
    [58, 142, 140],
    [86, 44, 62],
    [20, 180, 133],
    [100, 100, 100],
    [216, 175, 147],
    [127, 167, 150],
];

pub const NONE: u8 = 0;
pub const GRASS: u8 = 1;
pub const SAND: u8 = 2;
pub const WOOL: u8 = 3;
pub const FIRE: u8 = 4;
pub const ICE: u8 = 5;
pub const METAL: u8 = 6;
pub const PLANT: u8 = 7;
pub const SNOW: u8 = 8;
pub const CLAY: u8 = 9;
pub const DIRT: u8 = 10;
pub const STONE: u8 = 11;
pub const WATER: u8 = 12;
pub const WOOD: u8 = 13;
pub const QUARTZ: u8 = 14;
pub const COLOR_ORANGE: u8 = 15;
pub const COLOR_MAGENTA: u8 = 16;
pub const COLOR_LIGHT_BLUE: u8 = 17;
pub const COLOR_YELLOW: u8 = 18;
pub const COLOR_LIGHT_GREEN: u8 = 19;
pub const COLOR_PINK: u8 = 20;
pub const COLOR_GRAY: u8 = 21;
pub const COLOR_LIGHT_GRAY: u8 = 22;
pub const COLOR_CYAN: u8 = 23;
pub const COLOR_PURPLE: u8 = 24;
pub const COLOR_BLUE: u8 = 25;
pub const COLOR_BROWN: u8 = 26;
pub const COLOR_GREEN: u8 = 27;
pub const COLOR_RED: u8 = 28;
pub const COLOR_BLACK: u8 = 29;
pub const GOLD: u8 = 30;
pub const DIAMOND: u8 = 31;
pub const LAPIS: u8 = 32;
pub const EMERALD: u8 = 33;
pub const PODZOL: u8 = 34;
pub const NETHER: u8 = 35;
pub const TERRACOTTA_WHITE: u8 = 36;
pub const TERRACOTTA_GRAY: u8 = 43;
pub const TERRACOTTA_LIGHT_GRAY: u8 = 44;
pub const TERRACOTTA_CYAN: u8 = 45;
pub const TERRACOTTA_BROWN: u8 = 48;
pub const CRIMSON_NYLIUM: u8 = 52;
pub const CRIMSON_STEM: u8 = 53;
pub const WARPED_NYLIUM: u8 = 55;
pub const WARPED_STEM: u8 = 56;
pub const WARPED_WART_BLOCK: u8 = 58;
pub const DEEPSLATE: u8 = 59;
pub const RAW_IRON: u8 = 60;
pub const GLOW_LICHEN: u8 = 61;

/// `MapColor.Brightness`, by its id in a packed colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brightness {
    Low = 0,
    Normal = 1,
    High = 2,
    Lowest = 3,
}

impl Brightness {
    pub fn modifier(self) -> u32 {
        match self {
            Self::Low => 180,
            Self::Normal => 220,
            Self::High => 255,
            Self::Lowest => 135,
        }
    }
}

/// A pixel as the map keeps it (`MapColor.getPackedId`).
pub fn packed(base: u8, brightness: Brightness) -> u8 {
    base * 4 + brightness as u8
}

/// A packed pixel's colour (`MapColor.getColorFromPackedId`), `None` for
/// an unexplored or transparent one.
pub fn rgb(packed: u8) -> Option<[u8; 3]> {
    let base = usize::from(packed / 4);
    if base == 0 || base >= BASE_COLOURS.len() {
        return None;
    }
    let modifier = [180u32, 220, 255, 135][usize::from(packed & 3)];
    Some(BASE_COLOURS[base].map(|c| (u32::from(c) * modifier / 255) as u8))
}

/// Where a map made at this block point centres (`MapItemSavedData`'s
/// `createFresh`): the middle of the 128 << scale block square of the grid
/// that holds the point, the grid's squares starting at -64.
pub fn centre(x: f64, z: f64, scale: u8) -> (i32, i32) {
    let span = 128 * (1i32 << scale);
    let snap = |v: f64| ((v + 64.0) / f64::from(span)).floor() as i32 * span + span / 2 - 64;
    (snap(x), snap(z))
}

/// The map id a stack names, if it is a filled map with one.
pub fn map_id(id: &str, components: Option<&serde_json::Value>) -> Option<u32> {
    if id != ITEM {
        return None;
    }
    components?.get(MAP_ID)?.as_u64().and_then(|v| u32::try_from(v).ok())
}

/// The components of a filled map naming `id`.
pub fn components(id: u32) -> serde_json::Value {
    serde_json::json!({ MAP_ID: id })
}

/// The base map colour of a block (`BlockBehaviour.Properties.mapColor`),
/// from its name: the blocks of the generated surface exactly, others by
/// the material their name gives.
pub fn base_colour(block: &Block) -> u8 {
    let path = block.id.path.as_str();
    if is_water(block) && !block.properties.contains_key("waterlogged") {
        return WATER;
    }
    let axis_y = block.properties.get("axis").is_none_or(|a| a == "y");
    colour_of_name(path, axis_y)
}

fn dye_colour(name: &str) -> Option<u8> {
    Some(match name {
        "white" => SNOW,
        "orange" => COLOR_ORANGE,
        "magenta" => COLOR_MAGENTA,
        "light_blue" => COLOR_LIGHT_BLUE,
        "yellow" => COLOR_YELLOW,
        "lime" => COLOR_LIGHT_GREEN,
        "pink" => COLOR_PINK,
        "gray" => COLOR_GRAY,
        "light_gray" => COLOR_LIGHT_GRAY,
        "cyan" => COLOR_CYAN,
        "purple" => COLOR_PURPLE,
        "blue" => COLOR_BLUE,
        "brown" => COLOR_BROWN,
        "green" => COLOR_GREEN,
        "red" => COLOR_RED,
        "black" => COLOR_BLACK,
        _ => return None,
    })
}

const DYES: [&str; 16] = [
    "light_blue",
    "light_gray",
    "white",
    "orange",
    "magenta",
    "yellow",
    "lime",
    "pink",
    "gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];

/// A wood type's planks (top of a log) and bark colours.
fn wood(name: &str) -> Option<(u8, u8)> {
    Some(match name {
        "oak" => (WOOD, PODZOL),
        "spruce" => (PODZOL, COLOR_BROWN),
        "birch" => (SAND, QUARTZ),
        "jungle" => (DIRT, PODZOL),
        "acacia" => (COLOR_ORANGE, STONE),
        "dark_oak" => (COLOR_BROWN, COLOR_BROWN),
        "mangrove" => (COLOR_RED, PODZOL),
        "cherry" => (TERRACOTTA_WHITE, TERRACOTTA_GRAY),
        "pale_oak" => (QUARTZ, STONE),
        "bamboo" => (COLOR_YELLOW, COLOR_YELLOW),
        "crimson" => (CRIMSON_STEM, CRIMSON_STEM),
        "warped" => (WARPED_STEM, WARPED_STEM),
        _ => return None,
    })
}

const WOODS: [&str; 12] = [
    "dark_oak", "pale_oak", "oak", "spruce", "birch", "jungle", "acacia", "mangrove", "cherry", "bamboo", "crimson",
    "warped",
];

fn colour_of_name(path: &str, axis_y: bool) -> u8 {
    // Blocks no map shows: the column goes on down through them.
    match path {
        "air" | "cave_air" | "void_air" | "glass" | "glass_pane" | "barrier" | "light" | "structure_void" | "torch"
        | "wall_torch" | "soul_torch" | "soul_wall_torch" | "redstone_torch" | "redstone_wall_torch" | "redstone_wire"
        | "tripwire" | "tripwire_hook" | "ladder" | "lever" | "rail" | "powered_rail" | "detector_rail"
        | "activator_rail" | "end_rod" | "lightning_rod" | "repeater" | "comparator" | "flower_pot" | "cobweb"
        | "string" | "moving_piston" | "piston_head" | "candle" => return NONE,
        _ => {}
    }
    if path.ends_with("_button") || path.starts_with("potted_") {
        return NONE;
    }
    // Colours by dye name.
    for dye in DYES {
        if let Some(rest) = path.strip_prefix(dye).and_then(|r| r.strip_prefix('_')) {
            if rest == "terracotta" {
                return TERRACOTTA_WHITE + DYES_ORDER.iter().position(|d| *d == dye).unwrap_or(0) as u8;
            }
            return dye_colour(dye).expect("a dye");
        }
    }
    // Wood: planks, logs (planks on top, bark on the side), and what they make.
    for kind in WOODS {
        if let Some(rest) = path.strip_prefix(kind).and_then(|r| r.strip_prefix('_')) {
            let (planks, bark) = wood(kind).expect("a wood");
            return match rest {
                "log" | "stem" => {
                    if axis_y {
                        planks
                    } else {
                        bark
                    }
                }
                "wood" | "hyphae" => bark,
                "leaves" => PLANT,
                "sapling" | "fungus" | "roots" => PLANT,
                "nylium" => {
                    if kind == "crimson" {
                        CRIMSON_NYLIUM
                    } else {
                        WARPED_NYLIUM
                    }
                }
                "wart_block" => WARPED_WART_BLOCK,
                _ => planks,
            };
        }
        if path.strip_prefix("stripped_").is_some_and(|p| p.starts_with(kind)) {
            return wood(kind).expect("a wood").0;
        }
    }
    match path {
        "grass_block" | "slime_block" => GRASS,
        "sand" | "sandstone" | "chiseled_sandstone" | "cut_sandstone" | "smooth_sandstone" | "suspicious_sand"
        | "end_stone" | "end_stone_bricks" | "glowstone" | "bone_block" | "scaffolding" => SAND,
        "red_sand" | "red_sandstone" | "chiseled_red_sandstone" | "cut_red_sandstone" | "smooth_red_sandstone"
        | "terracotta" | "pumpkin" | "carved_pumpkin" | "jack_o_lantern" | "honey_block" | "honeycomb_block"
        | "copper_block" | "cut_copper" | "raw_copper_block" => COLOR_ORANGE,
        "water" | "bubble_column" | "kelp" | "kelp_plant" | "seagrass" | "tall_seagrass" => WATER,
        "lava" | "fire" | "tnt" | "redstone_block" => FIRE,
        "ice" | "packed_ice" | "blue_ice" | "frosted_ice" => ICE,
        "iron_block" | "iron_door" | "iron_trapdoor" | "iron_bars" | "anvil" | "chipped_anvil" | "damaged_anvil"
        | "cauldron" | "water_cauldron" | "lava_cauldron" | "hopper" | "heavy_weighted_pressure_plate"
        | "brewing_stand" | "lantern" | "soul_lantern" | "chain" | "heavy_core" => METAL,
        "snow" | "snow_block" | "powder_snow" => SNOW,
        "clay" => CLAY,
        "dirt" | "coarse_dirt" | "rooted_dirt" | "farmland" | "dirt_path" | "granite" | "polished_granite"
        | "packed_mud" | "mud_bricks" | "brown_mushroom_block" => DIRT,
        "podzol" => PODZOL,
        "mycelium" | "amethyst_block" | "budding_amethyst" | "amethyst_cluster" => COLOR_PURPLE,
        "mud" => TERRACOTTA_CYAN,
        "diorite" | "polished_diorite" | "quartz_block" | "quartz_bricks" | "quartz_pillar" | "chiseled_quartz_block"
        | "smooth_quartz" | "sea_lantern" | "target" => QUARTZ,
        "calcite" => TERRACOTTA_WHITE,
        "tuff" | "polished_tuff" | "tuff_bricks" | "chiseled_tuff" => TERRACOTTA_GRAY,
        "dripstone_block" | "pointed_dripstone" => TERRACOTTA_BROWN,
        "gold_block" | "bell" | "raw_gold_block" => GOLD,
        "diamond_block" | "prismarine_bricks" | "dark_prismarine" | "beacon" => DIAMOND,
        "lapis_block" => LAPIS,
        "emerald_block" => EMERALD,
        "raw_iron_block" => RAW_IRON,
        "glow_lichen" => GLOW_LICHEN,
        "netherrack" | "nether_bricks" | "nether_quartz_ore" | "nether_gold_ore" | "magma_block"
        | "nether_wart_block" | "red_nether_bricks" => NETHER,
        "obsidian" | "crying_obsidian" | "blackstone" | "polished_blackstone" | "basalt" | "polished_basalt"
        | "smooth_basalt" | "sculk" | "sculk_catalyst" | "sculk_vein" | "coal_block" | "dragon_egg" => COLOR_BLACK,
        "soul_sand" | "soul_soil" => COLOR_BROWN,
        "bricks" | "red_mushroom_block" | "nether_wart" | "enchanting_table" => COLOR_RED,
        "melon" => COLOR_LIGHT_GREEN,
        "hay_block" | "sponge" | "wet_sponge" | "bee_nest" | "beehive" => COLOR_YELLOW,
        "prismarine" => COLOR_CYAN,
        "purpur_block" | "purpur_pillar" => COLOR_MAGENTA,
        "moss_block" | "moss_carpet" | "dried_kelp_block" => COLOR_GREEN,
        "mushroom_stem" | "dead_bush" => WOOL,
        "crafting_table" | "chest" | "trapped_chest" | "barrel" | "bookshelf" | "composter" | "lectern"
        | "note_block" | "jukebox" | "loom" | "cartography_table" | "fletching_table" | "smithing_table" => WOOD,
        "short_grass" | "tall_grass" | "fern" | "large_fern" | "vine" | "sugar_cane" | "cactus" | "lily_pad"
        | "bamboo" | "sweet_berry_bush" | "wheat" | "carrots" | "potatoes" | "beetroots" | "melon_stem"
        | "pumpkin_stem" | "azalea" | "flowering_azalea" | "azalea_leaves" | "flowering_azalea_leaves"
        | "big_dripleaf" | "small_dripleaf" | "spore_blossom" | "hanging_roots" | "cave_vines" | "cave_vines_plant"
        | "leaf_litter" | "bush" | "firefly_bush" | "short_dry_grass" | "tall_dry_grass" | "cactus_flower"
        | "pink_petals" | "wildflowers" | "dandelion" | "poppy" | "blue_orchid" | "allium" | "azure_bluet"
        | "oxeye_daisy" | "cornflower" | "lily_of_the_valley" | "sunflower" | "lilac" | "rose_bush" | "peony"
        | "torchflower" | "pitcher_plant" | "open_eyeblossom" | "closed_eyeblossom" | "brown_mushroom"
        | "red_mushroom" => PLANT,
        p if p.ends_with("_tulip") || p.ends_with("_leaves") || p.ends_with("_sapling") || p.ends_with("_coral")
            || p.ends_with("_coral_fan") =>
        {
            PLANT
        }
        p if p.contains("deepslate") => DEEPSLATE,
        p if p.contains("copper") => COLOR_ORANGE,
        p if p.contains("sandstone") => SAND,
        p if p.contains("quartz") => QUARTZ,
        p if p.contains("blackstone") => COLOR_BLACK,
        p if p.contains("nether_brick") => NETHER,
        p if p.contains("prismarine") => COLOR_CYAN,
        p if p.contains("purpur") => COLOR_MAGENTA,
        p if p.contains("mud_brick") => DIRT,
        p if p.contains("brick") && !p.contains("stone") => COLOR_RED,
        p if p.contains("granite") => DIRT,
        p if p.contains("diorite") => QUARTZ,
        p if p.contains("tuff") => TERRACOTTA_GRAY,
        p if p.ends_with("_wool") || p.ends_with("_carpet") => WOOL,
        p if p.ends_with("_ore") => STONE,
        _ => STONE,
    }
}

/// The dyes in `MapColor`'s terracotta order (`TERRACOTTA_WHITE` first).
const DYES_ORDER: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];

/// Whether a block holds water (its own, or waterlogged).
pub fn is_water(block: &Block) -> bool {
    matches!(
        block.id.path.as_str(),
        "water" | "bubble_column" | "kelp" | "kelp_plant" | "seagrass" | "tall_seagrass"
    ) || block.properties.get("waterlogged").is_some_and(|v| v == "true")
}

fn has_fluid(block: &Block) -> bool {
    is_water(block) || block.id.path == "lava"
}

/// One block column as a map pixel at scale 0 reads it: the top block's
/// base colour, that block's height, and for water the blocks of water
/// under the surface counted as `MapItem.update` counts them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Column {
    pub colour: u8,
    pub y: i32,
    pub water_depth: u16,
}

/// A column from its top (the first free height over the highest block)
/// down, `block_at` giving each block (`None` for air): `MapItem.update`'s
/// search for the first block a map shows, and the water under it.
pub fn column<'b>(top: i32, min_y: i32, block_at: impl Fn(i32) -> Option<&'b Block>) -> Column {
    let mut y = top;
    if y <= min_y + 1 {
        return Column { colour: STONE, y: min_y, water_depth: 0 };
    }
    let mut found;
    loop {
        y -= 1;
        found = block_at(y);
        let colour = found.map_or(NONE, base_colour);
        if colour != NONE || y <= min_y {
            break;
        }
    }
    let Some(block) = found else {
        return Column { colour: NONE, y, water_depth: 0 };
    };
    let mut depth = 0u16;
    let mut colour = base_colour(block);
    if y > min_y && has_fluid(block) {
        let mut below = y - 1;
        loop {
            let at = block_at(below);
            below -= 1;
            depth = depth.saturating_add(1);
            if !(below > min_y && at.is_some_and(has_fluid)) {
                break;
            }
        }
        // A waterlogged block whose top is not full shows its water.
        if is_water(block) {
            colour = WATER;
        } else {
            colour = FIRE;
        }
    }
    Column { colour, y, water_depth: depth }
}

/// The first free height over a column of a streamed scene: its chunk's
/// surface, or over a block placed above it. `None` where the scene has no
/// chunk.
pub fn scene_top(scene: &HandcraftedScene, x: i32, z: i32) -> Option<(i32, i32)> {
    let chunk = scene.generated_chunk((x >> 4, z >> 4))?;
    let mut top = chunk.heightmaps.get(HeightmapKind::WorldSurface, (x & 15) as usize, (z & 15) as usize);
    if let (Some(placed), _) = scene.chunk_edits((x >> 4, z >> 4)) {
        for (&(px, py, pz), _) in placed.range((x, i32::MIN, i32::MIN)..=(x, i32::MAX, i32::MAX)) {
            if px == x && pz == z {
                top = top.max(py + 1);
            }
        }
    }
    Some((top, chunk.min_y()))
}

/// A column of a streamed scene, edits included.
pub fn scene_column(scene: &HandcraftedScene, x: i32, z: i32) -> Option<Column> {
    let (top, min_y) = scene_top(scene, x, z)?;
    Some(column(top, min_y, |y| Scene::block(scene, (x, y, z))))
}

/// Columns the map reads, a chunk at a time: from the scene where it has
/// the chunk (its edits included), else from the chunk as generated. A
/// host keeps them for every chunk a player has about it, so a map fills
/// in around each player who holds one, as a server's loaded chunks do.
#[derive(Default)]
pub struct Columns {
    chunks: HashMap<(i32, i32), Box<[Column; 256]>>,
}

impl Columns {
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    pub fn get(&self, x: i32, z: i32) -> Option<Column> {
        let chunk = self.chunks.get(&(x >> 4, z >> 4))?;
        Some(chunk[((z & 15) * 16 + (x & 15)) as usize])
    }

    pub fn remove(&mut self, chunk: (i32, i32)) {
        self.chunks.remove(&chunk);
    }

    /// A chunk's columns, x fastest then z.
    pub fn set_chunk(&mut self, chunk: (i32, i32), columns: Box<[Column; 256]>) {
        self.chunks.insert(chunk, columns);
    }

    /// Reads a whole chunk from the scene; false when the scene lacks it.
    pub fn sample_scene(&mut self, scene: &HandcraftedScene, chunk: (i32, i32)) -> bool {
        if scene.generated_chunk(chunk).is_none() {
            return false;
        }
        let mut columns = Box::new([Column::default(); 256]);
        for dz in 0..16 {
            for dx in 0..16 {
                let (x, z) = (chunk.0 * 16 + dx, chunk.1 * 16 + dz);
                if let Some(column) = scene_column(scene, x, z) {
                    columns[(dz * 16 + dx) as usize] = column;
                }
            }
        }
        self.chunks.insert(chunk, columns);
        true
    }

    /// Reads a chunk as generated, unless it is kept already.
    pub fn sample_generated(&mut self, generated: &Chunk, states: &crate::terrain::BlockStates) {
        let pos = (generated.pos.x, generated.pos.z);
        if self.chunks.contains_key(&pos) {
            return;
        }
        let mut columns = Box::new([Column::default(); 256]);
        let min_y = generated.min_y();
        for dz in 0..16usize {
            for dx in 0..16usize {
                let top = generated.heightmaps.get(HeightmapKind::WorldSurface, dx, dz);
                columns[dz * 16 + dx] = column(top, min_y, |y| states.block(generated.block(dx, y, dz)));
            }
        }
        self.chunks.insert(pos, columns);
    }

    /// Reads the columns of these blocks again from the scene, where it
    /// has their chunk and the chunk is kept.
    pub fn resample(&mut self, scene: &HandcraftedScene, positions: &[(i32, i32, i32)]) {
        for &(x, _, z) in positions {
            let Some(chunk) = self.chunks.get_mut(&(x >> 4, z >> 4)) else {
                continue;
            };
            if let Some(column) = scene_column(scene, x, z) {
                chunk[((z & 15) * 16 + (x & 15)) as usize] = column;
            }
        }
    }
}

/// A map's data (`MapItemSavedData`): its centre, scale and pixels, row by
/// row from the north-west corner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapData {
    pub centre: (i32, i32),
    pub scale: u8,
    pub colours: Vec<u8>,
}

impl MapData {
    /// A fresh map at scale `scale` made where a player stands.
    pub fn new(x: f64, z: f64, scale: u8) -> Self {
        Self {
            centre: centre(x, z, scale),
            scale,
            colours: vec![0; SIZE * SIZE],
        }
    }

    /// Pixels shown (not `NONE`).
    pub fn explored(&self) -> usize {
        self.colours.iter().filter(|&&c| c >= 4).count()
    }

    /// `updateColor`: whether the pixel changed.
    pub fn set(&mut self, px: usize, pz: usize, colour: u8) -> bool {
        let at = &mut self.colours[pz * SIZE + px];
        let changed = *at != colour;
        *at = colour;
        changed
    }

    /// The pixel over a block, if it is on the map.
    pub fn pixel_of(&self, x: f64, z: f64) -> Option<(usize, usize)> {
        let per = f64::from(1u32 << self.scale);
        let px = ((x - f64::from(self.centre.0)) / per).floor() as i64 + 64;
        let pz = ((z - f64::from(self.centre.1)) / per).floor() as i64 + 64;
        ((0..SIZE as i64).contains(&px) && (0..SIZE as i64).contains(&pz)).then_some((px as usize, pz as usize))
    }

    /// One tile's pixels.
    pub fn tile(&self, index: usize) -> [u8; TILE * TILE] {
        let (tx, tz) = (index % TILES, index / TILES);
        let mut out = [0u8; TILE * TILE];
        for row in 0..TILE {
            let from = (tz * TILE + row) * SIZE + tx * TILE;
            out[row * TILE..(row + 1) * TILE].copy_from_slice(&self.colours[from..from + TILE]);
        }
        out
    }

    pub fn set_tile(&mut self, index: usize, pixels: &[u8; TILE * TILE]) {
        let (tx, tz) = (index % TILES, index / TILES);
        for row in 0..TILE {
            let to = (tz * TILE + row) * SIZE + tx * TILE;
            self.colours[to..to + TILE].copy_from_slice(&pixels[row * TILE..(row + 1) * TILE]);
        }
    }

    /// The tiles where `self` differs from `sent` (all with any pixel when
    /// nothing was sent).
    pub fn changed_tiles(&self, sent: Option<&MapData>) -> Vec<usize> {
        (0..TILES * TILES)
            .filter(|&i| match sent {
                Some(sent) if sent.centre == self.centre && sent.scale == self.scale => sent.tile(i) != self.tile(i),
                _ => self.tile(i).iter().any(|&c| c != 0),
            })
            .collect()
    }
}

/// `MapItem.update`: a player holding the map at `feet` fills in the
/// pixels about it from `columns` (`None` where no chunk is loaded), one
/// column in sixteen each tick, `step` its own count of ticks with this
/// map. Whether any pixel changed.
pub fn update(map: &mut MapData, feet: [f64; 3], step: &mut u32, columns: impl Fn(i32, i32) -> Option<Column>) -> bool {
    let scale = 1i32 << map.scale;
    let (cx, cz) = map.centre;
    let px = ((feet[0] - f64::from(cx)).floor() as i32).div_euclid(scale) + 64;
    let pz = ((feet[2] - f64::from(cz)).floor() as i32).div_euclid(scale) + 64;
    let radius = 128 / scale;
    *step = step.wrapping_add(1);
    let mut changed = false;
    let mut carry = false;
    for k1 in (px - radius + 1)..(px + radius) {
        if (k1 & 15) != (*step & 15) as i32 && !carry {
            continue;
        }
        carry = false;
        let mut previous = 0.0f64;
        for l1 in (pz - radius - 1)..(pz + radius) {
            if !(k1 >= 0 && l1 >= -1 && k1 < SIZE as i32 && l1 < SIZE as i32) {
                continue;
            }
            let distance = (k1 - px).pow(2) + (l1 - pz).pow(2);
            let edge = distance > (radius - 2) * (radius - 2);
            let x0 = (cx / scale + k1 - 64) * scale;
            let z0 = (cz / scale + l1 - 64) * scale;
            // The most common colour over the pixel's blocks, their mean
            // height and water depth.
            let mut counts: Vec<(u8, u32)> = Vec::new();
            let mut height = 0.0f64;
            let mut depth = 0u32;
            let mut loaded = true;
            for i in 0..scale {
                for j in 0..scale {
                    let Some(column) = columns(x0 + i, z0 + j) else {
                        loaded = false;
                        continue;
                    };
                    height += f64::from(column.y) / f64::from(scale * scale);
                    depth += u32::from(column.water_depth);
                    match counts.iter_mut().find(|(c, _)| *c == column.colour) {
                        Some((_, n)) => *n += 1,
                        None => counts.push((column.colour, 1)),
                    }
                }
            }
            if !loaded {
                continue;
            }
            let depth = depth / (scale * scale) as u32;
            let colour = counts
                .iter()
                .fold(None::<(u8, u32)>, |best, &(c, n)| match best {
                    Some((_, m)) if m >= n => best,
                    _ => Some((c, n)),
                })
                .map_or(NONE, |(c, _)| c);
            let parity = f64::from((k1 + l1) & 1);
            let brightness = if colour == WATER {
                let d = f64::from(depth) * 0.1 + parity * 0.2;
                if d < 0.5 {
                    Brightness::High
                } else if d > 0.9 {
                    Brightness::Low
                } else {
                    Brightness::Normal
                }
            } else {
                let d = (height - previous) * 4.0 / f64::from(scale + 4) + (parity - 0.5) * 0.4;
                if d > 0.6 {
                    Brightness::High
                } else if d < -0.6 {
                    Brightness::Low
                } else {
                    Brightness::Normal
                }
            };
            previous = height;
            if l1 >= 0 && distance < radius * radius && (!edge || (k1 + l1) & 1 != 0) {
                let value = if colour == NONE { 0 } else { packed(colour, brightness) };
                if map.set(k1 as usize, l1 as usize, value) {
                    changed = true;
                    carry = true;
                }
            }
        }
    }
    changed
}

/// Every map of a world by id, and the next id to give (`maps.bin` beside
/// the world's other files).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapStore {
    pub next_id: u32,
    pub maps: BTreeMap<u32, MapData>,
}

const MAPS_MAGIC: &[u8; 4] = b"MCMP";
const MAPS_VERSION: u32 = 1;

impl MapStore {
    /// A new map id (its data comes when it is first held).
    pub fn allocate(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = crate::multiplayer::Writer::default();
        w.0.extend_from_slice(MAPS_MAGIC);
        w.u32(MAPS_VERSION);
        w.u32(self.next_id);
        w.u32(self.maps.len() as u32);
        for (id, map) in &self.maps {
            w.u32(*id);
            w.i32(map.centre.0);
            w.i32(map.centre.1);
            w.u8(map.scale);
            w.0.extend_from_slice(&map.colours);
        }
        w.0
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, crate::multiplayer::DecodeError> {
        use crate::multiplayer::DecodeError;
        let mut r = crate::multiplayer::Reader(bytes);
        if r.take(4)? != MAPS_MAGIC {
            return Err(DecodeError::Malformed("maps magic"));
        }
        if r.u32()? != MAPS_VERSION {
            return Err(DecodeError::Malformed("maps version"));
        }
        let next_id = r.u32()?;
        let n = r.u32()? as usize;
        if n > r.0.len() / (SIZE * SIZE) {
            return Err(DecodeError::Truncated);
        }
        let mut maps = BTreeMap::new();
        for _ in 0..n {
            let id = r.u32()?;
            let centre = (r.i32()?, r.i32()?);
            let scale = r.u8()?;
            if scale > 4 {
                return Err(DecodeError::Malformed("map scale"));
            }
            let colours = r.take(SIZE * SIZE)?.to_vec();
            maps.insert(id, MapData { centre, scale, colours });
        }
        r.done()?;
        Ok(Self { next_id, maps })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::ResourceId;

    fn block(id: &str) -> Block {
        Block::new(id)
    }

    #[test]
    fn packed_colours_shade_the_base_as_vanilla() {
        assert_eq!(rgb(0), None);
        assert_eq!(rgb(3), None);
        // GRASS at each brightness: 127,178,56 times 180, 220, 255, 135 / 255.
        assert_eq!(rgb(packed(GRASS, Brightness::Low)), Some([89, 125, 39]));
        assert_eq!(rgb(packed(GRASS, Brightness::Normal)), Some([109, 153, 48]));
        assert_eq!(rgb(packed(GRASS, Brightness::High)), Some([127, 178, 56]));
        assert_eq!(rgb(packed(GRASS, Brightness::Lowest)), Some([67, 94, 29]));
        assert_eq!(rgb(packed(WATER, Brightness::High)), Some([64, 64, 255]));
        assert_eq!(packed(STONE, Brightness::Normal), 45);
        assert_eq!(BASE_COLOURS.len(), 62);
        assert_eq!(BASE_COLOURS[GLOW_LICHEN as usize], [127, 167, 150]);
    }

    #[test]
    fn a_map_centres_on_vanilla_s_grid() {
        // Scale 0: 128-block squares from -64.
        assert_eq!(centre(90.5, -30.5, 0), (128, 0));
        assert_eq!(centre(0.0, 0.0, 0), (0, 0));
        assert_eq!(centre(63.9, -64.0, 0), (0, 0));
        assert_eq!(centre(64.0, -64.1, 0), (128, -128));
        // Scale 1: 256-block squares, centred 64 in.
        assert_eq!(centre(0.0, 0.0, 1), (64, 64));
        assert_eq!(centre(-65.0, 300.0, 1), (-192, 320));
        // A fresh map covers its centre +-64 at a pixel a block.
        let map = MapData::new(90.5, -30.5, 0);
        assert_eq!(map.pixel_of(64.0, -64.0), Some((0, 0)));
        assert_eq!(map.pixel_of(191.9, 63.9), Some((127, 127)));
        assert_eq!(map.pixel_of(90.5, -30.5), Some((26, 33)));
        assert_eq!(map.pixel_of(192.0, 0.0), None);
    }

    #[test]
    fn blocks_take_their_vanilla_map_colours() {
        assert_eq!(base_colour(&block("minecraft:grass_block")), GRASS);
        assert_eq!(base_colour(&block("minecraft:water")), WATER);
        assert_eq!(base_colour(&block("minecraft:sand")), SAND);
        assert_eq!(base_colour(&block("minecraft:stone")), STONE);
        assert_eq!(base_colour(&block("minecraft:deepslate")), DEEPSLATE);
        assert_eq!(base_colour(&block("minecraft:dirt")), DIRT);
        assert_eq!(base_colour(&block("minecraft:snow")), SNOW);
        assert_eq!(base_colour(&block("minecraft:ice")), ICE);
        assert_eq!(base_colour(&block("minecraft:oak_leaves")), PLANT);
        assert_eq!(base_colour(&block("minecraft:short_grass")), PLANT);
        assert_eq!(base_colour(&block("minecraft:oak_planks")), WOOD);
        assert_eq!(base_colour(&block("minecraft:oak_log").with("axis", "y")), WOOD);
        assert_eq!(base_colour(&block("minecraft:oak_log").with("axis", "x")), PODZOL);
        assert_eq!(base_colour(&block("minecraft:spruce_planks")), PODZOL);
        assert_eq!(base_colour(&block("minecraft:birch_log").with("axis", "z")), QUARTZ);
        assert_eq!(base_colour(&block("minecraft:gold_block")), GOLD);
        assert_eq!(base_colour(&block("minecraft:diamond_block")), DIAMOND);
        assert_eq!(base_colour(&block("minecraft:white_wool")), SNOW);
        assert_eq!(base_colour(&block("minecraft:red_wool")), COLOR_RED);
        assert_eq!(base_colour(&block("minecraft:light_blue_concrete")), COLOR_LIGHT_BLUE);
        assert_eq!(base_colour(&block("minecraft:white_terracotta")), TERRACOTTA_WHITE);
        assert_eq!(base_colour(&block("minecraft:black_terracotta")), 51);
        assert_eq!(base_colour(&block("minecraft:light_gray_terracotta")), TERRACOTTA_LIGHT_GRAY);
        assert_eq!(base_colour(&block("minecraft:terracotta")), COLOR_ORANGE);
        assert_eq!(base_colour(&block("minecraft:clay")), CLAY);
        assert_eq!(base_colour(&block("minecraft:lava")), FIRE);
        assert_eq!(base_colour(&block("minecraft:glass")), NONE);
        assert_eq!(base_colour(&block("minecraft:air")), NONE);
        assert_eq!(base_colour(&block("minecraft:torch")), NONE);
        assert_eq!(base_colour(&block("minecraft:cobblestone_stairs")), STONE);
        assert_eq!(ResourceId::parse(ITEM).unwrap().path, "filled_map");
    }

    /// A column: grass on dirt; water three deep over sand; glass over
    /// stone (a map sees through the glass).
    #[test]
    fn a_column_finds_the_top_block_a_map_shows_and_the_water_under_it() {
        let grass = block("minecraft:grass_block");
        let dirt = block("minecraft:dirt");
        let land = column(65, -64, |y| match y {
            64 => Some(&grass),
            ..=63 => Some(&dirt),
            _ => None,
        });
        assert_eq!(land, Column { colour: GRASS, y: 64, water_depth: 0 });
        let water = block("minecraft:water").with("level", "0");
        let sand = block("minecraft:sand");
        let sea = column(63, -64, |y| match y {
            60..=62 => Some(&water),
            ..=59 => Some(&sand),
            _ => None,
        });
        // Two waters under the surface and the sand under them, as vanilla
        // counts.
        assert_eq!(sea, Column { colour: WATER, y: 62, water_depth: 3 });
        let glass = block("minecraft:glass");
        let stone = block("minecraft:stone");
        let roofed = column(81, -64, |y| match y {
            80 => Some(&glass),
            ..=70 => Some(&stone),
            _ => None,
        });
        assert_eq!(roofed, Column { colour: STONE, y: 70, water_depth: 0 });
        assert_eq!(column(-63, -64, |_| None).colour, STONE, "nothing over the bottom: bedrock");
    }

    /// Flat grass at height 64, a hill one block higher to the south-east:
    /// held at the centre the map fills the circle about the player over
    /// sixteen ticks, flat ground at the normal brightness, the hill's
    /// northern edge brighter (dithered, as vanilla's slopes are).
    #[test]
    fn holding_a_map_fills_it_in_about_the_holder() {
        let mut map = MapData::new(0.5, 0.5, 0);
        assert_eq!(map.centre, (0, 0));
        let columns = |x: i32, z: i32| {
            Some(Column {
                colour: GRASS,
                y: if x >= 10 && z >= 10 { 65 } else { 64 },
                water_depth: 0,
            })
        };
        let mut step = 0;
        // The first column due this tick changes, so each one after it goes
        // too (vanilla's carry): all but the column before it.
        assert!(update(&mut map, [0.5, 65.0, 0.5], &mut step, columns));
        assert_eq!(map.explored(), (SIZE - 1) * SIZE);
        for _ in 0..15 {
            update(&mut map, [0.5, 65.0, 0.5], &mut step, columns);
        }
        // Radius 128 at scale 0 from the middle: the whole map.
        assert_eq!(map.explored(), SIZE * SIZE);
        let at = |x: i32, z: i32| map.colours[((z + 64) as usize) * SIZE + (x + 64) as usize];
        assert_eq!(at(0, 0), packed(GRASS, Brightness::Normal));
        assert_eq!(at(-30, 20), packed(GRASS, Brightness::Normal));
        // The hill's north edge: 0.8 higher, -0.2 on even pixels and +0.2
        // on odd ones; 0.8 - 0.2 is a hair over 0.6 in doubles, as in
        // vanilla, so a step of one block is bright on every pixel.
        assert_eq!(at(20, 10), packed(GRASS, Brightness::High));
        assert_eq!(at(21, 10), packed(GRASS, Brightness::High));
        assert_eq!(at(21, 11), packed(GRASS, Brightness::Normal), "on the hill: flat again");
        // Nothing changes standing still on the same ground.
        let before = map.clone();
        for _ in 0..16 {
            assert!(!update(&mut map, [0.5, 65.0, 0.5], &mut step, columns));
        }
        assert_eq!(map, before);
    }

    /// Unloaded ground stays unexplored; walking on reveals it.
    #[test]
    fn a_map_shows_only_loaded_ground_and_updates_as_the_holder_walks() {
        let mut map = MapData::new(0.0, 0.0, 0);
        let loaded_to = std::cell::Cell::new(0);
        let columns = |x: i32, _z: i32| {
            (x < loaded_to.get()).then_some(Column { colour: SAND, y: 64, water_depth: 0 })
        };
        let mut step = 0;
        for _ in 0..16 {
            update(&mut map, [-40.0, 64.0, 0.0], &mut step, columns);
        }
        let west = map.explored();
        assert!(west > 0);
        assert_eq!(map.colours[64 * SIZE + 64 + 10], 0, "east of x 0 is not loaded");
        loaded_to.set(64);
        for _ in 0..16 {
            update(&mut map, [30.0, 64.0, 0.0], &mut step, columns);
        }
        assert!(map.explored() > west);
        assert_eq!(map.colours[64 * SIZE + 64 + 10], packed(SAND, Brightness::Normal));
    }

    #[test]
    fn water_is_shaded_by_its_depth() {
        let mut map = MapData::new(0.0, 0.0, 0);
        let columns = |x: i32, _z: i32| {
            Some(Column {
                colour: WATER,
                y: 62,
                water_depth: if x < 0 { 1 } else { 12 },
            })
        };
        let mut step = 0;
        for _ in 0..16 {
            update(&mut map, [0.0, 64.0, 0.0], &mut step, columns);
        }
        let at = |x: i32, z: i32| map.colours[((z + 64) as usize) * SIZE + (x + 64) as usize];
        // Shallow: 0.1 or 0.3, high. Deep: 1.2 or 1.4, low.
        assert_eq!(at(-10, 0), packed(WATER, Brightness::High));
        assert_eq!(at(-11, 0), packed(WATER, Brightness::High));
        assert_eq!(at(10, 0), packed(WATER, Brightness::Low));
    }

    #[test]
    fn a_stack_names_its_map() {
        let components = components(7);
        assert_eq!(map_id(ITEM, Some(&components)), Some(7));
        assert_eq!(map_id(ITEM, None), None);
        assert_eq!(map_id("minecraft:map", Some(&components)), None);
    }

    #[test]
    fn tiles_carry_the_pixels_and_only_changes_go_again() {
        let mut map = MapData::new(0.0, 0.0, 0);
        assert!(map.changed_tiles(None).is_empty());
        map.set(17, 3, 5);
        map.set(127, 127, 9);
        assert_eq!(map.changed_tiles(None), vec![1, 63]);
        let mut copy = MapData { colours: vec![0; SIZE * SIZE], ..map.clone() };
        for i in map.changed_tiles(None) {
            copy.set_tile(i, &map.tile(i));
        }
        assert_eq!(copy, map);
        assert!(map.changed_tiles(Some(&copy)).is_empty());
        map.set(0, 120, 6);
        assert_eq!(map.changed_tiles(Some(&copy)), vec![56]);
    }

    #[test]
    fn the_map_store_reads_back_what_it_wrote() {
        let mut store = MapStore::default();
        assert_eq!(store.allocate(), 0);
        assert_eq!(store.allocate(), 1);
        let mut map = MapData::new(90.5, -30.5, 0);
        map.set(3, 4, 40);
        store.maps.insert(1, map);
        let read = MapStore::decode(&store.encode()).unwrap();
        assert_eq!(read, store);
        assert_eq!(read.next_id, 2);
        assert!(MapStore::decode(&store.encode()[..20]).is_err());
        assert!(MapStore::decode(b"nope").is_err());
    }
}
