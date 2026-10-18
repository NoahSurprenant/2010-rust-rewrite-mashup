//! Player keys and admins on the console:
//! - `whoami`: this game's player key (`player-key:` line);
//! - `admins`, `admins set <fingerprint,...|-> [label]`: the keys a host lets
//!   run debug commands (`net::DebugAdmins`; `debug-admins:` line);
//! - `admin transfer <old fingerprint|player name> <new fingerprint> [label]`:
//!   a player's kept record moved to a new key, on a host or (for an admin)
//!   asked of the host (`admin-transfer:` line, from the Minecraft world).
use bevy::prelude::*;

use crate::{ConsoleCommand, ConsoleLine, ConsoleSettings, ConsoleState};

pub(crate) const WHOAMI_USAGE: &str =
    "whoami - this game's player key (its fingerprint is who you are to every host)";
pub(crate) const ADMINS_USAGE: &str = "admins [set <fingerprint,...|-> [label]] - player keys allowed debug commands on this host (IW4L_ADMIN_KEYS)";
pub(crate) const ADMIN_USAGE: &str = "admin transfer <old fingerprint|player name> <new fingerprint> [label] - move a player's kept inventory (and admin place) to a new key";

#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct KeyState<'w> {
    admins: Option<ResMut<'w, net::DebugAdmins>>,
    key: Option<Res<'w, net::LocalPlayerKey>>,
    minecraft: Option<ResMut<'w, frame::MinecraftUi>>,
}

pub(crate) fn route_admins_commands(
    mut events: MessageReader<ConsoleCommand>,
    mut console: ResMut<ConsoleState>,
    settings: Res<ConsoleSettings>,
    mut line: ResMut<ConsoleLine>,
    mut state: KeyState,
) {
    let capacity = settings.log_capacity;
    for cmd in events.read() {
        let args: Vec<&str> = cmd.args.iter().map(String::as_str).collect();
        let text = match cmd.name.as_str() {
            "whoami" => whoami_line(state.key.as_deref()),
            "admins" => match (args.as_slice(), state.admins.as_mut()) {
                (_, None) => "admins: not a host".to_owned(),
                ([], Some(admins)) => admins_line(admins, None),
                (["set", list, label @ ..], Some(admins)) if label.len() <= 1 => {
                    let (parsed, skipped) =
                        net::DebugAdmins::parse(if *list == "-" { "" } else { list });
                    if skipped.is_empty() {
                        **admins = parsed;
                        diag::info!(Net, "admins: {}", admins.describe());
                        admins_line(admins, label.first().copied())
                    } else {
                        format!(
                            "admins: not player key fingerprints: {} (whoami prints one)",
                            skipped.join(", ")
                        )
                    }
                }
                _ => format!("usage: {ADMINS_USAGE}"),
            },
            "admin" => match args.as_slice() {
                ["transfer", from, to, label @ ..] if label.len() <= 1 => {
                    match transfer_request(from, to) {
                        Ok((from, to)) => match state.minecraft.as_mut() {
                            Some(ui) => {
                                ui.transfers.push((
                                    from,
                                    to,
                                    label.first().map(|l| (*l).to_owned()),
                                ));
                                continue;
                            }
                            None => "admin transfer: no Minecraft world here".to_owned(),
                        },
                        Err(error) => format!("admin transfer: {error}"),
                    }
                }
                _ => format!("usage: {ADMIN_USAGE}"),
            },
            _ => continue,
        };
        println!("{text}");
        diag::info!(Console, "{text}");
        line.0 = text.clone();
        console.echo(text, capacity);
    }
}

/// `admin transfer`'s keys as fingerprints: `from` a fingerprint or a name,
/// `to` a fingerprint.
fn transfer_request(from: &str, to: &str) -> Result<(String, String), String> {
    let to = player_key::normalize_fingerprint(to)
        .ok_or_else(|| format!("`{to}` is not a player key fingerprint (whoami prints one)"))?;
    let from = player_key::normalize_fingerprint(from).unwrap_or_else(|| from.to_owned());
    Ok((from, to))
}

fn whoami_line(key: Option<&net::LocalPlayerKey>) -> String {
    match key {
        Some(key) => {
            let fingerprint = key.0.fingerprint();
            let file = net::identity::player_key_path()
                .map_or_else(|| "-".to_owned(), |path| path.display().to_string());
            format!(
                "player-key: fingerprint={fingerprint} short={} file=\"{file}\"",
                player_key::short(&fingerprint)
            )
        }
        None => "player-key: none (a dedicated server has no player; or no key could be kept)"
            .to_owned(),
    }
}

fn admins_line(admins: &net::DebugAdmins, label: Option<&str>) -> String {
    match label {
        Some(label) => format!("debug-admins: label={label} keys={}", admins.describe()),
        None => format!("debug-admins: keys={}", admins.describe()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(seed: u8) -> String {
        player_key::PlayerKey::from_seed([seed; 32]).fingerprint()
    }

    #[test]
    fn the_line_names_the_admin_keys_or_none() {
        let (admins, _) = net::DebugAdmins::parse(&format!("{},{}", key(1), key(2)));
        assert_eq!(
            admins_line(&admins, Some("s_admins")),
            format!("debug-admins: label=s_admins keys={},{}", key(1), key(2))
        );
        assert_eq!(
            admins_line(&net::DebugAdmins::default(), None),
            "debug-admins: keys=-"
        );
    }

    #[test]
    fn a_transfer_names_its_new_key_in_full() {
        let spaced = key(2).to_ascii_uppercase();
        assert_eq!(
            transfer_request("client", &spaced),
            Ok(("client".to_owned(), key(2)))
        );
        assert_eq!(
            transfer_request(&key(1).to_ascii_uppercase(), &key(2)),
            Ok((key(1), key(2)))
        );
        assert!(transfer_request("client", player_key::short(&key(2))).is_err());
    }

    #[test]
    fn whoami_prints_the_whole_fingerprint() {
        let player = net::LocalPlayerKey(std::sync::Arc::new(player_key::PlayerKey::from_seed(
            [4; 32],
        )));
        let line = whoami_line(Some(&player));
        assert!(line.starts_with(&format!("player-key: fingerprint={} ", key(4))));
        assert!(whoami_line(None).starts_with("player-key: none"));
    }
}
