//! The players a host keeps (`players.json`): each under its player key's
//! fingerprint (`key:<fingerprint>`), with the name it last played under,
//! when it was first and last seen (Unix seconds), and its inventory. An
//! entry under a bare name is from before player keys: the first key to join
//! under that name claims it, once.
use std::collections::BTreeMap;

use super::InventoryState;

const KEY_PREFIX: &str = "key:";

/// A player key's fingerprint: 64 lowercase hex digits.
pub fn is_fingerprint(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn entry(fingerprint: &str) -> String {
    format!("{KEY_PREFIX}{fingerprint}")
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlayerRecord {
    /// The name it last played under.
    pub name: String,
    pub first_seen: u64,
    pub last_seen: u64,
    #[serde(default)]
    pub inventory: Option<InventoryState>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum PlayerEntry {
    Keyed(PlayerRecord),
    /// From before player keys, under a name.
    Named(InventoryState),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Players(BTreeMap<String, PlayerEntry>);

/// A transfer done: whose record moved where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    /// The old key, or `None` for an entry from before player keys.
    pub from: Option<String>,
    pub to: String,
    pub name: String,
    pub inventory: bool,
}

impl Players {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn record(&self, fingerprint: &str) -> Option<&PlayerRecord> {
        match self.0.get(&entry(fingerprint)) {
            Some(PlayerEntry::Keyed(record)) => Some(record),
            _ => None,
        }
    }

    /// The keys whose last name is `name`.
    pub fn keys_named(&self, name: &str) -> Vec<String> {
        self.0
            .iter()
            .filter_map(|(key, value)| match value {
                PlayerEntry::Keyed(record) if record.name == name => {
                    key.strip_prefix(KEY_PREFIX).map(str::to_owned)
                }
                _ => None,
            })
            .collect()
    }

    fn record_mut(&mut self, fingerprint: &str, name: &str, now: u64) -> &mut PlayerRecord {
        let key = entry(fingerprint);
        if !matches!(self.0.get(&key), Some(PlayerEntry::Keyed(_))) {
            // From before player keys: the first key under the name takes it.
            let claimed = (!name.is_empty() && !name.starts_with(KEY_PREFIX))
                .then(|| self.0.remove(name))
                .flatten()
                .and_then(|old| match old {
                    PlayerEntry::Named(inventory) => Some(inventory),
                    PlayerEntry::Keyed(_) => None,
                });
            self.0.insert(
                key.clone(),
                PlayerEntry::Keyed(PlayerRecord {
                    name: name.to_owned(),
                    first_seen: now,
                    last_seen: now,
                    inventory: claimed,
                }),
            );
        }
        let Some(PlayerEntry::Keyed(record)) = self.0.get_mut(&key) else {
            unreachable!("just made");
        };
        if !name.is_empty() {
            record.name = name.to_owned();
        }
        record.last_seen = record.last_seen.max(now);
        record
    }

    /// A player joining with the key `fingerprint` under `name`: its kept
    /// inventory, if any.
    pub fn join(&mut self, fingerprint: &str, name: &str, now: u64) -> Option<InventoryState> {
        self.record_mut(fingerprint, name, now).inventory.clone()
    }

    /// Keeps a player's inventory; whether anything changed.
    pub fn keep(
        &mut self,
        fingerprint: &str,
        name: &str,
        inventory: InventoryState,
        now: u64,
    ) -> bool {
        let record = self.record_mut(fingerprint, name, now);
        if record.inventory.as_ref() == Some(&inventory) {
            return false;
        }
        record.inventory = Some(inventory);
        true
    }

    /// Moves the record of `from` (a fingerprint, or a name: the one key
    /// whose last name it is, else an entry from before player keys) to the
    /// key `to`, replacing what `to` had.
    pub fn transfer(&mut self, from: &str, to: &str, now: u64) -> Result<Transfer, String> {
        if !is_fingerprint(to) {
            return Err(format!("`{to}` is not a player key fingerprint"));
        }
        let old_key = if is_fingerprint(from) {
            if self.record(from).is_none() {
                return Err(format!("no player with the key {from}"));
            }
            Some(from.to_owned())
        } else {
            match self.keys_named(from).as_slice() {
                [one] => Some(one.clone()),
                [] if matches!(self.0.get(from), Some(PlayerEntry::Named(_))) => None,
                [] => return Err(format!("no player called `{from}`")),
                many => {
                    return Err(format!(
                        "{} players have played as `{from}`: name the key",
                        many.len()
                    ));
                }
            }
        };
        if old_key.as_deref() == Some(to) {
            return Err("the old and the new key are the same".to_owned());
        }
        let (name, first_seen, inventory) = match &old_key {
            Some(old) => match self.0.remove(&entry(old)) {
                Some(PlayerEntry::Keyed(record)) => {
                    (record.name, record.first_seen, record.inventory)
                }
                _ => unreachable!("checked"),
            },
            None => match self.0.remove(from) {
                Some(PlayerEntry::Named(inventory)) => (from.to_owned(), now, Some(inventory)),
                _ => unreachable!("checked"),
            },
        };
        let moved = inventory.is_some();
        let first_seen = self
            .record(to)
            .map_or(first_seen, |record| record.first_seen.min(first_seen));
        self.0.insert(
            entry(to),
            PlayerEntry::Keyed(PlayerRecord {
                name: name.clone(),
                first_seen,
                last_seen: now,
                inventory,
            }),
        );
        Ok(Transfer {
            from: old_key,
            to: to.to_owned(),
            name,
            inventory: moved,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> String {
        format!("{n:02x}").repeat(32)
    }

    fn inventory(selected: u8) -> InventoryState {
        InventoryState {
            selected,
            ..InventoryState::default()
        }
    }

    #[test]
    fn an_inventory_follows_the_key_not_the_name() {
        let mut players = Players::default();
        assert_eq!(players.join(&key(1), "client", 10), None);
        assert_eq!(players.join(&key(2), "client", 11), None);
        assert!(players.keep(&key(1), "client", inventory(3), 20));
        assert!(players.keep(&key(2), "client", inventory(5), 21));
        assert_eq!(players.join(&key(1), "client", 30), Some(inventory(3)));
        assert_eq!(players.join(&key(2), "client", 31), Some(inventory(5)));
        // A new name for the same key keeps its inventory.
        assert_eq!(players.join(&key(1), "renamed", 40), Some(inventory(3)));
        let record = players.record(&key(1)).unwrap();
        assert_eq!(
            (record.name.as_str(), record.first_seen, record.last_seen),
            ("renamed", 10, 40)
        );
    }

    #[test]
    fn an_old_named_inventory_is_claimed_once() {
        let mut players: Players = serde_json::from_str(&format!(
            "{{\"noah\": {}}}",
            serde_json::to_string(&inventory(4)).unwrap()
        ))
        .unwrap();
        assert_eq!(players.join(&key(1), "noah", 1), Some(inventory(4)));
        // Claimed: another key under the same name gets nothing.
        assert_eq!(players.join(&key(2), "noah", 2), None);
        assert_eq!(players.join(&key(1), "noah", 3), Some(inventory(4)));
        // A name made to look like a key entry claims nothing.
        let mut tricked = Players::default();
        tricked.keep(&key(1), "noah", inventory(4), 1);
        assert_eq!(tricked.join(&key(9), &entry(&key(1)), 2), None);
        assert_eq!(tricked.join(&key(1), "noah", 3), Some(inventory(4)));
    }

    #[test]
    fn players_json_reads_back_both_kinds() {
        let mut players: Players = serde_json::from_str(&format!(
            "{{\"old\": {}}}",
            serde_json::to_string(&inventory(1)).unwrap()
        ))
        .unwrap();
        players.keep(&key(1), "new", inventory(2), 5);
        let text = serde_json::to_string(&players).unwrap();
        let read: Players = serde_json::from_str(&text).unwrap();
        assert_eq!(read, players);
        assert!(text.contains("\"old\""));
        assert!(text.contains(&format!("\"key:{}\"", key(1))));
    }

    #[test]
    fn a_transfer_by_key_moves_the_record() {
        let mut players = Players::default();
        players.keep(&key(1), "noah", inventory(3), 10);
        // The new key joined meanwhile with nothing.
        players.join(&key(2), "noah", 20);
        let done = players.transfer(&key(1), &key(2), 30).unwrap();
        assert_eq!(done.from, Some(key(1)));
        assert_eq!(done.name, "noah");
        assert!(done.inventory);
        assert!(players.record(&key(1)).is_none());
        let record = players.record(&key(2)).unwrap();
        assert_eq!(record.inventory, Some(inventory(3)));
        assert_eq!(record.first_seen, 10);
        assert_eq!(players.join(&key(2), "noah", 40), Some(inventory(3)));
    }

    #[test]
    fn a_transfer_by_name_takes_the_one_key_or_the_old_entry() {
        let mut players = Players::default();
        players.keep(&key(1), "noah", inventory(3), 10);
        let done = players.transfer("noah", &key(2), 20).unwrap();
        assert_eq!(done.from, Some(key(1)));
        assert_eq!(players.join(&key(2), "noah", 21), Some(inventory(3)));
        // An entry from before player keys, by its name.
        let mut old: Players = serde_json::from_str(&format!(
            "{{\"ann\": {}}}",
            serde_json::to_string(&inventory(6)).unwrap()
        ))
        .unwrap();
        let done = old.transfer("ann", &key(4), 30).unwrap();
        assert_eq!(done.from, None);
        assert_eq!(old.join(&key(4), "ann", 31), Some(inventory(6)));
    }

    #[test]
    fn an_ambiguous_or_unknown_transfer_is_refused() {
        let mut players = Players::default();
        players.keep(&key(1), "client", inventory(1), 10);
        players.keep(&key(2), "client", inventory(2), 11);
        let before = players.clone();
        assert!(
            players
                .transfer("client", &key(3), 20)
                .unwrap_err()
                .contains("2 players")
        );
        assert!(
            players
                .transfer("nobody", &key(3), 20)
                .unwrap_err()
                .contains("no player")
        );
        assert!(
            players
                .transfer(&key(7), &key(3), 20)
                .unwrap_err()
                .contains("no player")
        );
        assert!(
            players
                .transfer(&key(1), "short", 20)
                .unwrap_err()
                .contains("not a player key")
        );
        assert!(
            players
                .transfer(&key(1), &key(1), 20)
                .unwrap_err()
                .contains("same")
        );
        assert_eq!(players, before, "nothing changed");
    }
}
