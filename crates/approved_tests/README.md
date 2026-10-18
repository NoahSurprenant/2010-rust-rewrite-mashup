# approved_tests

APPROVED TEST POLICY

Only explicitly owner-approved end-to-end scenarios may exist in this crate.
Agents may create temporary tests and probes during development, but those
must be deleted before the final squash/push unless specifically approved. A
useful test is not automatically an approved test.

A new permanent scenario is added here only after the owner has approved that
scenario by name. The same holds for a unit test inside this crate: being
useful does not make it approved. `make publish-check` refuses `#[test]`,
`#[cfg(test)]`, `mod tests` and `tests/` directories anywhere else in the
workspace.

## Approved scenarios

| name | what it does |
| --- | --- |
| `heavy_gameplay_lifecycle` | cold load `mp_overgrown` with 16 players, three input scenes, production disconnect, a watched menu, a second map with three scenes, production quit |
| `crafting_table` | the Minecraft map at a fixed seed: builds a small arena, opens a crafting table with E, crafts a chest on its 3x3 grid (the chest's icon drawn from its block entity's boxes, `no_icon` naming neither it nor the planks), closes with Escape (the grid empties back, no MW2 menu), opens with E holding a gun, right click with a gun aims, right click with an item opens, crouched right click with a plank places against it and with an empty hand opens it, a key rebound to Use opens it, and the screen closes when the table breaks or the player moves away |
| `minecraft_multiplayer` | two games on the Minecraft map through an `iw4l-master` the run starts on the loopback (with a certificate minted for the run): the client generates the host's seed, sees a block the host sets and one it clears; the host sees a block the client sets, planks the client places by hand (given by the host on its `mc give`) and breaks by hand; the client picks the dropped plank up, and stands on a platform 320 blocks from the host (the host keeps ground under it); in a walled arena there it sees a husk the host runs, is hurt by it and shoots it dead, shoots a pig for its porkchop, hears the husk (the host's sounds), kills a husk with the knife and a penned pig with its fists, draws a witch's potion and gets its effect, and the host's mobs spawn and live 24 to 128 blocks around it (some past 48 blocks, which it sees) and despawn beyond 128; then both start again on the same settings and the host's world comes back (seed, blocks, the host's and the client's inventories, kept by name). It records each game's and the pair's peak memory |
| `dedicated_server` | a dedicated server (`iw4l dedicated minecraft:overworld`, no window, no player of its own) hosts a room on an `iw4l-master` the run starts on the loopback: it is in no roster, and a client joins it, sets a block and fills its inventory; in a walled arena far off it takes the mob steps of `minecraft_multiplayer` but the pig (husk, sounds, knife, fists, witch, spawn range) with no player of the server's own about, and leaves; the room stays listed and the server keeps running with nobody in it for 75 s (past the master session's 60 s silence deadline) while the server sets a 2048-block glass roof, more edit history than one master control message holds; the client joins again and finds its block, the roof and its inventory; the server is stopped with SIGTERM (exit 0, world saved) and started again, and the seed, the block and the client's inventory come back. It records the server's memory and CPU with no client and with one |
| `skate_multiplayer` | Skate 3 skating with other players on the Minecraft map, through an `iw4l-master` the run starts on the loopback (needs `IW4L_SKATE_ASSETS`, the converted Skate 3 data). In a listen host's room the client gets on its board on a stone runway and pushes off: the host has the client's player moving down the runway, on its board, its states taken, and draws the client's skater (35 bones) in place of its MW2 body; the client gets off and the host stops drawing it. Then the host skates and the client receives its pose in its snapshots. On a dedicated server (no player, no Skate 3 data) a client skates and the server moves its player down the runway. It records each skater's bytes a second and both rooms' memory |

The `master_pin` unit tests (`src/master_pin.rs`, approved 2026-10-17) check the
master key pin: the right pin is accepted, a wrong one refused, a mistyped one
reported, and the pin matches what the openssl command in `docs/MASTER.md`
prints. They need no game data.

The `control_relay` unit tests (`src/control_relay.rs`, approved 2026-10-17)
check the master's control stream bound: the rows a host owes a joining client
are cut, in order, into relay messages that each fit `MAX_CONTROL_BYTES`, a
message one byte too large is refused before the stream, and the master reads
back what the host relays. They need no game data.

`crafting_table` checks each step from a `minecraft-state:` line the game prints
(`mc state <label>`); its steps and expectations are in
`src/scenarios/crafting_table.rs`, with unit tests there for the judging (they
need no game data: `cargo test -p approved_tests`). It needs MW2 under the
games root and fetches Minecraft's files once into `iw4l-artifacts/minecraft-26.3`.
On a headless machine run it through `scripts/headless-display.sh`.

The scenario's steps are in `src/scenarios/heavy_gameplay_lifecycle.rs` and
nowhere else: maps, seed rule, scenes, positions, angles, weapons, durations
and input. To pin a scene to an owner-chosen position, replace its `Place`
there; the runner does not change.

The final `quit` phase sends `finish_run`, which uses the normal process-exit
boundary after the scenario's pending screenshots finish writing. Interactive
`quit` intentionally has only a short write grace period and can leave a
scenario-owned PNG incomplete under load.

## Running

```sh
make approved                                  # cold cache, fresh seed
make approved ARGS='--seed 1234'               # fixed seed
make approved ARGS='--replay iw4l-artifacts/approved-tests/<run>/run.json'
make approved ARGS='--cache shared'            # reuse the repo cache (not cold)
make approved SCENARIO=crafting_table           # the crafting table scenario
make approved SCENARIO=minecraft_multiplayer    # a host and a client through a local master
make approved SCENARIO=dedicated_server         # a dedicated server, and a client that comes and goes
make approved SCENARIO=skate_multiplayer        # skaters with a listen host and a dedicated server
```

The runner builds nothing; `make approved` builds `target/play/iw4l` (and `target/play/iw4l-master`, which `minecraft_multiplayer`, `dedicated_server` and `skate_multiplayer` run) first.
Each run owns one directory, `iw4l-artifacts/approved-tests/<run-id>/`. The
game is started with that directory as its working directory, so everything
IW4L writes (`iw4l-artifacts/cache`, logs, perf run, dumps, screenshots) lands
under it. On Windows the launcher enters the folder holding its executable, so
the runner hard-links (or copies) the binary into the run directory and starts
that one. With `--cache cold` that cache starts empty; with `--cache shared`
it is a symlink to the repository's cache.

A seed resolves to maps and spawn picks once. `run.json` keeps the resolved
set — the positions and angles the game actually used — and `--replay` runs
that set again rather than the seed, so a change in catalog order or spawn
selection cannot move a replayed scene.

## What a run leaves

- `run.json` — build, platform, render, content, maps, seed, the resolved
  scenes and the exact command script, cache mode, bot target and actual,
  lifecycle timestamps, phases with start/end or a failure reason, the GSC
  program each map installed (or the refusal that stopped it), liveness
  assertions, and paths to the large artifacts.
- `00_cold_load.png`, `01_overgrown_gameplay.{png,dump.txt}`,
  `02_after_disconnect.{png,dump.txt}`, `03_second_map_gameplay.{png,dump.txt}`
  when the run got that far.
- `child_stdout.txt`, `child_stderr.txt`, and the game's own
  `iw4l-artifacts/logs/latest.log` and `iw4l-artifacts/runs/<id>/` perf run.

Assertions are liveness only: the map loaded, its GSC program installed, the
players exist, simulation advanced, the authority applied the scene's movement
commands to the local player, the player covered a path of at least 32 units or
died, the lifecycle boundaries happened in order, no script fault ended the
process, and `quit` ended the process on its own.

Every match runs a GSC program, so the game announces it on stdout the way it
announces lifecycle boundaries: `gsc: installed map=… gametype=… fingerprint=…
modules=… functions=… natives=… entries=…` when the world is published, or
`gsc: refused map=… stage=compile|install|entry fault="…"` when the match load
stopped at the script. `run.json` keeps both under `gsc.a` / `gsc.b` and asserts
`a.gsc_installed`, `b.gsc_installed` and `gsc.no_execution_fault`. A phase that
fails after a refusal carries the refusal as its `failure_reason`, and a script
fault that ends the process (`GSC execution failed: …` in `child_stderr.txt`)
becomes the run's `failure`.

Movement is three facts, not one. `mark` reports the local player's input
receipt — commands the authority applied, how many of them carried a move, and
the summed length of every applied move — so a scene that turns while it walks
is judged by the path it walked, not by where it ended up. Each scene records an
`outcome`: `moved`, `died`, `died_before_input`, `held_in_place` (commands
applied, alive, no path: collision or a frozen player) or `input_not_applied`
(the break the scenario exists to catch). There
is no frame-time assertion. A run the controller had to kill is a failure,
never a quit.
