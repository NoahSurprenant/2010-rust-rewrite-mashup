//! GUI icons for `minecraft:special` item models, which vanilla draws with a
//! block entity's renderer rather than a baked model: chests, shulker boxes
//! and mob heads as their entity model's boxes (`ChestModel`,
//! `ShulkerModel`, `SkullModel`) with the entity texture, banners and
//! shields as the front of their cloth and board, and the rest (tridents,
//! conduits, decorated pots, copper golem statues, the dragon's head) as the
//! base model's particle texture.
use crate::item_icon::raster_model;
use crate::pack::{PackStack, ResourceId};
use anyhow::Result;
use image::{GenericImageView, Rgba, RgbaImage, imageops::FilterType};
use serde_json::{Value, json};
use std::collections::HashMap;

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
        Some("minecraft:chest") => {
            let texture = entity_texture(model, "entity/chest/", "normal")?;
            boxed_icon(packs, base, &texture, &chest_boxes(), icon_size)?
        }
        Some("minecraft:shulker_box") => {
            let texture = entity_texture(model, "entity/shulker/", "shulker")?;
            boxed_icon(packs, base, &texture, &shulker_boxes(), icon_size)?
        }
        Some(kind @ ("minecraft:head" | "minecraft:player_head")) => match head(match kind {
            "minecraft:player_head" => Some("player"),
            _ => model.get("kind").and_then(Value::as_str),
        }) {
            Some((texture, boxes)) => {
                boxed_icon(packs, base, &ResourceId::parse(texture)?, &boxes, icon_size)?
            }
            None => None,
        },
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
        _ => None,
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

/// One of a model's boxes as a `ModelPart` cube makes it: corners in block
/// pixels, its texture offset, and whether the model is built Y-down (as
/// shulker boxes and heads are, which their item turns over).
struct Cube {
    from: [f32; 3],
    size: [f32; 3],
    tex: [f32; 2],
    y_down: bool,
}

/// `ChestModel.createSingleBodyLayer`, closed, front to the south: the
/// pose the item's GUI rotation shows from the front, as the world's
/// `model::closed_chest_model` builds it facing south.
fn chest_boxes() -> Vec<Cube> {
    vec![
        Cube {
            from: [1.0, 0.0, 1.0],
            size: [14.0, 10.0, 14.0],
            tex: [0.0, 19.0],
            y_down: false,
        },
        Cube {
            from: [1.0, 9.0, 1.0],
            size: [14.0, 5.0, 14.0],
            tex: [0.0, 0.0],
            y_down: false,
        },
        Cube {
            from: [7.0, 7.0, 15.0],
            size: [2.0, 4.0, 1.0],
            tex: [0.0, 0.0],
            y_down: false,
        },
    ]
}

/// `ShulkerModel`, closed: the base and the lid over it.
fn shulker_boxes() -> Vec<Cube> {
    vec![
        Cube {
            from: [0.0, 0.0, 0.0],
            size: [16.0, 8.0, 16.0],
            tex: [0.0, 28.0],
            y_down: true,
        },
        Cube {
            from: [0.0, 4.0, 0.0],
            size: [16.0, 12.0, 16.0],
            tex: [0.0, 0.0],
            y_down: true,
        },
    ]
}

/// `SkullModel`'s head for a `minecraft:head` kind: its texture and box.
fn head(kind: Option<&str>) -> Option<(&'static str, Vec<Cube>)> {
    let cube = |width: f32| Cube {
        from: [8.0 - width / 2.0, 0.0, 4.0],
        size: [width, 8.0, 8.0],
        tex: [0.0, 0.0],
        y_down: true,
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

/// A cube as a block model element: its faces' texture regions as
/// `ModelPart.Cube` lays them out, each turned as the world's chest turns
/// them (`model::closed_chest_model`). A Y-down model turned over shows its
/// down region on top and its north region to the south.
fn cube_element(cube: &Cube, texture_size: (u32, u32)) -> Value {
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
        let uv = [
            r[0] / tw * 16.0,
            r[1] / th * 16.0,
            r[2] / tw * 16.0,
            r[3] / th * 16.0,
        ];
        faces.insert(direction.into(), json!({"uv": uv, "texture": "#entity"}));
    }
    let to = [cube.from[0] + w, cube.from[1] + h, cube.from[2] + d];
    json!({"from": cube.from, "to": to, "faces": faces})
}

/// Boxes with an entity texture, posed by the base model's GUI display.
fn boxed_icon(
    packs: &PackStack,
    base: Option<&str>,
    texture: &ResourceId,
    cubes: &[Cube],
    icon_size: usize,
) -> Result<Option<RgbaImage>> {
    let Some(bytes) = packs.texture(texture)? else {
        return Ok(None);
    };
    let size = image::load_from_memory(&bytes)?.dimensions();
    let elements: Vec<Value> = cubes.iter().map(|cube| cube_element(cube, size)).collect();
    let gui = match base {
        Some(base) => model_textures(packs, base)?.0,
        None => None,
    };
    let textures = HashMap::from([("entity".to_owned(), texture.key())]);
    raster_model(packs, &textures, &elements, gui.as_ref(), &[], icon_size)
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
mod tests {
    use super::*;
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

    /// A pack with vanilla's chest item (a `select` over special chest
    /// models), its base model and a chest texture painted by region: the
    /// lid's top red, the body's front green, everything else grey.
    fn chest_pack(root: &Path) -> PackStack {
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
            r#"{"display":{"gui":{"rotation":[30,45,0],"translation":[0,0,0],"scale":[0.625,0.625,0.625]}}}"#,
        );
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

    #[test]
    fn a_chest_icon_is_its_closed_box_from_the_front_and_above() {
        let temp = crate::test_dir::tempdir().unwrap();
        let packs = chest_pack(temp.path());
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
        let packs = chest_pack(temp.path());
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
