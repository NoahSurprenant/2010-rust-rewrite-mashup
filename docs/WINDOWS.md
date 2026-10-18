# Portable Windows folder

The zip on this fork's [Releases](../../../releases/latest),
`2010-Rust-Rewrite-Mashup-windows-x64.zip`, is built by
[`.github/workflows/release.yml`](../.github/workflows/release.yml) for every
push to `main` and published as `build-<run number>`. It has upstream's
layout: `iw4l.exe`, `iw4launcher.exe`, `skate/iw4l-skate-convert.exe` with its
licences, the four licence files below, `README.txt` and `Minecraft World.bat`.
It carries no `.env`, master address or CA: `iw4l.exe` writes `.env` on first
launch. Pull requests build the same zip as a workflow artifact.

### Updates from GitHub Releases

The fork's builds update themselves. `iw4l.exe` and `iw4launcher.exe` are
built with the newest release as their update URL
(`IW4L_DEFAULT_UPDATE_URL=https://github.com/NoahSurprenant/2010-rust-rewrite-mashup/releases/latest/download`):

1. Double-clicking `iw4l.exe` (or `Minecraft World.bat`) hands the start to
   `iw4launcher.exe` beside it, with the same arguments.
2. The launcher reads `manifest.json` from the newest release: the game
   (`iw4l.exe`, fetched as `iw4l.exe.zst`) and the launcher, each with its
   SHA-256 and size. A file already matching its hash is left alone.
3. Anything that differs is downloaded from GitHub (redirects are followed
   only within `github.com` and `*.githubusercontent.com`, over the public
   web PKI), checked against the manifest's size and SHA-256 (the download
   and the unpacked file both), and swapped in atomically (the old file is
   kept until the new one is in place).
4. It starts `iw4l.exe` with `IW4L_LAUNCHER=1`, which plays.

Offline, or with GitHub unreachable, it plays the build already in the
folder. `IW4L_UPDATE=0` (in `.env` or the environment) turns the check off;
`IW4L_UPDATE_URL` points it elsewhere. A folder from before the first build
with `iw4launcher.exe` does not update: extract a new zip over it once.

`iw4l.exe --version` prints the commit and the game and master protocols
without updating; `--help` is answered after the update, before first-run
setup. `IW4L_UPDATE_URL` in the environment (not `.env`) also makes a build
with no built-in URL, such as a pull request's, hand off and update. It may
name a folder, `file:///D:/updates`, holding `manifest.json` and the files it
lists, read from disk with the same checks (no percent-encoding).

The Release workflow's *Windows update hand-off (windows-2025)* job runs the
real hand-off on a GitHub-hosted runner for every pull request and main
(`scripts/windows-update-handoff.ps1`): Build 49's `iw4l.exe` in this run's
folder updates from this run's files through `file://` (with a launcher
that differs, so it replaces itself while running), and the new
`iw4l.exe --version` must name the commit; then Build 49 as published
updates from the newest release over HTTPS and starts its `iw4l.exe`. Each
file must end up with the manifest's SHA-256.

Trust: the manifest and the files come from this repository's releases
over HTTPS, and each file must match the manifest's hash before it is
installed. There is no signing key: whoever can publish a release here can
ship a build, as with the zip itself. Each release lists
`manifest.json`, `iw4l.exe.zst` and `iw4launcher.exe` beside the zip;
`iw4l-update-manifest` (crates/updater) writes the manifest in the release
workflow and checks it against the files before publishing, and
`cargo test -p updater` covers both.

## IW4L's own bundles

`make launcher windows` creates `dist/windows/iw4l-windows-{dev,prod}.zip`. Both
archives use the password `contextrot` and are self-contained — the game binary
is inside, so the folder runs with no network at all.

```text
iw4l-portable/
├── iw4launcher.exe       from the archive
├── iw4l.exe              from the archive; replaced only by `iw4launcher update`
├── iw4l-ca.pem           public trust anchor
├── LICENSE NOTICE OFL-Oxanium.txt COPYING-FreeFont.txt
├── .env                  from the archive; player edits win forever
├── Modern Warfare 2.lnk  target: game folder or title .exe
├── Black Ops.lnk         optional
├── Modern Warfare 3.lnk  optional
└── iw4l-artifacts/       saves, caches, demos, logs and captures
```

The four licence files are not decoration: `iw4l.exe` has both fonts compiled in
with `include_bytes!`, so an archive carrying the binary carries their licences
too. `release.rs::LEGAL_FILES` is the list; a build shipping without them is a
bug.

Extract into any dedicated folder and add ordinary Windows `.lnk` shortcuts to
installed Call of Duty title folders or executables. MW2 **Multiplayer** files
are required for the menu (`zone/**/common_mp.ff`); `zone/dlc/*.ff` alone is
insufficient. BO1 and MW3 are optional. The launcher starts the game without
modifying those installations.

**Updating is something you ask for** in these bundles (built with a CA of
their own and no built-in update URL). Plain `iw4launcher.exe` starts the
`iw4l.exe` already in the folder and touches nothing else. Fetching a newer
build is `iw4launcher.exe update` (or `IW4L_UPDATE=1` for a shortcut that cannot
pass an argument), which needs `IW4L_UPDATE_URL` in the portable `.env`. No
build replaces its own binary on its own. Publishing is [`DEPLOY.md`](DEPLOY.md).

The folder holding the two executables is the process working directory even
when Explorer supplies another; `IW4L_GAMES` defaults to it, and `.lnk` targets
inside it are additional read-only search roots. `.env` is loaded before
release-manifest defaults, created only when missing and never overwritten.

`.env` holds `IW4L_UPDATE_URL=https://host:8443/prod` plus the master keys
from [`MASTER.md`](MASTER.md). An unreachable master is not a launch error: the browser keeps retrying. All
writable state stays below `iw4l-artifacts/` (including `settings.cfg`, unless
`IW4L_SETTINGS_PATH` overrides it); shortcut targets are never output paths.
