use crate::anim::remote_body::{CpuBodyGeom, CpuSurfMeta};
use bevy::prelude::*;
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Deserialize)]
pub struct ReferenceBone {
    name: String,
    bind: [f32; 16],
    inverse_bind: [f32; 16],
}
pub fn reference() -> Option<&'static Vec<ReferenceBone>> {
    static RIG: OnceLock<Option<Vec<ReferenceBone>>> = OnceLock::new();
    RIG.get_or_init(|| {
        let root = std::env::var_os("IW4L_SKATE_ASSETS")?;
        let data = std::fs::read(std::path::Path::new(&root).join("rig.json")).ok()?;
        serde_json::from_slice(&data).ok()
    })
    .as_ref()
}

fn mapping(name: &str) -> Option<(&'static str, Option<(&'static str, &'static str)>)> {
    Some(match name {
        "j_mainroot" | "pelvis" => ("HIPS", Some(("j_spinelower", "SPINE"))),
        "j_spinelower" => ("SPINE", Some(("j_spineupper", "SPINE1"))),
        "j_spineupper" => ("SPINE1", Some(("j_spine4", "SPINE3"))),
        "j_spine4" => ("SPINE3", Some(("j_neck", "NECK"))),
        "j_neck" => ("NECK", Some(("j_head", "HEAD"))),
        "j_head" => ("HEAD", None),
        "j_clavicle_le" => ("LEFTSHOULDER", Some(("j_shoulder_le", "LEFTARM"))),
        "j_shoulder_le" => ("LEFTARM", Some(("j_elbow_le", "LEFTFOREARM"))),
        "j_elbow_le" => ("LEFTFOREARM", Some(("j_wrist_le", "LEFTHAND"))),
        "j_wrist_le" => ("LEFTHAND", None),
        "j_clavicle_ri" => ("RIGHTSHOULDER", Some(("j_shoulder_ri", "RIGHTARM"))),
        "j_shoulder_ri" => ("RIGHTARM", Some(("j_elbow_ri", "RIGHTFOREARM"))),
        "j_elbow_ri" => ("RIGHTFOREARM", Some(("j_wrist_ri", "RIGHTHAND"))),
        "j_wrist_ri" => ("RIGHTHAND", None),
        "j_hip_le" => ("LEFTUPLEG", Some(("j_knee_le", "LEFTLEG"))),
        "j_knee_le" => ("LEFTLEG", Some(("j_ankle_le", "LEFTFOOT"))),
        "j_ankle_le" => ("LEFTFOOT", Some(("j_ball_le", "LEFTTOEBASE"))),
        "j_ball_le" => ("LEFTTOEBASE", None),
        "j_hip_ri" => ("RIGHTUPLEG", Some(("j_knee_ri", "RIGHTLEG"))),
        "j_knee_ri" => ("RIGHTLEG", Some(("j_ankle_ri", "RIGHTFOOT"))),
        "j_ankle_ri" => ("RIGHTFOOT", Some(("j_ball_ri", "RIGHTTOEBASE"))),
        "j_ball_ri" => ("RIGHTTOEBASE", None),
        _ => return None,
    })
}

fn convert(m: Mat4) -> Mat4 {
    let b = super::collision::basis();
    let mut out = b * m * b.inverse();
    out.w_axis = super::collision::from_skate(m.w_axis.truncate()).extend(1.);
    out
}

/// Align each soldier bind segment to the reference skater, then apply the
/// game's solved pose. Unmapped fingers, equipment and helper bones inherit
/// the nearest mapped ancestor, preserving their original local offsets.
pub fn pose(dobj: &xmodel_runtime::DObj, names: &[String], bones: &[Mat4]) -> Vec<Mat4> {
    let Some(reference) = reference() else {
        return dobj.bones.iter().map(|b| b.bind_world).collect();
    };
    let source = |name: &str| {
        reference
            .iter()
            .find(|b| b.name == name)
            .map(|b| convert(Mat4::from_cols_array(&b.bind)))
    };
    let mut skin = Vec::with_capacity(dobj.bones.len());
    for bone in &dobj.bones {
        if bone.model != 0 {
            // Attached models use model-local bind positions. Their root must
            // start at the posed attachment bone, not at its skinning delta.
            let attachment = bone.parent.and_then(|parent| {
                let transform = *skin.get(parent)?;
                Some(if dobj.bones[parent].model == bone.model {
                    transform
                } else {
                    transform * dobj.bones[parent].bind_world
                })
            });
            skin.push(attachment.unwrap_or(Mat4::IDENTITY));
            continue;
        }
        if matches!(bone.name.as_str(), "j_spine4" | "j_neck" | "j_head") {
            skin.push(
                bone.parent
                    .and_then(|parent| skin.get(parent).copied())
                    .unwrap_or(Mat4::IDENTITY),
            );
            continue;
        }
        let mapped = mapping(&bone.name).and_then(|(name, child)| {
            let bind = source(name)?;
            let index = names.iter().position(|n| n == name)?;
            let posed = convert(*bones.get(index)?);
            let from = bone.bind_world.w_axis.truncate();
            let to = bind.w_axis.truncate();
            let facing = Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2);
            let rotation = child
                .and_then(|(iw, skate)| {
                    let iw = dobj
                        .bones
                        .iter()
                        .find(|b| b.name == iw)?
                        .bind_world
                        .w_axis
                        .truncate();
                    let sk = source(skate)?.w_axis.truncate();
                    Some(
                        Quat::from_rotation_arc(
                            (facing * (iw - from)).try_normalize()?,
                            (sk - to).try_normalize()?,
                        ) * facing,
                    )
                })
                .unwrap_or_else(|| {
                    // Terminal joints use their parent's alignment rather than a
                    // new arbitrary roll, keeping wrists and head seams attached.
                    bone.parent
                        .and_then(|p| dobj.bones.get(p))
                        .and_then(|p| mapping(&p.name))
                        .and_then(|(pn, _)| {
                            let s = source(pn)?.w_axis.truncate();
                            let p = p_from(dobj, bone.parent)?;
                            Some(
                                Quat::from_rotation_arc(
                                    (facing * (from - p)).try_normalize()?,
                                    (to - s).try_normalize()?,
                                ) * facing,
                            )
                        })
                        .unwrap_or(facing)
                });
            let fit = Mat4::from_rotation_translation(rotation, to - rotation * from);
            Some(posed * bind.inverse() * fit)
        });
        skin.push(
            mapped
                .or_else(|| bone.parent.and_then(|p| skin.get(p).copied()))
                .unwrap_or(Mat4::IDENTITY),
        );
    }
    dobj.bones
        .iter()
        .zip(skin)
        .map(|(b, s)| s * b.bind_world)
        .collect()
}
fn p_from(dobj: &xmodel_runtime::DObj, index: Option<usize>) -> Option<Vec3> {
    Some(dobj.bones.get(index?)?.bind_world.w_axis.truncate())
}

pub fn board(names: &[String], bones: &[Mat4], geom: &mut CpuBodyGeom) -> Result<(), String> {
    let model = assets::bot_model::local_skate_board().ok_or("missing board model")?;
    let reference = reference().ok_or("missing board bind pose")?;
    let rb = Mat4::from_cols(Vec4::X, -Vec4::Z, Vec4::Y, Vec4::W);
    let matrices: Vec<_> = model
        .joints
        .iter()
        .map(|j| {
            let i = names
                .iter()
                .position(|n| n == &j.target)
                .ok_or("missing Skate joint")?;
            let reference = reference
                .iter()
                .find(|b| b.name == j.target)
                .ok_or("missing reference joint")?;
            let bone = bones.get(i).ok_or("missing Skate bone")?;
            Ok(*bone * rb * Mat4::from_cols_array(&reference.inverse_bind))
        })
        .collect::<Result<_, String>>()?;
    for surface in &model.surfaces {
        let vertex_base = geom.packed.len() as u32;
        let index_start = geom.indices.len() as u32;
        for v in &surface.vertices {
            let mut pos = Vec3::ZERO;
            let mut normal = Vec3::ZERO;
            for i in 0..4 {
                let m = matrices[v.joints[i]];
                pos += m.transform_point3(Vec3::from_array(v.position)) * v.weights[i];
                normal += m.transform_vector3(Vec3::from_array(v.normal)) * v.weights[i];
            }
            pos = super::collision::from_skate(pos);
            normal = super::collision::basis()
                .transform_vector3(normal)
                .normalize_or(Vec3::Z);
            let mut packed = [0; asset_iw4::size::GFX_PACKED_VERTEX];
            for (i, f) in pos.to_array().iter().enumerate() {
                packed[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
            }
            packed[12..16].copy_from_slice(&1f32.to_le_bytes());
            packed[16..20].fill(255);
            packed[20..24].copy_from_slice(&v.uv.to_le_bytes());
            let pack = |n: Vec3| {
                [
                    (n.x * 127. + 127.5) as u8,
                    (n.y * 127. + 127.5) as u8,
                    (n.z * 127. + 127.5) as u8,
                    63,
                ]
            };
            packed[24..28].copy_from_slice(&pack(normal));
            packed[28..32].copy_from_slice(&pack(normal.any_orthonormal_vector()));
            geom.packed.push(packed);
        }
        geom.indices
            .extend(surface.indices.iter().map(|i| vertex_base + i));
        geom.surfaces.push(CpuSurfMeta {
            index_start,
            index_count: surface.indices.len() as u32,
            name: Some(surface.material.clone()),
        });
    }
    geom.decoded_n = geom.packed.len();
    Ok(())
}

/// The Skate 3 bones a soldier's body follows (`mapping`'s targets).
pub const BODY_BONES: [&str; 22] = [
    "HIPS",
    "SPINE",
    "SPINE1",
    "SPINE3",
    "NECK",
    "HEAD",
    "LEFTSHOULDER",
    "LEFTARM",
    "LEFTFOREARM",
    "LEFTHAND",
    "RIGHTSHOULDER",
    "RIGHTARM",
    "RIGHTFOREARM",
    "RIGHTHAND",
    "LEFTUPLEG",
    "LEFTLEG",
    "LEFTFOOT",
    "LEFTTOEBASE",
    "RIGHTUPLEG",
    "RIGHTLEG",
    "RIGHTFOOT",
    "RIGHTTOEBASE",
];

/// What `pose` takes from a skater for a body, without the body: every bone
/// the body follows, posed (finite) and in the reference rig. A game that
/// draws nothing checks a skater this far; one that draws goes on to
/// `pose` its soldier with it.
pub fn body_bones(names: &[String], bones: &[Mat4], reference: &[String]) -> Result<usize, String> {
    for name in BODY_BONES {
        let index = names
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| format!("skater has no `{name}`"))?;
        let bone = bones
            .get(index)
            .ok_or_else(|| format!("skater bone `{name}` missing"))?;
        if !bone.is_finite() {
            return Err(format!("skater bone `{name}` not finite"));
        }
        if !reference.iter().any(|r| r == name) {
            return Err(format!("reference rig has no `{name}`"));
        }
    }
    Ok(BODY_BONES.len())
}

/// The reference rig's bone names.
pub fn reference_names() -> Vec<String> {
    reference()
        .map(|rig| rig.iter().map(|b| b.name.clone()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn every_mapped_body_bone_is_listed() {
        for name in BODY_BONES {
            assert!(
                [
                    "j_mainroot", "j_spinelower", "j_spineupper", "j_spine4", "j_neck", "j_head",
                    "j_clavicle_le", "j_shoulder_le", "j_elbow_le", "j_wrist_le", "j_clavicle_ri",
                    "j_shoulder_ri", "j_elbow_ri", "j_wrist_ri", "j_hip_le", "j_knee_le",
                    "j_ankle_le", "j_ball_le", "j_hip_ri", "j_knee_ri", "j_ankle_ri", "j_ball_ri",
                ]
                .iter()
                .any(|iw| mapping(iw).is_some_and(|(skate, _)| skate == name)),
                "{name}"
            );
        }
    }

    #[test]
    fn the_wire_carries_every_body_bone() {
        for name in BODY_BONES {
            assert!(net::skate::BONES.contains(&name), "{name}");
        }
    }

    #[test]
    fn a_full_skater_poses_a_body() {
        let all = names(&BODY_BONES);
        let bones = vec![Mat4::IDENTITY; all.len()];
        assert_eq!(body_bones(&all, &bones, &all), Ok(22));
    }

    #[test]
    fn a_skater_missing_a_bone_or_posed_wrong_does_not() {
        let all = names(&BODY_BONES);
        let short = names(&BODY_BONES[..21]);
        assert!(body_bones(&short, &vec![Mat4::IDENTITY; 21], &all).is_err());
        let mut bones = vec![Mat4::IDENTITY; all.len()];
        bones[3] = Mat4::from_translation(Vec3::splat(f32::NAN));
        assert!(body_bones(&all, &bones, &all).unwrap_err().contains("SPINE3"));
        assert!(body_bones(&all, &vec![Mat4::IDENTITY; 22], &short).is_err());
    }
}
