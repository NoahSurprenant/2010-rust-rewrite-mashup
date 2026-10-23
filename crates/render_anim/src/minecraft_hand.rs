//! The held item on the Minecraft map when no gun is selected, placed as
//! MinecraftOSS's viewer places it (26.3 `applyItemArmTransform` with the
//! item's `firstperson_righthand` display), swinging as vanilla swings it.
//! Vertices are in view space: x right, y up, z back.
use glam::{Mat4, Vec3};

/// Ticks of a swing (`LivingEntity.getCurrentSwingDuration`).
pub(crate) const SWING_TICKS: f32 = 6.0;

/// The item's view-space pose (`applyItemArmTransform` then its display).
pub(crate) fn item_pose(display: Mat4, swing: f32, inverse_height: f32) -> Mat4 {
    Mat4::from_translation(Vec3::new(0.56, -0.52 - 0.6 * inverse_height, -0.72)) * item_swing_transform(swing) * display
}

fn item_swing_transform(swing: f32) -> Mat4 {
    let root = swing.sqrt();
    let x = -0.4 * (root * std::f32::consts::PI).sin();
    let y = 0.2 * (root * std::f32::consts::TAU).sin();
    let z = -0.2 * (swing * std::f32::consts::PI).sin();
    let y_rotation = (swing * swing * std::f32::consts::PI).sin();
    let xz_rotation = (root * std::f32::consts::PI).sin();
    Mat4::from_translation(Vec3::new(x, y, z))
        * Mat4::from_rotation_y((45.0 - 20.0 * y_rotation).to_radians())
        * Mat4::from_rotation_z((-20.0 * xz_rotation).to_radians())
        * Mat4::from_rotation_x((-80.0 * xz_rotation).to_radians())
        * Mat4::from_rotation_y((-45.0f32).to_radians())
}

/// The map's tilt as the player looks up or down (`calculateMapTilt`):
/// 1 looking ahead or up, 0 looking well down.
fn map_tilt(pitch: f32) -> f32 {
    let f = (1.0 - pitch / 45.0 + 0.1).clamp(0.0, 1.0);
    -(f * std::f32::consts::PI).cos() * 0.5 + 0.5
}

/// A filled map held in both hands (`renderTwoHandedMap` with nothing in
/// the other hand, fully raised), from map pixels (x right, y down, 128 a
/// side) to view space. `pitch` is Minecraft's, positive looking down.
pub(crate) fn map_pose(pitch: f32, swing: f32) -> Mat4 {
    use std::f32::consts::PI;
    let root = swing.sqrt();
    let lift = -0.2 * (swing * PI).sin();
    let push = -0.4 * (root * PI).sin();
    let tilt = map_tilt(pitch);
    Mat4::from_translation(Vec3::new(0.0, -lift / 2.0, push))
        * Mat4::from_translation(Vec3::new(0.0, 0.04 - tilt * 0.5, -0.72))
        * Mat4::from_rotation_x((tilt * -85.0).to_radians())
        * Mat4::from_rotation_x(((root * PI).sin() * 20.0).to_radians())
        * Mat4::from_scale(Vec3::splat(2.0))
        // `renderMap`: turned to face the player, 0.38 across, from its
        // corner, a pixel a 128th.
        * Mat4::from_rotation_y(PI)
        * Mat4::from_rotation_z(PI)
        * Mat4::from_scale(Vec3::splat(0.38))
        * Mat4::from_translation(Vec3::new(-0.5, -0.5, 0.0))
        * Mat4::from_scale(Vec3::splat(1.0 / 128.0))
}

/// The atlas regions a map in hand is drawn from.
pub(crate) struct MapSprites {
    pub white: [f32; 4],
    pub background: [f32; 4],
    pub checkerboard: [f32; 4],
    pub player: [f32; 4],
    pub off_map: [f32; 4],
}

/// The holder's marker on its map (`MapItemSavedData.addDecoration`): the
/// arrow at its place, turned in sixteenths, or on the edge when it is off
/// the map but within 320 blocks; none farther. Pixel position (from the
/// map's corner), sixteenths and whether it is the arrow.
pub(crate) fn map_marker(map: &minecraft_terrain::filled_map::MapData, x: f64, z: f64, yaw: f32) -> Option<([f32; 2], u8, bool)> {
    let per = f64::from(1u32 << map.scale);
    let rel_x = ((x - f64::from(map.centre.0)) / per) as f32;
    let rel_z = ((z - f64::from(map.centre.1)) / per) as f32;
    let inside = (-63.0..=63.0).contains(&rel_x) && (-63.0..=63.0).contains(&rel_z);
    let to_byte = |v: f32| ((v * 2.0 + 0.5) as i32).clamp(-128, 127) as f32;
    if inside {
        let turn = if yaw < 0.0 { yaw - 8.0 } else { yaw + 8.0 };
        let rot = ((turn * 16.0 / 360.0) as i32 & 15) as u8;
        return Some(([to_byte(rel_x) / 2.0 + 64.0, to_byte(rel_z) / 2.0 + 64.0], rot, true));
    }
    if rel_x.abs() >= 320.0 || rel_z.abs() >= 320.0 {
        return None;
    }
    let edge = |v: f32| if v <= -63.0 { -128.0 } else if v >= 63.0 { 127.0 } else { to_byte(v) };
    Some(([edge(rel_x) / 2.0 + 64.0, edge(rel_z) / 2.0 + 64.0], 0, false))
}

/// The vertices and indices of a map in hand: its paper, its pixels (runs
/// of one colour along each row as one quad, coloured over white) and the
/// holder's marker, posed by `pose`, lit by `light` (sky, block).
pub(crate) fn map_mesh(
    map: Option<&minecraft_terrain::filled_map::MapData>,
    marker: Option<([f32; 2], u8, bool)>,
    pose: Mat4,
    sprites: &MapSprites,
    light: [f32; 2],
) -> (Vec<minecraft_terrain::mesh::Vertex>, Vec<u32>) {
    use minecraft_terrain::filled_map::{SIZE, rgb};
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut quad = |corners: [[f32; 3]; 4], uv: [[f32; 2]; 4], colour: [f32; 3]| {
        let start = vertices.len() as u32;
        for (corner, uv) in corners.into_iter().zip(uv) {
            vertices.push(minecraft_terrain::mesh::Vertex {
                position: pose.transform_point3(Vec3::from_array(corner)).to_array(),
                uv,
                color: [colour[0], colour[1], colour[2], 1.0],
                sky_light: light[0],
                block_light: light[1],
            });
        }
        indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
    };
    let corners_uv = |[u0, v0, u1, v1]: [f32; 4]| [[u0, v1], [u1, v1], [u1, v0], [u0, v0]];
    let paper = if map.is_some() { sprites.checkerboard } else { sprites.background };
    quad(
        [[-7.0, 135.0, 0.0], [135.0, 135.0, 0.0], [135.0, -7.0, 0.0], [-7.0, -7.0, 0.0]],
        corners_uv(paper),
        [1.0; 3],
    );
    let Some(map) = map else {
        return (vertices, indices);
    };
    let [u0, v0, u1, v1] = sprites.white;
    let white = [(u0 + u1) / 2.0, (v0 + v1) / 2.0];
    for row in 0..SIZE {
        let mut x = 0;
        while x < SIZE {
            let packed = map.colours[row * SIZE + x];
            let mut end = x + 1;
            while end < SIZE && map.colours[row * SIZE + end] == packed {
                end += 1;
            }
            if let Some(colour) = rgb(packed) {
                let (x0, x1, y0, y1) = (x as f32, end as f32, row as f32, row as f32 + 1.0);
                quad(
                    [[x0, y1, -0.2], [x1, y1, -0.2], [x1, y0, -0.2], [x0, y0, -0.2]],
                    [white; 4],
                    colour.map(|c| f32::from(c) / 255.0),
                );
            }
            x = end;
        }
    }
    if let Some(([mx, mz], rot, arrow)) = marker {
        let turn = Mat4::from_translation(Vec3::new(mx, mz, -0.4))
            * Mat4::from_rotation_z((f32::from(rot) * 360.0 / 16.0).to_radians())
            * Mat4::from_scale(Vec3::new(4.0, 4.0, 3.0))
            * Mat4::from_translation(Vec3::new(-0.125, 0.125, 0.0));
        let at = |x: f32, y: f32| turn.transform_point3(Vec3::new(x, y, 0.0)).to_array();
        let [u0, v0, u1, v1] = if arrow { sprites.player } else { sprites.off_map };
        quad(
            [at(-1.0, 1.0), at(1.0, 1.0), at(1.0, -1.0), at(-1.0, -1.0)],
            [[u0, v0], [u1, v0], [u1, v1], [u0, v1]],
            [1.0; 3],
        );
    }
    (vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use minecraft_terrain::filled_map::{Brightness, GRASS, MapData, WATER, packed};

    fn sprites() -> MapSprites {
        MapSprites {
            white: [0.0, 0.0, 0.1, 0.1],
            background: [0.1, 0.0, 0.2, 0.1],
            checkerboard: [0.2, 0.0, 0.3, 0.1],
            player: [0.3, 0.0, 0.4, 0.1],
            off_map: [0.4, 0.0, 0.5, 0.1],
        }
    }

    /// Looking ahead the map is held low; looking down it rises before the
    /// eye, in front of it, facing it, about three quarters of a block
    /// across.
    #[test]
    fn a_map_is_held_low_looking_ahead_and_raised_looking_down() {
        let centre = |pitch: f32| map_pose(pitch, 0.0).transform_point3(Vec3::new(64.0, 64.0, 0.0));
        let ahead = centre(0.0);
        let down = centre(60.0);
        assert!(ahead.y < down.y, "ahead {ahead:?}, down {down:?}");
        assert!(ahead.z < 0.0 && down.z < 0.0, "in front of the eye");
        let pose = map_pose(60.0, 0.0);
        let left = pose.transform_point3(Vec3::new(0.0, 64.0, 0.0));
        let right = pose.transform_point3(Vec3::new(128.0, 64.0, 0.0));
        assert!((right.x - left.x - 0.76).abs() < 1e-4, "{}", right.x - left.x);
        // Its first row is at the top, as the map's north.
        let top = pose.transform_point3(Vec3::new(64.0, 0.0, 0.0));
        let bottom = pose.transform_point3(Vec3::new(64.0, 128.0, 0.0));
        assert!(top.y > bottom.y);
        // Pixels lie nearer the eye than the paper under them.
        let paper = pose.transform_point3(Vec3::new(64.0, 64.0, 0.0));
        let pixel = pose.transform_point3(Vec3::new(64.0, 64.0, -0.2));
        assert!(pixel.length() < paper.length());
    }

    #[test]
    fn a_map_s_pixels_are_drawn_a_run_of_one_colour_at_a_time() {
        let empty = map_mesh(None, None, Mat4::IDENTITY, &sprites(), [15.0, 0.0]);
        assert_eq!((empty.0.len(), empty.1.len()), (4, 6), "the paper only");
        let mut map = MapData::new(0.0, 0.0, 0);
        let grass = packed(GRASS, Brightness::Normal);
        for x in 0..128 {
            map.set(x, 0, grass);
        }
        map.set(5, 1, grass);
        map.set(6, 1, packed(WATER, Brightness::High));
        let (vertices, indices) = map_mesh(Some(&map), None, Mat4::IDENTITY, &sprites(), [15.0, 0.0]);
        // Paper, the first row whole, and two one-pixel runs.
        assert_eq!(vertices.len(), 4 * 4);
        assert_eq!(indices.len(), 4 * 6);
        let row = &vertices[4..8];
        assert_eq!(row[1].position[0] - row[0].position[0], 128.0);
        assert_eq!(row[0].uv, [0.05, 0.05], "the white texel");
        assert!((row[0].color[1] - 153.0 / 255.0).abs() < 1e-6, "grass, normal");
        let water = &vertices[12];
        assert_eq!(water.color[..3], [64.0 / 255.0, 64.0 / 255.0, 1.0]);
        assert_eq!(water.position[0], 6.0);
    }

    #[test]
    fn the_holder_s_marker_turns_with_it_and_waits_on_the_edge_off_the_map() {
        let map = MapData::new(0.0, 0.0, 0);
        // Facing south (yaw 0): the arrow as drawn; west (90): a quarter on.
        assert_eq!(map_marker(&map, 10.0, -20.0, 0.0), Some(([74.0, 44.5], 0, true)));
        assert_eq!(map_marker(&map, 0.0, 0.0, 90.0).map(|m| m.1), Some(4));
        assert_eq!(map_marker(&map, 0.0, 0.0, -90.0).map(|m| m.1), Some(12));
        assert_eq!(map_marker(&map, 0.0, 0.0, 180.0).map(|m| m.1), Some(8));
        // Off the map: on its east edge, not an arrow.
        assert_eq!(map_marker(&map, 100.0, 0.0, 0.0), Some(([127.5, 64.0], 0, false)));
        assert_eq!(map_marker(&map, 400.0, 0.0, 0.0), None);
        let with = map_mesh(Some(&map), map_marker(&map, 0.0, 0.0, 0.0), Mat4::IDENTITY, &sprites(), [15.0, 0.0]);
        assert_eq!(with.0.len(), 8, "paper and marker");
        assert_eq!(with.0[4].uv, [0.3, 0.0]);
    }
}
