# The Pulse server image (built by .github/workflows/release-server.yml for amd64 + arm64).
FROM rust:1-bookworm AS build
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

FROM debian:bookworm-slim
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
