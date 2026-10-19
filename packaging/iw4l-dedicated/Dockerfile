# The dedicated server: `iw4l dedicated <zone>`, a game with no player of its
# own that hosts one room on a master, headless (no window, no GPU, no
# sound). Run your own: docs/DEDICATED.md.
#
# No game data inside. MW2 is mounted read-only at /games/mw2 (IW4L_GAMES);
# the Minecraft map's files are fetched from Mojang with curl on the first
# start into the working directory, /data, which also keeps the asset cache,
# the logs and the Minecraft worlds: mount a volume there. Built, pushed and
# signed (cosign keyless) by .github/workflows/dedicated-image.yml as
# ghcr.io/noahsurprenant/iw4l-dedicated.

FROM docker.io/library/rust:1.99.0-slim-trixie@sha256:2752b332db73fdbb7dc576f06c82ed1f312005784ef913d7e04a28f5f55dc581 AS build
RUN apt-get update \
 && apt-get install -y --no-install-recommends g++ make pkg-config \
      libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
# rust-toolchain.toml asks for "stable" plus rustfmt and clippy, which rustup
# would download; the image's own release builds it. A one-off build keeps no
# incremental state (most of a play target's size).
ENV RUSTUP_TOOLCHAIN=1.99.0 \
    CARGO_PROFILE_PLAY_INCREMENTAL=false
RUN cargo build --profile play --locked -p launcher --bin iw4l \
 && install -D -m 0755 target/play/iw4l /out/iw4l \
 && (/out/iw4l --help 2>&1 || true) | grep -q 'dedicated <zone>'

# Bevy links ALSA, udev and the Wayland client library even when nothing
# opens a window or a sound device; curl fetches the Minecraft files.
FROM docker.io/library/debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
      libasound2t64 libudev1 libwayland-client0 libstdc++6 \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /out/iw4l /usr/local/bin/iw4l
RUN ! ldd /usr/local/bin/iw4l | grep -q 'not found' \
 && install -d -o 65532 -g 65532 /data /games
LABEL org.opencontainers.image.source="https://github.com/NoahSurprenant/2010-rust-rewrite-mashup" \
      org.opencontainers.image.description="IW4L dedicated server: hosts a room on an iw4l-master with no player of its own"
ENV IW4L_GAMES=/games \
    IW4L_SETTINGS_PATH=/data/settings.cfg \
    IW4L_MINECRAFT_WORLDS=/data/minecraft-worlds \
    IW4L_SOUND=off \
    IW4L_CONSOLE_SOCKET=/tmp/iw4l-console.sock \
    HOME=/data
WORKDIR /data
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/iw4l"]
CMD ["dedicated", "minecraft:overworld"]
