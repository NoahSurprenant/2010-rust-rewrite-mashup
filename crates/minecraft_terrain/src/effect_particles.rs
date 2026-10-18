//! What a player sees of a splash potion breaking and of a creeper's blast:
//! the hundred spell particles in the potion's colour that the splash level
//! event flings out (`SpellParticle`, the instant kind for an instant
//! potion), and the big puffs of the blast's emitter
//! (`HugeExplosionSeedParticle`: six `HugeExplosionParticle`s a tick for
//! eight ticks, within four blocks). Particles draw from their own unseeded
//! randoms in vanilla, so these match in kind, not particle for particle.
use crate::{
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh, Vertex},
    pack::ResourceId,
};
use glam::{DVec3, Vec2, Vec3};
use minecraftoss_player::rng::LegacyRandom;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Sprite {
    Effect,
    InstantEffect,
    Explosion,
}

#[derive(Clone, Debug)]
struct Particle {
    sprite: Sprite,
    position: DVec3,
    previous: DVec3,
    velocity: DVec3,
    color: [f32; 3],
    quad_size: f32,
    age: i32,
    lifetime: i32,
    /// Ticks before it appears.
    delay: i32,
}

impl Particle {
    /// `Particle.tick` for a particle without physics: `SpellParticle`'s
    /// friction 0.96 and gravity -0.1 (it rises); the blast's puffs stand.
    fn tick(&mut self) -> bool {
        if self.delay > 0 {
            self.delay -= 1;
            return true;
        }
        self.previous = self.position;
        let prior = self.age;
        self.age += 1;
        if prior >= self.lifetime {
            return false;
        }
        if self.sprite != Sprite::Explosion {
            self.velocity.y -= 0.04 * f64::from(-0.1_f32);
            self.position += self.velocity;
            self.velocity *= f64::from(0.96_f32);
        }
        true
    }

    fn sprite_id(&self) -> String {
        let frames = if self.sprite == Sprite::Explosion {
            16
        } else {
            8
        };
        let index = (self.age * (frames - 1) / self.lifetime.max(1)).clamp(0, frames - 1);
        match self.sprite {
            Sprite::Effect => format!("minecraft:particle/effect_{}", frames - 1 - index),
            Sprite::InstantEffect => {
                format!("minecraft:particle/instant_effect_{}", frames - 1 - index)
            }
            Sprite::Explosion => format!("minecraft:particle/explosion_{index}"),
        }
    }
}

/// A client's splash and blast particles.
pub struct EffectParticles {
    particles: Vec<Particle>,
    random: LegacyRandom,
}

impl Default for EffectParticles {
    fn default() -> Self {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        Self {
            particles: Vec::new(),
            random: LegacyRandom::new(seed ^ 0x2002),
        }
    }
}

impl EffectParticles {
    /// The splash level event at `at` in `color` (RGB): each particle starts
    /// a little out along its way, 0.3 up, with a random power of up to four
    /// (`Particle.setPower`) on the base particle's random direction.
    pub fn splash(&mut self, at: DVec3, color: u32, instant: bool) {
        let rgb = [(color >> 16) & 255, (color >> 8) & 255, color & 255].map(|c| c as f32 / 255.0);
        for _ in 0..100 {
            let r = &mut self.random;
            let power = r.next_double() * 4.0;
            let angle = r.next_double() * std::f64::consts::PI * 2.0;
            let (xs, zs) = (angle.cos() * power, angle.sin() * power);
            let ys = 0.01 + r.next_double() * 0.5;
            // `Particle(level, x, y, z, xa, ya, za)` with `SpellParticle`'s
            // own sideways draw.
            let mut v = DVec3::new(
                0.5 - r.next_double() + (r.next_double() * 2.0 - 1.0) * 0.4,
                ys + (r.next_double() * 2.0 - 1.0) * 0.4,
                0.5 - r.next_double() + (r.next_double() * 2.0 - 1.0) * 0.4,
            );
            let speed = (r.next_double() + r.next_double() + 1.0) * 0.15;
            v = v.normalize_or_zero() * speed * 0.4 + DVec3::Y * 0.1;
            v.y *= 0.2;
            let v = DVec3::new(v.x * power, (v.y - 0.1) * power + 0.1, v.z * power);
            let shade = 0.75 + r.next_float() * 0.25;
            let quad_size = 0.1 * (r.next_float() * 0.5 + 0.5) * 2.0 * 0.75;
            let lifetime = (8.0 / (r.next_double() * 0.8 + 0.2)) as i32;
            let position = at + DVec3::new(xs * 0.1, 0.3, zs * 0.1);
            self.particles.push(Particle {
                sprite: if instant {
                    Sprite::InstantEffect
                } else {
                    Sprite::Effect
                },
                position,
                previous: position,
                velocity: v,
                color: rgb.map(|c| c * shade),
                quad_size,
                age: 0,
                lifetime,
                delay: 0,
            });
        }
    }

    /// A blast of `radius` at `at`: the emitter's puffs for a blast of two
    /// or more, one puff for a smaller one.
    pub fn explosion(&mut self, at: DVec3, radius: f32) {
        let ticks = if radius >= 2.0 { 8 } else { 1 };
        let per_tick = if radius >= 2.0 { 6 } else { 1 };
        for tick in 0..ticks {
            for _ in 0..per_tick {
                let r = &mut self.random;
                let offset = if radius >= 2.0 {
                    DVec3::new(
                        r.next_double() - r.next_double(),
                        r.next_double() - r.next_double(),
                        r.next_double() - r.next_double(),
                    ) * 4.0
                } else {
                    DVec3::ZERO
                };
                let lifetime = 6 + r.next_int(4) as i32;
                let gray = r.next_float() * 0.6 + 0.4;
                let quad_size = 2.0 * (1.0 - r.next_float() * 0.5);
                let position = at + offset;
                self.particles.push(Particle {
                    sprite: Sprite::Explosion,
                    position,
                    previous: position,
                    velocity: DVec3::ZERO,
                    color: [gray; 3],
                    quad_size,
                    age: 0,
                    lifetime,
                    delay: tick,
                });
            }
        }
    }

    pub fn tick(&mut self) {
        self.particles.retain_mut(Particle::tick);
    }

    /// Camera-facing quads with the sprite for each one's age; the blast's
    /// puffs at full brightness.
    pub fn append_mesh(
        &self,
        mesh: &mut ChunkMesh,
        atlas: &Atlas,
        forward: Vec3,
        partial: f32,
        light: &SkyLight,
    ) {
        let partial = f64::from(partial.clamp(0.0, 1.0));
        let right = forward.cross(Vec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        for particle in self.particles.iter().filter(|p| p.delay == 0) {
            let Ok(sprite) = ResourceId::parse(&particle.sprite_id()) else {
                continue;
            };
            if !atlas.contains(&sprite) {
                continue;
            }
            let [u0, v0, u1, v1] = atlas.region(&sprite);
            let center = particle.previous.lerp(particle.position, partial);
            let cell = (
                center.x.floor() as i32,
                center.y.floor() as i32,
                center.z.floor() as i32,
            );
            let (sky, block) = if particle.sprite == Sprite::Explosion {
                (15.0, 15.0)
            } else {
                (light.get(cell) as f32, light.get_block(cell) as f32)
            };
            let center = center.as_vec3();
            let [r, g, b] = particle.color;
            let start = mesh.vertices.len() as u32;
            for (corner, uv) in [
                (Vec2::new(-1.0, -1.0), [u0, v1]),
                (Vec2::new(-1.0, 1.0), [u0, v0]),
                (Vec2::new(1.0, 1.0), [u1, v0]),
                (Vec2::new(1.0, -1.0), [u1, v1]),
            ] {
                let point = center + (right * corner.x + up * corner.y) * particle.quad_size;
                mesh.vertices.push(Vertex {
                    position: point.to_array(),
                    uv,
                    color: [r, g, b, 1.0],
                    sky_light: sky,
                    block_light: block,
                });
            }
            mesh.indices.extend_from_slice(&[
                start,
                start + 2,
                start + 1,
                start,
                start + 3,
                start + 2,
            ]);
            mesh.faces += 1;
        }
    }

    pub fn len(&self) -> usize {
        self.particles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.particles.is_empty()
    }
}
