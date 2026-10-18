//! Players by key (`player_key`). Each game keeps a key pair in `player.key`
//! beside `settings.cfg`; on joining a room the client signs the room's id
//! and its member id there, and the host binds that client to the key's
//! fingerprint. The host keeps inventories and its admin list
//! (`IW4L_ADMIN_KEYS`) by fingerprint. A client whose proof is missing or
//! wrong plays as a **guest**: never an admin, and its inventory is not
//! kept. The listen host's own player is bound to the host's own key.
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use bevy::prelude::*;
use sim::ClientId;

/// How long an admitted client may go without a proof before it plays as
/// a guest. A client sends its proof as soon as it is accepted, well before
/// its admission completes.
pub const GUEST_AFTER_SECS: f64 = 10.0;

/// This game's player key, when it has one (a dedicated server has none).
#[derive(Resource, Clone, Debug)]
pub struct LocalPlayerKey(pub Arc<player_key::PlayerKey>);

/// Where this game keeps its key: `IW4L_PLAYER_KEY`, else `player.key`
/// beside `settings.cfg` (`IW4L_SETTINGS_PATH`'s folder; the game's
/// `iw4l-artifacts` folder on Windows; `$XDG_CONFIG_HOME/iw4l` or
/// `~/.config/iw4l` elsewhere).
pub fn player_key_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("IW4L_PLAYER_KEY").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let dir = std::env::var_os("IW4L_SETTINGS_PATH")
        .filter(|p| !p.is_empty())
        .and_then(|p| PathBuf::from(p).parent().map(PathBuf::from))
        .or_else(|| {
            if cfg!(windows) {
                return std::env::current_dir()
                    .ok()
                    .map(|dir| dir.join("iw4l-artifacts"));
            }
            std::env::var_os("XDG_CONFIG_HOME")
                .filter(|d| !d.is_empty())
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
                })
                .map(|config| config.join("iw4l"))
        })?;
    Some(dir.join(player_key::KEY_FILE))
}

/// Loads (or makes) this game's key; `None`, logged, when it cannot.
pub fn load_local_key() -> Option<LocalPlayerKey> {
    let Some(path) = player_key_path() else {
        diag::warn!(
            Net,
            "player key: no place to keep one; joining rooms as a guest"
        );
        return None;
    };
    match player_key::PlayerKey::load_or_create(&path) {
        Ok(key) => {
            diag::info!(Net, "player key {} ({})", key.fingerprint(), path.display());
            Some(LocalPlayerKey(Arc::new(key)))
        }
        Err(error) => {
            diag::warn!(Net, "player key: {error}; joining rooms as a guest");
            None
        }
    }
}

/// Who a client of this host is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlayerIdentity {
    /// Admitted, its proof not yet in.
    Pending,
    /// Proved this key (its fingerprint).
    Key(String),
    /// No valid proof: plays, but is never an admin and keeps nothing.
    Guest,
}

/// The host's clients by key.
#[derive(Resource, Debug, Default)]
pub struct PlayerIdentities {
    keys: HashMap<ClientId, String>,
    guests: HashSet<ClientId>,
    /// When each client was first seen admitted without a key, in seconds.
    waiting_since: HashMap<ClientId, f64>,
    /// The clients whose join was logged.
    announced: HashSet<ClientId>,
}

impl PlayerIdentities {
    pub fn of(&self, client: ClientId) -> PlayerIdentity {
        if let Some(key) = self.keys.get(&client) {
            PlayerIdentity::Key(key.clone())
        } else if self.guests.contains(&client) {
            PlayerIdentity::Guest
        } else {
            PlayerIdentity::Pending
        }
    }

    pub fn key_of(&self, client: ClientId) -> Option<&str> {
        self.keys.get(&client).map(String::as_str)
    }

    /// Binds `client` to the key `fingerprint`; a guest that proves itself
    /// later stays a guest for this join (what it did was a guest's).
    pub fn bind(&mut self, client: ClientId, fingerprint: String) -> bool {
        if self.guests.contains(&client) {
            return false;
        }
        self.waiting_since.remove(&client);
        self.keys.insert(client, fingerprint);
        true
    }

    /// The line that logs `client`'s join, once it is known as a key or a
    /// guest and has a name: `player joined: name=<name> key=<fingerprint>`
    /// (`key=guest` for a guest). Once per join.
    pub fn announce(&mut self, client: ClientId, name: &str) -> Option<String> {
        if name.is_empty() || self.announced.contains(&client) {
            return None;
        }
        let key = match self.of(client) {
            PlayerIdentity::Key(key) => key,
            PlayerIdentity::Guest => "guest".to_owned(),
            PlayerIdentity::Pending => return None,
        };
        self.announced.insert(client);
        Some(format!("player joined: name={name} key={key}"))
    }

    pub fn make_guest(&mut self, client: ClientId) {
        self.waiting_since.remove(&client);
        self.keys.remove(&client);
        self.guests.insert(client);
    }

    /// Notes the host's clients at `now` seconds: `enrolled` (joining or in)
    /// and `admitted` (in). An admitted one waiting longer than
    /// `GUEST_AFTER_SECS` without a proof becomes a guest (returned); one no
    /// longer enrolled is forgotten.
    pub fn note_admitted(
        &mut self,
        enrolled: &[ClientId],
        admitted: &[ClientId],
        now: f64,
    ) -> Vec<ClientId> {
        let present: HashSet<ClientId> = enrolled.iter().chain(admitted).copied().collect();
        self.keys.retain(|client, _| present.contains(client));
        self.guests.retain(|client| present.contains(client));
        self.waiting_since
            .retain(|client, _| present.contains(client));
        self.announced.retain(|client| present.contains(client));
        let mut late = Vec::new();
        for &client in admitted {
            if self.keys.contains_key(&client) || self.guests.contains(&client) {
                continue;
            }
            let since = *self.waiting_since.entry(client).or_insert(now);
            if now - since >= GUEST_AFTER_SECS {
                late.push(client);
            }
        }
        for &client in &late {
            self.make_guest(client);
        }
        late
    }
}

pub(crate) fn register(app: &mut App, role: frame::RuntimeRole) {
    app.init_resource::<PlayerIdentities>();
    // A dedicated server has no player, so no key (it runs as a listen host).
    let dedicated = role == frame::RuntimeRole::Dedicated
        || app.world().contains_resource::<frame::DedicatedServer>();
    if !dedicated && let Some(key) = load_local_key() {
        app.insert_resource(key);
    }
    app.add_systems(Update, (send_player_proof, bind_player_identities));
}

/// A client's proof, once its host has accepted it in a room: the room's
/// id and this member's, signed with this game's key. Again for each join.
fn send_player_proof(
    link: Option<ResMut<crate::UdpClientLink>>,
    bridge: Option<Res<crate::MasterBridge>>,
    key: Option<Res<LocalPlayerKey>>,
    mut sent: Local<Option<(crate::transport::protocol::ConnectionId, [u8; 16], [u8; 16])>>,
) {
    let (Some(mut link), Some(bridge), Some(key)) = (link, bridge, key) else {
        return;
    };
    let Some(connection) = link.connection else {
        return;
    };
    let identity = bridge.state().identity();
    let context = player_key::ProofContext {
        room: identity.room_id.0,
        member: identity.member_id.0,
    };
    if context.member == [0; 16] || *sent == Some((connection, context.room, context.member)) {
        return;
    }
    match link.send_identity(key.0.prove(&context)) {
        Ok(true) => {
            *sent = Some((connection, context.room, context.member));
            diag::info!(
                Net,
                "player key: proof sent ({})",
                player_key::short(&key.0.fingerprint())
            );
        }
        Ok(false) => {}
        Err(error) => diag::warn!(Net, "player key: proof not sent: {error}"),
    }
}

#[derive(bevy::ecs::system::SystemParam)]
struct IdentityHost<'w> {
    hub: Option<ResMut<'w, crate::UdpAuthorityHub>>,
    bridge: Option<Res<'w, crate::MasterBridge>>,
    key: Option<Res<'w, LocalPlayerKey>>,
    local: Option<Res<'w, crate::LocalPresentClient>>,
    dedicated: Option<Res<'w, frame::DedicatedServer>>,
    reliable: Option<ResMut<'w, crate::ReliableEventHub>>,
    notify: Option<ResMut<'w, crate::PendingGameNotify>>,
    world: Option<Res<'w, crate::AuthorityWorld>>,
}

/// The host binds each client that proves a key, makes guests of those
/// whose proof is wrong or never comes, and binds its own player to its own
/// key.
fn bind_player_identities(
    mut host: IdentityHost,
    mut identities: ResMut<PlayerIdentities>,
    time: Res<Time<Real>>,
) {
    // The listen host's own player is its own key.
    let own = match (
        host.dedicated.as_ref(),
        host.local.as_ref(),
        host.key.as_ref(),
    ) {
        (None, Some(local), Some(key)) => Some((local.0, key.0.fingerprint())),
        _ => None,
    };
    if let Some((local, fingerprint)) = own.clone() {
        identities.keys.insert(local, fingerprint);
    }
    let Some(hub) = host.hub.as_mut() else {
        return;
    };
    let room = host
        .bridge
        .as_ref()
        .map(|bridge| bridge.state().identity().room_id.0)
        .unwrap_or([0; 16]);
    let name = |world: Option<&Res<crate::AuthorityWorld>>, client: ClientId| {
        world
            .map(|world| crate::client_name_string(&world.0, client))
            .unwrap_or_default()
    };
    let mut refused = Vec::new();
    for (member, client, proof) in hub.take_identity_inbound() {
        let context = player_key::ProofContext {
            room,
            member: member.0,
        };
        match proof.verify(&context) {
            Some(fingerprint) => {
                diag::info!(
                    Net,
                    "player key: client {} ({}) is {}",
                    client.0,
                    name(host.world.as_ref(), client),
                    fingerprint
                );
                identities.bind(client, fingerprint);
            }
            None => {
                diag::warn!(
                    Net,
                    "player key: client {} ({}) sent a proof that does not hold; a guest",
                    client.0,
                    name(host.world.as_ref(), client)
                );
                identities.make_guest(client);
                refused.push(client);
            }
        }
    }
    let mut enrolled = hub.enrolled_clients();
    enrolled.extend(own.as_ref().map(|(local, _)| *local));
    let late =
        identities.note_admitted(&enrolled, &hub.admitted_clients(), time.elapsed_secs_f64());
    // Every join on the host's stdout (its log): who, by key.
    for &client in &enrolled {
        if own.as_ref().is_some_and(|(local, _)| *local == client) {
            continue;
        }
        if let Some(line) = identities.announce(client, &name(host.world.as_ref(), client)) {
            diag::info!(Net, "{line}");
            println!("{line}");
        }
    }
    for client in late {
        diag::warn!(
            Net,
            "player key: client {} ({}) proved no key in {GUEST_AFTER_SECS:.0}s; a guest",
            client.0,
            name(host.world.as_ref(), client)
        );
        refused.push(client);
    }
    if let Some(reliable) = host.reliable.as_mut() {
        for client in refused {
            let text = format!(
                "{}Your player key could not be checked: you play as a guest (nothing is kept)",
                sim::HUD_STRING_PLAIN
            );
            if let Some(notify) = host.notify.as_mut() {
                let row = notify.plain_print(text);
                reliable
                    .queue_mut(client)
                    .push(crate::ReliableRow::Notify(row));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proved_client_is_its_key_and_a_silent_one_a_guest_in_time() {
        let mut ids = PlayerIdentities::default();
        let (a, b) = (ClientId(1), ClientId(2));
        // A proof may come while its client still joins: kept.
        assert!(ids.bind(a, "aa".into()));
        assert_eq!(ids.note_admitted(&[a, b], &[], 0.0), []);
        assert_eq!(ids.of(a), PlayerIdentity::Key("aa".into()));
        // A client still joining waits for nothing.
        assert_eq!(ids.note_admitted(&[a, b], &[], 100.0), []);
        assert_eq!(ids.of(b), PlayerIdentity::Pending);
        assert_eq!(ids.note_admitted(&[a, b], &[a, b], 200.0), []);
        assert_eq!(
            ids.note_admitted(&[a, b], &[a, b], 200.0 + GUEST_AFTER_SECS - 1.0),
            []
        );
        assert_eq!(
            ids.note_admitted(&[a, b], &[a, b], 200.0 + GUEST_AFTER_SECS),
            [b]
        );
        assert_eq!(ids.of(a), PlayerIdentity::Key("aa".into()));
        assert_eq!(ids.of(b), PlayerIdentity::Guest);
        assert_eq!(ids.key_of(b), None);
        // A guest stays one for this join.
        assert!(!ids.bind(b, "bb".into()));
        assert_eq!(ids.of(b), PlayerIdentity::Guest);
        // Each join logged once, by key, when its name is known.
        assert_eq!(ids.announce(a, ""), None);
        assert_eq!(
            ids.announce(a, "client").as_deref(),
            Some("player joined: name=client key=aa")
        );
        assert_eq!(ids.announce(a, "client"), None);
        assert_eq!(
            ids.announce(b, "other").as_deref(),
            Some("player joined: name=other key=guest")
        );
        // Gone, forgotten: the next client with that number starts over.
        ids.note_admitted(&[], &[], 300.0);
        assert_eq!(ids.of(a), PlayerIdentity::Pending);
        assert_eq!(ids.of(b), PlayerIdentity::Pending);
    }

    #[test]
    fn a_proof_binds_only_for_its_own_join() {
        let key = player_key::PlayerKey::from_seed([5; 32]);
        let here = player_key::ProofContext {
            room: [1; 16],
            member: [2; 16],
        };
        let proof = key.prove(&here);
        assert_eq!(proof.verify(&here), Some(key.fingerprint()));
        // Replayed by another member of the room.
        let other = player_key::ProofContext {
            member: [3; 16],
            ..here
        };
        assert_eq!(proof.verify(&other), None);
    }
}
