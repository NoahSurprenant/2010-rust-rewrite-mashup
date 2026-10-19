//! Test settings of the Minecraft map, from the environment: where the
//! world's first player stands, and whether mobs come of themselves. The
//! approved scenarios set them so a run plays the same world the same way;
//! a player never needs them. (The seed is `IW4L_MINECRAFT_SEED`.)

/// `IW4L_MINECRAFT_SPAWN="x y z"`: the feet of the world's first player, in
/// Minecraft block coordinates (`90.5 130 -30.5`), instead of vanilla's
/// spot picked at random within ten blocks of the world spawn. The map's
/// origin sits there, so everything placed relative to the spawn has a
/// known place. Only for a new world: a kept one keeps the spot it was
/// made with, and a client takes its host's.
pub(crate) const SPAWN_ENV: &str = "IW4L_MINECRAFT_SPAWN";

/// `IW4L_MINECRAFT_MOBS=off`: no mobs but summoned ones (`mc summon`):
/// natural spawning is off and the creatures chunk generation places are
/// left out. `on` (or unset) is the game as it is.
pub(crate) const MOBS_ENV: &str = "IW4L_MINECRAFT_MOBS";

/// The spawn `IW4L_MINECRAFT_SPAWN` sets, if it is set; a value that is not
/// three numbers is ignored, loudly.
pub(crate) fn spawn() -> Option<[f64; 3]> {
    let value = std::env::var(SPAWN_ENV).ok();
    match parse_spawn(value.as_deref()) {
        Ok(spawn) => spawn,
        Err(error) => {
            diag::warn!(World, "{SPAWN_ENV}: {error}; vanilla's spawn instead");
            None
        }
    }
}

/// Whether mobs come of themselves (`IW4L_MINECRAFT_MOBS`); a value that is
/// neither `on` nor `off` leaves them on, loudly.
pub(crate) fn mobs() -> bool {
    let value = std::env::var(MOBS_ENV).ok();
    parse_mobs(value.as_deref()).unwrap_or_else(|error| {
        diag::warn!(World, "{MOBS_ENV}: {error}; mobs on");
        true
    })
}

pub(crate) fn parse_spawn(value: Option<&str>) -> Result<Option<[f64; 3]>, String> {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let numbers: Vec<f64> = value
        .split_whitespace()
        .map(|n| n.parse::<f64>().ok().filter(|n| n.is_finite()))
        .collect::<Option<_>>()
        .ok_or_else(|| format!("`{value}` is not three numbers"))?;
    match numbers.as_slice() {
        &[x, y, z] => Ok(Some([x, y, z])),
        _ => Err(format!("`{value}` is not three numbers")),
    }
}

pub(crate) fn parse_mobs(value: Option<&str>) -> Result<bool, String> {
    match value.map(str::trim) {
        None | Some("" | "on") => Ok(true),
        Some("off") => Ok(false),
        Some(other) => Err(format!("`{other}` is neither on nor off")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spawn_is_three_numbers() {
        assert_eq!(parse_spawn(None), Ok(None));
        assert_eq!(parse_spawn(Some("  ")), Ok(None));
        assert_eq!(
            parse_spawn(Some("90.5 130 -30.5")),
            Ok(Some([90.5, 130.0, -30.5]))
        );
        assert_eq!(parse_spawn(Some(" 1  2\t3 ")), Ok(Some([1.0, 2.0, 3.0])));
        for bad in ["1 2", "1 2 3 4", "x 2 3", "1 2 inf", "1,2,3"] {
            assert!(parse_spawn(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn mobs_are_on_unless_off() {
        assert_eq!(parse_mobs(None), Ok(true));
        assert_eq!(parse_mobs(Some("")), Ok(true));
        assert_eq!(parse_mobs(Some("on")), Ok(true));
        assert_eq!(parse_mobs(Some("off")), Ok(false));
        assert_eq!(parse_mobs(Some(" off ")), Ok(false));
        assert!(parse_mobs(Some("peaceful")).is_err());
    }
}
