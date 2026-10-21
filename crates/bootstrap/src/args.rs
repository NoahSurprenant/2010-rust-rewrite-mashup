use std::path::PathBuf;

use render::diag::acceptance::{ACCEPTANCE_ENV, ACCEPTANCE_FLAG, ACCEPTANCE_MAPS};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcceptanceLaunch {
    pub dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchMode {
    Menu,
    Map(String),
    Dedicated(String),
    ExportGltf(String),
    Play {
        name: String,
        zone_override: Option<String>,
    },
}

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cli {
    pub mode: LaunchMode,
    pub acceptance: Option<AcceptanceLaunch>,
    /// No window, no renderer, no sound (`--headless`, or `IW4L_HEADLESS=1`):
    /// the game plays (simulation, networking, the Minecraft world, the
    /// console and its input) and draws nothing, as a dedicated server
    /// always does.
    pub headless: bool,
}

/// `iw4l --headless menu | map <zone>`: a player's game that draws nothing.
pub const HEADLESS_FLAG: &str = "--headless";
/// The same as `--headless`, from the environment (`1`, `true` or `on`).
pub const HEADLESS_ENV: &str = "IW4L_HEADLESS";

pub fn parse_cli(args: impl Iterator<Item = String>) -> Result<Cli, String> {
    let (args, acceptance) = parse_acceptance_flag(args)?;
    let args = console::strip_cmds_flag(args.into_iter());
    let (args, flag) = take_headless_flag(args);
    let headless = flag || headless_env(std::env::var(HEADLESS_ENV).ok().as_deref());
    let mode = parse_launch_args(args.into_iter())?;
    check_headless(&mode, acceptance.is_some(), headless)?;
    Ok(Cli {
        mode,
        acceptance,
        headless,
    })
}

/// The arguments without `--headless`, and whether it was there.
fn take_headless_flag(args: Vec<String>) -> (Vec<String>, bool) {
    let before = args.len();
    let rest: Vec<String> = args.into_iter().filter(|a| a != HEADLESS_FLAG).collect();
    let found = rest.len() != before;
    (rest, found)
}

fn headless_env(value: Option<&str>) -> bool {
    value.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "on"))
}

/// A headless game draws nothing, so nothing that is only a picture can be
/// asked of it.
fn check_headless(mode: &LaunchMode, acceptance: bool, headless: bool) -> Result<(), String> {
    if !headless {
        return Ok(());
    }
    if acceptance {
        return Err("--headless draws nothing: render acceptance needs a window".into());
    }
    match mode {
        LaunchMode::ExportGltf(_) => {
            Err("--headless is for a game: export-gltf opens no window".into())
        }
        _ => Ok(()),
    }
}

fn parse_acceptance_flag(
    args: impl Iterator<Item = String>,
) -> Result<(Vec<String>, Option<AcceptanceLaunch>), String> {
    let mut out = Vec::new();
    let mut acceptance = None;
    let mut iter = args.peekable();
    while let Some(arg) = iter.next() {
        if arg == ACCEPTANCE_FLAG {
            match iter.next() {
                Some(dir) if !dir.is_empty() && !dir.starts_with('-') => {
                    acceptance = Some(AcceptanceLaunch {
                        dir: PathBuf::from(dir),
                    });
                }
                _ => {
                    return Err(format!(
                        "usage: iw4l --render-acceptance <artifact-dir> map <zone>\n\
                         env alternative: {env}=<artifact-dir>\n\
                         expected maps later: {maps}",
                        env = ACCEPTANCE_ENV,
                        maps = ACCEPTANCE_MAPS.join(", ")
                    ));
                }
            }
        } else if let Some(dir) = arg.strip_prefix(&(ACCEPTANCE_FLAG.to_owned() + "=")) {
            if dir.is_empty() {
                return Err("empty --render-acceptance path".into());
            }
            acceptance = Some(AcceptanceLaunch {
                dir: PathBuf::from(dir),
            });
        } else {
            out.push(arg);
        }
    }
    if acceptance.is_none()
        && let Some(dir) = std::env::var_os(ACCEPTANCE_ENV)
    {
        acceptance = Some(AcceptanceLaunch {
            dir: PathBuf::from(dir),
        });
    }
    Ok((out, acceptance))
}

pub const USAGE: &str = "usage: iw4l [--headless] [--cmds '<script>'] map <zone> | dedicated <zone> | menu | play <demo>\n       iw4l export-gltf <zone>\n       iw4l dedicated-console '<command>'\n       iw4l --help | --version";

/// `iw4l --version`: the commit and the protocols a peer or a master must share.
pub fn version() -> String {
    let git = [
        option_env!("IW4L_GIT_HASH"),
        Some(env!("IW4L_BUILD_GIT_SHA")),
    ]
    .into_iter()
    .flatten()
    .find(|hash| !hash.is_empty())
    .unwrap_or("unknown");
    format!(
        "iw4l {git} protocol={} master_protocol={}",
        net::PROTOCOL_VERSION,
        master_protocol::PROTOCOL_VERSION
    )
}

pub fn parse_launch_args(mut args: impl Iterator<Item = String>) -> Result<LaunchMode, String> {
    match args.next().as_deref() {
        Some("--help") | Some("-h") => Err(USAGE.into()),
        Some("map") => {
            let Some(zone) = args.next().filter(|z| !z.is_empty()) else {
                return Err("usage: iw4l map <zone> [--cmds '<script>']".into());
            };
            Ok(LaunchMode::Map(zone))
        }
        Some("dedicated") => {
            let Some(zone) = args.next().filter(|z| !z.is_empty()) else {
                return Err("usage: iw4l dedicated <zone> [--cmds '<script>']".into());
            };
            Ok(LaunchMode::Dedicated(zone))
        }
        Some("export-gltf") => {
            let Some(zone) = args.next().filter(|zone| !zone.is_empty()) else {
                return Err("usage: iw4l export-gltf <zone>".into());
            };
            if args.next().is_some() {
                return Err("usage: iw4l export-gltf <zone>".into());
            }
            Ok(LaunchMode::ExportGltf(zone))
        }
        Some("menu") => {
            if args.next().is_some() {
                return Err("usage: iw4l menu [--cmds '<script>']".into());
            }
            Ok(LaunchMode::Menu)
        }
        Some("play") => parse_play_args(args),
        Some(other) => Err(format!(
            "unknown launch args starting with `{other}` — expected: map <zone> | dedicated <zone> | menu | play <demo> [--cmds '<script>'] | export-gltf <zone>"
        )),
        None => Ok(LaunchMode::Menu),
    }
}

fn parse_play_args(args: impl Iterator<Item = String>) -> Result<LaunchMode, String> {
    let usage = "usage: iw4l play <demo> [--zone <zone>] [--cmds '<script>']";
    let mut name = None;
    let mut zone_override = None;
    let mut iter = args.peekable();
    while let Some(arg) = iter.next() {
        if arg == "--zone" {
            match iter.next() {
                Some(zone) if !zone.is_empty() && !zone.starts_with('-') => {
                    zone_override = Some(zone);
                }
                _ => return Err(usage.into()),
            }
            continue;
        }
        if let Some(zone) = arg.strip_prefix("--zone=") {
            if zone.is_empty() {
                return Err("empty --zone".into());
            }
            zone_override = Some(zone.to_owned());
            continue;
        }
        if arg.starts_with('-') {
            return Err(format!("unknown play flag `{arg}` — expected --zone"));
        }
        if name.is_some() {
            return Err(format!("{usage} (extra `{arg}`)"));
        }
        name = Some(replay::demo_stem(&arg).to_owned());
    }
    let Some(name) = name.filter(|n| !n.is_empty()) else {
        return Err(usage.into());
    };
    Ok(LaunchMode::Play {
        name,
        zone_override,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    /// `--headless`, the mode and the check, as `parse_cli` takes them
    /// (without the environment, which a test must not depend on).
    fn cli(list: &[&str]) -> Result<(LaunchMode, bool), String> {
        let (rest, headless) = take_headless_flag(args(list));
        let mode = parse_launch_args(rest.into_iter())?;
        check_headless(&mode, false, headless)?;
        Ok((mode, headless))
    }

    #[test]
    fn headless_flag_is_taken_wherever_it_stands() {
        assert_eq!(
            take_headless_flag(args(&["--headless", "menu"])),
            (args(&["menu"]), true)
        );
        assert_eq!(
            take_headless_flag(args(&["map", "mp_rust", "--headless"])),
            (args(&["map", "mp_rust"]), true)
        );
        assert_eq!(
            take_headless_flag(args(&["map", "mp_rust"])),
            (args(&["map", "mp_rust"]), false)
        );
    }

    #[test]
    fn headless_env_takes_on_values_only() {
        for on in ["1", "true", "on", "TRUE", " 1 "] {
            assert!(headless_env(Some(on)), "{on}");
        }
        for off in ["0", "false", "off", "", "yes please"] {
            assert!(!headless_env(Some(off)), "{off}");
        }
        assert!(!headless_env(None));
    }

    #[test]
    fn headless_menu_and_map_parse() {
        assert_eq!(cli(&["--headless", "menu"]), Ok((LaunchMode::Menu, true)));
        assert_eq!(
            cli(&["--headless", "map", "minecraft:overworld"]),
            Ok((LaunchMode::Map("minecraft:overworld".into()), true))
        );
        assert_eq!(
            cli(&["--headless", "dedicated", "minecraft:overworld"]),
            Ok((LaunchMode::Dedicated("minecraft:overworld".into()), true))
        );
        assert_eq!(cli(&["menu"]), Ok((LaunchMode::Menu, false)));
        assert_eq!(cli(&[]), Ok((LaunchMode::Menu, false)));
    }

    #[test]
    fn headless_refuses_what_is_only_a_picture() {
        assert!(cli(&["--headless", "export-gltf", "mp_rust"]).is_err());
        let map = LaunchMode::Map("mp_rust".into());
        assert!(check_headless(&map, true, true).is_err());
        assert!(check_headless(&map, true, false).is_ok());
    }

    #[test]
    fn a_cmds_script_naming_headless_stays_a_script() {
        // `--cmds` and its script go first, so a script that is the word
        // itself is never taken for the flag.
        let rest = console::strip_cmds_flag(args(&["--cmds", "--headless", "menu"]).into_iter());
        assert_eq!(take_headless_flag(rest), (args(&["menu"]), false));
    }
}
