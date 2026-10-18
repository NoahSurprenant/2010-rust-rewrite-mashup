//! Who may run debug commands on a host. A host's own console and its bots
//! always may; a remote player only when its name is on the host's admin
//! list, which `IW4L_ADMINS` (comma-separated player names) sets when the
//! game starts and the `admins` console command changes. Player names are
//! the only identity a player has: an admin's name is trusted as given.
//! With no list, no remote player is an admin.
//!
//! A refused command is logged on the host (`debug-refused:` on stdout and
//! a warning) and the player is told on its HUD (`host-message:` on its
//! stdout).
use bevy::prelude::Resource;
use sim::ClientId;

/// The environment variable a host reads its admin list from.
pub const ADMINS_ENV: &str = "IW4L_ADMINS";

#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct DebugAdmins {
    names: Vec<String>,
}

impl DebugAdmins {
    /// The admins in `list`: comma-separated names, each as the game keeps
    /// a player name (at most 15 bytes); blanks and repeats are dropped.
    pub fn parse(list: &str) -> Self {
        let mut names: Vec<String> = Vec::new();
        for name in list.split(',').map(str::trim).filter_map(kept_name) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        Self { names }
    }

    /// The list `IW4L_ADMINS` holds, or none.
    pub fn from_env() -> Self {
        let admins =
            std::env::var(ADMINS_ENV).map_or_else(|_| Self::default(), |list| Self::parse(&list));
        if !admins.names.is_empty() {
            diag::info!(Net, "admins: {}", admins.describe());
        }
        admins
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The names, comma-separated, or `-` for none.
    pub fn describe(&self) -> String {
        if self.names.is_empty() {
            "-".to_owned()
        } else {
            self.names.join(",")
        }
    }

    /// Whether the player called `name` is an admin.
    pub fn admits(&self, name: &str) -> bool {
        !name.is_empty() && self.names.iter().any(|admin| admin == name)
    }

    /// Whether the host's player `client` is an admin, by the name the
    /// host's world knows it by.
    pub fn allows(&self, world: &sim::SimWorld, client: ClientId) -> bool {
        self.admits(&crate::client_name_string(world, client))
    }
}

/// A name as the game keeps it: cut to a client state's 15 bytes.
fn kept_name(name: &str) -> Option<String> {
    let packed = entity_iw4::pack_client_state_name(name);
    entity_iw4::client_state_name(&packed).map(str::to_owned)
}

/// What a refused player reads on its HUD.
pub fn refusal_text(command: &str) -> String {
    format!(
        "{}^1{command}^7 refused: only the host's admins may run debug commands",
        sim::HUD_STRING_PLAIN
    )
}

/// Logs a refused debug command on the host and tells the player: a plain
/// print on its reliable stream, which its HUD shows.
pub fn refuse_debug(
    reliable: &mut crate::ReliableEventHub,
    notify: Option<&mut crate::PendingGameNotify>,
    client: ClientId,
    name: &str,
    command: &str,
) {
    diag::warn!(
        Net,
        "debug command `{command}` from client {} ({name}) refused: not an admin",
        client.0
    );
    println!(
        "debug-refused: client={} name={name} command=\"{command}\"",
        client.0
    );
    if let Some(notify) = notify {
        let row = notify.plain_print(refusal_text(command));
        reliable
            .queue_mut(client)
            .push(crate::ReliableRow::Notify(row));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_list_admits_nobody() {
        let admins = DebugAdmins::parse("");
        assert!(admins.names().is_empty());
        assert!(!admins.admits("client"));
        assert!(!admins.admits(""));
        assert_eq!(admins.describe(), "-");
        assert!(!DebugAdmins::default().admits("anyone"));
    }

    #[test]
    fn a_list_admits_exactly_its_names() {
        let admins = DebugAdmins::parse(" noah , client,,noah ");
        assert_eq!(admins.names(), ["noah", "client"]);
        assert!(admins.admits("noah"));
        assert!(admins.admits("client"));
        assert!(!admins.admits("Noah"));
        assert!(!admins.admits("noa"));
        assert!(!admins.admits("intruder"));
        assert_eq!(admins.describe(), "noah,client");
    }

    #[test]
    fn a_long_name_matches_as_the_game_keeps_it() {
        let admins = DebugAdmins::parse("a_very_long_player_name");
        assert_eq!(admins.names(), ["a_very_long_pla"]);
        assert!(admins.admits("a_very_long_pla"));
    }

    #[test]
    fn only_a_peer_s_actions_are_marked_remote() {
        let mut inbox = crate::ClientActionInbox::default();
        let tp = |request_id| sim::ClientAction::Move {
            request_id,
            origin: [0.0; 3],
            angles: [0.0; 3],
        };
        inbox.push(ClientId(0), tp(1)).unwrap();
        inbox.push_from_peer(ClientId(1), tp(1));
        let gathered = inbox.gather();
        assert_eq!(
            gathered,
            [(ClientId(0), tp(1), false), (ClientId(1), tp(1), true)]
        );
        assert!(inbox.gather().iter().all(|(_, _, from_peer)| !from_peer));
    }

    #[test]
    fn debug_actions_are_the_console_s_cheats() {
        let tp = sim::ClientAction::Move {
            request_id: 1,
            origin: [0.0; 3],
            angles: [0.0; 3],
        };
        assert_eq!(sim::debug_action_name(&tp), Some("tp"));
        let give = sim::ClientAction::GiveWeapon {
            request_id: 2,
            weapon: 5,
        };
        assert_eq!(sim::debug_action_name(&give), Some("give"));
        let kill = sim::ClientAction::ForceDeath { request_id: 3 };
        assert_eq!(sim::debug_action_name(&kill), Some("kill"));
        // Play: joining, naming oneself, a menu answer, a Minecraft intent
        // (the world sorts those).
        for action in [
            sim::ClientAction::JoinMatch { request_id: 4 },
            sim::ClientAction::SetName {
                request_id: 5,
                name: [0; 16],
            },
            sim::ClientAction::ActionSlot {
                request_id: 6,
                slot: 1,
            },
            sim::ClientAction::Minecraft {
                request_id: 7,
                intent: sim::MinecraftIntent::new(&[7]).unwrap(),
            },
        ] {
            assert_eq!(sim::debug_action_name(&action), None, "{action:?}");
        }
    }

    #[test]
    fn a_refusal_tells_the_player_on_its_reliable_stream() {
        let mut reliable = crate::ReliableEventHub::default();
        let mut notify = crate::PendingGameNotify::default();
        refuse_debug(
            &mut reliable,
            Some(&mut notify),
            ClientId(2),
            "intruder",
            "mc fill",
        );
        let rows = reliable.queue(ClientId(2)).expect("a queue").pending();
        assert_eq!(rows.len(), 1);
        let crate::ReliableRow::Notify(print) = &rows[0].1 else {
            panic!("not a print: {:?}", rows[0].1);
        };
        assert_eq!(print.tag, crate::SVC_PRINT);
        assert_eq!(print.key, refusal_text("mc fill"));
        assert!(reliable.queue(ClientId(3)).is_none());
        // Not also every player's.
        assert!(notify.is_empty());
    }

    #[test]
    fn the_refusal_names_the_command_in_plain_text() {
        let text = refusal_text("mc fill");
        assert!(text.starts_with(sim::HUD_STRING_PLAIN));
        assert!(text.contains("mc fill"));
        assert!(text.len() <= usize::from(u8::MAX));
    }
}
