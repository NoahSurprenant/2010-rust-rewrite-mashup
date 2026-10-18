//! `minecraft:special` item models, which vanilla draws with a block
//! entity's renderer rather than a baked model: chests, shulker boxes, mob
//! heads, banners and shields as their entity model's boxes (`ChestModel`,
//! `ShulkerModel`, `SkullModel`, `BannerModel` with `BannerFlagModel`,
//! `ShieldModel`) with the entity texture, in the item's space as the item
//! definition's `transformation` places them; the rest (tridents, conduits,
//! decorated pots, copper golem statues, the dragon's head) as a plate of
//! the base model's particle texture. Their GUI icons draw banners and
//! shields as the front of their cloth and board, and the rest as the
//! particle texture.
use crate::item_icon::raster_model;
use crate::pack::{PackStack, ResourceId};
use anyhow::Result;
use image::{GenericImageView, Rgba, RgbaImage, imageops::FilterType};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};

/// The `minecraft:special` model an item definition's model shows by
/// default, by the same choices as `interface::item_model_reference`.
pub(crate) fn special_model(value: &Value) -> Option<&Value> {
    match value.get("type")?.as_str()? {
        "minecraft:special" => Some(value),
        "minecraft:select" | "minecraft:range_dispatch" => {
            value.get("fallback").and_then(special_model).or_else(|| {
                value
                    .get("cases")
                    .or_else(|| value.get("entries"))?
                    .as_array()?
                    .iter()
                    .find_map(|entry| special_model(&entry["model"]))
            })
        }
        "minecraft:condition" => value
            .get("on_false")
            .or_else(|| value.get("on_true"))
            .and_then(special_model),
        "minecraft:composite" => value
            .get("models")?
            .as_array()?
            .iter()
            .find_map(special_model),
        _ => None,
    }
}

/// Every `minecraft:special` model anywhere in an item definition's model:
/// the ones its selects show on some date or in some state too.
fn every_special_model<'a>(value: &'a Value, found: &mut Vec<&'a Value>) {
    match value {
        Value::Object(entries) => {
            if entries.get("type").and_then(Value::as_str) == Some("minecraft:special") {
                found.push(value);
            } else {
                entries
                    .values()
                    .for_each(|child| every_special_model(child, found));
            }
        }
        Value::Array(entries) => entries
            .iter()
            .for_each(|child| every_special_model(child, found)),
        _ => {}
    }
}

/// The entity textures the pack's special items are drawn with, so the
/// world's atlas holds them before an item is held or dropped.
pub(crate) fn special_textures(packs: &PackStack) -> Result<BTreeSet<ResourceId>> {
    let mut textures = BTreeSet::new();
    for path in packs.list("minecraft", "items/")? {
        let Some(name) = path
            .strip_prefix("assets/minecraft/items/")
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        // An item the pack cannot define leaves the atlas as it was.
        let Some(definition) = ResourceId::parse(&format!("minecraft:{name}"))
            .and_then(|id| packs.item_definition(&id))
            .ok()
            .flatten()
        else {
            continue;
        };
        let mut specials = Vec::new();
        every_special_model(&definition["model"], &mut specials);
        for special in specials {
            for part in special_parts(&special["model"]).unwrap_or_default() {
                if packs.texture(&part.texture)?.is_some() {
                    textures.insert(part.texture);
                }
            }
        }
    }
    Ok(textures)
}

/// A special model as block model elements: the texture each `#name`
/// stands for and the elements, in block pixels as a model file has them
/// (a tinted face takes `special_tints`).
pub(crate) struct SpecialBoxes {
    pub textures: HashMap<String, String>,
    pub elements: Vec<Value>,
}

/// The boxes a `minecraft:special` model (`special`) is drawn as, or the
/// plate of its base model's particle texture when it has no renderer here
/// (or its texture is missing); `None` when neither can be had.
pub(crate) fn special_boxes(packs: &PackStack, special: &Value) -> Result<Option<SpecialBoxes>> {
    let parts = special_parts(&special["model"])?;
    let (textures, elements) = part_elements(packs, &parts)?;
    if !elements.is_empty() {
        return Ok(Some(SpecialBoxes { textures, elements }));
    }
    let Some(base) = special.get("base").and_then(Value::as_str) else {
        return Ok(None);
    };
    let Some(particle) = model_textures(packs, base)?.1.get("particle").cloned() else {
        return Ok(None);
    };
    let face = json!({"texture": "#particle"});
    let side = json!({"uv": [0, 0, 16, 16], "texture": "#particle"});
    Ok(Some(SpecialBoxes {
        textures: HashMap::from([("particle".to_owned(), particle)]),
        elements: vec![json!({
            "from": [0, 0, 7.5],
            "to": [16, 16, 8.5],
            "faces": {"north": side, "south": side, "up": face, "down": face, "west": face, "east": face},
        })],
    }))
}

/// The tints a special item's faces take: a banner's cloth its dye colour.
pub(crate) fn special_tints(value: &Value) -> Vec<[u8; 3]> {
    let Some(model) = special_model(value).map(|special| &special["model"]) else {
        return Vec::new();
    };
    match model.get("type").and_then(Value::as_str) {
        Some("minecraft:banner") => vec![dye_color(
            model
                .get("color")
                .and_then(Value::as_str)
                .unwrap_or("white"),
        )],
        _ => Vec::new(),
    }
}

/// The icon of a `minecraft:special` model (`special`), or `None` when
/// neither its renderer's look nor its base model's particle texture can be
/// had.
pub(crate) fn special_icon(
    packs: &PackStack,
    special: &Value,
    icon_size: usize,
) -> Result<Option<RgbaImage>> {
    let base = special.get("base").and_then(Value::as_str);
    let model = &special["model"];
    let drawn = match model.get("type").and_then(Value::as_str) {
        Some("minecraft:banner") => {
            let color = model
                .get("color")
                .and_then(Value::as_str)
                .unwrap_or("white");
            let cloth = ResourceId::parse("minecraft:entity/banner/base")?;
            flat_icon(packs, &cloth, [1, 1, 21, 41], dye_color(color), icon_size)?
        }
        Some("minecraft:shield") => {
            let board = ResourceId::parse("minecraft:entity/shield/shield_base_nopattern")?;
            flat_icon(packs, &board, [1, 1, 13, 23], [255; 3], icon_size)?
        }
        _ => {
            let (textures, elements) = part_elements(packs, &special_parts(model)?)?;
            if elements.is_empty() {
                None
            } else {
                let gui = match base {
                    Some(base) => model_textures(packs, base)?.0,
                    None => None,
                };
                raster_model(packs, &textures, &elements, gui.as_ref(), &[], icon_size)?
            }
        }
    };
    if drawn.is_some() {
        return Ok(drawn);
    }
    let Some(base) = base else {
        return Ok(None);
    };
    let Some(particle) = model_textures(packs, base)?.1.get("particle").cloned() else {
        return Ok(None);
    };
    flat_icon(
        packs,
        &ResourceId::parse(&particle)?,
        [0, 0, 0, 0],
        [255; 3],
        icon_size,
    )
}

/// `{"texture": "minecraft:<name>"}` as `minecraft:<folder><name>`.
fn entity_texture(model: &Value, folder: &str, default: &str) -> Result<ResourceId> {
    let name = model
        .get("texture")
        .and_then(Value::as_str)
        .unwrap_or(default);
    let id = ResourceId::parse(name)?;
    ResourceId::parse(&format!("{}:{folder}{}", id.namespace, id.path))
}

/// Boxes drawn with one texture, named `#name` in their elements; a tinted
/// part's faces take the first tint.
struct Part {
    name: &'static str,
    texture: ResourceId,
    cubes: Vec<Cube>,
    tinted: bool,
}

/// The parts a special model's renderer draws, or none when it has no
/// renderer here.
fn special_parts(model: &Value) -> Result<Vec<Part>> {
    let part = |name, texture, cubes| Part {
        name,
        texture,
        cubes,
        tinted: false,
    };
    Ok(match model.get("type").and_then(Value::as_str) {
        Some("minecraft:chest") => vec![part(
            "entity",
            entity_texture(model, "entity/chest/", "normal")?,
            chest_boxes(),
        )],
        Some("minecraft:shulker_box") => vec![part(
            "entity",
            entity_texture(model, "entity/shulker/", "shulker")?,
            shulker_boxes(),
        )],
        Some(kind @ ("minecraft:head" | "minecraft:player_head")) => match head(match kind {
            "minecraft:player_head" => Some("player"),
            _ => model.get("kind").and_then(Value::as_str),
        }) {
            Some((texture, boxes)) => vec![part("entity", ResourceId::parse(texture)?, boxes)],
            None => Vec::new(),
        },
        Some("minecraft:banner") => {
            let (pole, flag) = banner_boxes();
            vec![
                part(
                    "pole",
                    ResourceId::parse("minecraft:entity/banner/banner_base")?,
                    pole,
                ),
                Part {
                    tinted: true,
                    ..part(
                        "cloth",
                        ResourceId::parse("minecraft:entity/banner/base")?,
                        flag,
                    )
                },
            ]
        }
        Some("minecraft:shield") => vec![part(
            "board",
            ResourceId::parse("minecraft:entity/shield/shield_base_nopattern")?,
            shield_boxes(),
        )],
        _ => Vec::new(),
    })
}

/// The parts' elements and their textures by name; a part whose texture
/// the pack lacks is left out.
fn part_elements(
    packs: &PackStack,
    parts: &[Part],
) -> Result<(HashMap<String, String>, Vec<Value>)> {
    let mut textures = HashMap::new();
    let mut elements = Vec::new();
    for part in parts {
        let Some(bytes) = packs.texture(&part.texture)? else {
            continue;
        };
        let size = image::load_from_memory(&bytes)?.dimensions();
        textures.insert(part.name.to_owned(), part.texture.key());
        elements.extend(
            part.cubes
                .iter()
                .map(|cube| cube_element(cube, size, part.name, part.tinted)),
        );
    }
    Ok((textures, elements))
}

/// One of a model's boxes as a `ModelPart` cube makes it: corners in block
/// pixels, its size and texture offset in texture pixels, how many block
/// pixels a texture pixel spans, and whether the model is built Y-down (as
/// shulker boxes, heads, banners and shields are, which their item turns
/// over about X).
struct Cube {
    from: [f32; 3],
    size: [f32; 3],
    tex: [f32; 2],
    scale: f32,
    y_down: bool,
}

/// A Y-down model's box (`addBox` corner and size, texture offset) as its
/// item places it: scaled by `scale`, turned over about X and moved to
/// `origin` (in block pixels).
fn turned(origin: [f32; 3], scale: f32, corner: [f32; 3], size: [f32; 3], tex: [f32; 2]) -> Cube {
    Cube {
        from: [
            origin[0] + scale * corner[0],
            origin[1] - scale * (corner[1] + size[1]),
            origin[2] - scale * (corner[2] + size[2]),
        ],
        size,
        tex,
        scale,
        y_down: true,
    }
}

/// An upright box at its block corner, a texture pixel to a block pixel.
fn upright(from: [f32; 3], size: [f32; 3], tex: [f32; 2]) -> Cube {
    Cube {
        from,
        size,
        tex,
        scale: 1.0,
        y_down: false,
    }
}

/// `ChestModel.createSingleBodyLayer`, closed, front to the south: the
/// pose the item's GUI rotation shows from the front, as the world's
/// `model::closed_chest_model` builds it facing south.
fn chest_boxes() -> Vec<Cube> {
    vec![
        upright([1.0, 0.0, 1.0], [14.0, 10.0, 14.0], [0.0, 19.0]),
        upright([1.0, 9.0, 1.0], [14.0, 5.0, 14.0], [0.0, 0.0]),
        upright([7.0, 7.0, 15.0], [2.0, 4.0, 1.0], [0.0, 0.0]),
    ]
}

/// `ShulkerModel`, closed: the base and the lid over it.
fn shulker_boxes() -> Vec<Cube> {
    let origin = [8.0, 24.0, 8.0];
    vec![
        turned(
            origin,
            1.0,
            [-8.0, 16.0, -8.0],
            [16.0, 8.0, 16.0],
            [0.0, 28.0],
        ),
        turned(
            origin,
            1.0,
            [-8.0, 8.0, -8.0],
            [16.0, 12.0, 16.0],
            [0.0, 0.0],
        ),
    ]
}

/// `SkullModel`'s head for a `minecraft:head` kind: its texture and box.
fn head(kind: Option<&str>) -> Option<(&'static str, Vec<Cube>)> {
    let cube = |width: f32| {
        turned(
            [8.0, 0.0, 8.0],
            1.0,
            [-width / 2.0, -8.0, -4.0],
            [width, 8.0, 8.0],
            [0.0, 0.0],
        )
    };
    let texture = match kind? {
        "skeleton" => "minecraft:entity/skeleton/skeleton",
        "wither_skeleton" => "minecraft:entity/skeleton/wither_skeleton",
        "zombie" => "minecraft:entity/zombie/zombie",
        "creeper" => "minecraft:entity/creeper/creeper",
        "player" => "minecraft:entity/player/wide/steve",
        // The piglin's snout is wider than its head's 8 pixels.
        "piglin" => return Some(("minecraft:entity/piglin/piglin", vec![cube(10.0)])),
        _ => return None,
    };
    Some((texture, vec![cube(8.0)]))
}

/// `BannerModel`'s standing pole and bar, and `BannerFlagModel`'s flag
/// hanging still, at the two-thirds scale the item draws them.
fn banner_boxes() -> (Vec<Cube>, Vec<Cube>) {
    let origin = [8.0, 0.0, 8.0];
    let scale = 2.0 / 3.0;
    (
        vec![
            turned(
                origin,
                scale,
                [-1.0, -42.0, -1.0],
                [2.0, 42.0, 2.0],
                [44.0, 0.0],
            ),
            turned(
                origin,
                scale,
                [-10.0, -44.0, -1.0],
                [20.0, 2.0, 2.0],
                [0.0, 42.0],
            ),
        ],
        vec![turned(
            origin,
            scale,
            [-10.0, -44.0, -2.0],
            [20.0, 40.0, 1.0],
            [0.0, 0.0],
        )],
    )
}

/// `ShieldModel`'s board and handle, about the item's origin.
fn shield_boxes() -> Vec<Cube> {
    let origin = [0.0; 3];
    vec![
        turned(
            origin,
            1.0,
            [-6.0, -11.0, -2.0],
            [12.0, 22.0, 1.0],
            [0.0, 0.0],
        ),
        turned(
            origin,
            1.0,
            [-1.0, -3.0, -1.0],
            [2.0, 6.0, 6.0],
            [26.0, 0.0],
        ),
    ]
}

/// A cube as a block model element whose faces use `#name`: their texture
/// regions as `ModelPart.Cube` lays them out, each turned as the world's
/// chest turns them (`model::closed_chest_model`), kept a sliver inside
/// their texels so the atlas's neighbours never show at an edge. A Y-down
/// model turned over shows its down region on top and its north region to
/// the south.
fn cube_element(cube: &Cube, texture_size: (u32, u32), name: &str, tinted: bool) -> Value {
    let [w, h, d] = cube.size;
    let [u, v] = cube.tex;
    let region = |name: &str| -> [f32; 4] {
        match name {
            "down" => [u + d, v, u + d + w, v + d],
            "up" => [u + d + w, v, u + d + w * 2.0, v + d],
            "north" => [u + d, v + d, u + d + w, v + d + h],
            "south" => [u + d * 2.0 + w, v + d, u + d * 2.0 + w * 2.0, v + d + h],
            "west" => [u, v + d, u + d, v + d + h],
            _ => [u + d + w, v + d, u + d * 2.0 + w, v + d + h],
        }
    };
    let inside = |a: f32, b: f32| {
        let inset = 1.0 / 64.0;
        if a < b {
            (a + inset, b - inset)
        } else {
            (a - inset, b + inset)
        }
    };
    let (tw, th) = (texture_size.0 as f32, texture_size.1 as f32);
    let mut faces = serde_json::Map::new();
    for direction in ["down", "up", "north", "south", "west", "east"] {
        let source = match (cube.y_down, direction) {
            (true, "up") => "down",
            (true, "down") => "up",
            (true, "north") => "south",
            (true, "south") => "north",
            _ => direction,
        };
        let r = region(source);
        let r = match (cube.y_down, direction) {
            (_, "north" | "south" | "west" | "east") => [r[2], r[3], r[0], r[1]],
            (false, "up") => [r[0], r[3], r[2], r[1]],
            _ => r,
        };
        let (u0, u1) = inside(r[0], r[2]);
        let (v0, v1) = inside(r[1], r[3]);
        let uv = [
            u0 / tw * 16.0,
            v0 / th * 16.0,
            u1 / tw * 16.0,
            v1 / th * 16.0,
        ];
        let mut face = json!({"uv": uv, "texture": format!("#{name}")});
        if tinted {
            face["tintindex"] = json!(0);
        }
        faces.insert(direction.into(), face);
    }
    let to = [
        cube.from[0] + w * cube.scale,
        cube.from[1] + h * cube.scale,
        cube.from[2] + d * cube.scale,
    ];
    json!({"from": cube.from, "to": to, "faces": faces})
}

/// A texture's pixel rectangle `[x0, y0, x1, y1]` (all of it when empty),
/// tinted and centred in the icon at its own proportions, nearest-sampled.
fn flat_icon(
    packs: &PackStack,
    texture: &ResourceId,
    rect: [u32; 4],
    tint: [u8; 3],
    icon_size: usize,
) -> Result<Option<RgbaImage>> {
    let Some(bytes) = packs.texture(texture)? else {
        return Ok(None);
    };
    let source = image::load_from_memory(&bytes)?.to_rgba8();
    let (width, height) = source.dimensions();
    let [x0, y0, x1, y1] = if rect == [0; 4] {
        // A whole texture, or an animation strip's first frame.
        [0, 0, width, if height > width { width } else { height }]
    } else {
        rect
    };
    if x1 <= x0 || y1 <= y0 || x1 > width || y1 > height {
        return Ok(None);
    }
    let mut part = image::imageops::crop_imm(&source, x0, y0, x1 - x0, y1 - y0).to_image();
    for pixel in part.pixels_mut() {
        for channel in 0..3 {
            pixel[channel] = (u16::from(pixel[channel]) * u16::from(tint[channel]) / 255) as u8;
        }
    }
    let (pw, ph) = part.dimensions();
    let fit = icon_size as f32 / pw.max(ph) as f32;
    let (sw, sh) = (
        ((pw as f32 * fit).round() as u32).max(1),
        ((ph as f32 * fit).round() as u32).max(1),
    );
    let scaled = image::imageops::resize(&part, sw, sh, FilterType::Nearest);
    let mut icon = RgbaImage::from_pixel(icon_size as u32, icon_size as u32, Rgba([0; 4]));
    let (x, y) = ((icon_size as u32 - sw) / 2, (icon_size as u32 - sh) / 2);
    image::imageops::overlay(&mut icon, &scaled, i64::from(x), i64::from(y));
    Ok(icon.pixels().any(|pixel| pixel[3] > 0).then_some(icon))
}

/// A model's GUI display and its textures, through its parents.
fn model_textures(
    packs: &PackStack,
    model: &str,
) -> Result<(Option<Value>, HashMap<String, String>)> {
    let mut gui = None;
    let mut textures = HashMap::new();
    let mut current = Some(ResourceId::parse(model)?);
    for _ in 0..12 {
        let Some(id) = current.take() else { break };
        let Some(value) = packs.model(&id)? else {
            break;
        };
        if gui.is_none() {
            gui = value
                .get("display")
                .and_then(|display| display.get("gui"))
                .cloned();
        }
        if let Some(entries) = value.get("textures").and_then(Value::as_object) {
            for (key, texture) in entries {
                if let Some(texture) = texture
                    .as_str()
                    .or_else(|| texture.get("sprite").and_then(Value::as_str))
                {
                    textures
                        .entry(key.clone())
                        .or_insert_with(|| texture.to_owned());
                }
            }
        }
        current = value
            .get("parent")
            .and_then(Value::as_str)
            .map(ResourceId::parse)
            .transpose()?;
    }
    Ok((gui, textures))
}

/// `DyeColor.getTextureDiffuseColor`: a banner's cloth tint.
fn dye_color(color: &str) -> [u8; 3] {
    let rgb: u32 = match color {
        "orange" => 0xF9801D,
        "magenta" => 0xC74EBD,
        "light_blue" => 0x3AB3DA,
        "yellow" => 0xFED83D,
        "lime" => 0x80C71F,
        "pink" => 0xF38BAA,
        "gray" => 0x474F52,
        "light_gray" => 0x9D9D97,
        "cyan" => 0x169C9C,
        "purple" => 0x8932B8,
        "blue" => 0x3C44AA,
        "brown" => 0x835432,
        "green" => 0x5E7C16,
        "red" => 0xB02E26,
        "black" => 0x1D1D21,
        _ => 0xF9FFFE,
    };
    [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8]
}

#[cfg(test)]
pub(crate) mod test_pack {
    use crate::pack::PackStack;
    use image::{Rgba, RgbaImage};
    use std::fs;
    use std::path::Path;

    fn write(root: &Path, path: &str, text: &str) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn png(root: &Path, path: &str, image: &RgbaImage) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        image.save(path).unwrap();
    }

    /// A pack with special items as vanilla defines them: the chest (a
    /// `select` over special chest models, its texture painted by region:
    /// the lid's top red, the body's front green, everything else grey), a
    /// shulker box, a skeleton skull, a red banner, a shield (a `condition`)
    /// and a conduit (no renderer here), with their base models.
    pub(crate) fn special_pack(root: &Path) -> PackStack {
        write(
            root,
            "pack.mcmeta",
            r#"{"pack":{"min_format":[97,1],"max_format":[97,1]}}"#,
        );
        write(
            root,
            "assets/minecraft/items/chest.json",
            r#"{"model":{"type":"minecraft:select","property":"minecraft:local_time","pattern":"MM-dd",
                "cases":[{"when":["12-24"],"model":{"type":"minecraft:special","base":"minecraft:item/chest",
                    "model":{"type":"minecraft:chest","texture":"minecraft:christmas"}}}],
                "fallback":{"type":"minecraft:special","base":"minecraft:item/chest",
                    "model":{"type":"minecraft:chest","texture":"minecraft:normal"}}}}"#,
        );
        write(
            root,
            "assets/minecraft/items/conduit.json",
            r#"{"model":{"type":"minecraft:special","base":"minecraft:item/conduit","model":{"type":"minecraft:conduit"}}}"#,
        );
        write(
            root,
            "assets/minecraft/models/item/chest.json",
            r#"{"parent":"minecraft:item/template_chest","textures":{"particle":"minecraft:block/oak_planks"}}"#,
        );
        write(
            root,
            "assets/minecraft/models/item/template_chest.json",
            r#"{"display":{"gui":{"rotation":[30,45,0],"translation":[0,0,0],"scale":[0.625,0.625,0.625]},
                "ground":{"rotation":[0,0,0],"translation":[0,3,0],"scale":[0.25,0.25,0.25]},
                "thirdperson_righthand":{"rotation":[75,315,0],"translation":[0,2.5,0],"scale":[0.375,0.375,0.375]},
                "firstperson_righthand":{"rotation":[0,315,0],"translation":[0,0,0],"scale":[0.4,0.4,0.4]}}}"#,
        );
        write(
            root,
            "assets/minecraft/items/shulker_box.json",
            r#"{"model":{"type":"minecraft:special","base":"minecraft:item/shulker_box",
                "model":{"type":"minecraft:shulker_box","texture":"minecraft:shulker"}}}"#,
        );
        write(
            root,
            "assets/minecraft/models/item/shulker_box.json",
            r#"{"display":{"firstperson_righthand":{"rotation":[0,315,0],"scale":[0.4,0.4,0.4]}}}"#,
        );
        write(
            root,
            "assets/minecraft/items/skeleton_skull.json",
            r#"{"model":{"type":"minecraft:special","base":"minecraft:item/template_skull",
                "model":{"type":"minecraft:head","kind":"skeleton"}}}"#,
        );
        write(
            root,
            "assets/minecraft/models/item/template_skull.json",
            r#"{"display":{"ground":{"translation":[0,3,0],"scale":[0.5,0.5,0.5]}}}"#,
        );
        write(
            root,
            "assets/minecraft/items/red_banner.json",
            r#"{"model":{"type":"minecraft:special","base":"minecraft:item/template_banner",
                "model":{"type":"minecraft:banner","color":"red"}}}"#,
        );
        write(
            root,
            "assets/minecraft/models/item/template_banner.json",
            r#"{"display":{"thirdperson_righthand":{"rotation":[0,90,0],"translation":[0,2,0.5],"scale":[0.375,0.375,0.375]}}}"#,
        );
        write(
            root,
            "assets/minecraft/items/shield.json",
            r#"{"model":{"type":"minecraft:condition","property":"minecraft:using_item",
                "on_false":{"type":"minecraft:special","base":"minecraft:item/shield","model":{"type":"minecraft:shield"}},
                "on_true":{"type":"minecraft:special","base":"minecraft:item/shield_blocking","model":{"type":"minecraft:shield"}}}}"#,
        );
        write(
            root,
            "assets/minecraft/models/item/shield.json",
            r#"{"display":{"thirdperson_righthand":{"rotation":[0,90,0],"translation":[10,6,-4],"scale":[1,1,1]}}}"#,
        );
        for (path, size, colour) in [
            ("entity/shulker/shulker", (64, 64), [150, 90, 160]),
            ("entity/skeleton/skeleton", (64, 32), [200, 200, 200]),
            ("entity/banner/banner_base", (64, 64), [120, 90, 50]),
            ("entity/banner/base", (64, 64), [255, 255, 255]),
            (
                "entity/shield/shield_base_nopattern",
                (64, 64),
                [140, 110, 70],
            ),
        ] {
            png(
                root,
                &format!("assets/minecraft/textures/{path}.png"),
                &RgbaImage::from_pixel(
                    size.0,
                    size.1,
                    Rgba([colour[0], colour[1], colour[2], 255]),
                ),
            );
        }
        write(
            root,
            "assets/minecraft/models/item/conduit.json",
            r#"{"textures":{"particle":"block/conduit"}}"#,
        );
        let mut chest = RgbaImage::from_pixel(64, 64, Rgba([128, 128, 128, 255]));
        // ModelPart.Cube regions: the lid (texture offset 0,0; 14x5x14) has
        // its up region at u 28..42, v 0..14; the body (0,19; 14x10x14) its
        // south region at u 42..56, v 33..43 (as the world's chest maps it).
        for (x, y) in (28..42).flat_map(|x| (0..14).map(move |y| (x, y))) {
            chest.put_pixel(x, y, Rgba([200, 0, 0, 255]));
        }
        for (x, y) in (42..56).flat_map(|x| (33..43).map(move |y| (x, y))) {
            chest.put_pixel(x, y, Rgba([0, 200, 0, 255]));
        }
        png(
            root,
            "assets/minecraft/textures/entity/chest/normal.png",
            &chest,
        );
        png(
            root,
            "assets/minecraft/textures/block/conduit.png",
            &RgbaImage::from_pixel(16, 16, Rgba([20, 90, 160, 255])),
        );
        PackStack::open(vec![root.into()]).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::test_pack::special_pack;
    use super::*;
    use std::fs;
    use std::path::Path;

    /// The resolved model's boxes, in block pixels.
    fn boxes(packs: &PackStack, item: &str) -> Vec<([f32; 3], [f32; 3])> {
        let model = crate::model::resolve_item_model(packs, &ResourceId::parse(item).unwrap())
            .unwrap()
            .expect("a model");
        model
            .elements
            .iter()
            .map(|e| (e.from.map(|v| v * 16.0), e.to.map(|v| v * 16.0)))
            .collect()
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3)
    }

    #[test]
    fn a_special_item_model_is_its_renderers_boxes() {
        let temp = crate::test_dir::tempdir().unwrap();
        let packs = special_pack(temp.path());
        let chest = boxes(&packs, "minecraft:chest");
        assert_eq!(chest.len(), 3);
        assert!(close(chest[0].0, [1.0, 0.0, 1.0]) && close(chest[0].1, [15.0, 10.0, 15.0]));
        let shulker = boxes(&packs, "minecraft:shulker_box");
        assert!(close(shulker[0].0, [0.0; 3]) && close(shulker[0].1, [16.0, 8.0, 16.0]));
        assert!(close(shulker[1].0, [0.0, 4.0, 0.0]) && close(shulker[1].1, [16.0; 3]));
        let skull = boxes(&packs, "minecraft:skeleton_skull");
        assert_eq!(skull.len(), 1);
        assert!(close(skull[0].0, [4.0, 0.0, 4.0]) && close(skull[0].1, [12.0, 8.0, 12.0]));
        let model = crate::model::resolve_item_model(
            &packs,
            &ResourceId::parse("minecraft:chest").unwrap(),
        )
        .unwrap()
        .unwrap();
        assert!(model.elements.iter().all(|e| {
            e.faces.len() == 6
                && e.faces
                    .iter()
                    .all(|f| f.texture.key() == "minecraft:entity/chest/normal")
        }));
    }

    #[test]
    fn a_banner_stands_on_its_pole_with_its_cloth_in_front_dyed() {
        let temp = crate::test_dir::tempdir().unwrap();
        let packs = special_pack(temp.path());
        let banner = boxes(&packs, "minecraft:red_banner");
        assert_eq!(banner.len(), 3);
        let third = 2.0 / 3.0;
        // The pole from the ground, the bar across its top, the flag hung
        // from the bar on the pole's south side.
        assert!(close(banner[0].0, [8.0 - third, 0.0, 8.0 - third]));
        assert!(close(banner[0].1, [8.0 + third, 28.0, 8.0 + third]));
        assert!(close(
            banner[1].0,
            [8.0 - 20.0 * third / 2.0, 28.0, 8.0 - third]
        ));
        assert!(close(
            banner[2].0,
            [8.0 - 20.0 * third / 2.0, 4.0 * third, 8.0 + third]
        ));
        assert!(close(
            banner[2].1,
            [8.0 + 20.0 * third / 2.0, 44.0 * third, 8.0 + 2.0 * third]
        ));
        let model = crate::model::resolve_item_model(
            &packs,
            &ResourceId::parse("minecraft:red_banner").unwrap(),
        )
        .unwrap()
        .unwrap();
        let flag = &model.elements[2];
        assert!(
            flag.faces
                .iter()
                .all(|f| f.tint_index == Some(0)
                    && f.texture.key() == "minecraft:entity/banner/base")
        );
        assert!(
            model.elements[0]
                .faces
                .iter()
                .all(|f| f.tint_index.is_none())
        );
        let definition = packs
            .item_definition(&ResourceId::parse("minecraft:red_banner").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            crate::interface::item_model_tints(&packs, &definition["model"]).unwrap(),
            [[0xB0, 0x2E, 0x26]]
        );
    }

    #[test]
    fn a_shield_is_its_board_and_handle_about_the_item_origin() {
        let temp = crate::test_dir::tempdir().unwrap();
        let packs = special_pack(temp.path());
        let shield = boxes(&packs, "minecraft:shield");
        assert_eq!(shield.len(), 2);
        assert!(close(shield[0].0, [-6.0, -11.0, 1.0]) && close(shield[0].1, [6.0, 11.0, 2.0]));
        assert!(close(shield[1].0, [-1.0, -3.0, -5.0]) && close(shield[1].1, [1.0, 3.0, 1.0]));
    }

    #[test]
    fn a_special_item_with_no_renderer_here_is_a_plate_of_its_particle_texture() {
        let temp = crate::test_dir::tempdir().unwrap();
        let packs = special_pack(temp.path());
        let model = crate::model::resolve_item_model(
            &packs,
            &ResourceId::parse("minecraft:conduit").unwrap(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(model.elements.len(), 1);
        assert_eq!(model.elements[0].faces.len(), 6);
        assert!(
            model.elements[0]
                .faces
                .iter()
                .all(|f| f.texture.key() == "minecraft:block/conduit")
        );
    }

    #[test]
    fn a_special_item_is_placed_by_its_base_models_display() {
        let temp = crate::test_dir::tempdir().unwrap();
        let packs = special_pack(temp.path());
        let chest = ResourceId::parse("minecraft:chest").unwrap();
        let scale = |context: &str| {
            crate::model::item_display_transform(&packs, &chest, context)
                .unwrap()
                .transform_vector3(glam::Vec3::X)
                .length()
        };
        assert!((scale("firstperson_righthand") - 0.4).abs() < 1e-5);
        assert!((scale("thirdperson_righthand") - 0.375).abs() < 1e-5);
        assert!((scale("ground") - 0.25).abs() < 1e-5);
        let ground = crate::model::item_display_transform(&packs, &chest, "ground").unwrap();
        let centre = ground.transform_point3(glam::Vec3::splat(0.5));
        assert!((centre.y - 3.0 / 16.0).abs() < 1e-5);
        let shield = ResourceId::parse("minecraft:shield").unwrap();
        let held =
            crate::model::item_display_transform(&packs, &shield, "thirdperson_righthand").unwrap();
        assert_ne!(held, glam::Mat4::IDENTITY);
    }

    #[test]
    fn the_atlas_is_given_every_special_items_texture_the_pack_has() {
        let temp = crate::test_dir::tempdir().unwrap();
        let packs = special_pack(temp.path());
        let keys: Vec<String> = special_textures(&packs)
            .unwrap()
            .iter()
            .map(ResourceId::key)
            .collect();
        for want in [
            "minecraft:entity/chest/normal",
            "minecraft:entity/shulker/shulker",
            "minecraft:entity/skeleton/skeleton",
            "minecraft:entity/banner/banner_base",
            "minecraft:entity/banner/base",
            "minecraft:entity/shield/shield_base_nopattern",
        ] {
            assert!(keys.iter().any(|key| key == want), "{want} in {keys:?}");
        }
        // The Christmas chest's texture is not in this pack.
        assert!(!keys.iter().any(|key| key.ends_with("christmas")));
    }

    #[test]
    fn a_chest_icon_is_its_closed_box_from_the_front_and_above() {
        let temp = crate::test_dir::tempdir().unwrap();
        let packs = special_pack(temp.path());
        let icon = crate::item_icons::item_icon(&packs, "minecraft:chest", 48)
            .unwrap()
            .expect("an icon");
        let visible = icon.pixels().filter(|p| p[3] > 0).count();
        assert!(visible > 48 * 48 / 4, "{visible} pixels drawn");
        let reddish = icon
            .pixels()
            .filter(|p| p[3] > 0 && u16::from(p[0]) > 2 * u16::from(p[1]) && p[0] > 60)
            .count();
        let greenish = icon
            .pixels()
            .filter(|p| p[3] > 0 && u16::from(p[1]) > 2 * u16::from(p[0]) && p[1] > 60)
            .count();
        // The lid's top faces up, the body's front towards the viewer.
        assert!(reddish > 40, "top {reddish}");
        assert!(greenish > 40, "front {greenish}");
        // The top is above the front.
        let mean_y = |want: &dyn Fn(&Rgba<u8>) -> bool| {
            let ys: Vec<u32> = icon
                .enumerate_pixels()
                .filter(|(_, _, p)| want(p))
                .map(|(_, y, _)| y)
                .collect();
            ys.iter().sum::<u32>() as f32 / ys.len() as f32
        };
        let top = mean_y(&|p| p[3] > 0 && u16::from(p[0]) > 2 * u16::from(p[1]) && p[0] > 60);
        let front = mean_y(&|p| p[3] > 0 && u16::from(p[1]) > 2 * u16::from(p[0]) && p[1] > 60);
        assert!(top < front, "top at {top}, front at {front}");
    }

    #[test]
    fn a_special_model_with_no_renderer_here_shows_its_particle_texture() {
        let temp = crate::test_dir::tempdir().unwrap();
        let packs = special_pack(temp.path());
        let icon = crate::item_icons::item_icon(&packs, "minecraft:conduit", 32)
            .unwrap()
            .expect("an icon");
        assert_eq!(icon.dimensions(), (32, 32));
        assert!(icon.pixels().all(|p| *p == Rgba([20, 90, 160, 255])));
    }

    #[test]
    fn the_default_special_model_is_found_through_selects_and_conditions() {
        let value: Value = serde_json::from_str(
            r#"{"type":"minecraft:condition","property":"minecraft:using_item",
                "on_false":{"type":"minecraft:special","base":"minecraft:item/shield","model":{"type":"minecraft:shield"}},
                "on_true":{"type":"minecraft:special","base":"minecraft:item/shield_blocking","model":{"type":"minecraft:shield"}}}"#,
        )
        .unwrap();
        assert_eq!(
            special_model(&value).unwrap()["base"],
            "minecraft:item/shield"
        );
        let plain: Value =
            serde_json::from_str(r#"{"type":"minecraft:model","model":"minecraft:item/stick"}"#)
                .unwrap();
        assert!(special_model(&plain).is_none());
    }

    #[test]
    fn a_banner_is_its_cloth_in_its_colour() {
        assert_eq!(dye_color("red"), [0xB0, 0x2E, 0x26]);
        assert_eq!(dye_color("white"), [0xF9, 0xFF, 0xFE]);
    }

    /// Draws every special item of a real pack (`IW4L_SPECIAL_ICON_PACK`, a
    /// folder with `assets/minecraft`) to `IW4L_SPECIAL_ICON_OUT`, four
    /// times as large, to look at.
    #[test]
    #[ignore]
    fn draw_the_special_items_of_a_pack() {
        let (Ok(pack), Ok(out)) = (
            std::env::var("IW4L_SPECIAL_ICON_PACK"),
            std::env::var("IW4L_SPECIAL_ICON_OUT"),
        ) else {
            return;
        };
        let packs = PackStack::open(vec![pack.clone().into()]).unwrap();
        fs::create_dir_all(&out).unwrap();
        let items = Path::new(&pack).join("assets/minecraft/items");
        let mut drawn = 0;
        let mut sheet = RgbaImage::from_pixel(64 * 10, 64 * 8, Rgba([60, 60, 70, 255]));
        for entry in fs::read_dir(items).unwrap() {
            let path = entry.unwrap().path();
            let text = fs::read_to_string(&path).unwrap();
            if !text.contains("minecraft:special") {
                continue;
            }
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            match crate::item_icons::item_icon(&packs, &format!("minecraft:{name}"), 32) {
                Ok(Some(icon)) => {
                    let big = image::imageops::resize(&icon, 128, 128, FilterType::Nearest);
                    big.save(Path::new(&out).join(format!("{name}.png")))
                        .unwrap();
                    let cell = image::imageops::resize(&icon, 64, 64, FilterType::Nearest);
                    let (x, y) = ((drawn % 10) * 64, (drawn / 10) * 64);
                    image::imageops::overlay(&mut sheet, &cell, x as i64, y as i64);
                    drawn += 1;
                }
                other => println!("{name}: {:?}", other.map(|icon| icon.is_some())),
            }
        }
        // Block items beside them, to compare which way a front faces.
        for name in ["furnace", "crafting_table", "carved_pumpkin"] {
            if let Ok(Some(icon)) =
                crate::item_icons::item_icon(&packs, &format!("minecraft:{name}"), 32)
            {
                let big = image::imageops::resize(&icon, 128, 128, FilterType::Nearest);
                big.save(Path::new(&out).join(format!("{name}.png")))
                    .unwrap();
            }
        }
        sheet.save(Path::new(&out).join("_sheet.png")).unwrap();
        println!("drew {drawn}");
    }
}
