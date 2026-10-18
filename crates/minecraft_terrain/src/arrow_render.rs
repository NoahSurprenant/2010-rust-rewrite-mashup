//! Arrows as `ArrowRenderer` draws them: two crossed planes along the
//! arrow and the square of its fletching at the back, from
//! `entity/projectiles/arrow` (or `tipped_arrow`), 32 by 32: a plane takes
//! the top 16 by 5 texels, the back the 5 by 5 below. Model units are
//! 0.05625 of a block, the arrow set back four units from where it is.
use crate::{
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh, Vertex},
    pack::ResourceId,
};
use glam::{DVec3, Vec3};

const UNIT: f32 = 0.05625;

/// The unit vector an arrow points along, from its yaw and pitch
/// (`Projectile.updateRotation`'s angles).
pub fn direction(y_rot: f32, x_rot: f32) -> Vec3 {
    let (yaw, pitch) = (y_rot.to_radians(), x_rot.to_radians());
    Vec3::new(
        yaw.sin() * pitch.cos(),
        pitch.sin(),
        yaw.cos() * pitch.cos(),
    )
}

/// Appends each arrow: where it is, its yaw and pitch, and whether it is
/// tipped.
pub fn append_arrows(
    mesh: &mut ChunkMesh,
    arrows: impl IntoIterator<Item = (DVec3, f32, f32, bool)>,
    atlas: &Atlas,
    light: &SkyLight,
) {
    let (Ok(plain), Ok(tipped)) = (
        ResourceId::parse("minecraft:entity/projectiles/arrow"),
        ResourceId::parse("minecraft:entity/projectiles/tipped_arrow"),
    ) else {
        return;
    };
    for (at, y_rot, x_rot, is_tipped) in arrows {
        let texture = if is_tipped && atlas.contains(&tipped) {
            &tipped
        } else {
            &plain
        };
        if !atlas.contains(texture) {
            continue;
        }
        let [u0, v0, u1, v1] = atlas.region(texture);
        let uv = |u: f32, v: f32| [u0 + (u1 - u0) * u, v0 + (v1 - v0) * v];
        let cell = (
            at.x.floor() as i32,
            at.y.floor() as i32,
            at.z.floor() as i32,
        );
        let (sky, block) = (light.get(cell) as f32, light.get_block(cell) as f32);
        let forward = direction(y_rot, x_rot);
        let side = forward.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
        let up = side.cross(forward);
        let origin = at.as_vec3();
        let point = |x: f32, lateral: Vec3, y: f32| {
            origin + forward * ((x - 4.0) * UNIT) + lateral * (y * UNIT)
        };
        let mut quad = |corners: [(Vec3, [f32; 2]); 4]| {
            let start = mesh.vertices.len() as u32;
            for (position, uv) in corners {
                mesh.vertices.push(Vertex {
                    position: position.to_array(),
                    uv,
                    color: [1.0; 4],
                    sky_light: sky,
                    block_light: block,
                });
            }
            mesh.indices.extend_from_slice(&[
                start,
                start + 1,
                start + 2,
                start,
                start + 2,
                start + 3,
            ]);
            mesh.faces += 1;
        };
        // The two planes, a quarter turn apart and an eighth off the side.
        for lateral in [(side + up).normalize(), (side - up).normalize()] {
            quad([
                (point(-8.0, lateral, -2.0), uv(0.0, 0.0)),
                (point(8.0, lateral, -2.0), uv(0.5, 0.0)),
                (point(8.0, lateral, 2.0), uv(0.5, 0.15625)),
                (point(-8.0, lateral, 2.0), uv(0.0, 0.15625)),
            ]);
        }
        let back = |a: f32, b: f32| {
            origin + forward * (-11.0 * UNIT) + side * (a * UNIT) + up * (b * UNIT)
        };
        quad([
            (back(-2.0, -2.0), uv(0.0, 0.15625)),
            (back(-2.0, 2.0), uv(0.15625, 0.15625)),
            (back(2.0, 2.0), uv(0.15625, 0.3125)),
            (back(2.0, -2.0), uv(0.0, 0.3125)),
        ]);
    }
}
