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

The room is `IW4L_MASTER_HOST_NAME` (default `Dedicated server`), with
`IW4L_MASTER_MAX_PLAYERS` (the server holds one of the slots) and
`IW4L_MASTER_PASSWORD` as for any host. Its log is its stdout, and
`dedicated-players: count=N clients=… names=…` is printed whenever who is in
the match changes. `IW4L_CONSOLE_STDIN=1` takes console commands on stdin
(`mc state`, `mc block`, `mc set` work with no player).

It runs one match for the life of its room. SIGTERM (or Ctrl-C) saves and
exits with 0; a room the master lost (the master restarted, the network
dropped) or a match that ended exits with 1, for a supervisor to start it
again, and the new room has a new id.

**The Minecraft map** keeps its world in `minecraft-worlds/<name>/` beside
`settings.cfg`, or under `IW4L_MINECRAFT_WORLDS` (`IW4L_MINECRAFT_WORLD`
names it): seed, block changes and every player's inventory by player name.
It is saved every ten seconds when something changed, when a player leaves
and on SIGTERM. Terrain is generated around each connected player, and the
world's spawn and the first player's surroundings run the mobs and items.
Minecraft's files are fetched from Mojang with `curl` on the first start into
`iw4l-artifacts/minecraft-26.3` in the working directory.

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
