# Pulse Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove voice works (throwaway spike), then lay the real foundation: workspace, shared protocol, a server with
auth, servers/channels/DMs, messages, a privacy-enforcing gateway, voice tokens and webhooks, CI, and a bare Tauri
client that can log in and show your servers.

**Architecture:** A Cargo workspace with `protocol` (shared types, TS generation), `server` (axum + SQLite) and
`client/src-tauri` (Tauri 2 Rust core), plus `client/ui` (Svelte 5). LiveKit runs in Docker for local dev. All
fan-out goes through `audience_for()`.

**Tech Stack:** Rust stable (2024 edition), tokio, axum 0.8, sqlx 0.8 (sqlite), argon2, ulid, ts-rs 12, livekit
0.9.3, livekit-api 0.8.1, tokio-tungstenite, Tauri 2.12, Svelte 5 + Vite + TypeScript, pnpm, just, Docker (LiveKit
server v1.13 / lk CLI v2.18).

**Spec:** `docs/superpowers/specs/2026-10-01-pulse-design.md`

## Global Constraints

- Local ports (checked against MyCar 8080/8090/9000/9001/5434, gesturepad 8765, KDE Connect 1716, test-engine 5432, JC 5433): **LiveKit 7880 (HTTP/WS), 7881 (TCP), 7882/udp; pulse-app-server 7890; Vite dev 1420.**

- Technical name `pulse-app` everywhere (crates `pulse-protocol`, `pulse-server`, `pulse-client`; binaries `pulse-app-server`, `pulse-app`); display name "Pulse" only in UI copy.
- SQLite in WAL mode, `STRICT` tables; IDs are ULIDs stored as TEXT.
- Session tokens: 256-bit random, only the SHA-256 hash stored, 90-day expiry renewed on use.
- Non-members of a private (`dm`/`group`) channel get **404** on every route.
- Every outbound gateway event goes through `audience_for()`.
- The UI never talks to the network directly; only the Rust core does.
- No secrets in git: config via env (`PULSE_*`); `.env` gitignored.
- Windows target `x86_64-pc-windows-msvc` with `+crt-static` (`.cargo/config.toml`).
- No `pull_request_target` workflows. Windows build only on tags or `workflow_dispatch`.
- No friends' real names in committed code, fixtures or docs (the repo goes public).

## Review Focus

1. **Outsider probing a private channel ID on every route** (list/send/edit/delete messages, voice token) → 404 every time, no data. Pinned in Task 7 + Task 9.
2. **Expired or garbage token on the gateway `Hello`** → socket closed with close code 4001 promptly, never left hanging or half-subscribed. Pinned in Task 8.
3. **Invite code redeemed twice (including concurrently)** → exactly one account is created; the second gets 410. Pinned in Task 5.
4. **Editing or deleting someone else's message, or one already deleted** → 403 for others' messages, 404 for deleted. Pinned in Task 7.
5. **A slow or stalled gateway client** → it must not block broadcasts to everyone else; it is dropped when its bounded queue fills. Pinned in Task 8.

---

## File Structure

```
pulse-app/
├─ Cargo.toml                    # workspace
├─ .cargo/config.toml            # windows crt-static
├─ rust-toolchain.toml           # stable
├─ justfile                      # check, test, gen-types, dev-livekit, dev-server, dev-client
├─ docker-compose.yml            # livekit dev server
├─ .gitignore  .env.example
├─ .github/workflows/ci.yml  .github/workflows/release-windows.yml
├─ spikes/voice/                 # THROWAWAY, own Cargo project, excluded from workspace
├─ protocol/src/{lib.rs, ids.rs, rest.rs, gateway.rs}  protocol/tests/export_types.rs
├─ server/
│  ├─ migrations/0001_init.sql
│  ├─ src/{main.rs, lib.rs, config.rs, db.rs, error.rs, state.rs}
│  ├─ src/auth/{mod.rs, password.rs, session.rs, extractor.rs, routes.rs, invites.rs}
│  ├─ src/servers/{mod.rs, routes.rs}     # servers, channels, membership, dms
│  ├─ src/access.rs                       # can_access_channel()
│  ├─ src/messages/{mod.rs, routes.rs}
│  ├─ src/gateway/{mod.rs, hub.rs, audience.rs, socket.rs}
│  ├─ src/voice/{mod.rs, routes.rs, webhook.rs}
│  └─ tests/{common/mod.rs, auth.rs, servers.rs, messages.rs, privacy.rs, gateway.rs, voice.rs}
└─ client/
   ├─ src-tauri/{Cargo.toml, tauri.conf.json, build.rs, src/{main.rs, lib.rs, api.rs, session.rs, commands.rs}}
   └─ ui/{package.json, vite.config.ts, svelte.config.js, tsconfig.json, index.html, src/{main.ts, App.svelte, app.css, lib/protocol/*.ts (generated), lib/tauri.ts, lib/stores.ts, routes/Login.svelte, routes/Shell.svelte}}
```

---

### Task 0: Voice spike (THROWAWAY)

**Goal:** Answer spec §6.3 and §15: does `PlatformAudio` (libwebrtc's own device handling) give good two-way voice and
device switching on Linux, and can we do per-user volume with it? If not, does the cpal + `NativeAudioSource` + APM
path work? The output is a written finding; the code is never imported elsewhere.

**Files:**
- Create: `docker-compose.yml`, `spikes/voice/Cargo.toml`, `spikes/voice/src/main.rs`, `spikes/voice/FINDINGS.md`

**Interfaces:**
- Consumes: nothing
- Produces: `docker-compose.yml` (reused by everything later); FINDINGS.md decision (platform vs manual)

- [ ] **Step 1: LiveKit dev server in Docker**

`docker-compose.yml`:
```yaml
services:
  livekit:
    image: livekit/livekit-server:v1.13
    command: --dev --bind 0.0.0.0 --node-ip 127.0.0.1
    ports: ["7880:7880", "7881:7881", "7882:7882/udp"]
```
Run: `docker compose up -d livekit && curl -s localhost:7880` → Expected: `OK`.

- [ ] **Step 2: Spike crate**

`spikes/voice/Cargo.toml`: `livekit = "0.9.3"`, `livekit-api = "0.8.1"`, `tokio = { version = "1", features = ["full"] }`, `futures = "0.3"`, `anyhow`, `clap` (derive), `cpal = "0.15"` (matches the upstream example, avoiding the 0.18 API breaks). Add `[workspace]` (empty) so it is excluded from the root workspace.

`main.rs` subcommands:
- `devices`: `PlatformAudio::new()?` → print `recording_devices()` / `playout_devices()`.
- `platform --room R --identity I [--mic <id>] [--speaker <id>]`: mint a token (`AccessToken::with_api_key("devkey","secret").with_identity(I).with_grants(VideoGrants{room_join:true, room:R.into(), ..Default::default()}).to_jwt()?`), `Room::connect("ws://localhost:7880", &jwt, RoomOptions::default())`, `PlatformAudio::new()`, set devices, `LocalAudioTrack::create_audio_track("mic", audio.rtc_source())`, publish with `TrackPublishOptions{ source: TrackSource::Microphone, dtx: true, red: true, ..Default::default() }`. Log every `RoomEvent` (ActiveSpeakersChanged, ConnectionQualityChanged, Reconnecting/Reconnected). stdin commands: `m` toggles mute (`track.mute()/unmute()`), `s <id>` = `switch_playout_device`, `q` quits.
- `manual --room R --identity I --gain <f32> --peer-volume <f32>`: port of upstream `examples/local_audio` (cpal capture → ×gain → APM `process_stream` in 10 ms chunks → `NativeAudioSource::capture_frame`; per remote track `NativeAudioStream::new(t.rtc_track(), 48000, 1)` → ×peer-volume → mix → cpal output, mixed output → `process_reverse_stream`).
- `sink --room R --seconds N --out file.wav`: subscribe only and write received audio to WAV (automated proof that audio flows).

- [ ] **Step 3: Automated flow check (no human needed)**

Make a 5 s Opus test file: `ffmpeg -f lavfi -i "sine=frequency=440:duration=5" -c:a libopus -ar 48000 spikes/voice/tone.ogg`.
Run the `lk` CLI in Docker as a fake participant:
`docker run --rm --network host -v $PWD/spikes/voice:/w livekit/livekit-cli:v2.18 room join --url ws://localhost:7880 --api-key devkey --api-secret secret --identity bot --publish /w/tone.ogg spike`
At the same time: `cargo run -- sink --room spike --seconds 6 --out /tmp/rx.wav`.
Expected: `/tmp/rx.wav` is non-silent (log the RMS; it must be > 0.01).

- [ ] **Step 4: Human ear test (Greg), checklist in FINDINGS.md**

Two instances on one machine (`platform` as A on headset, `platform` as B on speakers, or `manual` for B). Check: hearing each other, echo, mute, `s` device hot-swap, and the CPU/RSS of each process (`ps -o rss,pcpu`). Repeat with `manual` and different `--peer-volume`.

- [ ] **Step 5: Record the decision**

Write `FINDINGS.md`: which mode, measured RSS/CPU, per-user volume answer, issues. Commit `docker-compose.yml` + `spikes/voice/` with message `spike: voice feasibility (throwaway)`.

> Windows leg (MSVC Build Tools + the same commands) runs later on Greg's Windows partition; tracked in FINDINGS.md as pending.

---

### Task 1: Workspace skeleton + tooling

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `.cargo/config.toml`, `.gitignore`, `.env.example`, `justfile`, `protocol/Cargo.toml`, `protocol/src/lib.rs`, `server/Cargo.toml`, `server/src/main.rs`, `server/src/lib.rs`

**Interfaces:**
- Produces: workspace members `protocol`, `server`, `client/src-tauri` (the last added in Task 11); `just check` / `just test`

- [ ] **Step 1: Workspace files**

`Cargo.toml`:
```toml
[workspace]
resolver = "3"
members = ["protocol", "server"]
exclude = ["spikes"]

[workspace.package]
edition = "2024"
version = "0.1.0"
license = "LicenseRef-All-Rights-Reserved"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
ulid = { version = "1", features = ["serde"] }
ts-rs = { version = "12", features = ["serde-compat", "chrono-impl"] }
tokio = { version = "1", features = ["full"] }
thiserror = "2"
anyhow = "1"
tracing = "0.1"
```
`rust-toolchain.toml`: `[toolchain]\nchannel = "stable"\ncomponents = ["rustfmt", "clippy"]`.
`.cargo/config.toml`: `[target.x86_64-pc-windows-msvc]\nrustflags = ["-C", "target-feature=+crt-static"]`.
`.gitignore`: `target/`, `node_modules/`, `.env`, `*.db`, `*.db-*`, `client/ui/dist/`, `spikes/**/*.wav`.
`.env.example`: `PULSE_DB_URL=sqlite://pulse.db`, `PULSE_BIND=127.0.0.1:7890`, `PULSE_LIVEKIT_URL=ws://localhost:7880`, `PULSE_LIVEKIT_KEY=devkey`, `PULSE_LIVEKIT_SECRET=secret`.

`justfile`:
```make
set dotenv-load := true
check: fmt-check clippy test types-check ui-check
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
ui-check:
    cd client/ui && pnpm install --frozen-lockfile && pnpm check
dev-livekit:
    docker compose up -d livekit
dev-server:
    cargo run -p pulse-server --bin pulse-app-server -- serve
dev-client:
    cd client/src-tauri && cargo tauri dev
```
(`types-check` and `ui-check` become meaningful once Tasks 2 and 11 land; until then they are no-ops, and the `ui-check` recipe is added in Task 11.)

- [ ] **Step 2: Minimal crates compile**

`protocol/src/lib.rs`: `pub mod ids; pub mod rest; pub mod gateway;` (empty modules for now). `server/src/main.rs`: `fn main() { println!("pulse-app-server"); }`. Server `[[bin]] name = "pulse-app-server"`.

Run: `cargo build --workspace && cargo fmt --all -- --check && cargo clippy --workspace -- -D warnings` → Expected: success.

- [ ] **Step 3: Commit** `chore: cargo workspace, justfile, tooling`

---

### Task 2: Protocol crate + TS generation

**Files:**
- Create: `protocol/src/ids.rs`, `protocol/src/rest.rs`, `protocol/src/gateway.rs`, `protocol/tests/export_types.rs`; generated `client/ui/src/lib/protocol/*.ts`

**Interfaces (Produces, used by server and client):**
```rust
// ids.rs: newtype per entity; string on the wire
macro_rules! id { ($n:ident) => {
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
  #[serde(transparent)] #[ts(export, type = "string")]
  pub struct $n(pub Ulid);
  impl $n { pub fn new() -> Self { Self(Ulid::new()) } }
  impl std::fmt::Display for $n { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.0.fmt(f) } }
  impl std::str::FromStr for $n { type Err = ulid::DecodeError; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Self(s.parse()?)) } }
}}
id!(UserId); id!(ServerId); id!(ChannelId); id!(MessageId); id!(RoleId);

// rest.rs (all #[derive(Serialize, Deserialize, TS, Clone, Debug)] #[ts(export)])
pub struct RegisterRequest { pub invite_code: String, pub username: String, pub password: String }
pub struct LoginRequest { pub username: String, pub password: String }
pub struct SessionResponse { pub token: String, pub user: User }
pub struct User { pub id: UserId, pub username: String, pub avatar_hash: Option<String> }
pub struct InviteResponse { pub code: String }
pub struct Server { pub id: ServerId, pub name: String, pub icon_hash: Option<String> }
#[serde(rename_all = "snake_case")] pub enum ChannelKind { Text, Voice, Dm, Group }
pub struct Channel { pub id: ChannelId, pub server_id: Option<ServerId>, pub kind: ChannelKind, pub name: Option<String>, pub position: i64 }
pub struct CreateServerRequest { pub name: String }
pub struct CreateChannelRequest { pub kind: ChannelKind, pub name: String }
pub struct CreateDmRequest { pub user_ids: Vec<UserId> }   // 1 other → dm, 2..9 → group
pub struct Member { pub user: User, pub nickname: Option<String> }
#[serde(rename_all = "snake_case")] pub enum MessageKind { Normal, System }
pub struct Message { pub id: MessageId, pub channel_id: ChannelId, pub author_id: Option<UserId>, pub kind: MessageKind,
  pub content: String, pub reply_to_id: Option<MessageId>, pub created_at: String, pub edited_at: Option<String>, pub deleted: bool }
pub struct SendMessageRequest { pub content: String, pub reply_to_id: Option<MessageId>, pub nonce: Option<String> }
pub struct EditMessageRequest { pub content: String }
pub struct VoiceTokenResponse { pub url: String, pub token: String }
pub struct ApiError { pub code: String, pub message: String }

// gateway.rs
#[serde(tag = "op", content = "d")] pub enum ClientFrame { Hello { token: String }, Heartbeat, Typing { channel_id: ChannelId } }
pub struct Ready { pub me: User, pub servers: Vec<Server>, pub channels: Vec<Channel>, pub members: Vec<ServerMembers>, pub dm_members: Vec<DmMembers> }
pub struct ServerMembers { pub server_id: ServerId, pub members: Vec<Member> }
pub struct DmMembers { pub channel_id: ChannelId, pub user_ids: Vec<UserId> }
#[serde(tag = "op", content = "d")] pub enum ServerFrame { Ready(Ready), HeartbeatAck, Event(Event) }
#[serde(tag = "t", content = "d")] pub enum Event {
  MessageCreated { message: Message, nonce: Option<String> }, MessageUpdated { message: Message }, MessageDeleted { channel_id: ChannelId, message_id: MessageId },
  ChannelCreated { channel: Channel }, ServerCreated { server: Server }, MemberJoined { server_id: ServerId, member: Member },
  Typing { channel_id: ChannelId, user_id: UserId },
  VoiceJoined { channel_id: ChannelId, user_id: UserId }, VoiceLeft { channel_id: ChannelId, user_id: UserId } }
```

- [ ] **Step 1: Failing test**: `protocol/tests/export_types.rs`
```rust
#[test]
fn export_bindings() {
    // ts-rs writes on `TS::export_all`; TS_RS_EXPORT_DIR points at the UI.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../client/ui/src/lib/protocol");
    unsafe { std::env::set_var("TS_RS_EXPORT_DIR", &dir) };
    pulse_protocol::export_all().unwrap();
    assert!(dir.join("Message.ts").exists());
    assert!(dir.join("ServerFrame.ts").exists());
}
#[test]
fn event_wire_format_is_stable() {
    let e = pulse_protocol::gateway::Event::Typing { channel_id: "01J0000000000000000000000A".parse().unwrap(), user_id: "01J0000000000000000000000B".parse().unwrap() };
    assert_eq!(serde_json::to_string(&e).unwrap(),
      r#"{"t":"Typing","d":{"channel_id":"01J0000000000000000000000A","user_id":"01J0000000000000000000000B"}}"#);
}
```
Run `cargo test -p pulse-protocol` → FAIL (types missing).

- [ ] **Step 2: Implement** the types above plus `pub fn export_all() -> Result<(), ts_rs::ExportError>` in `lib.rs` that calls `X::export_all()` on the root types (`ServerFrame`, `ClientFrame`, every REST struct). Run → PASS; the `.ts` files exist.

- [ ] **Step 3: Commit** `feat(protocol): shared REST + gateway types with TS generation` (include the generated TS).

---

### Task 3: Server foundation: config, DB, migrations, errors, health

**Files:**
- Create: `server/migrations/0001_init.sql`, `server/src/{config.rs, db.rs, error.rs, state.rs, lib.rs}`, `server/tests/common/mod.rs`

**Interfaces:**
- Produces: `pub struct Config { pub db_url: String, pub bind: SocketAddr, pub livekit_url: String, pub livekit_key: String, pub livekit_secret: String }` + `Config::from_env()`; `pub async fn connect(url: &str) -> sqlx::Result<SqlitePool>` (WAL, foreign_keys=ON, runs migrations); `pub struct AppState { pub db: SqlitePool, pub cfg: Arc<Config>, pub hub: Hub }` (Hub from Task 8; until then `Hub::default()` stub); `pub enum AppError { NotFound, Forbidden, Unauthorized, Gone, BadRequest(String), Conflict(String), Internal(anyhow::Error) }` implementing `IntoResponse` → JSON `ApiError` with 404/403/401/410/400/409/500; `pub fn router(state: AppState) -> Router`; test helper `common::spawn() -> TestApp { addr, http: reqwest::Client, db }` (in-memory db `sqlite::memory:` with a single shared connection, server on `127.0.0.1:0`).

- [ ] **Step 1: Migration**: `0001_init.sql` creates every table from spec §4 as `STRICT`, including `users, sessions, invites, servers, server_members, channels, channel_members, nickname_history, roles, member_roles, messages, reactions, mentions, read_states, notification_prefs, images, voice_sessions, user_badges`, with FKs, plus indexes `messages(channel_id, id)`, `sessions(user_id)`, `channel_members(user_id)`. Timestamps are TEXT RFC 3339 UTC.

- [ ] **Step 2: Failing test**: `server/tests/health.rs`
```rust
mod common;
#[tokio::test]
async fn health_ok_and_schema_migrated() {
    let app = common::spawn().await;
    let r = app.http.get(app.url("/health")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='messages'")
        .fetch_one(&app.db).await.unwrap();
    assert_eq!(n, 1);
}
```
- [ ] **Step 3: Implement** config/db/error/state/router + `GET /health` → `"ok"`. Deps: axum 0.8, sqlx 0.8 (`runtime-tokio`, `sqlite`, `migrate`, `macros`), tower-http (trace, cors), tracing-subscriber, reqwest (dev, json), chrono. `main.rs`: `serve` subcommand (clap) runs `Config::from_env()` → `connect` → bind. Run tests → PASS.
- [ ] **Step 4: Commit** `feat(server): config, sqlite + migrations, error type, health`

---

### Task 4: Passwords & sessions core

**Files:** `server/src/auth/{password.rs, session.rs}`; unit tests inline.

**Interfaces (Produces):** `password::hash(&str) -> anyhow::Result<String>`, `password::verify(&str, &str) -> bool` (argon2id defaults); `session::create(db, UserId) -> Result<String /*raw token*/>` (32 random bytes → base64url, stores sha256 hex, expires now+90d); `session::authenticate(db, &str) -> Result<Option<UserId>>` (looks up by hash, rejects expired, slides `expires_at` to now+90d and `last_used_at`; updates at most once per hour to avoid write churn); `session::revoke(db, &str)`.

- [ ] **Step 1: Failing tests**: `hash_then_verify_roundtrip`, `verify_rejects_wrong_password`, `token_authenticates_and_slides_expiry`, `expired_token_rejected` (manually set `expires_at` in the past), `revoked_token_rejected`, `stored_value_is_hash_not_token` (assert DB row != raw token).
- [ ] **Step 2: Implement**, run → PASS.
- [ ] **Step 3: Commit** `feat(server): argon2 passwords + hashed sliding sessions`

---

### Task 5: Auth routes: invites, register, login, logout, extractor, bootstrap CLI

**Files:** `server/src/auth/{routes.rs, invites.rs, extractor.rs, mod.rs}`, `server/tests/auth.rs`

**Interfaces:**
- Produces: `AuthUser(pub UserId)` axum extractor (reads `Authorization: Bearer <token>`, → 401 otherwise); routes `POST /auth/register`, `POST /auth/login`, `POST /auth/logout`, `GET /me`, `POST /invites` (auth) → `InviteResponse`; CLI `pulse-app-server create-invite` prints a code (for the very first user); test helper `common::register(app, name) -> (UserId, token)` that mints an invite directly in the DB.

- [ ] **Step 1: Failing tests** (`server/tests/auth.rs`):
  - `register_with_invite_then_me` → 200, `/me` returns the username
  - `register_bad_invite_404`
  - `invite_single_use_410`: second register with the same code → 410
  - `concurrent_redemption_creates_one_user`: 10 parallel registers with one code → exactly one 200, `count(users)=1`. Redemption is one `UPDATE invites SET used_by=?, used_at=? WHERE code=? AND used_by IS NULL` inside the same transaction as the user insert; `rows_affected()==0` → 410.
  - `duplicate_username_409`
  - `login_wrong_password_401` (same response for unknown user)
  - `logout_revokes_token` → `/me` 401 afterwards
  - `me_without_token_401`
  - Username rules: 2–32 chars `[a-z0-9_.]`; password ≥ 8 → 400 otherwise.
- [ ] **Step 2: Implement**, run → PASS.
- [ ] **Step 3: Commit** `feat(server): invite-only registration, login/logout, auth extractor`

---

### Task 6: Servers, channels, membership, DMs + access rule

**Files:** `server/src/servers/{mod.rs, routes.rs}`, `server/src/access.rs`, `server/tests/servers.rs`

**Interfaces:**
- Produces: `access::channel_for(db, UserId, ChannelId) -> Result<Channel, AppError>`, which returns `NotFound` if the channel doesn't exist **or** is dm/group and the user isn't in `channel_members`. **Every channel-scoped route uses this.** Routes:
  - `POST /servers` → creates the server, adds the creator as a member, creates `#general` (text, pos 0) and `Lounge` (voice, pos 0); returns `Server`
  - `GET /servers` → all servers (public to every user)
  - `POST /servers/{id}/join` → idempotent
  - `GET /servers/{id}/channels`
  - `POST /servers/{id}/channels` (must be a member; kind text|voice)
  - `GET /servers/{id}/members`
  - `POST /dms` (`CreateDmRequest`): 1 other user → reuses the existing 1:1 dm if present, else creates a `dm`; 2–9 others → a new `group`; the caller is always included; unknown user → 400
  - `GET /dms` → only the caller's dm/group channels
- [ ] **Step 1: Failing tests**:
  - `create_server_seeds_general_and_lounge`
  - `all_servers_visible_to_everyone`
  - `join_is_idempotent`
  - `non_member_cannot_create_channel_403`
  - `dm_reused_for_same_pair`
  - `group_includes_creator`
  - `dms_list_only_mine`: C does not see A↔B
  - `access_rule_hides_private_channel`: `channel_for(C, ab_dm)` → NotFound
- [ ] **Step 2: Implement**, run → PASS.
- [ ] **Step 3: Commit** `feat(server): servers, channels, membership, DMs/groups, access rule`

---

### Task 7: Messages

**Files:** `server/src/messages/{mod.rs, routes.rs}`, `server/tests/messages.rs`, `server/tests/privacy.rs`

**Interfaces:**
- Produces:
  - `GET /channels/{id}/messages?before=<MessageId>&limit=<1..=100, default 50>` → `Vec<Message>`, newest first
  - `POST /channels/{id}/messages` (`SendMessageRequest`) → `Message`
  - `PATCH /messages/{id}` → `Message`
  - `DELETE /messages/{id}` → 204
  - Each publishes through `hub.publish(&db, Event)` (Task 8; until then a no-op stub with the same signature)
  - Content 1–4000 chars after trim; stored as raw markdown (rendering and sanitising happen client-side, spec §11)
  - Soft delete: `content=''`, `deleted_at=now`; listed messages have `deleted: true`
  - `reply_to_id` must be in the same channel, else 400
- [ ] **Step 1: Failing tests**:
  - `send_then_list_newest_first`
  - `pagination_before_cursor` (120 messages → pages of 50/50/20)
  - `edit_own_sets_edited_at`
  - `edit_others_403`
  - `delete_own_soft_deletes`
  - `edit_deleted_404`
  - `reply_must_be_same_channel_400`
  - `empty_or_too_long_400`
  - `privacy.rs`: `outsider_gets_404_everywhere`: A and B have a dm with a message M; C gets 404 on GET list, POST send, PATCH M, DELETE M; then assert M is unchanged in the DB
- [ ] **Step 2: Implement**, run → PASS.
- [ ] **Step 3: Commit** `feat(server): messages with pagination, edit, soft delete, privacy tests`

---

### Task 8: Gateway: hub, `audience_for`, socket protocol

**Files:** `server/src/gateway/{mod.rs, hub.rs, audience.rs, socket.rs}`, `server/tests/gateway.rs`

**Interfaces:**
- Produces:
  - `pub enum Audience { Everyone, Users(HashSet<UserId>) }`
  - `pub async fn audience_for(db: &SqlitePool, event: &Event) -> anyhow::Result<Audience>`:
    - events carrying a `channel_id` → look up the channel; `dm`/`group` → `Users(channel_members)`, else `Everyone`
    - `ServerCreated`/`MemberJoined` → `Everyone`
    - `ChannelCreated` → per channel kind as above
    - `VoiceJoined`/`VoiceLeft` → per channel kind
  - `Hub` (`Clone`, `Arc` inside): `register(UserId) -> (ConnId, mpsc::Receiver<ServerFrame>)` with a **bounded** channel (256); `unregister(ConnId)`; `publish(&db, Event)` computes the audience and `try_send`s to each matching connection, and **a full queue drops that connection** (its sender is removed so its socket task ends).
  - `GET /gateway` WebSocket:
    1. the first frame must be `Hello{token}` within 10 s, else close 4001
    2. invalid token → close 4001 "unauthorized"
    3. then send `Ready` (servers, channels incl. the user's dms, members, dm_members)
    4. `Heartbeat` → `HeartbeatAck`; no frame for 60 s → close 4002
    5. `Typing{channel_id}` → access check via `channel_for`, then publish `Event::Typing` (silently ignored if not allowed)
- [ ] **Step 1: Failing unit tests** (`audience.rs`, table-driven):
  - text channel event → Everyone
  - dm event → exactly {A, B}
  - group → its 3 members
  - `ServerCreated` → Everyone
  - `VoiceJoined` in a dm → {A, B}
- [ ] **Step 2: Failing integration tests** (`server/tests/gateway.rs`, using `tokio-tungstenite`):
  - `hello_then_ready_contains_my_servers_and_dms`
  - `bad_token_closed_4001`
  - `no_hello_closed_after_timeout` (make the timeout a config value; 200 ms in tests)
  - `message_event_reaches_server_members`
  - **`outsider_receives_zero_private_events`**:
    - A, B and C are connected; A↔B have a dm.
    - A sends 3 dm messages, A types in the dm, and A edits and deletes one.
    - Then A posts in a public channel (the marker).
    - Read C's frames until the marker → assert the marker is the **only** `Event` C received.
  - **`slow_client_does_not_block_others`**: a client that never reads; publish 1000 events; a second client still receives the last event within 2 s, and the slow client's socket is closed.
- [ ] **Step 3: Implement**, wire `hub.publish` into the Task 6/7 routes (`ServerCreated`, `ChannelCreated`, `MemberJoined`, message events). Run all tests → PASS.
- [ ] **Step 4: Commit** `feat(server): gateway hub with audience_for privacy fan-out`

---

### Task 9: Voice tokens + LiveKit webhook

**Files:** `server/src/voice/{mod.rs, routes.rs, webhook.rs}`, `server/tests/voice.rs`

**Interfaces:**
- Produces:
  - `POST /voice/{channel_id}/token` → `VoiceTokenResponse`. Allowed for voice channels (any user) and dm/group (members only, via `channel_for`); text channel → 400. The token is a livekit-api `AccessToken` with identity = UserId, room = ChannelId, `room_join` + `can_publish` + `can_subscribe`, TTL 10 min.
  - `POST /livekit/webhook`: verify with livekit-api's webhook receiver (`livekit_api::webhooks::WebhookReceiver` + `TokenVerifier::with_api_key`). Verify the exact API against livekit-api 0.8.1 docs.rs at implementation time; if the module is missing, verify the JWT `sha256` claim against the body manually with `livekit_api::access_token::TokenVerifier`.
    - `participant_joined` → in-memory voice state insert + `voice_sessions` row (only if the channel has a `server_id`) + publish `VoiceJoined`
    - `participant_left` → the reverse, closing the session row
    - bad signature → 401
- [ ] **Step 1: Failing tests**:
  - `token_for_server_voice_channel`: decode the JWT with the secret and check identity and room
  - `token_refused_for_outsider_dm_404`
  - `token_text_channel_400`
  - `webhook_bad_signature_401`
  - `webhook_join_server_channel_records_session_and_broadcasts`
  - `webhook_join_dm_call_writes_no_voice_session`: spec §8 rule, DM calls never count
- [ ] **Step 2: Implement**, run → PASS.
- [ ] **Step 3: Commit** `feat(server): LiveKit tokens + verified webhooks, server-only voice sessions`

---

### Task 10: CI

**Files:** `.github/workflows/ci.yml`, `.github/workflows/release-windows.yml`

- [ ] **Step 1: `ci.yml`**:
  - on `pull_request` and `push` to `main`
  - `concurrency: { group: ci-${{ github.ref }}, cancel-in-progress: true }`
  - `dorny/paths-filter` → jobs:
    - `rust` (ubuntu-latest, when protocol/server/client/src-tauri/Cargo.* changes): apt `libglib2.0-dev libclang-dev pkg-config libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev libasound2-dev`, `dtolnay/rust-toolchain@stable` with rustfmt and clippy, `Swatinem/rust-cache@v2`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `just types-check` (via `extractions/setup-just`)
    - `ui` (when client/ui changes): pnpm setup, `pnpm install --frozen-lockfile`, `pnpm check`
  - `permissions: contents: read`
- [ ] **Step 2: `release-windows.yml`**:
  - on `push: tags: ['v*']` and `workflow_dispatch`
  - windows-latest, rust stable, `Swatinem/rust-cache`, pnpm
  - `cargo tauri build` in `client/src-tauri`
  - upload the `.msi`/`.exe` artifacts
- [ ] **Step 3: Validate locally**: `just check` passes. Push, then `gh run watch` → the CI run is green.
- [ ] **Step 4: Commit** `ci: path-filtered checks + tag-only Windows build`

---

### Task 11: Tauri client shell: login + servers

**Files:** `client/src-tauri/*`, `client/ui/*` (see File Structure)

**Interfaces:**
- Consumes: REST `/auth/login`, `/auth/register`, `/me`, `/servers`; protocol TS types
- Produces:
  - Tauri commands `login(server_url, username, password) -> User`, `register(server_url, invite_code, username, password) -> User`, `restore_session() -> Option<User>`, `logout()`, `list_servers() -> Vec<Server>`
  - The token is stored with the `keyring` crate (service `pulse-app`, account = server_url) and **never returned to the UI**
  - UI: `Login.svelte` (server URL, username, password, an invite-code field for register mode) and `Shell.svelte` (pill tab bar listing servers + Home, rounded panels, palette from the mockup tokens: `--bg-0:#141519 … --bg-4:#34363d`, Onest font, `--accent:#8b9cff`)
- [ ] **Step 1: Scaffold.**
  - `client/ui`: Vite + Svelte 5 + TS (`pnpm create vite ui --template svelte-ts`), add `@tauri-apps/api`, set the `check` script to `svelte-check`.
  - `client/src-tauri`: Tauri 2 (`tauri = "2.12"`, `tauri-build`). `tauri.conf.json`: productName "Pulse", identifier `app.pulse.client`, `frontendDist: ../ui/dist`, `devUrl: http://localhost:1420` (vite `server: { port: 1420, strictPort: true }`), a CSP that blocks all remote origins except the IPC.
  - Add `client/src-tauri` to the workspace members. Binary name `pulse-app`.
  - Install the Tauri CLI as a cargo binary: `cargo install tauri-cli --version "^2" --locked`.
- [ ] **Step 2: Failing Rust test** (`api.rs`): `login_against_test_server_returns_user`. It spawns `pulse_server` in-process using the `tests/common`-style spawn exposed as `pulse_server::testing::spawn()` behind a `testing` feature, then calls `api::login(...)`.
- [ ] **Step 3: Implement** `api.rs` (reqwest, JSON, maps `ApiError`), `session.rs` (keyring get/set/delete), `commands.rs`, and the UI screens. Run `cargo test -p pulse-client`, then `cd client/ui && pnpm check` → PASS.
- [ ] **Step 4: Manual smoke**: `just dev-livekit`, `just dev-server`, `pulse-app-server create-invite`, `just dev-client`. Register, see the server tabs, restart the app, and you're still logged in.
- [ ] **Step 5: Commit** `feat(client): Tauri shell with keychain session, login/register, server tabs`

---

## Out of scope for this plan (next plans)

VoiceManager in the client (after the spike decision), the call UI and ringing, text-channel UI, the remaining text
features, identity (nicknames, roles, avatars, profiles), badges, settings, mute/DND, hosting.
