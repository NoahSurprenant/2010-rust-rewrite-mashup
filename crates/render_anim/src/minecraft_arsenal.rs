//! Bridges a Minecraft player's permanent gun unlocks and universal ammo
//! item to MW2's own weapon and ammo system
//! (`sim::item::sync_minecraft_arsenal`, `drain_minecraft_ammo_consumption`),
//! host side only: crafting a gun at a table is this player's permanent
//! unlock of that MW2 weapon (no class loadout hands one out on this map),
//! and a reload draws its rounds from the Minecraft inventory's universal
//! ammo item, since there is no MW2 reserve ammo pool here.
//!
//! `sim` has no unit-test harness of its own (a `FrameWorld` is only ever
//! built by the real simulation), so the two functions here that touch one
//! stay thin and are covered end to end by the `minecraft_multiplayer` and
//! `crafting_table` approved scenarios; `resolve_weapon_indices` is the
//! pure part, unit-tested below.
use minecraftoss_player::inventory::Inventory;

/// `unlocked`'s weapon script names resolved against `names` (`world.weapon_script_names()`'s
/// order, each index its weapon id), dropping any this build does not
/// know — a saved name from a different MW2 content set, say.
pub(crate) fn resolve_weapon_indices(unlocked: &[String], names: &[String]) -> Vec<u32> {
    unlocked
        .iter()
        .filter_map(|name| names.iter().position(|n| n == name))
        .filter_map(|i| u32::try_from(i).ok())
        .collect()
}

/// Before this tick's simulation step: promotes a crafting table's just
/// taken gun result into `unlocked` (`promote_crafted_guns`), telling
/// `persist` of every weapon newly added (a keyed player's record), then
/// feeds `world` the player's full unlocked set and how many universal
/// ammo items its inventory holds, for `sync_minecraft_arsenal` to grant or
/// strip guns and seed the active one's reserve.
pub(crate) fn feed_arsenal(
    world: &mut sim::SimWorld,
    id: sim::ClientId,
    inventory: &mut Inventory,
    unlocked: &mut Vec<String>,
    mut persist: impl FnMut(&str),
) {
    for weapon in crate::minecraft_inventory::promote_crafted_guns(inventory, unlocked) {
        persist(&weapon);
    }
    let names = world.weapon_script_names();
    world.set_minecraft_unlocked_weapons(id, &resolve_weapon_indices(unlocked, &names));
    let ammo = i32::try_from(inventory.count(crate::minecraft_inventory::AMMO_ITEM)).unwrap_or(i32::MAX);
    world.set_minecraft_ammo_available(id, ammo);
}

/// After the tick: removes however many universal ammo items a reload drew
/// this tick (`drain_minecraft_ammo_consumption` fed this into `world`).
pub(crate) fn drain_ammo(world: &mut sim::SimWorld, id: sim::ClientId, inventory: &mut Inventory) {
    let consumed = world.take_minecraft_ammo_consumed(id);
    if consumed > 0 {
        inventory.remove_count(crate::minecraft_inventory::AMMO_ITEM, consumed as u32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_names_resolve_to_their_index() {
        let names: Vec<String> = ["", "usp_mp", "ak47_mp", "spas12_mp"].iter().map(|s| s.to_string()).collect();
        let unlocked = vec!["ak47_mp".to_owned(), "usp_mp".to_owned()];
        let mut got = resolve_weapon_indices(&unlocked, &names);
        got.sort_unstable();
        assert_eq!(got, vec![1, 2]);
    }

    #[test]
    fn an_unknown_name_is_dropped_not_zero() {
        let names: Vec<String> = ["", "usp_mp"].iter().map(|s| s.to_string()).collect();
        let unlocked = vec!["usp_mp".to_owned(), "a_weapon_this_build_does_not_have".to_owned()];
        assert_eq!(resolve_weapon_indices(&unlocked, &names), vec![1]);
    }

    #[test]
    fn an_empty_unlocked_set_resolves_to_nothing() {
        let names: Vec<String> = ["", "usp_mp"].iter().map(|s| s.to_string()).collect();
        assert_eq!(resolve_weapon_indices(&[], &names), Vec::<u32>::new());
    }
}
