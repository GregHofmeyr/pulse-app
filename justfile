set dotenv-load := true

# everything CI runs
check: fmt-check clippy test types-check

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

gen-types:
    cargo test -p pulse-protocol export_bindings

types-check: gen-types
    git diff --exit-code -- client/ui/src/lib/protocol

dev-livekit:
    docker compose up -d livekit

dev-server:
    cargo run -p pulse-server --bin pulse-app-server -- serve
