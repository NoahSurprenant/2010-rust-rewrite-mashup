# The master server (crates/iw4l-master): lobby browser and QUIC packet relay
# for players' games. It needs no game data and no GPU, and keeps nothing on
# disk. Run your own: docs/MASTER.md, "Docker".
#
# Built from this checkout the way `cargo xtask master install` builds it
# (xtask/src/release.rs, build_master): static, so it runs on
# distroless/static with no libc. Built, pushed and signed (cosign keyless) by
# .github/workflows/master-image.yml as ghcr.io/noahsurprenant/iw4l-master.

FROM docker.io/library/rust:1.99.0-slim-trixie@sha256:2752b332db73fdbb7dc576f06c82ed1f312005784ef913d7e04a28f5f55dc581 AS build
WORKDIR /src
COPY . .
# rust-toolchain.toml asks for "stable" plus rustfmt and clippy, which rustup
# would download; the image's own release builds it.
ENV RUSTUP_TOOLCHAIN=1.99.0 \
    RUSTFLAGS="-C target-feature=+crt-static"
# Checks the binary is static (no program interpreter), or it would not start
# on distroless/static, and that it runs.
RUN cargo build --profile release --locked --target x86_64-unknown-linux-gnu -p iw4l-master \
 && install -D -m 0755 target/x86_64-unknown-linux-gnu/release/iw4l-master /out/iw4l-master \
 && ! readelf -l /out/iw4l-master | grep -q "program interpreter" \
 && /out/iw4l-master 2>&1 | grep -q 'usage: iw4l-master'

# No shell, no package manager, no libc; nonroot (65532). Mount the certificate
# and key at /etc/iw4l/tls (tls.crt, tls.key) or pass other `serve` arguments.
FROM gcr.io/distroless/static-debian13:nonroot@sha256:e2e927ec666bae08560abb3c55d0659eceabb657f56b6782ab500a9fc7f555e3
LABEL org.opencontainers.image.source="https://github.com/NoahSurprenant/2010-rust-rewrite-mashup" \
      org.opencontainers.image.description="IW4L master server: lobby browser and QUIC relay for iw4l games"
COPY --from=build /out/iw4l-master /usr/local/bin/iw4l-master
USER 65532:65532
EXPOSE 4433/udp
ENTRYPOINT ["/usr/local/bin/iw4l-master"]
CMD ["serve", "--bind", "0.0.0.0:4433", "--cert", "/etc/iw4l/tls/tls.crt", "--key", "/etc/iw4l/tls/tls.key"]
