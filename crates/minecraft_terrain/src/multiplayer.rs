//! The Minecraft map in multiplayer: what a client asks of the host (an
//! `Intent`), what the host tells its clients (a `HostMessage`), the host's
//! log of every block it changed, and the world a host keeps between
//! sessions.
//!
//! The host's game is authoritative. A client sends its block edits and
//! inventory clicks as intents; the host applies them with the same vanilla
//! rules it applies its own, then sends every client the world's seed and
//! map origin, each block change in order (a late joiner starts at the
//! log's beginning) and that client's own inventory.
//!
//! Everything here is plain data and bytes, independent of the engine, so it
//! is tested on its own.
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use minecraftoss_player::inventory::{Inventory, ItemStack};

use crate::pack::ResourceId;
use crate::scene::{Block, BlockPos};

pub mod players;
pub use players::{PlayerEntry, PlayerRecord, Players, Transfer, is_fingerprint};

/// The most bytes an encoded intent takes: it rides in a fixed-size
/// `ClientAction`.
pub const INTENT_BYTES: usize = 96;
/// The longest item or block id an intent carries.
pub const MAX_ID_BYTES: usize = 64;
/// Block changes in one `HostMessage::Edits`, so one reliable row stays small.
pub const EDITS_PER_MESSAGE: usize = 256;
/// The most bytes the changes of one `HostMessage::Edits` take: a row has to
/// fit one message on the master's control stream with room to spare.
pub const EDITS_BYTES_PER_MESSAGE: usize = 4 * 1024;
/// Item entities in one `HostMessage::Items`.
pub const MAX_ITEMS: usize = 64;

const LEVEL_FILE: &str = "level.json";
const EDITS_FILE: &str = "edits.bin";
const PLAYERS_FILE: &str = "players.json";
const MAPS_FILE: &str = "maps.bin";
const EDITS_MAGIC: &[u8; 4] = b"MCED";
const SAVE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    Truncated,
    Malformed(&'static str),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => f.write_str("truncated"),
            Self::Malformed(what) => write!(f, "malformed: {what}"),
        }
    }
}

impl std::error::Error for DecodeError {}

#[derive(Default)]
pub(crate) struct Writer(pub(crate) Vec<u8>);

impl Writer {
    pub(crate) fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    pub(crate) fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn i64(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn f64(&mut self, v: f64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn pos(&mut self, p: BlockPos) {
        self.i32(p.0);
        self.i32(p.1);
        self.i32(p.2);
    }
    /// A string of at most `u16::MAX` bytes (longer is cut at a char boundary).
    pub(crate) fn str(&mut self, s: &str) {
        let mut end = s.len().min(u16::MAX as usize);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        self.u16(end as u16);
        self.0.extend_from_slice(&s.as_bytes()[..end]);
    }
    pub(crate) fn opt_str(&mut self, s: Option<&str>) {
        match s {
            None => self.u8(0),
            Some(s) => {
                self.u8(1);
                self.str(s);
            }
        }
    }
}

pub(crate) struct Reader<'a>(pub(crate) &'a [u8]);

impl Reader<'_> {
    pub(crate) fn take(&mut self, n: usize) -> Result<&[u8], DecodeError> {
        if self.0.len() < n {
            return Err(DecodeError::Truncated);
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }
    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        Ok(self.take(N)?.try_into().expect("sized"))
    }
    pub(crate) fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    pub(crate) fn bool(&mut self) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::Malformed("bool")),
        }
    }
    pub(crate) fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    pub(crate) fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    pub(crate) fn i32(&mut self) -> Result<i32, DecodeError> {
        Ok(i32::from_le_bytes(self.array()?))
    }
    pub(crate) fn i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_le_bytes(self.array()?))
    }
    pub(crate) fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_le_bytes(self.array()?))
    }
    pub(crate) fn f64(&mut self) -> Result<f64, DecodeError> {
        Ok(f64::from_le_bytes(self.array()?))
    }
    pub(crate) fn pos(&mut self) -> Result<BlockPos, DecodeError> {
        Ok((self.i32()?, self.i32()?, self.i32()?))
    }
    pub(crate) fn str(&mut self) -> Result<String, DecodeError> {
        let len = self.u16()? as usize;
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| DecodeError::Malformed("utf-8"))
    }
    pub(crate) fn opt_str(&mut self) -> Result<Option<String>, DecodeError> {
        Ok(if self.bool()? {
            Some(self.str()?)
        } else {
            None
        })
    }
    pub(crate) fn done(&self) -> Result<(), DecodeError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(DecodeError::Malformed("trailing bytes"))
        }
    }
}

/// A block as text: `minecraft:oak_stairs[facing=east,half=bottom]`.
pub fn block_text(block: &Block) -> String {
    let mut text = block.id.key();
    if !block.properties.is_empty() {
        let props: Vec<String> = block
            .properties
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        text.push('[');
        text.push_str(&props.join(","));
        text.push(']');
    }
    text
}

/// The block `block_text` wrote.
pub fn parse_block(text: &str) -> Option<Block> {
    let (id, props) = match text.split_once('[') {
        Some((id, rest)) => (id, Some(rest.strip_suffix(']')?)),
        None => (text, None),
    };
    let mut block = Block {
        id: ResourceId::parse(id).ok()?,
        properties: BTreeMap::new(),
    };
    for pair in props
        .into_iter()
        .flat_map(|p| p.split(','))
        .filter(|p| !p.is_empty())
    {
        let (k, v) = pair.split_once('=')?;
        block.properties.insert(k.to_owned(), v.to_owned());
    }
    Some(block)
}

/// A slot of the inventory screen, as `frame::McSlot`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotRef {
    Inventory(u8),
    Crafting(u8),
    Result,
}

/// A click on the inventory screen, as `frame::McClick`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Click {
    Slot {
        slot: SlotRef,
        right: bool,
        shift: bool,
    },
    Outside {
        right: bool,
    },
    Gather {
        right: bool,
    },
    Swap {
        slot: SlotRef,
        hotbar: u8,
    },
    Spread {
        slots: Vec<u8>,
        right: bool,
    },
    Close,
}

/// What a client asks the host to do in the world or its inventory.
#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    /// The client finished breaking this block by hand.
    Break {
        pos: BlockPos,
    },
    /// A right click with the held stack, from where the client stood and
    /// looked (block space and Minecraft degrees): the host places as the
    /// client did.
    Place {
        feet: [f64; 3],
        yaw: f32,
        pitch: f32,
        selected: u8,
    },
    Click(Click),
    Select {
        slot: u8,
    },
    Drop {
        whole: bool,
    },
    /// The client opened the crafting table at this block.
    OpenTable {
        pos: BlockPos,
    },
    /// The console's `mc give`.
    Give {
        id: String,
        count: u8,
    },
    /// The console's `mc clear`.
    Clear,
    /// The console's block box, in world blocks; no block is air.
    Fill {
        a: BlockPos,
        b: BlockPos,
        block: Option<String>,
    },
    /// A punch on the mob with this key (`minecraft_entities`' bullet
    /// key), which the client saw within its reach.
    Attack {
        key: u64,
    },
    /// The console's `mc summon`: a mob of this kind at this block point.
    Summon {
        kind: String,
        at: [f64; 3],
    },
    /// The console's `admin transfer`: a player's kept record (by its old
    /// key, or by the name it played under) moved to a new key.
    Transfer {
        from: TransferFrom,
        to: [u8; 32],
    },
}

/// Whose record an `admin transfer` moves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransferFrom {
    Key([u8; 32]),
    Name(String),
}

/// A fingerprint's 32 bytes, from its 64 hex digits.
pub fn fingerprint_bytes(fingerprint: &str) -> Option<[u8; 32]> {
    if !is_fingerprint(fingerprint) {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&fingerprint[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

/// 32 bytes as a fingerprint.
pub fn fingerprint_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn encode_slot(w: &mut Writer, slot: SlotRef) {
    match slot {
        SlotRef::Inventory(n) => {
            w.u8(0);
            w.u8(n);
        }
        SlotRef::Crafting(n) => {
            w.u8(1);
            w.u8(n);
        }
        SlotRef::Result => w.u8(2),
    }
}

fn decode_slot(r: &mut Reader<'_>) -> Result<SlotRef, DecodeError> {
    Ok(match r.u8()? {
        0 => SlotRef::Inventory(r.u8()?),
        1 => SlotRef::Crafting(r.u8()?),
        2 => SlotRef::Result,
        _ => return Err(DecodeError::Malformed("slot")),
    })
}

impl Intent {
    /// The console command behind an intent that changes the world or an
    /// inventory beyond what play does (`mc give`, `mc clear`, `mc fill` or
    /// `mc set`, `mc summon`), or `None` for one a player's own play makes.
    /// A host applies these only for the players on its admin list.
    pub fn debug_name(&self) -> Option<&'static str> {
        match self {
            Self::Give { .. } => Some("mc give"),
            Self::Clear => Some("mc clear"),
            Self::Fill { .. } => Some("mc fill"),
            Self::Summon { .. } => Some("mc summon"),
            Self::Transfer { .. } => Some("admin transfer"),
            Self::Break { .. }
            | Self::Place { .. }
            | Self::Click(_)
            | Self::Select { .. }
            | Self::Drop { .. }
            | Self::OpenTable { .. }
            | Self::Attack { .. } => None,
        }
    }

    /// The intent as at most `INTENT_BYTES` bytes, or `None` when an id or
    /// a spread is too long to send.
    pub fn encode(&self) -> Option<Vec<u8>> {
        let mut w = Writer::default();
        let id_fits = |id: &str| id.len() <= MAX_ID_BYTES;
        match self {
            Self::Break { pos } => {
                w.u8(1);
                w.pos(*pos);
            }
            Self::Place {
                feet,
                yaw,
                pitch,
                selected,
            } => {
                w.u8(2);
                for v in feet {
                    w.f64(*v);
                }
                w.f32(*yaw);
                w.f32(*pitch);
                w.u8(*selected);
            }
            Self::Click(click) => {
                w.u8(3);
                match click {
                    Click::Slot { slot, right, shift } => {
                        w.u8(0);
                        encode_slot(&mut w, *slot);
                        w.u8(u8::from(*right));
                        w.u8(u8::from(*shift));
                    }
                    Click::Outside { right } => {
                        w.u8(1);
                        w.u8(u8::from(*right));
                    }
                    Click::Gather { right } => {
                        w.u8(2);
                        w.u8(u8::from(*right));
                    }
                    Click::Swap { slot, hotbar } => {
                        w.u8(3);
                        encode_slot(&mut w, *slot);
                        w.u8(*hotbar);
                    }
                    Click::Spread { slots, right } => {
                        if slots.len() > INTENT_BYTES - 5 {
                            return None;
                        }
                        w.u8(4);
                        w.u8(u8::from(*right));
                        w.u8(slots.len() as u8);
                        w.0.extend_from_slice(slots);
                    }
                    Click::Close => w.u8(5),
                }
            }
            Self::Select { slot } => {
                w.u8(4);
                w.u8(*slot);
            }
            Self::Drop { whole } => {
                w.u8(5);
                w.u8(u8::from(*whole));
            }
            Self::OpenTable { pos } => {
                w.u8(6);
                w.pos(*pos);
            }
            Self::Give { id, count } => {
                if !id_fits(id) {
                    return None;
                }
                w.u8(7);
                w.str(id);
                w.u8(*count);
            }
            Self::Clear => w.u8(8),
            Self::Fill { a, b, block } => {
                if block.as_deref().is_some_and(|id| !id_fits(id)) {
                    return None;
                }
                w.u8(9);
                w.pos(*a);
                w.pos(*b);
                w.opt_str(block.as_deref());
            }
            Self::Attack { key } => {
                w.u8(10);
                w.0.extend_from_slice(&key.to_le_bytes());
            }
            Self::Summon { kind, at } => {
                if !id_fits(kind) {
                    return None;
                }
                w.u8(11);
                w.str(kind);
                for v in at {
                    w.f64(*v);
                }
            }
            Self::Transfer { from, to } => {
                w.u8(12);
                match from {
                    TransferFrom::Key(key) => {
                        w.u8(0);
                        w.0.extend_from_slice(key);
                    }
                    TransferFrom::Name(name) => {
                        if !id_fits(name) {
                            return None;
                        }
                        w.u8(1);
                        w.str(name);
                    }
                }
                w.0.extend_from_slice(to);
            }
        }
        (w.0.len() <= INTENT_BYTES).then_some(w.0)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader(bytes);
        let intent = match r.u8()? {
            1 => Self::Break { pos: r.pos()? },
            2 => Self::Place {
                feet: [r.f64()?, r.f64()?, r.f64()?],
                yaw: r.f32()?,
                pitch: r.f32()?,
                selected: r.u8()?,
            },
            3 => Self::Click(match r.u8()? {
                0 => Click::Slot {
                    slot: decode_slot(&mut r)?,
                    right: r.bool()?,
                    shift: r.bool()?,
                },
                1 => Click::Outside { right: r.bool()? },
                2 => Click::Gather { right: r.bool()? },
                3 => Click::Swap {
                    slot: decode_slot(&mut r)?,
                    hotbar: r.u8()?,
                },
                4 => {
                    let right = r.bool()?;
                    let n = r.u8()? as usize;
                    Click::Spread {
                        slots: r.take(n)?.to_vec(),
                        right,
                    }
                }
                5 => Click::Close,
                _ => return Err(DecodeError::Malformed("click")),
            }),
            4 => Self::Select { slot: r.u8()? },
            5 => Self::Drop { whole: r.bool()? },
            6 => Self::OpenTable { pos: r.pos()? },
            7 => Self::Give {
                id: r.str()?,
                count: r.u8()?,
            },
            8 => Self::Clear,
            9 => Self::Fill {
                a: r.pos()?,
                b: r.pos()?,
                block: r.opt_str()?,
            },
            10 => Self::Attack {
                key: u64::from_le_bytes(r.array()?),
            },
            11 => Self::Summon {
                kind: r.str()?,
                at: [r.f64()?, r.f64()?, r.f64()?],
            },
            12 => Self::Transfer {
                from: match r.u8()? {
                    0 => TransferFrom::Key(r.array()?),
                    1 => TransferFrom::Name(r.str()?),
                    _ => return Err(DecodeError::Malformed("transfer")),
                },
                to: r.array()?,
            },
            _ => return Err(DecodeError::Malformed("intent")),
        };
        r.done()?;
        Ok(intent)
    }
}

/// One stack as it travels and is saved; `components` is its JSON.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WireStack {
    pub id: String,
    pub count: u8,
    pub max: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub components: Option<String>,
}

impl WireStack {
    pub fn from_stack(stack: &ItemStack) -> Self {
        Self {
            id: stack.id.clone(),
            count: stack.count,
            max: stack.max,
            components: stack.components.as_ref().map(|c| c.to_string()),
        }
    }

    pub fn to_stack(&self) -> ItemStack {
        let mut stack = ItemStack::new(self.id.clone(), self.count);
        stack.max = self.max;
        stack.components = self
            .components
            .as_deref()
            .and_then(|c| serde_json::from_str(c).ok());
        stack
    }
}

fn encode_stack(w: &mut Writer, stack: &Option<WireStack>) {
    match stack {
        None => w.u8(0),
        Some(s) => {
            w.u8(1);
            w.str(&s.id);
            w.u8(s.count);
            w.u8(s.max);
            w.opt_str(s.components.as_deref());
        }
    }
}

fn decode_stack(r: &mut Reader<'_>) -> Result<Option<WireStack>, DecodeError> {
    Ok(if r.bool()? {
        Some(WireStack {
            id: r.str()?,
            count: r.u8()?,
            max: r.u8()?,
            components: r.opt_str()?,
        })
    } else {
        None
    })
}

/// A player's inventory: every slot, both crafting grids and the cursor.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InventoryState {
    pub selected: u8,
    pub slots: Vec<Option<WireStack>>,
    pub crafting: Vec<Option<WireStack>>,
    pub workbench: Vec<Option<WireStack>>,
    #[serde(default)]
    pub cursor: Option<WireStack>,
}

impl InventoryState {
    pub fn capture(inventory: &Inventory, selected: usize) -> Self {
        let wire = |s: &Option<ItemStack>| s.as_ref().map(WireStack::from_stack);
        Self {
            selected: selected.min(u8::MAX as usize) as u8,
            slots: inventory.slots.iter().map(wire).collect(),
            crafting: inventory.crafting.iter().map(wire).collect(),
            workbench: inventory.workbench.iter().map(wire).collect(),
            cursor: wire(&inventory.cursor),
        }
    }

    /// Puts these stacks into `inventory` (its recipes stay); returns the
    /// selected slot.
    pub fn restore(&self, inventory: &mut Inventory) -> usize {
        let stack = |s: &Option<WireStack>| s.as_ref().map(WireStack::to_stack);
        for (i, slot) in inventory.slots.iter_mut().enumerate() {
            *slot = self.slots.get(i).and_then(stack);
        }
        for (i, slot) in inventory.crafting.iter_mut().enumerate() {
            *slot = self.crafting.get(i).and_then(stack);
        }
        for (i, slot) in inventory.workbench.iter_mut().enumerate() {
            *slot = self.workbench.get(i).and_then(stack);
        }
        inventory.cursor = stack(&self.cursor);
        usize::from(self.selected)
    }

    /// This inventory without the stacks `drop` matches (the MW2 guns,
    /// which a match gives anew).
    pub fn without(mut self, drop: impl Fn(&WireStack) -> bool) -> Self {
        let keep = |s: &mut Option<WireStack>| {
            if s.as_ref().is_some_and(&drop) {
                *s = None;
            }
        };
        self.slots.iter_mut().for_each(keep);
        self.crafting.iter_mut().for_each(keep);
        self.workbench.iter_mut().for_each(keep);
        keep(&mut self.cursor);
        self
    }
}

/// An item entity on the ground, as clients draw it.
#[derive(Clone, Debug, PartialEq)]
pub struct WireItem {
    pub id: u32,
    pub stack: WireStack,
    pub position: [f64; 3],
    pub velocity: [f32; 3],
    pub age: u32,
    pub on_ground: bool,
}

/// What the host tells a client.
#[derive(Clone, Debug, PartialEq)]
pub enum HostMessage {
    /// The world to generate: its seed, and the block at map origin.
    World { seed: i64, origin: [f64; 3] },
    /// Block changes in the order the host made them; `None` is air.
    Edits(Vec<(BlockPos, Option<String>)>),
    /// The client's own inventory after the host applied its intents up to
    /// `applied` (a request id), and the crafting table it has open.
    Inventory {
        applied: u32,
        table: Option<BlockPos>,
        state: InventoryState,
    },
    /// The item entities near the client.
    Items(Vec<WireItem>),
    /// The host moved the client here (map units) for its arrival in the
    /// world.
    Placed { at: [f32; 3] },
    /// Mobs that came into the client's view, or whose looks changed.
    Mobs(Vec<crate::mob_sync::MobSpawn>),
    /// Mobs that left the client's view or the level, by id.
    MobsGone(Vec<u64>),
    /// Pixels of a map in the client's inventory: its centre and scale, and
    /// 16 by 16 tiles of it by index (`filled_map::TILE`), the ones that
    /// changed since the client was last sent them.
    MapTiles {
        id: u32,
        centre: [i32; 2],
        scale: u8,
        tiles: Vec<(u8, Vec<u8>)>,
    },
}

impl HostMessage {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        match self {
            Self::World { seed, origin } => {
                w.u8(1);
                w.i64(*seed);
                for v in origin {
                    w.f64(*v);
                }
            }
            Self::Edits(edits) => {
                w.u8(2);
                w.u32(edits.len() as u32);
                for (pos, block) in edits {
                    w.pos(*pos);
                    w.opt_str(block.as_deref());
                }
            }
            Self::Inventory {
                applied,
                table,
                state,
            } => {
                w.u8(3);
                w.u32(*applied);
                match table {
                    None => w.u8(0),
                    Some(pos) => {
                        w.u8(1);
                        w.pos(*pos);
                    }
                }
                w.u8(state.selected);
                for list in [&state.slots, &state.crafting, &state.workbench] {
                    w.u8(list.len().min(u8::MAX as usize) as u8);
                    for stack in list.iter().take(u8::MAX as usize) {
                        encode_stack(&mut w, stack);
                    }
                }
                encode_stack(&mut w, &state.cursor);
            }
            Self::Items(items) => {
                w.u8(4);
                w.u16(items.len().min(u16::MAX as usize) as u16);
                for item in items.iter().take(u16::MAX as usize) {
                    w.u32(item.id);
                    encode_stack(&mut w, &Some(item.stack.clone()));
                    for v in item.position {
                        w.f64(v);
                    }
                    for v in item.velocity {
                        w.f32(v);
                    }
                    w.u32(item.age);
                    w.u8(u8::from(item.on_ground));
                }
            }
            Self::Placed { at } => {
                w.u8(5);
                for v in at {
                    w.f32(*v);
                }
            }
            Self::Mobs(spawns) => {
                w.u8(6);
                w.u16(spawns.len().min(u16::MAX as usize) as u16);
                for spawn in spawns.iter().take(u16::MAX as usize) {
                    crate::mob_sync::encode_spawn(&mut w, spawn);
                }
            }
            Self::MobsGone(ids) => {
                w.u8(7);
                crate::mob_sync::encode_ids(&mut w, ids);
            }
            Self::MapTiles {
                id,
                centre,
                scale,
                tiles,
            } => {
                w.u8(8);
                w.u32(*id);
                w.i32(centre[0]);
                w.i32(centre[1]);
                w.u8(*scale);
                w.u8(tiles.len().min(u8::MAX as usize) as u8);
                for (index, pixels) in tiles.iter().take(u8::MAX as usize) {
                    w.u8(*index);
                    let mut tile = [0u8; crate::filled_map::TILE * crate::filled_map::TILE];
                    let n = pixels.len().min(tile.len());
                    tile[..n].copy_from_slice(&pixels[..n]);
                    w.0.extend_from_slice(&tile);
                }
            }
        }
        w.0
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader(bytes);
        let message = match r.u8()? {
            1 => Self::World {
                seed: r.i64()?,
                origin: [r.f64()?, r.f64()?, r.f64()?],
            },
            2 => {
                let n = r.u32()? as usize;
                // Each edit is at least 13 bytes: a short count cannot claim more.
                if n > r.0.len() / 13 {
                    return Err(DecodeError::Truncated);
                }
                let mut edits = Vec::with_capacity(n);
                for _ in 0..n {
                    edits.push((r.pos()?, r.opt_str()?));
                }
                Self::Edits(edits)
            }
            3 => {
                let applied = r.u32()?;
                let table = if r.bool()? { Some(r.pos()?) } else { None };
                let selected = r.u8()?;
                let mut lists: [Vec<Option<WireStack>>; 3] = Default::default();
                for list in &mut lists {
                    let n = r.u8()?;
                    for _ in 0..n {
                        list.push(decode_stack(&mut r)?);
                    }
                }
                let cursor = decode_stack(&mut r)?;
                let [slots, crafting, workbench] = lists;
                Self::Inventory {
                    applied,
                    table,
                    state: InventoryState {
                        selected,
                        slots,
                        crafting,
                        workbench,
                        cursor,
                    },
                }
            }
            4 => {
                let n = r.u16()? as usize;
                let mut items = Vec::with_capacity(n.min(MAX_ITEMS));
                for _ in 0..n {
                    let id = r.u32()?;
                    let stack = decode_stack(&mut r)?
                        .ok_or(DecodeError::Malformed("item without stack"))?;
                    items.push(WireItem {
                        id,
                        stack,
                        position: [r.f64()?, r.f64()?, r.f64()?],
                        velocity: [r.f32()?, r.f32()?, r.f32()?],
                        age: r.u32()?,
                        on_ground: r.bool()?,
                    });
                }
                Self::Items(items)
            }
            5 => Self::Placed {
                at: [r.f32()?, r.f32()?, r.f32()?],
            },
            6 => {
                let n = r.u16()? as usize;
                // Each spawn is at least 39 bytes: a short count cannot claim more.
                if n > r.0.len() / 39 {
                    return Err(DecodeError::Truncated);
                }
                let mut spawns = Vec::with_capacity(n);
                for _ in 0..n {
                    spawns.push(crate::mob_sync::decode_spawn(&mut r)?);
                }
                Self::Mobs(spawns)
            }
            7 => Self::MobsGone(crate::mob_sync::decode_ids(&mut r)?),
            8 => {
                use crate::filled_map::{TILE, TILES};
                let id = r.u32()?;
                let centre = [r.i32()?, r.i32()?];
                let scale = r.u8()?;
                let n = r.u8()? as usize;
                let mut tiles = Vec::with_capacity(n);
                for _ in 0..n {
                    let index = r.u8()?;
                    if usize::from(index) >= TILES * TILES {
                        return Err(DecodeError::Malformed("map tile"));
                    }
                    tiles.push((index, r.take(TILE * TILE)?.to_vec()));
                }
                Self::MapTiles {
                    id,
                    centre,
                    scale,
                    tiles,
                }
            }
            _ => return Err(DecodeError::Malformed("host message")),
        };
        r.done()?;
        Ok(message)
    }
}

/// How many of `edits`, from the first, one `HostMessage::Edits` takes: at
/// most `EDITS_PER_MESSAGE` and `EDITS_BYTES_PER_MESSAGE`, and at least one.
pub fn edits_that_fit(edits: &[(BlockPos, Option<String>)]) -> usize {
    let mut bytes = 0;
    let mut count = 0;
    for (pos, block) in edits.iter().take(EDITS_PER_MESSAGE) {
        let mut w = Writer::default();
        w.pos(*pos);
        w.opt_str(block.as_deref());
        bytes += w.0.len();
        if count > 0 && bytes > EDITS_BYTES_PER_MESSAGE {
            break;
        }
        count += 1;
    }
    count
}

/// Every block the host changed, in order. Each client reads it from its own
/// cursor, so one that joins late is sent the whole history first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditLog {
    entries: Vec<(BlockPos, Option<String>)>,
}

impl EditLog {
    pub fn push(&mut self, pos: BlockPos, block: Option<String>) {
        self.entries.push((pos, block));
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[(BlockPos, Option<String>)] {
        &self.entries
    }

    /// The changes from `from` that fit one `HostMessage::Edits`, and the
    /// cursor after them.
    pub fn batch(&self, from: usize) -> (Vec<(BlockPos, Option<String>)>, usize) {
        let from = from.min(self.entries.len());
        let to = from + edits_that_fit(&self.entries[from..]);
        (self.entries[from..to].to_vec(), to)
    }

    /// The same world with one change per position: the last, in the order
    /// the last changes were made.
    pub fn compacted(&self) -> Self {
        let mut last: HashMap<BlockPos, usize> = HashMap::new();
        for (i, (pos, _)) in self.entries.iter().enumerate() {
            last.insert(*pos, i);
        }
        let entries = self
            .entries
            .iter()
            .enumerate()
            .filter(|(i, (pos, _))| last.get(pos) == Some(i))
            .map(|(_, e)| e.clone())
            .collect();
        Self { entries }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.0.extend_from_slice(EDITS_MAGIC);
        w.u32(SAVE_VERSION);
        w.u32(self.entries.len() as u32);
        for (pos, block) in &self.entries {
            w.pos(*pos);
            w.opt_str(block.as_deref());
        }
        w.0
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader(bytes);
        if r.take(4)? != EDITS_MAGIC {
            return Err(DecodeError::Malformed("edits magic"));
        }
        if r.u32()? != SAVE_VERSION {
            return Err(DecodeError::Malformed("edits version"));
        }
        let n = r.u32()? as usize;
        if n > r.0.len() / 13 {
            return Err(DecodeError::Truncated);
        }
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            entries.push((r.pos()?, r.opt_str()?));
        }
        r.done()?;
        Ok(Self { entries })
    }
}

/// `level.json`: what a saved world is.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Level {
    pub version: u32,
    pub seed: i64,
    /// The block at map origin, so the world sits where it sat.
    pub origin: [f64; 3],
}

/// A world a host keeps: `level.json`, `edits.bin` and `players.json` in its
/// directory, beside the region files its chunks save to.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedWorld {
    pub level: Level,
    pub edits: EditLog,
    /// Each player by its key (`players::Players`).
    pub players: Players,
    /// Its maps (`maps.bin`, none in a world from before maps).
    pub maps: crate::filled_map::MapStore,
}

/// A world name as a directory name: letters, digits, `-`, `_` and `.`
/// (not leading), at most 64 bytes; anything else becomes `_`.
pub fn sanitize_world_name(name: &str) -> String {
    let mut out: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    while out.starts_with('.') {
        out.replace_range(0..1, "_");
    }
    if out.is_empty() {
        out.push_str("world");
    }
    out
}

/// `<worlds>/<name>`.
pub fn world_dir(worlds: &Path, name: &str) -> PathBuf {
    worlds.join(sanitize_world_name(name))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
}

fn io_error(error: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
}

impl SavedWorld {
    pub fn new(seed: i64, origin: [f64; 3]) -> Self {
        Self {
            level: Level {
                version: SAVE_VERSION,
                seed,
                origin,
            },
            edits: EditLog::default(),
            players: Players::default(),
            maps: crate::filled_map::MapStore::default(),
        }
    }

    /// The world saved in `dir`, or `None` when there is none yet.
    pub fn load(dir: &Path) -> std::io::Result<Option<Self>> {
        let level = match std::fs::read(dir.join(LEVEL_FILE)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let level: Level = serde_json::from_slice(&level).map_err(io_error)?;
        if level.version != SAVE_VERSION {
            return Err(io_error(format!(
                "level.json version {} (this game reads {SAVE_VERSION})",
                level.version
            )));
        }
        let edits = match std::fs::read(dir.join(EDITS_FILE)) {
            Ok(bytes) => EditLog::decode(&bytes).map_err(io_error)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => EditLog::default(),
            Err(e) => return Err(e),
        };
        let players = match std::fs::read(dir.join(PLAYERS_FILE)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io_error)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Players::default(),
            Err(e) => return Err(e),
        };
        let maps = match std::fs::read(dir.join(MAPS_FILE)) {
            Ok(bytes) => crate::filled_map::MapStore::decode(&bytes).map_err(io_error)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => crate::filled_map::MapStore::default(),
            Err(e) => return Err(e),
        };
        Ok(Some(Self {
            level,
            edits,
            players,
            maps,
        }))
    }

    /// Writes its files, each replaced whole; the edits compacted.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let level = serde_json::to_vec_pretty(&self.level).map_err(io_error)?;
        write_atomic(&dir.join(LEVEL_FILE), &level)?;
        write_atomic(&dir.join(EDITS_FILE), &self.edits.compacted().encode())?;
        let players = serde_json::to_vec_pretty(&self.players).map_err(io_error)?;
        write_atomic(&dir.join(PLAYERS_FILE), &players)?;
        write_atomic(&dir.join(MAPS_FILE), &self.maps.encode())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_intent() -> Vec<Intent> {
        vec![
            Intent::Break {
                pos: (-3, 70, 1_000_000),
            },
            Intent::Place {
                feet: [0.5, 64.0, -12.25],
                yaw: 91.5,
                pitch: -30.0,
                selected: 8,
            },
            Intent::Click(Click::Slot {
                slot: SlotRef::Inventory(35),
                right: true,
                shift: false,
            }),
            Intent::Click(Click::Slot {
                slot: SlotRef::Crafting(8),
                right: false,
                shift: true,
            }),
            Intent::Click(Click::Slot {
                slot: SlotRef::Result,
                right: false,
                shift: false,
            }),
            Intent::Click(Click::Outside { right: true }),
            Intent::Click(Click::Gather { right: false }),
            Intent::Click(Click::Swap {
                slot: SlotRef::Inventory(20),
                hotbar: 3,
            }),
            Intent::Click(Click::Spread {
                slots: vec![1, 2, 3, 40],
                right: true,
            }),
            Intent::Click(Click::Close),
            Intent::Select { slot: 4 },
            Intent::Drop { whole: true },
            Intent::OpenTable { pos: (3, 0, 0) },
            Intent::Give {
                id: "minecraft:oak_planks".into(),
                count: 16,
            },
            Intent::Clear,
            Intent::Fill {
                a: (-1, 0, -2),
                b: (4, 3, 2),
                block: None,
            },
            Intent::Fill {
                a: (0, 0, 0),
                b: (0, 0, 0),
                block: Some("minecraft:stone".into()),
            },
            Intent::Attack {
                key: (3 << 56) | 77,
            },
            Intent::Summon {
                kind: "minecraft:husk".into(),
                at: [320.5, 61.0, -2.25],
            },
            Intent::Transfer {
                from: TransferFrom::Key([0xab; 32]),
                to: [0xcd; 32],
            },
            Intent::Transfer {
                from: TransferFrom::Name("a_fifteen_bytes".into()),
                to: [0x01; 32],
            },
        ]
    }

    #[test]
    fn every_intent_round_trips_within_its_size() {
        for intent in every_intent() {
            let bytes = intent.encode().expect("fits");
            assert!(
                bytes.len() <= INTENT_BYTES,
                "{intent:?}: {} bytes",
                bytes.len()
            );
            assert_eq!(Intent::decode(&bytes).unwrap(), intent);
        }
    }

    #[test]
    fn fingerprints_travel_as_their_bytes() {
        let fingerprint = "0123456789abcdef".repeat(4);
        let bytes = fingerprint_bytes(&fingerprint).unwrap();
        assert_eq!(fingerprint_hex(&bytes), fingerprint);
        assert_eq!(fingerprint_bytes("0123"), None);
        assert_eq!(fingerprint_bytes(&"G".repeat(64)), None);
    }

    #[test]
    fn only_the_console_cheats_need_an_admin() {
        let debug: Vec<&str> = every_intent()
            .iter()
            .filter_map(Intent::debug_name)
            .collect();
        assert_eq!(
            debug,
            [
                "mc give",
                "mc clear",
                "mc fill",
                "mc fill",
                "mc summon",
                "admin transfer",
                "admin transfer"
            ]
        );
        // What play makes never does: mining, placing, clicks, punches.
        for intent in every_intent() {
            let play = matches!(
                intent,
                Intent::Break { .. }
                    | Intent::Place { .. }
                    | Intent::Click(_)
                    | Intent::Select { .. }
                    | Intent::Drop { .. }
                    | Intent::OpenTable { .. }
                    | Intent::Attack { .. }
            );
            assert_eq!(intent.debug_name().is_none(), play, "{intent:?}");
        }
    }

    #[test]
    fn an_intent_too_long_to_send_is_refused() {
        let id = format!("minecraft:{}", "a".repeat(MAX_ID_BYTES));
        assert_eq!(
            Intent::Give {
                id: id.clone(),
                count: 1
            }
            .encode(),
            None
        );
        assert_eq!(
            Intent::Fill {
                a: (0, 0, 0),
                b: (0, 0, 0),
                block: Some(id)
            }
            .encode(),
            None
        );
        let slots = vec![0; INTENT_BYTES];
        assert_eq!(
            Intent::Click(Click::Spread {
                slots,
                right: false
            })
            .encode(),
            None
        );
    }

    #[test]
    fn a_damaged_intent_is_an_error() {
        let bytes = Intent::Break { pos: (1, 2, 3) }.encode().unwrap();
        assert_eq!(
            Intent::decode(&bytes[..bytes.len() - 1]),
            Err(DecodeError::Truncated)
        );
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(matches!(
            Intent::decode(&longer),
            Err(DecodeError::Malformed(_))
        ));
        assert!(matches!(
            Intent::decode(&[200]),
            Err(DecodeError::Malformed(_))
        ));
        assert_eq!(Intent::decode(&[]), Err(DecodeError::Truncated));
    }

    fn inventory_state() -> InventoryState {
        let stack = |id: &str, count| {
            Some(WireStack {
                id: id.into(),
                count,
                max: 64,
                components: None,
            })
        };
        let mut slots = vec![None; 43];
        slots[0] = Some(WireStack {
            id: "iw4:weapon/7".into(),
            count: 1,
            max: 1,
            components: None,
        });
        slots[5] = stack("minecraft:oak_planks", 12);
        slots[20] = Some(WireStack {
            id: "minecraft:diamond_pickaxe".into(),
            count: 1,
            max: 1,
            components: Some(r#"{"minecraft:damage":3}"#.into()),
        });
        let mut workbench = vec![None; 9];
        workbench[4] = stack("minecraft:stick", 2);
        InventoryState {
            selected: 5,
            slots,
            crafting: vec![None; 4],
            workbench,
            cursor: stack("minecraft:dirt", 1),
        }
    }

    #[test]
    fn every_host_message_round_trips() {
        let messages = vec![
            HostMessage::World {
                seed: -4_172_144_997_902_289_642,
                origin: [98.5, 134.0, -22.5],
            },
            HostMessage::Edits(vec![
                ((1, 64, -1), Some("minecraft:stone".into())),
                ((1, 65, -1), None),
                (
                    (2, 64, 9),
                    Some("minecraft:oak_stairs[facing=east,half=bottom]".into()),
                ),
            ]),
            HostMessage::Edits(Vec::new()),
            HostMessage::Inventory {
                applied: 41,
                table: Some((3, 0, 0)),
                state: inventory_state(),
            },
            HostMessage::Inventory {
                applied: 0,
                table: None,
                state: InventoryState::default(),
            },
            HostMessage::Items(vec![WireItem {
                id: 9,
                stack: WireStack {
                    id: "minecraft:cobblestone".into(),
                    count: 3,
                    max: 64,
                    components: None,
                },
                position: [1.5, 64.2, -3.25],
                velocity: [0.0, -0.04, 0.1],
                age: 120,
                on_ground: true,
            }]),
            HostMessage::Placed {
                at: [0.0, -108.0, 36.5],
            },
            HostMessage::Mobs(vec![crate::mob_sync::MobSpawn {
                id: 1234,
                look: crate::mob_sync::Look::Zombie {
                    kind: 2,
                    main_hand: Some("minecraft:iron_shovel".into()),
                    armor: [None, Some("minecraft:chainmail_chestplate".into()), None, None],
                    villager: None,
                },
                baby: true,
                width: 0.3,
                height: 0.975,
                position: [320.5, 61.0, -2.25],
                yaw: -45.0,
            }]),
            HostMessage::Mobs(Vec::new()),
            HostMessage::MobsGone(vec![1, 1234, 1 << 40]),
            HostMessage::MapTiles {
                id: 3,
                centre: [128, -256],
                scale: 0,
                tiles: vec![(0, vec![45; 256]), (63, (0..=255).collect())],
            },
            HostMessage::MapTiles {
                id: u32::MAX,
                centre: [0, 0],
                scale: 4,
                tiles: Vec::new(),
            },
        ];
        for message in messages {
            assert_eq!(HostMessage::decode(&message.encode()).unwrap(), message);
        }
    }

    #[test]
    fn a_host_message_claiming_more_edits_than_it_holds_is_refused() {
        let mut bytes = HostMessage::Edits(vec![((0, 0, 0), None)]).encode();
        bytes[1..5].copy_from_slice(&1_000_000u32.to_le_bytes());
        assert_eq!(HostMessage::decode(&bytes), Err(DecodeError::Truncated));
    }

    #[test]
    fn a_host_message_claiming_more_mobs_than_it_holds_is_refused() {
        let mut bytes = HostMessage::Mobs(Vec::new()).encode();
        bytes[1..3].copy_from_slice(&500u16.to_le_bytes());
        assert_eq!(HostMessage::decode(&bytes), Err(DecodeError::Truncated));
    }

    #[test]
    fn blocks_read_back_from_their_text() {
        let block = Block::new("minecraft:oak_stairs")
            .with("facing", "east")
            .with("half", "bottom");
        assert_eq!(
            block_text(&block),
            "minecraft:oak_stairs[facing=east,half=bottom]"
        );
        assert_eq!(parse_block(&block_text(&block)), Some(block));
        assert_eq!(parse_block("stone"), Some(Block::new("minecraft:stone")));
        assert_eq!(parse_block("minecraft:stone[facing"), None);
        assert_eq!(parse_block("minecraft:stone[facing]"), None);
    }

    #[test]
    fn a_late_joiner_reads_the_whole_log_in_batches() {
        let mut log = EditLog::default();
        for i in 0..(EDITS_PER_MESSAGE as i32 * 2 + 10) {
            log.push((i, 64, 0), Some("minecraft:stone".into()));
        }
        let mut cursor = 0;
        let mut seen = Vec::new();
        while cursor < log.len() {
            let (batch, next) = log.batch(cursor);
            assert!(!batch.is_empty() && batch.len() <= EDITS_PER_MESSAGE);
            seen.extend(batch);
            cursor = next;
        }
        assert_eq!(seen, log.entries());
        let (rest, at) = log.batch(cursor);
        assert!(rest.is_empty());
        assert_eq!(at, log.len());
        // A live edit after the joiner caught up is the next batch.
        log.push((0, 70, 0), None);
        assert_eq!(log.batch(cursor), (vec![((0, 70, 0), None)], log.len()));
    }

    #[test]
    fn a_long_log_of_long_block_ids_goes_in_small_rows() {
        let long = "minecraft:oak_stairs[facing=east,half=bottom,shape=straight,waterlogged=false]";
        let mut log = EditLog::default();
        for i in 0..20_000 {
            log.push((i, 64, -i), (i % 3 != 0).then(|| long.to_owned()));
        }
        let mut cursor = 0;
        let mut seen = Vec::new();
        while cursor < log.len() {
            let (batch, next) = log.batch(cursor);
            assert!(!batch.is_empty() && batch.len() <= EDITS_PER_MESSAGE);
            let row = HostMessage::Edits(batch.clone()).encode();
            assert!(row.len() <= EDITS_BYTES_PER_MESSAGE + 5, "{}", row.len());
            seen.extend(batch);
            cursor = next;
        }
        assert_eq!(seen, log.entries());
    }

    #[test]
    fn a_change_larger_than_a_row_still_goes_alone() {
        let huge = "x".repeat(EDITS_BYTES_PER_MESSAGE * 2);
        let edits = vec![((0, 0, 0), Some(huge)), ((1, 0, 0), None)];
        assert_eq!(edits_that_fit(&edits), 1);
        assert_eq!(edits_that_fit(&edits[1..]), 1);
        assert_eq!(edits_that_fit(&[]), 0);
    }

    #[test]
    fn compaction_keeps_the_last_change_of_each_block() {
        let mut log = EditLog::default();
        log.push((0, 0, 0), Some("minecraft:stone".into()));
        log.push((1, 0, 0), Some("minecraft:dirt".into()));
        log.push((0, 0, 0), None);
        log.push((2, 0, 0), Some("minecraft:glass".into()));
        log.push((1, 0, 0), Some("minecraft:oak_planks".into()));
        assert_eq!(
            log.compacted().entries(),
            [
                ((0, 0, 0), None),
                ((2, 0, 0), Some("minecraft:glass".into())),
                ((1, 0, 0), Some("minecraft:oak_planks".into())),
            ]
        );
    }

    #[test]
    fn the_edit_file_round_trips_and_refuses_damage() {
        let mut log = EditLog::default();
        log.push(
            (-5, -64, 7),
            Some("minecraft:chest[facing=north,type=single,waterlogged=false]".into()),
        );
        log.push((-5, -63, 7), None);
        let bytes = log.encode();
        assert_eq!(EditLog::decode(&bytes).unwrap(), log);
        assert!(EditLog::decode(&bytes[..bytes.len() - 2]).is_err());
        let mut wrong = bytes.clone();
        wrong[0] = b'X';
        assert_eq!(
            EditLog::decode(&wrong),
            Err(DecodeError::Malformed("edits magic"))
        );
    }

    #[test]
    fn a_saved_world_loads_back_with_each_players_inventory() {
        let dir = crate::test_dir::tempdir().unwrap();
        let world = dir.path().join("world");
        assert_eq!(SavedWorld::load(&world).unwrap(), None);
        let mut saved = SavedWorld::new(12345, [98.5, 134.0, -22.5]);
        saved
            .edits
            .push((99, 134, -22), Some("minecraft:oak_planks".into()));
        saved
            .edits
            .push((99, 135, -22), Some("minecraft:stone".into()));
        saved.edits.push((99, 135, -22), None);
        saved
            .players
            .keep(&"a".repeat(64), "host", inventory_state(), 1);
        let mut other = InventoryState::default();
        other.slots = vec![None; 43];
        other.slots[3] = Some(WireStack {
            id: "minecraft:torch".into(),
            count: 9,
            max: 64,
            components: None,
        });
        saved
            .players
            .keep(&"b".repeat(64), "client", other.clone(), 2);
        saved.save(&world).unwrap();
        let loaded = SavedWorld::load(&world).unwrap().unwrap();
        assert_eq!(loaded.level, saved.level);
        assert_eq!(loaded.edits, saved.edits.compacted());
        assert_eq!(loaded.players, saved.players);
        let client = loaded.players.record(&"b".repeat(64)).unwrap();
        assert_eq!(client.inventory, Some(other));
        assert_eq!(client.name, "client");
        // Saving again replaces the files whole.
        let mut again = loaded.clone();
        again.players = Players::default();
        again.save(&world).unwrap();
        assert_eq!(
            SavedWorld::load(&world).unwrap().unwrap().players,
            Players::default()
        );
    }

    /// A world keeps its maps (`maps.bin`); one saved before maps has no
    /// such file and loads with none.
    #[test]
    fn a_saved_world_keeps_its_maps_and_an_older_one_loads_without() {
        let dir = crate::test_dir::tempdir().unwrap();
        let world = dir.path().join("world");
        let mut saved = SavedWorld::new(12345, [90.5, 130.0, -30.5]);
        let id = saved.maps.allocate();
        let mut map = crate::filled_map::MapData::new(90.5, -30.5, 0);
        map.set(10, 20, 45);
        saved.maps.maps.insert(id, map);
        saved.save(&world).unwrap();
        let loaded = SavedWorld::load(&world).unwrap().unwrap();
        assert_eq!(loaded.maps, saved.maps);
        std::fs::remove_file(world.join(MAPS_FILE)).unwrap();
        let older = SavedWorld::load(&world).unwrap().unwrap();
        assert_eq!(older.maps, crate::filled_map::MapStore::default());
        assert_eq!(older.players, saved.players);
    }

    #[test]
    fn a_map_tile_out_of_the_map_is_refused() {
        let mut bytes = HostMessage::MapTiles { id: 1, centre: [0, 0], scale: 0, tiles: vec![(0, vec![0; 256])] }.encode();
        bytes[15] = 64;
        assert_eq!(HostMessage::decode(&bytes), Err(DecodeError::Malformed("map tile")));
    }

    #[test]
    fn a_save_from_another_version_is_refused() {
        let dir = crate::test_dir::tempdir().unwrap();
        std::fs::write(
            dir.path().join(LEVEL_FILE),
            r#"{"version":99,"seed":1,"origin":[0.0,0.0,0.0]}"#,
        )
        .unwrap();
        assert!(SavedWorld::load(dir.path()).is_err());
    }

    #[test]
    fn inventories_survive_capture_and_restore() {
        let mut inventory = Inventory::default();
        inventory.slots[2] = Some(ItemStack::new("minecraft:stone", 33));
        inventory.workbench[0] = Some(ItemStack::new("minecraft:stick", 1));
        inventory.cursor = Some(ItemStack::new("minecraft:dirt", 2));
        let state = InventoryState::capture(&inventory, 2);
        let mut fresh = Inventory::default();
        assert_eq!(state.restore(&mut fresh), 2);
        assert_eq!(fresh.slots, inventory.slots);
        assert_eq!(fresh.workbench, inventory.workbench);
        assert_eq!(fresh.cursor, inventory.cursor);
        let no_guns = inventory_state().without(|s| s.id.starts_with("iw4:"));
        assert_eq!(no_guns.slots[0], None);
        assert!(no_guns.slots[5].is_some());
    }

    #[test]
    fn world_names_stay_inside_the_worlds_directory() {
        assert_eq!(sanitize_world_name("My World"), "My_World");
        assert_eq!(sanitize_world_name("../../etc"), "_._.._etc");
        assert_eq!(sanitize_world_name(""), "world");
        assert_eq!(sanitize_world_name("..."), "_..");
        let root = Path::new("/saves");
        assert_eq!(world_dir(root, "a/b"), Path::new("/saves/a_b"));
    }
}
