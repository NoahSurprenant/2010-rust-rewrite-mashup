//! Vanilla's tool rules for breaking blocks: which held item is the correct
//! tool for a block's drops (`Player.hasCorrectToolForDrops`) and how fast it
//! mines (`Player.getDestroySpeed`, `BlockState.getDestroyProgress`).
//!
//! What vanilla keeps as data comes from the data JAR: the block tags
//! (`mineable/pickaxe`, `incorrect_for_wooden_tool` and the other
//! `needs_*_tool` tiers, `sword_efficient`, the shears' speed tags) and the
//! item tags (`pickaxes`, `axes`, `swords`...). Which blocks need the correct
//! tool at all (`BlockBehaviour.Properties.requiresCorrectToolForDrops`) is
//! code, not data: it is read from the JAR's `Blocks` class (the 26.x JAR is
//! not obfuscated), see [`requiring_blocks`]. Also code, and so written here:
//! each tool material's speed and incorrect-blocks tag (`ToolMaterial`), and
//! the sword's and shears' rules (`Item.Properties.sword`,
//! `ShearsItem.createToolProperties`), cobweb being named in them directly.
use std::collections::{HashMap, HashSet};
use std::path::Path;

use minecraftoss_player::inventory::ItemStack;
use minecraftoss_player::loot::LootBook;
use minecraftoss_player::rng::XoroshiroRandom;
use serde_json::Value;

/// What broke a block, as its loot sees it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Breaker {
    /// A player's hand holding this (`None`: empty). A knife is an empty
    /// hand: vanilla has no melee weapon that is a tool but the sword.
    Hand(Option<ItemStack>),
    /// A bullet: as [`gun_break_tool`] says.
    Gun,
    /// An explosion of this radius (vanilla's `EXPLOSION_RADIUS`): no tool,
    /// no correct-tool check, `survives_explosion` and `explosion_decay`
    /// keep each roll one time in the radius.
    Explosion(f32),
}

impl Breaker {
    /// What broke a block mining reports broken: `hand` is this game's own
    /// player's (only it mines here by hand; a client's hand breaks come as
    /// its `Intent::Break`), a knife is an empty hand, a blast is TNT's.
    pub(crate) fn of(cause: crate::minecraft_mining::Cause, hand: &Breaker) -> Self {
        match cause {
            crate::minecraft_mining::Cause::Hand => hand.clone(),
            crate::minecraft_mining::Cause::Knife => Self::Hand(None),
            crate::minecraft_mining::Cause::Gun => Self::Gun,
            crate::minecraft_mining::Cause::Blast => Self::Explosion(crate::minecraft_mining::TNT_POWER),
        }
    }
}

/// THE ONE POLICY POINT for blocks broken by gunfire: what their loot
/// counts the gun as holding. The owner's decision: an empty hand, so a shot
/// block drops what a punched-out one does (logs, dirt, glass none; stone and
/// ores nothing, as they need a pickaxe). Any other policy (the block's
/// lowest correct tool, say) is a change here alone.
pub(crate) fn gun_break_tool(_block: &str) -> Option<ItemStack> {
    None
}

/// The one rule from a broken block to its drops, whatever broke it: the
/// breaker's tool must be correct for a block that needs it
/// (`Player.hasCorrectToolForDrops`, an explosion skipping it as vanilla's
/// does), then the block's loot table rolls with that tool (its `match_tool`
/// conditions, silk touch and fortune) from the table's world-seeded random
/// sequence. `None` for a block with no table, or a rule the loot book cannot
/// evaluate; without tool rules (no data JAR) no block needs a tool.
pub(crate) fn block_drops(
    loot: &LootBook,
    rules: Option<&ToolRules>,
    block: &minecraftoss_player::Block,
    breaker: &Breaker,
    seed: u64,
    sequences: &mut HashMap<String, XoroshiroRandom>,
) -> Option<Vec<ItemStack>> {
    let (tool, radius) = match breaker {
        Breaker::Hand(held) => (held.clone(), None),
        Breaker::Gun => (gun_break_tool(&block.id), None),
        Breaker::Explosion(radius) => (None, Some(*radius)),
    };
    if radius.is_none() && rules.is_some_and(|rules| !rules.correct_for_drops(&block.id, tool.as_ref())) {
        return Some(Vec::new());
    }
    loot.roll_drops_named_exploded(block, tool.as_ref(), radius, seed, sequences)
}

/// `ToolMaterial`: the item ID prefix, the tag of blocks it cannot get drops
/// from, and its mining speed.
const MATERIALS: [(&str, &str, f32); 7] = [
    ("wooden", "minecraft:incorrect_for_wooden_tool", 2.0),
    ("stone", "minecraft:incorrect_for_stone_tool", 4.0),
    ("copper", "minecraft:incorrect_for_copper_tool", 5.0),
    ("iron", "minecraft:incorrect_for_iron_tool", 6.0),
    ("golden", "minecraft:incorrect_for_gold_tool", 12.0),
    ("diamond", "minecraft:incorrect_for_diamond_tool", 8.0),
    ("netherite", "minecraft:incorrect_for_netherite_tool", 9.0),
];

/// The mining tools: their item tag and the tag of blocks they mine.
const DIGGERS: [(&str, &str); 4] = [
    ("minecraft:pickaxes", "minecraft:mineable/pickaxe"),
    ("minecraft:axes", "minecraft:mineable/axe"),
    ("minecraft:shovels", "minecraft:mineable/shovel"),
    ("minecraft:hoes", "minecraft:mineable/hoe"),
];

const COBWEB: &str = "minecraft:cobweb";

/// The blocks a `Tool.Rule` applies to.
#[derive(Clone, Debug, PartialEq)]
enum Blocks {
    Tag(&'static str),
    Id(&'static str),
}

/// `Tool.Rule`: for its blocks, a mining speed and whether it gets drops,
/// each when it says.
#[derive(Clone, Debug, PartialEq)]
struct Rule {
    blocks: Blocks,
    speed: Option<f32>,
    correct: Option<bool>,
}

impl Rule {
    fn denies_drops(blocks: Blocks) -> Self {
        Self { blocks, speed: None, correct: Some(false) }
    }
    fn mines_and_drops(blocks: Blocks, speed: f32) -> Self {
        Self { blocks, speed: Some(speed), correct: Some(true) }
    }
    fn override_speed(blocks: Blocks, speed: f32) -> Self {
        Self { blocks, speed: Some(speed), correct: None }
    }
}

/// The `minecraft:tool` component of an item's default stack.
#[derive(Clone, Debug, PartialEq)]
struct Tool {
    rules: Vec<Rule>,
}

/// Block and item tags resolved to their members, and the blocks that need
/// the correct tool for drops.
#[derive(Default)]
pub(crate) struct ToolRules {
    block_tags: HashMap<String, HashSet<String>>,
    item_tags: HashMap<String, HashSet<String>>,
    requires_tool: HashSet<String>,
}

const BLOCKS_CLASS: &str = "net/minecraft/world/level/block/Blocks.class";
const COLOR_COLLECTION_CLASS: &str = "net/minecraft/world/level/block/ColorCollection.class";
const COPPER_COLLECTION_CLASS: &str = "net/minecraft/world/level/block/WeatheringCopperCollection.class";
const BLOCK_TAGS: &str = "data/minecraft/tags/block/";
/// The classes of ID constants, such as `BlockItemIds.STONE`.
const REFERENCES: &str = "net/minecraft/references/";
const ITEM_TAGS: &str = "data/minecraft/tags/item/";

impl ToolRules {
    /// From the data JAR: its tags and its `Blocks` class.
    pub(crate) fn from_jar(path: &Path) -> anyhow::Result<Self> {
        let entries = minecraftoss_player::loot::jar_entries(path, |name| {
            name == BLOCKS_CLASS
                || name == COLOR_COLLECTION_CLASS
                || name == COPPER_COLLECTION_CLASS
                || (name.starts_with(REFERENCES) && name.ends_with(".class"))
                || (name.ends_with(".json") && (name.starts_with(BLOCK_TAGS) || name.starts_with(ITEM_TAGS)))
        })?;
        let mut block_raw = HashMap::new();
        let mut item_raw = HashMap::new();
        let mut classes = HashMap::new();
        for (name, bytes) in entries {
            if name.ends_with(".class") {
                classes.insert(name, bytes);
                continue;
            }
            let (raw, prefix) = if name.starts_with(BLOCK_TAGS) {
                (&mut block_raw, BLOCK_TAGS)
            } else {
                (&mut item_raw, ITEM_TAGS)
            };
            let id = format!("minecraft:{}", &name[prefix.len()..name.len() - ".json".len()]);
            let value: Value = serde_json::from_slice(&bytes)?;
            let members = value["values"]
                .as_array()
                .map(|values| {
                    values.iter().filter_map(|v| v.as_str().or_else(|| v["id"].as_str())).map(str::to_owned).collect()
                })
                .unwrap_or_default();
            raw.insert(id, members);
        }
        let families = Families::read(
            classes.get(COLOR_COLLECTION_CLASS).map(Vec::as_slice),
            classes.get(COPPER_COLLECTION_CLASS).map(Vec::as_slice),
        );
        let mut ids = HashMap::new();
        for (name, bytes) in &classes {
            if name.starts_with(REFERENCES) {
                ids.extend(id_constants(bytes, &families));
            }
        }
        let class = classes.get(BLOCKS_CLASS).ok_or_else(|| anyhow::anyhow!("{BLOCKS_CLASS} not in the JAR"))?;
        let requires_tool =
            requiring_blocks(class, &ids).ok_or_else(|| anyhow::anyhow!("{BLOCKS_CLASS}: no block properties read"))?;
        Ok(Self::from_parts(resolve_tags(&block_raw), resolve_tags(&item_raw), requires_tool))
    }

    /// From resolved tags and the blocks that need the correct tool.
    pub(crate) fn from_parts(
        block_tags: HashMap<String, HashSet<String>>,
        item_tags: HashMap<String, HashSet<String>>,
        requires_tool: HashSet<String>,
    ) -> Self {
        Self { block_tags, item_tags, requires_tool }
    }

    fn in_block_tag(&self, tag: &str, block: &str) -> bool {
        self.block_tags.get(tag).is_some_and(|members| members.contains(block))
    }

    fn in_item_tag(&self, tag: &str, item: &str) -> bool {
        self.item_tags.get(tag).is_some_and(|members| members.contains(item))
    }

    fn matches(&self, blocks: &Blocks, block: &str) -> bool {
        match blocks {
            Blocks::Tag(tag) => self.in_block_tag(tag, block),
            Blocks::Id(id) => *id == block,
        }
    }

    /// The default `minecraft:tool` component of an item, `None` for an item
    /// that is not a tool (and for an empty hand).
    fn tool(&self, item: &str) -> Option<Tool> {
        if let Some((_, mineable)) = DIGGERS.iter().find(|(tag, _)| self.in_item_tag(tag, item)) {
            let path = item.strip_prefix("minecraft:").unwrap_or(item);
            let (_, incorrect, speed) = MATERIALS.iter().find(|(prefix, ..)| path.starts_with(&format!("{prefix}_")))?;
            // `ToolMaterial.createToolProperties`.
            return Some(Tool {
                rules: vec![Rule::denies_drops(Blocks::Tag(incorrect)), Rule::mines_and_drops(Blocks::Tag(mineable), *speed)],
            });
        }
        if self.in_item_tag("minecraft:swords", item) {
            return Some(Tool {
                rules: vec![
                    Rule::mines_and_drops(Blocks::Id(COBWEB), 15.0),
                    Rule::override_speed(Blocks::Tag("minecraft:sword_instantly_mines"), f32::MAX),
                    Rule::override_speed(Blocks::Tag("minecraft:sword_efficient"), 1.5),
                ],
            });
        }
        if item == "minecraft:shears" {
            return Some(Tool {
                rules: vec![
                    Rule::mines_and_drops(Blocks::Id(COBWEB), 15.0),
                    Rule::override_speed(Blocks::Tag("minecraft:shears_extreme_breaking_speed"), 15.0),
                    Rule::override_speed(Blocks::Tag("minecraft:shears_major_breaking_speed"), 5.0),
                    Rule::override_speed(Blocks::Tag("minecraft:shears_minor_breaking_speed"), 2.0),
                ],
            });
        }
        None
    }

    /// `Tool.isCorrectForDrops`: the first rule for the block that says.
    fn tool_correct(&self, tool: &Tool, block: &str) -> bool {
        tool.rules
            .iter()
            .filter(|rule| self.matches(&rule.blocks, block))
            .find_map(|rule| rule.correct)
            .unwrap_or(false)
    }

    /// `Player.hasCorrectToolForDrops`: whether a block broken with this in
    /// hand drops its loot.
    pub(crate) fn correct_for_drops(&self, block: &str, held: Option<&ItemStack>) -> bool {
        if !self.requires_tool.contains(block) {
            return true;
        }
        held.and_then(|stack| self.tool(&stack.id)).is_some_and(|tool| self.tool_correct(&tool, block))
    }

    /// `Player.getDestroySpeed` (on the ground, in air, without effects):
    /// the tool's speed for the block, plus efficiency's.
    pub(crate) fn destroy_speed(&self, block: &str, held: Option<&ItemStack>) -> f32 {
        let Some(stack) = held else {
            return 1.0;
        };
        let speed = self
            .tool(&stack.id)
            .and_then(|tool| tool.rules.iter().filter(|rule| self.matches(&rule.blocks, block)).find_map(|rule| rule.speed))
            .unwrap_or(1.0);
        let efficiency = enchantment_level(stack, "minecraft:efficiency");
        if speed > 1.0 && efficiency > 0 {
            speed + (efficiency * efficiency + 1) as f32
        } else {
            speed
        }
    }

    /// `BlockState.getDestroyProgress` times the block's hardness: what one
    /// tick of mining adds, before dividing by the hardness.
    pub(crate) fn progress_times_hardness(&self, block: &str, held: Option<&ItemStack>) -> f32 {
        let divisor = if self.correct_for_drops(block, held) { 30.0 } else { 100.0 };
        self.destroy_speed(block, held) / divisor
    }

    /// How many blocks need the correct tool.
    pub(crate) fn requiring_count(&self) -> usize {
        self.requires_tool.len()
    }
}

/// An enchantment's level on a stack (`minecraft:enchantments`, with or
/// without its `levels` wrapper).
fn enchantment_level(stack: &ItemStack, id: &str) -> u32 {
    let Some(enchantments) = stack.components.as_ref().and_then(|c| c.get("minecraft:enchantments")) else {
        return 0;
    };
    let levels = enchantments.get("levels").unwrap_or(enchantments);
    levels.get(id).and_then(Value::as_u64).unwrap_or(0) as u32
}

/// Tags with their nested `#tag` references expanded.
fn resolve_tags(raw: &HashMap<String, Vec<String>>) -> HashMap<String, HashSet<String>> {
    fn expand(
        tag: &str,
        raw: &HashMap<String, Vec<String>>,
        done: &mut HashMap<String, HashSet<String>>,
        visiting: &mut HashSet<String>,
    ) -> HashSet<String> {
        if let Some(members) = done.get(tag) {
            return members.clone();
        }
        if !visiting.insert(tag.to_owned()) {
            return HashSet::new();
        }
        let mut members = HashSet::new();
        for member in raw.get(tag).into_iter().flatten() {
            if let Some(nested) = member.strip_prefix('#') {
                members.extend(expand(nested, raw, done, visiting));
            } else {
                members.insert(member.clone());
            }
        }
        visiting.remove(tag);
        done.insert(tag.to_owned(), members.clone());
        members
    }
    let mut done = HashMap::new();
    for tag in raw.keys() {
        expand(tag, raw, &mut done, &mut HashSet::new());
    }
    done
}

const PROPERTIES: &str = "net/minecraft/world/level/block/state/BlockBehaviour$Properties";
const BLOCKS: &str = "net/minecraft/world/level/block/Blocks";
#[cfg(test)]
const BLOCK_DESC: &str = "Lnet/minecraft/world/level/block/Block;";

/// How `BlockItemIds` names the blocks of its families: the dye colours of
/// `ColorCollection` (`white_concrete`...) and the prefixes of
/// `WeatheringCopperCollection` (`exposed_cut_copper`, `waxed_...`).
#[derive(Default)]
pub(crate) struct Families {
    colors: Vec<String>,
    copper_prefixes: Vec<String>,
}

impl Families {
    /// From the two collection classes' `<clinit>`: `ColorCollection.VALUES`
    /// lists the `DyeColor`s, `WeatheringCopperCollection.PREFIXES` the
    /// prefixes.
    fn read(color_class: Option<&[u8]>, copper_class: Option<&[u8]>) -> Self {
        let clinit_ops = |bytes: Option<&[u8]>| {
            let class = class_file::parse(bytes?)?;
            let ops = class_file::ops(&class.methods.iter().find(|m| m.name == "<clinit>")?.code);
            Some((class, ops))
        };
        let mut families = Self::default();
        if let Some((class, ops)) = clinit_ops(color_class) {
            for op in ops {
                match op {
                    class_file::Op::GetStatic(index) => {
                        if let Some((owner, field, _)) = class.member(index)
                            && owner.ends_with("/DyeColor")
                        {
                            families.colors.push(field.to_lowercase());
                        }
                    }
                    class_file::Op::PutStatic(_) if !families.colors.is_empty() => break,
                    _ => {}
                }
            }
        }
        if let Some((class, ops)) = clinit_ops(copper_class) {
            let mut strings = Vec::new();
            for op in ops {
                match op {
                    class_file::Op::Ldc(index) => strings.extend(class.string(index)),
                    class_file::Op::PutStatic(index) => {
                        if class.member(index).is_some_and(|(_, field, _)| field == "PREFIXES") {
                            families.copper_prefixes = std::mem::take(&mut strings);
                        }
                        strings.clear();
                    }
                    _ => {}
                }
            }
        }
        families
    }
}

/// A class's static ID constants (`BlockItemIds.STONE`, by `owner.FIELD`):
/// the block IDs each names. One is made from a string; a family from a
/// base name, by colour (`createSimpleColored`) or copper state
/// (`createSimpleCopper`, or `prefixWithState` over a list of names).
fn id_constants(class: &[u8], families: &Families) -> HashMap<String, Vec<String>> {
    let mut ids = HashMap::new();
    let Some(class) = class_file::parse(class) else {
        return ids;
    };
    let Some(clinit) = class.methods.iter().find(|m| m.name == "<clinit>") else {
        return ids;
    };
    let mut raw: HashMap<String, Vec<String>> = HashMap::new();
    let (mut strings, mut gets, mut calls) = (Vec::new(), Vec::new(), Vec::new());
    for op in class_file::ops(&clinit.code) {
        match op {
            class_file::Op::Ldc(index) => strings.extend(class.string(index)),
            class_file::Op::GetStatic(index) => {
                if let Some((owner, field, _)) = class.member(index) {
                    gets.push(format!("{owner}.{field}"));
                }
            }
            class_file::Op::Invoke(index) => {
                if let Some((_, method, _)) = class.member(index) {
                    calls.push(method.to_owned());
                }
            }
            class_file::Op::InvokeDynamic(_) => {}
            class_file::Op::PutStatic(index) => {
                let Some((owner, field, _)) = class.member(index) else {
                    continue;
                };
                let key = format!("{owner}.{field}");
                let called = |name: &str| calls.iter().any(|c| c == name);
                let names: Vec<String> = match strings.first() {
                    Some(base) if called("createSimpleColored") => {
                        families.colors.iter().map(|color| format!("{color}_{base}")).collect()
                    }
                    Some(base) if called("createSimpleCopper") => {
                        families.copper_prefixes.iter().map(|prefix| format!("{prefix}{base}")).collect()
                    }
                    _ if called("prefixWithState") => gets
                        .iter()
                        .find_map(|get| raw.get(get).filter(|names| !names.is_empty()))
                        .map(|names| {
                            let prefixes = &families.copper_prefixes;
                            prefixes.iter().enumerate().map(|(i, prefix)| format!("{prefix}{}", names[i % names.len()])).collect()
                        })
                        .unwrap_or_default(),
                    Some(id) => vec![id.clone()],
                    None => Vec::new(),
                };
                raw.insert(key.clone(), std::mem::take(&mut strings));
                if !names.is_empty() {
                    ids.insert(key, names);
                }
                gets.clear();
                calls.clear();
            }
        }
    }
    ids
}

/// The blocks that `Blocks` registers with `requiresCorrectToolForDrops()`,
/// from its bytecode. `Blocks.<clinit>` registers one block (or one family
/// of blocks) per static field: from its ID (a constant such as
/// `BlockItemIds.STONE`, by `ids`, or a string) to the `putstatic` of its
/// field. A block needs the correct tool when its properties call
/// `requiresCorrectToolForDrops`, directly or in a `Blocks` method (a helper
/// or the lambda a family makes its properties with), or copy another
/// block's properties (`ofLegacyCopy`, `ofFullCopy`, directly or in a helper
/// such as `registerStair`) from a block that does.
pub(crate) fn requiring_blocks(class: &[u8], ids: &HashMap<String, Vec<String>>) -> Option<HashSet<String>> {
    let class = class_file::parse(class)?;
    #[derive(Default, Clone)]
    struct Calls {
        requires: bool,
        copies: bool,
        /// The `Blocks` methods it calls or makes lambdas of.
        helpers: Vec<String>,
        /// The `Blocks` fields it reads.
        reads: Vec<String>,
    }
    let key = |name: &str, desc: &str| format!("{name}{desc}");
    let properties_call = |owner: &str, name: &str| -> (bool, bool) {
        (
            owner == PROPERTIES && name == "requiresCorrectToolForDrops",
            owner == PROPERTIES && (name == "ofLegacyCopy" || name == "ofFullCopy"),
        )
    };
    let mut helpers: HashMap<String, Calls> = HashMap::new();
    for method in &class.methods {
        let mut calls = Calls::default();
        for op in class_file::ops(&method.code) {
            let member = match op {
                class_file::Op::Invoke(index) => class.member(index),
                class_file::Op::InvokeDynamic(index) => class.lambda(index),
                class_file::Op::GetStatic(index) => {
                    if let Some((BLOCKS, field, _)) = class.member(index) {
                        calls.reads.push(field.to_owned());
                    }
                    continue;
                }
                _ => continue,
            };
            let Some((owner, name, desc)) = member else {
                continue;
            };
            let (requires, copies) = properties_call(owner, name);
            calls.requires |= requires;
            calls.copies |= copies;
            if owner == BLOCKS {
                calls.helpers.push(key(name, desc));
            }
        }
        helpers.insert(key(&method.name, &method.desc), calls);
    }
    // What each method does through the methods it calls.
    loop {
        let mut changed = false;
        for name in helpers.keys().cloned().collect::<Vec<_>>() {
            let calls = helpers[&name].clone();
            let through = |flag: fn(&Calls) -> bool| calls.helpers.iter().any(|h| helpers.get(h).is_some_and(flag));
            let requires = calls.requires || through(|c| c.requires);
            let copies = calls.copies || through(|c| c.copies);
            let entry = helpers.get_mut(&name)?;
            changed |= requires != entry.requires || copies != entry.copies;
            entry.requires = requires;
            entry.copies = copies;
        }
        if !changed {
            break;
        }
    }
    let clinit = class.methods.iter().find(|m| m.name == "<clinit>")?;
    let mut by_field: HashMap<String, bool> = HashMap::new();
    let mut requiring = HashSet::new();
    let (mut names, mut requires, mut copies, mut reads) = (Vec::<String>::new(), false, false, Vec::<String>::new());
    for op in class_file::ops(&clinit.code) {
        let helper = match op {
            class_file::Op::Ldc(index) => {
                if names.is_empty() {
                    names.extend(class.string(index));
                }
                continue;
            }
            class_file::Op::GetStatic(index) => {
                if let Some((owner, field, _)) = class.member(index) {
                    if owner == BLOCKS {
                        reads.push(field.to_owned());
                    } else if names.is_empty()
                        && let Some(id) = ids.get(&format!("{owner}.{field}"))
                    {
                        names = id.clone();
                    }
                }
                continue;
            }
            class_file::Op::Invoke(index) => class.member(index),
            class_file::Op::InvokeDynamic(index) => class.lambda(index),
            class_file::Op::PutStatic(index) => {
                let Some((BLOCKS, field, _)) = class.member(index) else {
                    continue;
                };
                let needs = requires || (copies && reads.iter().any(|f| by_field.get(f) == Some(&true)));
                by_field.insert(field.to_owned(), needs);
                if needs {
                    for id in names.drain(..) {
                        requiring.insert(if id.contains(':') { id } else { format!("minecraft:{id}") });
                    }
                }
                names.clear();
                (requires, copies) = (false, false);
                reads.clear();
                continue;
            }
        };
        let Some((owner, method, desc)) = helper else {
            continue;
        };
        let (direct_requires, direct_copies) = properties_call(owner, method);
        requires |= direct_requires;
        copies |= direct_copies;
        if owner == BLOCKS
            && let Some(calls) = helpers.get(&key(method, desc))
        {
            requires |= calls.requires;
            copies |= calls.copies;
            reads.extend(calls.reads.iter().cloned());
        }
    }
    (!by_field.is_empty()).then_some(requiring)
}

/// Just enough of the Java class file format to follow `Blocks`' bytecode.
mod class_file {
    pub(super) enum Const {
        Utf8(String),
        Class(u16),
        String(u16),
        Member { class: u16, name_and_type: u16 },
        NameAndType { name: u16, desc: u16 },
        MethodHandle { reference: u16 },
        InvokeDynamic { bootstrap: u16 },
        Other,
    }

    pub(super) struct Method {
        pub name: String,
        pub desc: String,
        pub code: Vec<u8>,
    }

    pub(super) struct Class {
        pool: Vec<Const>,
        pub methods: Vec<Method>,
        /// `BootstrapMethods`: each bootstrap's static arguments.
        bootstraps: Vec<Vec<u16>>,
    }

    struct Reader<'a> {
        bytes: &'a [u8],
        at: usize,
    }

    impl Reader<'_> {
        fn take(&mut self, n: usize) -> Option<&[u8]> {
            let slice = self.bytes.get(self.at..self.at + n)?;
            self.at += n;
            Some(slice)
        }
        fn u1(&mut self) -> Option<u8> {
            Some(self.take(1)?[0])
        }
        fn u2(&mut self) -> Option<u16> {
            let b = self.take(2)?;
            Some(u16::from_be_bytes([b[0], b[1]]))
        }
        fn u4(&mut self) -> Option<u32> {
            let b = self.take(4)?;
            Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        }
    }

    pub(super) fn parse(bytes: &[u8]) -> Option<Class> {
        let mut r = Reader { bytes, at: 0 };
        if r.u4()? != 0xCAFE_BABE {
            return None;
        }
        r.take(4)?;
        let count = r.u2()?;
        let mut pool = vec![Const::Other];
        while pool.len() < usize::from(count) {
            let tag = r.u1()?;
            let entry = match tag {
                1 => {
                    let len = usize::from(r.u2()?);
                    Const::Utf8(String::from_utf8_lossy(r.take(len)?).into_owned())
                }
                7 => Const::Class(r.u2()?),
                8 => Const::String(r.u2()?),
                9..=11 => Const::Member { class: r.u2()?, name_and_type: r.u2()? },
                12 => Const::NameAndType { name: r.u2()?, desc: r.u2()? },
                3 | 4 => {
                    r.take(4)?;
                    Const::Other
                }
                5 | 6 => {
                    r.take(8)?;
                    pool.push(Const::Other);
                    Const::Other
                }
                15 => {
                    r.take(1)?;
                    Const::MethodHandle { reference: r.u2()? }
                }
                16 | 19 | 20 => {
                    r.take(2)?;
                    Const::Other
                }
                17 => {
                    r.take(4)?;
                    Const::Other
                }
                18 => {
                    let bootstrap = r.u2()?;
                    r.take(2)?;
                    Const::InvokeDynamic { bootstrap }
                }
                _ => return None,
            };
            pool.push(entry);
        }
        r.take(6)?;
        let interfaces = usize::from(r.u2()?);
        r.take(interfaces * 2)?;
        let skip_members = |r: &mut Reader<'_>| -> Option<()> {
            for _ in 0..r.u2()? {
                r.take(6)?;
                for _ in 0..r.u2()? {
                    r.take(2)?;
                    let len = r.u4()? as usize;
                    r.take(len)?;
                }
            }
            Some(())
        };
        skip_members(&mut r)?;
        let mut class = Class { pool, methods: Vec::new(), bootstraps: Vec::new() };
        for _ in 0..r.u2()? {
            r.take(2)?;
            let name = class.utf8(r.u2()?)?.to_owned();
            let desc = class.utf8(r.u2()?)?.to_owned();
            let mut code = Vec::new();
            for _ in 0..r.u2()? {
                let attribute = class.utf8(r.u2()?)?.to_owned();
                let len = r.u4()? as usize;
                let body = r.take(len)?;
                if attribute == "Code" && body.len() >= 8 {
                    let code_len = u32::from_be_bytes([body[4], body[5], body[6], body[7]]) as usize;
                    code = body.get(8..8 + code_len)?.to_vec();
                }
            }
            class.methods.push(Method { name, desc, code });
        }
        for _ in 0..r.u2()? {
            let attribute = class.utf8(r.u2()?)?.to_owned();
            let len = r.u4()? as usize;
            let body = r.take(len)?;
            if attribute == "BootstrapMethods" {
                let mut b = Reader { bytes: body, at: 0 };
                for _ in 0..b.u2()? {
                    b.u2()?;
                    let count = b.u2()?;
                    class.bootstraps.push((0..count).map(|_| b.u2()).collect::<Option<_>>()?);
                }
            }
        }
        Some(class)
    }

    impl Class {
        fn utf8(&self, index: u16) -> Option<&str> {
            match self.pool.get(usize::from(index))? {
                Const::Utf8(text) => Some(text),
                _ => None,
            }
        }

        /// A string constant's text.
        pub(super) fn string(&self, index: u16) -> Option<String> {
            match self.pool.get(usize::from(index))? {
                Const::String(utf8) => self.utf8(*utf8).map(str::to_owned),
                _ => None,
            }
        }

        /// The method an `invokedynamic` makes a lambda of (its bootstrap's
        /// method handle argument).
        pub(super) fn lambda(&self, index: u16) -> Option<(&str, &str, &str)> {
            let Const::InvokeDynamic { bootstrap } = self.pool.get(usize::from(index))? else {
                return None;
            };
            self.bootstraps.get(usize::from(*bootstrap))?.iter().find_map(|arg| match self.pool.get(usize::from(*arg))? {
                Const::MethodHandle { reference } => self.member(*reference),
                _ => None,
            })
        }

        /// A field or method reference: its class, name and descriptor.
        pub(super) fn member(&self, index: u16) -> Option<(&str, &str, &str)> {
            let Const::Member { class, name_and_type } = self.pool.get(usize::from(index))? else {
                return None;
            };
            let Const::Class(class_name) = self.pool.get(usize::from(*class))? else {
                return None;
            };
            let Const::NameAndType { name, desc } = self.pool.get(usize::from(*name_and_type))? else {
                return None;
            };
            Some((self.utf8(*class_name)?, self.utf8(*name)?, self.utf8(*desc)?))
        }
    }

    /// The instructions `requiring_blocks` follows, with their constant
    /// pool index.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(super) enum Op {
        Ldc(u16),
        GetStatic(u16),
        PutStatic(u16),
        Invoke(u16),
        InvokeDynamic(u16),
    }

    /// The followed instructions of a method's code, in order.
    pub(super) fn ops(code: &[u8]) -> Vec<Op> {
        let mut out = Vec::new();
        let mut at = 0usize;
        let u2 = |at: usize| code.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
        let i4 = |at: usize| code.get(at..at + 4).map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]));
        while let Some(&opcode) = code.get(at) {
            let len = match opcode {
                0x12 => {
                    if let Some(&index) = code.get(at + 1) {
                        out.push(Op::Ldc(u16::from(index)));
                    }
                    2
                }
                0x13 => {
                    if let Some(index) = u2(at + 1) {
                        out.push(Op::Ldc(index));
                    }
                    3
                }
                0xb2 | 0xb3 | 0xb6..=0xb9 => {
                    if let Some(index) = u2(at + 1) {
                        out.push(match opcode {
                            0xb2 => Op::GetStatic(index),
                            0xb3 => Op::PutStatic(index),
                            _ => Op::Invoke(index),
                        });
                    }
                    if opcode == 0xb9 { 5 } else { 3 }
                }
                0x10 | 0x15..=0x19 | 0x36..=0x3a | 0xa9 | 0xbc => 2,
                0x11 | 0x14 | 0x84 | 0x99..=0xa8 | 0xb4 | 0xb5 | 0xbb | 0xbd | 0xc0 | 0xc1 | 0xc6 | 0xc7 => 3,
                0xc5 => 4,
                0xba => {
                    if let Some(index) = u2(at + 1) {
                        out.push(Op::InvokeDynamic(index));
                    }
                    5
                }
                0xc8 | 0xc9 => 5,
                0xc4 => {
                    if code.get(at + 1) == Some(&0x84) {
                        6
                    } else {
                        4
                    }
                }
                0xaa | 0xab => {
                    let pad = 3 - (at % 4);
                    let base = at + 1 + pad;
                    let Some(rest) = (if opcode == 0xaa {
                        i4(base + 4).zip(i4(base + 8)).map(|(low, high)| 12 + 4 * (high - low + 1).max(0) as usize)
                    } else {
                        i4(base + 4).map(|pairs| 8 + 8 * pairs.max(0) as usize)
                    }) else {
                        break;
                    };
                    1 + pad + rest
                }
                _ => 1,
            };
            at += len;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    /// A few blocks and tools, tagged as vanilla tags them.
    fn fixture() -> ToolRules {
        let mut block_raw: HashMap<String, Vec<String>> = HashMap::new();
        let mut add = |tag: &str, members: &[&str]| {
            block_raw.insert(tag.to_owned(), members.iter().map(|s| (*s).to_owned()).collect());
        };
        add("minecraft:mineable/pickaxe", &["minecraft:stone", "minecraft:iron_ore", "minecraft:obsidian", "minecraft:ice"]);
        add("minecraft:mineable/axe", &["minecraft:oak_log"]);
        add("minecraft:mineable/shovel", &["minecraft:dirt"]);
        add("minecraft:mineable/hoe", &["minecraft:oak_leaves"]);
        add("minecraft:needs_stone_tool", &["minecraft:iron_ore"]);
        add("minecraft:needs_iron_tool", &["minecraft:diamond_ore"]);
        add("minecraft:needs_diamond_tool", &["minecraft:obsidian"]);
        add(
            "minecraft:incorrect_for_wooden_tool",
            &["#minecraft:needs_diamond_tool", "#minecraft:needs_iron_tool", "#minecraft:needs_stone_tool"],
        );
        add("minecraft:incorrect_for_stone_tool", &["#minecraft:needs_diamond_tool", "#minecraft:needs_iron_tool"]);
        add("minecraft:incorrect_for_iron_tool", &["#minecraft:needs_diamond_tool"]);
        add("minecraft:incorrect_for_diamond_tool", &[]);
        add("minecraft:sword_efficient", &["minecraft:oak_leaves"]);
        let mut item_raw: HashMap<String, Vec<String>> = HashMap::new();
        for (tag, kind) in [("minecraft:pickaxes", "pickaxe"), ("minecraft:axes", "axe"), ("minecraft:swords", "sword")] {
            item_raw.insert(
                tag.to_owned(),
                ["wooden", "stone", "iron", "diamond"].iter().map(|m| format!("minecraft:{m}_{kind}")).collect(),
            );
        }
        ToolRules::from_parts(
            resolve_tags(&block_raw),
            resolve_tags(&item_raw),
            set(&["minecraft:stone", "minecraft:iron_ore", "minecraft:obsidian", "minecraft:cobweb"]),
        )
    }

    fn held(id: &str) -> Option<ItemStack> {
        Some(ItemStack::new(id, 1))
    }

    #[test]
    fn nested_tags_resolve() {
        let rules = fixture();
        assert!(rules.in_block_tag("minecraft:incorrect_for_wooden_tool", "minecraft:iron_ore"));
        assert!(rules.in_block_tag("minecraft:incorrect_for_stone_tool", "minecraft:obsidian"));
        assert!(!rules.in_block_tag("minecraft:incorrect_for_stone_tool", "minecraft:iron_ore"));
    }

    #[test]
    fn a_block_that_needs_no_tool_drops_by_hand() {
        let rules = fixture();
        for block in ["minecraft:oak_log", "minecraft:dirt", "minecraft:glass", "minecraft:ice"] {
            assert!(rules.correct_for_drops(block, None), "{block}");
            assert!(rules.correct_for_drops(block, held("minecraft:wooden_pickaxe").as_ref()), "{block}");
        }
    }

    #[test]
    fn stone_needs_a_pickaxe() {
        let rules = fixture();
        assert!(!rules.correct_for_drops("minecraft:stone", None));
        assert!(!rules.correct_for_drops("minecraft:stone", held("minecraft:diamond_axe").as_ref()));
        assert!(!rules.correct_for_drops("minecraft:stone", held("minecraft:oak_log").as_ref()));
        assert!(rules.correct_for_drops("minecraft:stone", held("minecraft:wooden_pickaxe").as_ref()));
    }

    #[test]
    fn ores_and_obsidian_need_their_tier() {
        let rules = fixture();
        let ok = |block: &str, tool: &str| rules.correct_for_drops(block, held(tool).as_ref());
        assert!(!ok("minecraft:iron_ore", "minecraft:wooden_pickaxe"));
        assert!(ok("minecraft:iron_ore", "minecraft:stone_pickaxe"));
        assert!(ok("minecraft:iron_ore", "minecraft:diamond_pickaxe"));
        assert!(!ok("minecraft:obsidian", "minecraft:stone_pickaxe"));
        assert!(!ok("minecraft:obsidian", "minecraft:iron_pickaxe"));
        assert!(ok("minecraft:obsidian", "minecraft:diamond_pickaxe"));
    }

    #[test]
    fn swords_get_cobweb() {
        let rules = fixture();
        assert!(!rules.correct_for_drops("minecraft:cobweb", None));
        assert!(rules.correct_for_drops("minecraft:cobweb", held("minecraft:wooden_sword").as_ref()));
        assert_eq!(rules.destroy_speed("minecraft:cobweb", held("minecraft:stone_sword").as_ref()), 15.0);
        assert_eq!(rules.destroy_speed("minecraft:oak_leaves", held("minecraft:stone_sword").as_ref()), 1.5);
    }

    /// `getDestroyProgress`: the tool's speed over 30 with the correct tool
    /// (or none needed), over 100 without.
    #[test]
    fn break_speed_follows_the_tool() {
        let rules = fixture();
        let per = |block: &str, tool: Option<&str>| {
            rules.progress_times_hardness(block, tool.map(|t| ItemStack::new(t, 1)).as_ref())
        };
        assert_eq!(per("minecraft:oak_log", None), 1.0 / 30.0);
        assert_eq!(per("minecraft:stone", None), 1.0 / 100.0);
        assert_eq!(per("minecraft:stone", Some("minecraft:wooden_pickaxe")), 2.0 / 30.0);
        assert_eq!(per("minecraft:stone", Some("minecraft:iron_pickaxe")), 6.0 / 30.0);
        // The pickaxe's speed, but not its tier: no drops, so over 100.
        assert_eq!(per("minecraft:iron_ore", Some("minecraft:wooden_pickaxe")), 2.0 / 100.0);
        assert_eq!(per("minecraft:oak_log", Some("minecraft:stone_axe")), 4.0 / 30.0);
        assert_eq!(per("minecraft:oak_log", Some("minecraft:stone_pickaxe")), 1.0 / 30.0);
        let mut efficient = ItemStack::new("minecraft:iron_pickaxe", 1);
        efficient.components = Some(serde_json::json!({"minecraft:enchantments": {"levels": {"minecraft:efficiency": 2}}}));
        assert_eq!(rules.destroy_speed("minecraft:stone", Some(&efficient)), 6.0 + 5.0);
    }

    /// The correct-tool gate before any loot: a block whose tool is wrong
    /// drops nothing (`Some(empty)`) without its table; otherwise the table
    /// rolls (none here: `None`). An explosion skips the gate.
    #[test]
    fn block_drops_gates_on_the_correct_tool() {
        let rules = fixture();
        let loot = LootBook::default();
        let mut sequences = HashMap::new();
        let mut drops = |block: &str, breaker: Breaker| {
            block_drops(&loot, Some(&rules), &minecraftoss_player::Block::new(block), &breaker, 12345, &mut sequences)
        };
        let nothing = Some(Vec::new());
        assert_eq!(drops("minecraft:stone", Breaker::Hand(None)), nothing);
        assert_eq!(drops("minecraft:stone", Breaker::Hand(held("minecraft:wooden_pickaxe"))), None);
        assert_eq!(drops("minecraft:iron_ore", Breaker::Hand(held("minecraft:wooden_pickaxe"))), nothing);
        assert_eq!(drops("minecraft:iron_ore", Breaker::Hand(held("minecraft:stone_pickaxe"))), None);
        assert_eq!(drops("minecraft:obsidian", Breaker::Hand(held("minecraft:iron_pickaxe"))), nothing);
        assert_eq!(drops("minecraft:obsidian", Breaker::Hand(held("minecraft:diamond_pickaxe"))), None);
        assert_eq!(drops("minecraft:oak_log", Breaker::Hand(None)), None);
        // Guns are an empty hand (the owner's policy), blasts need no tool.
        assert_eq!(drops("minecraft:stone", Breaker::Gun), nothing);
        assert_eq!(drops("minecraft:oak_log", Breaker::Gun), None);
        assert_eq!(drops("minecraft:stone", Breaker::Explosion(4.0)), None);
    }

    #[test]
    fn a_knife_is_an_empty_hand_and_a_blast_is_tnt() {
        use crate::minecraft_mining::Cause;
        let hand = Breaker::Hand(held("minecraft:stone_pickaxe"));
        assert_eq!(Breaker::of(Cause::Hand, &hand), hand);
        assert_eq!(Breaker::of(Cause::Knife, &hand), Breaker::Hand(None));
        assert_eq!(Breaker::of(Cause::Gun, &hand), Breaker::Gun);
        assert_eq!(Breaker::of(Cause::Blast, &hand), Breaker::Explosion(4.0));
        assert_eq!(gun_break_tool("minecraft:stone"), None);
    }

    /// A hand-made class: `<clinit>` registering stone (needing the tool),
    /// dirt, and stone stairs copying stone's properties through a helper.
    #[test]
    fn requiring_blocks_follow_the_bytecode() {
        let mut pool: Vec<Vec<u8>> = Vec::new();
        let utf8 = |pool: &mut Vec<Vec<u8>>, s: &str| {
            let mut e = vec![1];
            e.extend((s.len() as u16).to_be_bytes());
            e.extend(s.as_bytes());
            pool.push(e);
            pool.len() as u16
        };
        let two = |pool: &mut Vec<Vec<u8>>, tag: u8, a: u16, b: Option<u16>| {
            let mut e = vec![tag];
            e.extend(a.to_be_bytes());
            if let Some(b) = b {
                e.extend(b.to_be_bytes());
            }
            pool.push(e);
            pool.len() as u16
        };
        let blocks_name = utf8(&mut pool, BLOCKS);
        let blocks = two(&mut pool, 7, blocks_name, None);
        let props_name = utf8(&mut pool, PROPERTIES);
        let props = two(&mut pool, 7, props_name, None);
        let block_desc = utf8(&mut pool, BLOCK_DESC);
        let member = |pool: &mut Vec<Vec<u8>>, tag: u8, class: u16, name: &str, desc: u16| {
            let n = utf8(pool, name);
            let nat = two(pool, 12, n, Some(desc));
            two(pool, tag, class, Some(nat))
        };
        let void = utf8(&mut pool, "()V");
        let f_stone = member(&mut pool, 9, blocks, "STONE", block_desc);
        let f_dirt = member(&mut pool, 9, blocks, "DIRT", block_desc);
        let f_stairs = member(&mut pool, 9, blocks, "STONE_STAIRS", block_desc);
        let m_requires = member(&mut pool, 10, props, "requiresCorrectToolForDrops", void);
        let m_stair = member(&mut pool, 10, blocks, "registerStair", void);
        let m_copy = member(&mut pool, 10, props, "ofLegacyCopy", void);
        let string = |pool: &mut Vec<Vec<u8>>, s: &str| {
            let u = utf8(pool, s);
            two(pool, 8, u, None)
        };
        let s_stone = string(&mut pool, "stone");
        let s_dirt = string(&mut pool, "dirt");
        let s_stairs = string(&mut pool, "stone_stairs");
        let clinit_name = utf8(&mut pool, "<clinit>");
        let stair_name = utf8(&mut pool, "registerStair");
        let code_name = utf8(&mut pool, "Code");
        let op3 = |op: u8, index: u16| {
            let mut v = vec![op];
            v.extend(index.to_be_bytes());
            v
        };
        let clinit: Vec<u8> = [
            op3(0x13, s_stone),
            vec![0x03, 0x57], // iconst_0; pop: other instructions in between
            op3(0xb6, m_requires),
            op3(0xb3, f_stone),
            op3(0x13, s_dirt),
            op3(0xb3, f_dirt),
            op3(0x13, s_stairs),
            op3(0xb2, f_stone),
            op3(0xb8, m_stair),
            op3(0xb3, f_stairs),
            vec![0xb1],
        ]
        .concat();
        let stair: Vec<u8> = [op3(0xb8, m_copy), vec![0xb1]].concat();
        let method = |name: u16, code: &[u8]| {
            let mut m = vec![0, 8];
            m.extend(name.to_be_bytes());
            m.extend(void.to_be_bytes());
            m.extend(1u16.to_be_bytes());
            m.extend(code_name.to_be_bytes());
            m.extend((code.len() as u32 + 12).to_be_bytes());
            m.extend([0, 4, 0, 0]);
            m.extend((code.len() as u32).to_be_bytes());
            m.extend(code);
            m.extend([0, 0, 0, 0]);
            m
        };
        let mut class = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 69];
        class.extend((pool.len() as u16 + 1).to_be_bytes());
        for entry in &pool {
            class.extend(entry);
        }
        class.extend([0, 0x21]);
        class.extend(blocks.to_be_bytes());
        class.extend([0, 0, 0, 0, 0, 0]);
        class.extend(2u16.to_be_bytes());
        class.extend(method(clinit_name, &clinit));
        class.extend(method(stair_name, &stair));
        class.extend([0, 0]);
        let requiring = requiring_blocks(&class, &HashMap::new()).expect("parsed");
        assert_eq!(requiring, set(&["minecraft:stone", "minecraft:stone_stairs"]));
    }

    /// The real tags and `Blocks` class, when the data JAR is there.
    #[test]
    fn rules_from_the_data_jar() {
        let Some(jar) = crate::minecraft_entities::data_jar() else {
            eprintln!("no Minecraft data JAR: tool rules not checked");
            return;
        };
        let rules = ToolRules::from_jar(&jar).expect("tool rules");
        eprintln!("{} blocks need the correct tool", rules.requiring_count());
        let mineable = &rules.block_tags["minecraft:mineable/pickaxe"];
        let mut extra: Vec<_> = rules.requires_tool.difference(mineable).collect();
        let mut free: Vec<_> = mineable.difference(&rules.requires_tool).collect();
        extra.sort();
        free.sort();
        eprintln!("need a tool, not mineable/pickaxe: {extra:?}");
        eprintln!("mineable/pickaxe, need no tool: {free:?}");
        for block in [
            "minecraft:stone",
            "minecraft:cobblestone",
            "minecraft:stone_stairs",
            "minecraft:stone_slab",
            "minecraft:cobblestone_wall",
            "minecraft:iron_ore",
            "minecraft:deepslate_iron_ore",
            "minecraft:diamond_ore",
            "minecraft:obsidian",
            "minecraft:cobweb",
            "minecraft:snow_block",
            "minecraft:furnace",
            "minecraft:terracotta",
            "minecraft:red_sandstone",
            "minecraft:white_concrete",
            "minecraft:black_terracotta",
            "minecraft:copper_block",
            "minecraft:exposed_copper",
            "minecraft:waxed_copper_block",
            "minecraft:waxed_oxidized_cut_copper_slab",
        ] {
            assert!(rules.requires_tool.contains(block), "{block} should need the correct tool");
        }
        for block in [
            "minecraft:oak_log",
            "minecraft:oak_planks",
            "minecraft:dirt",
            "minecraft:grass_block",
            "minecraft:glass",
            "minecraft:oak_leaves",
            "minecraft:ice",
            "minecraft:crafting_table",
            "minecraft:torch",
            "minecraft:white_wool",
            "minecraft:piston",
            "minecraft:rail",
        ] {
            assert!(!rules.requires_tool.contains(block), "{block} should need no tool");
        }
        let ok = |block: &str, tool: Option<&str>| rules.correct_for_drops(block, tool.map(|t| ItemStack::new(t, 1)).as_ref());
        assert!(!ok("minecraft:stone", None));
        assert!(ok("minecraft:stone", Some("minecraft:wooden_pickaxe")));
        assert!(!ok("minecraft:iron_ore", Some("minecraft:wooden_pickaxe")));
        assert!(ok("minecraft:iron_ore", Some("minecraft:stone_pickaxe")));
        assert!(!ok("minecraft:diamond_ore", Some("minecraft:stone_pickaxe")));
        assert!(ok("minecraft:diamond_ore", Some("minecraft:iron_pickaxe")));
        assert!(!ok("minecraft:obsidian", Some("minecraft:iron_pickaxe")));
        assert!(ok("minecraft:obsidian", Some("minecraft:diamond_pickaxe")));
        assert!(ok("minecraft:cobweb", Some("minecraft:shears")));
        assert!(ok("minecraft:snow_block", Some("minecraft:wooden_shovel")));
        assert!(!ok("minecraft:snow_block", None));
    }

    /// Every way a block breaks, through `block_drops` with the real loot
    /// tables and tool rules, from the fixed world seed.
    #[test]
    fn vanilla_drops_from_the_data_jar() {
        let Some(jar) = crate::minecraft_entities::data_jar() else {
            eprintln!("no Minecraft data JAR: block drops not checked");
            return;
        };
        let loot = LootBook::from_jar(&jar).expect("block loot");
        let rules = ToolRules::from_jar(&jar).expect("tool rules");
        let mut sequences = HashMap::new();
        let mut drops = |block: minecraftoss_player::Block, breaker: Breaker| -> Vec<(String, u8)> {
            block_drops(&loot, Some(&rules), &block, &breaker, 12345, &mut sequences)
                .unwrap_or_else(|| panic!("{} unresolved", block.id))
                .into_iter()
                .map(|stack| (stack.id, stack.count))
                .collect()
        };
        let block = minecraftoss_player::Block::new;
        let tool = |id: &str| Breaker::Hand(held(id));
        let one = |id: &str| vec![(id.to_owned(), 1u8)];
        let log = || block("minecraft:oak_log").with("axis", "y");
        assert_eq!(drops(log(), Breaker::Hand(None)), one("minecraft:oak_log"));
        assert_eq!(drops(block("minecraft:stone"), Breaker::Hand(None)), vec![]);
        assert_eq!(drops(block("minecraft:stone"), tool("minecraft:wooden_pickaxe")), one("minecraft:cobblestone"));
        assert_eq!(drops(block("minecraft:stone"), tool("minecraft:diamond_axe")), vec![]);
        assert_eq!(drops(block("minecraft:iron_ore"), tool("minecraft:wooden_pickaxe")), vec![]);
        assert_eq!(drops(block("minecraft:iron_ore"), tool("minecraft:stone_pickaxe")), one("minecraft:raw_iron"));
        assert_eq!(drops(block("minecraft:obsidian"), tool("minecraft:iron_pickaxe")), vec![]);
        assert_eq!(drops(block("minecraft:obsidian"), tool("minecraft:diamond_pickaxe")), one("minecraft:obsidian"));
        assert_eq!(drops(block("minecraft:glass"), Breaker::Hand(None)), vec![]);
        assert_eq!(drops(block("minecraft:cobweb"), Breaker::Hand(None)), vec![]);
        assert_eq!(drops(block("minecraft:cobweb"), tool("minecraft:wooden_sword")), one("minecraft:string"));
        assert_eq!(drops(block("minecraft:cobweb"), tool("minecraft:shears")), one("minecraft:cobweb"));
        // A knife is an empty hand.
        let knife = Breaker::of(crate::minecraft_mining::Cause::Knife, &tool("minecraft:diamond_pickaxe"));
        assert_eq!(drops(block("minecraft:stone"), knife.clone()), vec![]);
        assert_eq!(drops(log(), knife), one("minecraft:oak_log"));
        // Guns: an empty hand.
        assert_eq!(drops(block("minecraft:stone"), Breaker::Gun), vec![]);
        assert_eq!(drops(log(), Breaker::Gun), one("minecraft:oak_log"));
        // A TNT blast keeps stone's cobblestone one time in four.
        let blasts = 400;
        let mut kept = 0;
        for _ in 0..blasts {
            let got = drops(block("minecraft:stone"), Breaker::Explosion(crate::minecraft_mining::TNT_POWER));
            assert!(got.is_empty() || got == one("minecraft:cobblestone"), "{got:?}");
            kept += usize::from(!got.is_empty());
        }
        eprintln!("stone blasted {blasts} times: {kept} cobblestone");
        assert!((70..=130).contains(&kept), "{kept} of {blasts}");
    }
}




