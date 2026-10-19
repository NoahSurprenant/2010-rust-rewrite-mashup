# Dedicated server

`iw4l dedicated <zone>` hosts one room on a master with no player of its
own: nobody plays on it, it is in no scoreboard, and the room stays listed
and joinable with nobody in it. Players join it from `make menu` like any
room. It needs MW2 multiplayer data and a master ([`MASTER.md`](MASTER.md));
it opens no window, uses no GPU and plays no sound.

```bash
IW4L_MASTER_ADDR=1.2.3.4:4433 IW4L_MASTER_SERVER_NAME=iw4l-prod IW4L_MASTER_KEY_SHA256=<pin> \
IW4L_MASTER_HOST_NAME='Minecraft' iw4l dedicated minecraft:overworld
```

The room is `IW4L_MASTER_HOST_NAME` (default `Dedicated server`), listed whole up to its 48 bytes: the browser's
column shows 22 characters and the line above the list the focused room's
whole name (a master older than this cuts it to 16 characters), with `IW4L_MASTER_MAX_PLAYERS` (the server
holds one of the slots) and `IW4L_MASTER_PASSWORD` as for any host. Its log is its stdout, and
`dedicated-players: count=N clients=… names=…` is printed whenever who is in
the match changes. `IW4L_CONSOLE_STDIN=1` takes console commands on stdin
(`mc state`, `mc block`, `mc set` work with no player), and `IW4L_CONSOLE_SOCKET=<path>`
on a Unix socket there (mode 0600, the server's user only; the image and the Helm chart set
`/tmp/iw4l-console.sock`): `iw4l dedicated-console "<command>"` on the same machine sends one
command and prints what the console printed for it, for a server with no terminal:

```bash
kubectl exec -n iw4l-dedicated deploy/iw4l-dedicated -- iw4l dedicated-console whoami
docker exec iw4l-dedicated iw4l dedicated-console admins
```

**Debug commands** (`mc give`, `mc clear`, `mc fill`, `mc set`, `mc summon`,
`give`, `tp`, `kill`, `damage`, `force_spawn`, `admin transfer`, ...) change
the world for whoever runs them, so a host takes them only from its own
console, its bots and the players whose **player key** is on its admin list
("Player keys and admins" below). A refused command changes nothing; the
host prints `debug-refused: client=… who="<name> <key, short>" command=…` and
the player is told on its HUD (and its stdout, `host-message: …`).

It runs one match for the life of its room. SIGTERM (or Ctrl-C) saves and
exits with 0; a room the master lost (the master restarted, the network
dropped) or a match that ended exits with 1, for a supervisor to start it
again, and the new room has a new id.

**The Minecraft map** keeps its world in `minecraft-worlds/<name>/` beside
`settings.cfg`, or under `IW4L_MINECRAFT_WORLDS` (`IW4L_MINECRAFT_WORLD`
names it): seed, block changes and every player's inventory by player key.
It is saved every ten seconds when something changed, when a player leaves
and on SIGTERM. Terrain is generated around each connected player, and the
world's spawn and the first player's surroundings run the mobs and items.
Minecraft's files are fetched from Mojang with `curl` on the first start into
`iw4l-artifacts/minecraft-26.3` in the working directory.
For tests, the world takes settings from the environment:
`IW4L_MINECRAFT_SEED` (an i64; random when unset), `IW4L_MINECRAFT_SPAWN="x y
z"` (the first player's feet in Minecraft block coordinates, where the map's
origin sits, instead of vanilla's spot picked at random near the world spawn)
and `IW4L_MINECRAFT_MOBS=off` (no natural spawning and no creatures from chunk
generation: only summoned mobs, in any world; a host's console turns natural
spawning back on with `mc spawning on`). A kept world keeps its seed and spawn.
The approved scenarios set all three (`crates/approved_tests/README.md`).

## Player keys and admins

Every game keeps a key pair in `player.key` beside its `settings.cfg`
(`iw4l-artifacts\player.key` in the Windows folder, `~/.config/iw4l/player.key`
on Linux, or `IW4L_PLAYER_KEY`), made on its first run. The private half never
leaves the machine: on joining a room the game signs that room's id and the
member id the master gave it for this join, and the host checks the
signature, so a key cannot be borrowed or replayed. The key's **fingerprint**
(SHA-256 of the public key, 64 hex digits) is who the player is to every
host: `whoami` on the console prints it, and a host logs every join as
`player joined: name=<name> key=<fingerprint>` (in the dedicated server's
log, so `kubectl logs` and Loki have it). A game without a valid proof plays
as a **guest** (`key=guest`): never an admin, and its inventory is not kept.

**Admins** are listed by fingerprint: `IW4L_ADMIN_KEYS=<fingerprint>,...` (the
Helm chart's `adminKeys`). The listen host's own player always is one; with
no list, no other player is. On the host's console, `admins` shows the list
and `admins set <fingerprint,...|->` replaces it until the next start.

**Inventories** are kept by key in the world's `players.json`, with the name
each key last played under and when it was first and last seen. Two players
with the same name keep separate inventories. An inventory kept by name from
before player keys is claimed, once, by the first key that joins under that
name.

**A lost key cannot be recovered**: the file is the only copy. Back up
`player.key` (it is small; keep it private: anyone with it is you to every
host). A player who lost theirs gets a new key on the next start; to give
them back what they had:

- `whoami` on their console prints the new fingerprint; or the host's log
  shows it on their next join (`player joined: name=… key=…`).
- On the host's console (the dedicated server's: `iw4l dedicated-console`
  above, or `IW4L_CONSOLE_STDIN=1`; or an admin's own console, which asks the host), `admin transfer <old
  fingerprint|player name> <new fingerprint>` moves their kept inventory
  (and a place on the admin list given at runtime) to the new key. A name
  works when exactly one key last played under it; otherwise name the old
  fingerprint (the join lines have it). Neither key may be in the game. It
  prints `admin-transfer: … result=ok from=… to=… name=… inventory=…`.
- An admin of a dedicated server who lost their key: put the new fingerprint
  in `IW4L_ADMIN_KEYS` (the chart's `adminKeys`) and restart, then transfer.

## Docker

`ghcr.io/noahsurprenant/iw4l-dedicated` is built from `main` by
`.github/workflows/dedicated-image.yml` (`packaging/iw4l-dedicated/Dockerfile`)
and signed keyless with cosign; tags `sha-<commit>` and `latest`. It holds no
game data: mount MW2 read-only at `/games/mw2` and a writable volume at
`/data` (cache, logs, Minecraft files and worlds). It runs as 65532. Run the
tag of the commit the players' build and the master came from.

```bash
docker run -d --name iw4l-dedicated --user 65532:65532 \
  -v /path/to/mw2:/games/mw2:ro -v iw4l-data:/data \
  -e IW4L_MASTER_ADDR=1.2.3.4:4433 -e IW4L_MASTER_SERVER_NAME=iw4l-prod \
  -e IW4L_MASTER_KEY_SHA256=<pin> -e IW4L_MASTER_HOST_NAME='Minecraft' \
  ghcr.io/noahsurprenant/iw4l-dedicated:latest
```

## Kubernetes — the Helm chart

`charts/iw4l-dedicated` runs the image on Kubernetes: one pod, MW2 from a
volume you give it (an existing claim, NFS or any volume source), `/data` on
a claim of its own so the world survives restarts, readiness on its room
being open, and a NetworkPolicy that lets nothing in. It takes the master's
address, server name and key pin as values, and a master in the same cluster
is typically the `iw4l-master` chart ([`MASTER.md`](MASTER.md), "Kubernetes").
Published as `oci://ghcr.io/noahsurprenant/charts/iw4l-dedicated` and signed
with cosign by `.github/workflows/helm-charts.yml`; its
[`README.md`](../charts/iw4l-dedicated/README.md) and `values.yaml` document
every value.

```bash
helm install iw4l-dedicated oci://ghcr.io/noahsurprenant/charts/iw4l-dedicated --version <version> \
  -n iw4l-dedicated --create-namespace --set image.tag=sha-<commit> \
  --set master.address=iw4l-master.iw4l-master.svc.cluster.local:4433 \
  --set master.keySha256=<pin> --set room.name=Minecraft \
  --set gameData.existingClaim=<a claim with MW2>
```
