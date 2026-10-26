# The Pulse server image (built by .github/workflows/release-server.yml for amd64 + arm64).
FROM rust:1.99.1-bookworm@sha256:48b7f435b8fc6378ce43b44f15e3a47fd8919b2825f835c39bd4bd3da50af5b8 AS build
WORKDIR /src
COPY . .
# Only the server is built here: drop the desktop client from the workspace so its heavy
# dependencies (libwebrtc, DeepFilterNet, WebKitGTK) are never fetched, and use the image's
# toolchain rather than downloading one. Versions of everything the server uses stay as
# locked in Cargo.lock.
RUN sed -i 's#members = \["protocol", "server", "client/src-tauri"\]#members = ["protocol", "server"]#' Cargo.toml \
 && grep -q 'members = \["protocol", "server"\]' Cargo.toml \
 && rm -f rust-toolchain.toml
RUN cargo build --release -p pulse-server --bin pulse-app-server \
 && cp target/release/pulse-app-server /pulse-app-server

FROM debian:bookworm-20261020-slim@sha256:7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && mkdir /data
COPY --from=build /pulse-app-server /usr/local/bin/pulse-app-server
# Compose runs it as the host's `pulse` user; this is the fallback for a bare `docker run`.
USER 65534
VOLUME /data
ENV PULSE_DB_URL=sqlite:///data/pulse.db \
    PULSE_BIND=127.0.0.1:7890 \
    RUST_LOG=info,sqlx=warn
ENTRYPOINT ["pulse-app-server"]
CMD ["serve"]
