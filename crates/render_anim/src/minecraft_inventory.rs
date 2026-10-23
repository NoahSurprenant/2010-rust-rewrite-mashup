//! The player's inventory on the Minecraft map: MinecraftOSS's vanilla
//! `Inventory` (slots, stacking, container clicks, the 2x2 crafting grid and
//! a crafting table's 3x3),
//! published for the HUD to draw with item icons and names made the way the
//! viewer makes them. The player's MW2 guns are items in it too: selecting
//! one on the hotbar raises that gun.
use std::collections::{HashMap, HashSet};

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use frame::{McClick, McSlot, McStack, MinecraftUi};
use glam::DVec3;
use minecraft_terrain::pack::PackStack;
use minecraftoss_player::inventory::{Inventory, ItemStack};

/// Items that are MW2 guns: `iw4:weapon/<weapon index>`.
const WEAPON_PREFIX: &str = "iw4:weapon/";
/// Icon cells: 32 pixels (a 16-pixel item at GUI scale 2), 16 to a row.
const ICON: u32 = 32;
const ATLAS: u32 = ICON * 16;

pub(crate) fn weapon_of(stack: &ItemStack) -> Option<u32> {
    stack.id.strip_prefix(WEAPON_PREFIX)?.parse().ok()
}

fn weapon_item(weapon: u32) -> ItemStack {
    let mut stack = ItemStack::new(format!("{WEAPON_PREFIX}{weapon}"), 1);
    stack.max = 1;
    stack
}

#[derive(Default)]
pub(crate) struct InventoryUi {
    atlas: Option<image::RgbaImage>,
    handle: Option<Handle<Image>>,
    cells: HashMap<String, u32>,
    failed: HashSet<String>,
    language: Option<HashMap<String, String>>,
    dirty: bool,
    /// The gun last asked for, until the player state holds it.
    pending_weapon: Option<u32>,
    last_selected: Option<usize>,
}

/// What the player holds and where it looks, for throwing items.
pub(crate) struct Thrower {
    pub eye: DVec3,
    /// Minecraft yaw and pitch, in degrees.
    pub yaw: f32,
    pub pitch: f32,
}

impl InventoryUi {
    /// Keeps the player's MW2 guns in the inventory: each gun the player has
    /// is one item, placed on the first free hotbar slot when it arrives, and
    /// a gun the player no longer has leaves.
    pub(crate) fn sync_weapons(&mut self, inventory: &mut Inventory, owned: &[u32]) {
        let keep = |stack: &Option<ItemStack>| {
            stack.as_ref().is_none_or(|s| weapon_of(s).is_none_or(|w| owned.contains(&w)))
        };
        for slot in inventory
            .slots
            .iter_mut()
            .chain(inventory.crafting.iter_mut())
            .chain(inventory.workbench.iter_mut())
        {
            if !keep(slot) {
                *slot = None;
            }
        }
        if !keep(&inventory.cursor) {
            inventory.cursor = None;
        }
        let held: HashSet<u32> = inventory
            .slots
            .iter()
            .chain(inventory.crafting.iter())
            .chain(inventory.workbench.iter())
            .chain(std::iter::once(&inventory.cursor))
            .filter_map(|s| s.as_ref().and_then(weapon_of))
            .collect();
        for &weapon in owned {
            if held.contains(&weapon) {
                continue;
            }
            let free = (0..frame::minecraft_ui::MC_HOTBAR)
                .chain(frame::minecraft_ui::MC_HOTBAR..36)
                .find(|&i| inventory.slots[i].is_none());
            if let Some(i) = free {
                inventory.slots[i] = Some(weapon_item(weapon));
            }
        }
    }

    /// Applies the HUD's clicks, selection and drops with vanilla's rules.
    /// Stacks thrown out of the inventory are returned, except guns, which
    /// stay.
    pub(crate) fn apply_input(
        &mut self,
        ui: &mut MinecraftUi,
        inventory: &mut Inventory,
        selected: &mut usize,
    ) -> Vec<ItemStack> {
        let mut thrown = Vec::new();
        if std::mem::take(&mut ui.clear_items) {
            clear_items(inventory);
        }
        for (id, count) in std::mem::take(&mut ui.give) {
            thrown.extend(give(inventory, &id, count, *selected));
        }
        for click in std::mem::take(&mut ui.clicks) {
            if click == McClick::Close {
                ui.workbench = None;
            }
            thrown.extend(apply_click(inventory, click, ui.workbench.is_some()));
        }
        thrown.extend(inventory.take_pending_drops());
        if let Some(slot) = ui.select.take() {
            *selected = slot.min(frame::minecraft_ui::MC_HOTBAR - 1);
        }
        if let Some(whole) = ui.drop_selected.take()
            && let Some(stack) = inventory.drop_selected(*selected, whole)
        {
            thrown.push(stack);
        }
        keep_guns(inventory, thrown, *selected)
    }

    /// The gun the hotbar selection asks for, if the player holds another;
    /// and the selection follows a gun MW2 raised on its own.
    pub(crate) fn weapon_request(
        &mut self,
        inventory: &Inventory,
        selected: &mut usize,
        held: u32,
    ) -> Option<u32> {
        let selected_gun = inventory.slots[*selected].as_ref().and_then(weapon_of);
        let changed = self.last_selected != Some(*selected);
        self.last_selected = Some(*selected);
        if self.pending_weapon == Some(held) {
            self.pending_weapon = None;
        }
        if changed {
            if let Some(gun) = selected_gun.filter(|&gun| gun != held) {
                self.pending_weapon = Some(gun);
            }
        } else if self.pending_weapon.is_none()
            && selected_gun.is_some_and(|gun| gun != held)
            && let Some(slot) = (0..frame::minecraft_ui::MC_HOTBAR)
                .find(|&i| inventory.slots[i].as_ref().and_then(weapon_of) == Some(held))
        {
            // MW2 changed weapons (out of ammo, a pickup): the hotbar follows.
            *selected = slot;
            self.last_selected = Some(slot);
        }
        self.pending_weapon
    }

    /// Publishes the inventory for the HUD, with icons for every item in it
    /// when this game draws (`icons`); a headless one has the stacks only.
    pub(crate) fn publish(
        &mut self,
        ui: &mut MinecraftUi,
        inventory: &Inventory,
        selected: usize,
        packs: &PackStack,
        images: &mut Assets<Image>,
        icons: bool,
    ) {
        let language = self
            .language
            .get_or_insert_with(|| minecraft_terrain::item_icons::language(packs).unwrap_or_default());
        let mut stacks = Vec::new();
        let mut convert = |stack: &Option<ItemStack>| {
            stack.as_ref().map(|s| {
                stacks.push(s.id.clone());
                McStack {
                    id: s.id.clone(),
                    count: s.count,
                    weapon: weapon_of(s),
                    durability: None,
                }
            })
        };
        ui.slots = inventory.slots.iter().take(frame::minecraft_ui::MC_INVENTORY_SLOTS).map(&mut convert).collect();
        if ui.workbench.is_some() {
            ui.crafting = inventory.workbench.iter().map(&mut convert).collect();
            ui.result = convert(&inventory.workbench_output());
        } else {
            ui.crafting = inventory.crafting.iter().map(&mut convert).collect();
            ui.result = convert(&inventory.crafting_output());
        }
        ui.cursor = convert(&inventory.cursor);
        if ui.selected != selected {
            let name = inventory.slots[selected]
                .as_ref()
                .filter(|s| weapon_of(s).is_none())
                .map(|s| minecraft_terrain::item_icons::item_name(language, &s.id));
            ui.selected_name = name.map(|name| (name, 0.0));
        }
        ui.selected = selected;
        for id in stacks {
            if id.starts_with(WEAPON_PREFIX) {
                continue;
            }
            if !ui.names.contains_key(&id) {
                ui.names.insert(id.clone(), minecraft_terrain::item_icons::item_name(language, &id));
            }
            if !icons {
                continue;
            }
            if self.cells.contains_key(&id) || self.failed.contains(&id) {
                continue;
            }
            let cell = self.cells.len() as u32;
            if cell >= (ATLAS / ICON) * (ATLAS / ICON) {
                continue;
            }
            match minecraft_terrain::item_icons::item_icon(packs, &id, ICON as usize) {
                Ok(Some(icon)) => {
                    let atlas = self.atlas.get_or_insert_with(|| image::RgbaImage::new(ATLAS, ATLAS));
                    let (x, y) = ((cell % (ATLAS / ICON)) * ICON, (cell / (ATLAS / ICON)) * ICON);
                    let icon = image::imageops::resize(&icon, ICON, ICON, image::imageops::FilterType::Nearest);
                    image::imageops::replace(atlas, &icon, i64::from(x), i64::from(y));
                    let size = ATLAS as f32;
                    ui.icon_rects.insert(
                        id.clone(),
                        [x as f32 / size, y as f32 / size, (x + ICON) as f32 / size, (y + ICON) as f32 / size],
                    );
                    self.cells.insert(id, cell);
                    self.dirty = true;
                }
                Ok(None) => {
                    diag::warn!(World, "Minecraft item icon: no model for `{id}`");
                    self.failed.insert(id);
                }
                Err(error) => {
                    diag::warn!(World, "Minecraft item icon for `{id}` failed: {error}");
                    self.failed.insert(id);
                }
            }
        }
        if self.dirty
            && let Some(atlas) = self.atlas.as_ref()
        {
            self.dirty = false;
            let image = Image {
                sampler: ImageSampler::nearest(),
                ..Image::new(
                    Extent3d { width: ATLAS, height: ATLAS, depth_or_array_layers: 1 },
                    TextureDimension::D2,
                    atlas.as_raw().clone(),
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::default(),
                )
            };
            match self.handle.as_ref() {
                Some(handle) => {
                    let _ = images.insert(handle.id(), image);
                }
                None => self.handle = Some(images.add(image)),
            }
            ui.icons = self.handle.clone();
        }
    }
}

/// Throws stacks from the player's eye, as `Player.drop` does.
pub(crate) fn throw(items: &mut minecraftoss_player::items::WorldItems, stacks: Vec<ItemStack>, thrower: &Thrower) {
    for stack in stacks {
        items.toss(stack, thrower.eye, thrower.yaw, thrower.pitch);
    }
}

/// The console's `mc clear`: everything but the guns goes.
pub(crate) fn clear_items(inventory: &mut Inventory) {
    for slot in &mut inventory.slots {
        if slot.as_ref().is_some_and(|s| weapon_of(s).is_none()) {
            *slot = None;
        }
    }
}

/// The console's `mc give`; what does not fit.
pub(crate) fn give(inventory: &mut Inventory, id: &str, count: u8, selected: usize) -> Option<ItemStack> {
    let stack = inventory.recipes.stack(id, count);
    inventory.add_item(stack, selected)
}

/// One click on the inventory screen (the 3x3 grid's when a crafting table
/// is open) with vanilla's rules; the stacks it throws out.
pub(crate) fn apply_click(inventory: &mut Inventory, click: McClick, workbench: bool) -> Vec<ItemStack> {
    let mut thrown = Vec::new();
    match click {
        McClick::Slot { slot: McSlot::Inventory(index), right, shift } => {
            if let Some(stack) = inventory.click(Some(index), right, shift) {
                thrown.push(stack);
            }
        }
        McClick::Slot { slot: McSlot::Crafting(index), right, shift } => {
            if workbench {
                inventory.click_workbench_slot(index, right, shift);
            } else {
                inventory.click_crafting_slot(index, right, shift);
            }
        }
        McClick::Slot { slot: McSlot::Result, shift, .. } => {
            if workbench {
                inventory.take_workbench_output(shift);
            } else {
                inventory.take_crafting_output(shift);
            }
        }
        McClick::Outside { right } => {
            if let Some(stack) = inventory.click(None, right, false) {
                thrown.push(stack);
            }
        }
        McClick::Gather { right } => inventory.pickup_all(right),
        McClick::Swap { slot: McSlot::Inventory(index), hotbar } => inventory.number_swap(index, hotbar),
        McClick::Swap { .. } => {}
        McClick::Spread { slots, right } => inventory.distribute(&slots, right),
        McClick::Close => {
            thrown.extend(inventory.settle_crafting());
            thrown.extend(inventory.settle_workbench());
            if let Some(rest) = inventory.settle_cursor() {
                thrown.push(rest);
            }
        }
    }
    thrown
}

/// A gun never leaves the inventory: thrown guns go back in, and the rest
/// are returned.
pub(crate) fn keep_guns(inventory: &mut Inventory, thrown: Vec<ItemStack>, selected: usize) -> Vec<ItemStack> {
    let mut out = Vec::new();
    for stack in thrown {
        if weapon_of(&stack).is_some() {
            if let Some(back) = inventory.add_item(stack, selected) {
                inventory.cursor = Some(back);
            }
        } else {
            out.push(stack);
        }
    }
    out
}

/// A player's starting map, naming the map `id`: on the hotbar's last slot,
/// clear of the guns that fill it from the first, else the first free
/// slot. False when the inventory is full.
pub(crate) fn give_starting_map(inventory: &mut Inventory, id: u32) -> bool {
    let mut stack = ItemStack::new(minecraft_terrain::filled_map::ITEM, 1);
    stack.components = Some(minecraft_terrain::filled_map::components(id));
    let Some(slot) = starting_map_slot(inventory) else {
        return false;
    };
    inventory.slots[slot] = Some(stack);
    true
}

/// Where a starting map goes.
pub(crate) fn starting_map_slot(inventory: &Inventory) -> Option<usize> {
    let last = frame::minecraft_ui::MC_HOTBAR - 1;
    std::iter::once(last)
        .chain(frame::minecraft_ui::MC_HOTBAR..36)
        .chain(0..last)
        .find(|&i| inventory.slots[i].is_none())
}

/// The map a stack is, by its id.
pub(crate) fn map_of(stack: &ItemStack) -> Option<u32> {
    minecraft_terrain::filled_map::map_id(&stack.id, stack.components.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_starting_map_goes_on_the_last_hotbar_slot_clear_of_the_guns() {
        let mut inventory = Inventory::default();
        assert!(give_starting_map(&mut inventory, 4));
        let map = inventory.slots[8].clone().unwrap();
        assert_eq!(map.id, "minecraft:filled_map");
        assert_eq!(map_of(&map), Some(4));
        // Guns that arrive after it take the first slots.
        InventoryUi::default().sync_weapons(&mut inventory, &[1, 2, 3, 4, 5]);
        assert!((0..5).all(|i| inventory.slots[i].as_ref().and_then(weapon_of).is_some()));
        assert_eq!(inventory.slots[8], Some(map.clone()));
        // `mc clear` takes it as any item; a second map goes there again.
        clear_items(&mut inventory);
        assert_eq!(inventory.slots[8], None);
        assert!(give_starting_map(&mut inventory, 5));
        assert_eq!(inventory.slots[8].as_ref().and_then(map_of), Some(5));
        // With the last hotbar slot taken: the first free one inside.
        assert!(give_starting_map(&mut inventory, 6));
        assert_eq!(inventory.slots[9].as_ref().and_then(map_of), Some(6));
        // Two maps never stack.
        assert!(!inventory.slots[8].as_ref().unwrap().same_item(inventory.slots[9].as_ref().unwrap()));
        for slot in &mut inventory.slots[..36] {
            slot.get_or_insert_with(|| ItemStack::new("minecraft:dirt", 1));
        }
        assert!(!give_starting_map(&mut inventory, 7), "a full inventory");
    }
}
