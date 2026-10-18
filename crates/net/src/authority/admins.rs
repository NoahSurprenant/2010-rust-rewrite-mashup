//! Who may run debug commands on a host. A host's own console and its bots
//! always may; a remote player only when the player key it proved on
//! joining (`crate::identity`) is on the host's admin list: `IW4L_ADMIN_KEYS`
//! (comma-separated fingerprints, as `whoami` prints them) when the game
//! starts, or the `admins` console command. With no list, no remote player
//! is an admin; a guest (no valid proof) never is.
//!
//! A refused command is logged on the host (`debug-refused:` on stdout and
//! a warning) and the player is told on its HUD (`host-message:` on its
//! stdout).
use bevy::prelude::Resource;
use sim::ClientId;

/// The environment variable a host reads its admin keys from.
pub const ADMINS_ENV: &str = "IW4L_ADMIN_KEYS";

#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct DebugAdmins {
    keys: Vec<String>,
}

impl DebugAdmins {
    /// The admins in `list`: comma-separated fingerprints (64 hex digits,
    /// any case, spaces or colons between digits); repeats are dropped, and
    /// anything not a fingerprint is skipped (returned).
    pub fn parse(list: &str) -> (Self, Vec<String>) {
        let mut keys: Vec<String> = Vec::new();
        let mut skipped = Vec::new();
        for entry in list.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            match player_key::normalize_fingerprint(entry) {
                Some(key) if !keys.contains(&key) => keys.push(key),
                Some(_) => {}
                None => skipped.push(entry.to_owned()),
            }
        }
        (Self { keys }, skipped)
    }

    /// The list `IW4L_ADMIN_KEYS` holds, or none.
    pub fn from_env() -> Self {
        let Ok(list) = std::env::var(ADMINS_ENV) else {
            return Self::default();
        };
        let (admins, skipped) = Self::parse(&list);
        for entry in skipped {
            diag::warn!(
                Net,
                "{ADMINS_ENV}: `{entry}` is not a player key fingerprint; skipped"
            );
        }
        if !admins.keys.is_empty() {
            diag::info!(Net, "admins: {}", admins.describe());
        }
        admins
    }

    pub fn keys(&self) -> &[String] {
        &self.keys
    }

    /// The fingerprints, comma-separated, or `-` for none.
    pub fn describe(&self) -> String {
        if self.keys.is_empty() {
            "-".to_owned()
        } else {
            self.keys.join(",")
        }
    }

    /// Whether the player with the key `fingerprint` is an admin.
    pub fn admits(&self, fingerprint: &str) -> bool {
        self.keys.iter().any(|key| key == fingerprint)
    }

    /// Puts `to` where `from` was on the list (an `admin transfer`); whether
    /// `from` was on it.
    pub fn replace(&mut self, from: &str, to: &str) -> bool {
        let Some(at) = self.keys.iter().position(|key| key == from) else {
            return false;
        };
        if self.keys.iter().any(|key| key == to) {
            self.keys.remove(at);
        } else {
            self.keys[at] = to.to_owned();
        }
        true
    }

    /// Whether `client` proved a key on the list.
    pub fn allows(&self, identities: &crate::PlayerIdentities, client: ClientId) -> bool {
        identities
            .key_of(client)
            .is_some_and(|fingerprint| self.admits(fingerprint))
    }
}

/// Who a client is, for a log line: its name and its key's short form, or
/// `guest`.
pub fn who(identities: &crate::PlayerIdentities, name: &str, client: ClientId) -> String {
    match identities.key_of(client) {
        Some(key) => format!("{name} {}", player_key::short(key)),
        None => format!("{name} guest"),
    }
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
    who: &str,
    command: &str,
) {
    diag::warn!(
        Net,
        "debug command `{command}` from client {} ({who}) refused: not an admin",
        client.0
    );
    println!(
        "debug-refused: client={} who=\"{who}\" command=\"{command}\"",
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

    fn key(seed: u8) -> String {
        player_key::PlayerKey::from_seed([seed; 32]).fingerprint()
    }

    #[test]
    fn no_list_admits_nobody() {
        let (admins, skipped) = DebugAdmins::parse("");
        assert!(admins.keys().is_empty() && skipped.is_empty());
        assert!(!admins.admits(&key(1)));
        assert_eq!(admins.describe(), "-");
        assert!(!DebugAdmins::default().admits(&key(1)));
    }

    #[test]
    fn a_list_admits_exactly_its_keys() {
        let (a, b) = (key(1), key(2));
        let (admins, skipped) =
            DebugAdmins::parse(&format!(" {a} ,{},, {a}, noah", b.to_ascii_uppercase()));
        assert_eq!(admins.keys(), [a.clone(), b.clone()]);
        assert_eq!(skipped, ["noah"]);
        assert!(admins.admits(&a));
        assert!(admins.admits(&b));
        assert!(!admins.admits(&key(3)));
        // A short form is not a key.
        assert!(!admins.admits(player_key::short(&a)));
        // A transfer puts the new key in the old one's place.
        let mut moved = admins.clone();
        assert!(moved.replace(&a, &key(3)));
        assert!(moved.admits(&key(3)) && !moved.admits(&a));
        assert!(!moved.replace(&key(9), &key(4)));
    }

    #[test]
    fn an_admin_is_its_proved_key_not_its_name() {
        let (admins, _) = DebugAdmins::parse(&key(1));
        let mut identities = crate::PlayerIdentities::default();
        let (admin, other, guest) = (ClientId(1), ClientId(2), ClientId(3));
        identities.bind(admin, key(1));
        identities.bind(other, key(2));
        identities.make_guest(guest);
        assert!(admins.allows(&identities, admin));
        assert!(!admins.allows(&identities, other));
        assert!(!admins.allows(&identities, guest));
        assert!(!admins.allows(&identities, ClientId(9)), "still proving");
        assert_eq!(who(&identities, "client", guest), "client guest");
        assert_eq!(
            who(&identities, "client", admin),
            format!("client {}", player_key::short(&key(1)))
        );
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
            "intruder guest",
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
