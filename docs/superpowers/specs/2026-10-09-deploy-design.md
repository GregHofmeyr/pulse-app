# Pulse deployment — design

**Date:** 2026-10-09 · **Status:** approved in conversation, awaiting written-spec review
**Goal:** Pulse reachable over the internet at `pulsechat.co.za` for ~10 friends in South Africa, before the
Windows test, so that test exercises the real setup (internet, NAT, TLS) instead of a LAN.

## 1. Decisions (settled — don't re-litigate)

- **Host:** Oracle Cloud Always Free, home region **South Africa Central (Johannesburg)**, shape
  `VM.Standard.A1.Flex` (ARM) 2 OCPU / 12 GB, Ubuntu 24.04. **Fallback:** xneelo Cloud (Johannesburg,
  ~R120/month for 1 vCPU / 2 GB, unlimited traffic) if Oracle has no capacity or is a pain.
  Everything below is host-agnostic; switching = run `setup.sh` on the new box and change two DNS records.
- **Domain:** `pulsechat.co.za`, registered at xneelo, nameservers at **Cloudflare (DNS only, proxy OFF)**
  — the proxy cannot carry WebRTC media. Cloudflare chosen over xneelo DNS for minute-level record
  changes (the Oracle → xneelo move is a record change) and an API.
- **Names:** `pulsechat.co.za` = Pulse server (REST + gateway). `voice.pulsechat.co.za` = LiveKit signalling.
- **Runtime:** Docker Compose, three containers. No Kubernetes, no Redis (single node; the in-memory hub
  already fans out events — text traffic is push over WebSocket, nothing polls).
- **Data:** disposable until the real launch. Server and clients ship in lockstep; no backward compatibility,
  but old clients are told to update (§6).
- **Accounts:** every account (Oracle, Cloudflare, Tailscale, GitHub, xneelo) on Greg's **personal** email.
- **Nothing exists only on Greg's laptop** (it is a work machine) — see §9.

## 2. Server layout

| Container | Role | Exposed |
|---|---|---|
| `caddy` (official image, pinned) | TLS (Let's Encrypt, auto-renew), reverse proxy by hostname | 80, 443 |
| `pulse-server` (`ghcr.io/greghofmeyr/pulse-server`, pinned by commit) | REST, gateway, SQLite | no — `127.0.0.1:7890` |
| `livekit` (`livekit/livekit-server`, pinned) | voice SFU + built-in TURN | media ports only; signalling `127.0.0.1:7880` |

- All three use **host networking** (LiveKit's recommendation for media; matches dev). Internal services bind
  to loopback, so Caddy is the only HTTP entry point.
- Caddy routes: `pulsechat.co.za` → `127.0.0.1:7890`; `voice.pulsechat.co.za` → `127.0.0.1:7880`.
  `pulsechat.co.za/livekit/webhook` returns 404 from Caddy (LiveKit posts to it over loopback; the signature
  check stays as a second layer). Port 80 only serves ACME challenges and redirects to HTTPS.
- LiveKit: `rtc.udp_port: 7882` (single-port UDP mux), `rtc.tcp_port: 7881`, `use_external_ip: true`,
  TURN on `3478/udp`, webhook → `http://127.0.0.1:7890/livekit/webhook`, fresh random key/secret.
  TURN over TLS/443 is **deferred** until a friend can't connect (needs cert sharing with Caddy).
- Server env: `PULSE_LIVEKIT_URL=wss://voice.pulsechat.co.za` (the URL handed to clients),
  `PULSE_DB_URL=sqlite:///data/pulse.db`, `PULSE_BIND=127.0.0.1:7890`, LiveKit key/secret.
- Containers run as a non-root user; Docker log driver `json-file` with rotation (10 MB × 3) so logs can't
  fill the disk. Compose `healthcheck` on `GET /health`.

**On disk** (owned by a dedicated `pulse` user):
```
/srv/pulse/compose.yml  Caddyfile  livekit.yaml  .env (mode 600)
/srv/pulse/data/pulse.db      ← what backups copy
/srv/pulse/caddy/             ← certificates + Caddy state
```

**Firewall** (default deny inbound): `80/tcp`, `443/tcp`, `7881/tcp`, `7882/udp`, `3478/udp`. **No public SSH**
(§4). On Oracle the rules must be opened in **both** the VCN security list and the instance's own
iptables (Oracle's Ubuntu image blocks almost everything by default). xneelo: security groups only.

## 3. App hardening (code changes)

1. **Rate limits** (in the server, keyed by client IP and/or user):
   - `POST /auth/login`: 5/min per IP and 10/hour per username → 429.
   - `POST /auth/register`: 5/hour per IP.
   - `POST /channels/{id}/messages`: burst 20, refill 5/s per user.
   - Gateway `Typing` frames: at most 1/s per user (extra ones dropped silently).
   - Client IP = the address Caddy appends to `X-Forwarded-For` (rightmost entry). Trusting it is safe only
     because the server listens on loopback; without the header (dev, tests) the socket address is used.
2. **Invites expire after 24 hours** (`created_at` + 24 h), on top of single use. Expired → same error as unknown.
3. **Version handshake:** `Hello` gains `client_version: u32` (a protocol number, not the app semver). Server
   const `MIN_CLIENT_VERSION`; older or missing → close code **4005** "update required". The client shows
   **"plz update :)"** with a line on where to get the new build, and stops reconnecting.
4. **`pulse-app-server backup <path>`** subcommand: consistent online snapshot via `VACUUM INTO`.
5. **Logs:** audit that no message content, passwords, tokens or invite codes are ever logged; add a test
   where practical.
6. **Default server URL:** the login screen's server field is pre-filled from a build-time value
   (`VITE_DEFAULT_SERVER`); release builds use `https://pulsechat.co.za`, dev keeps `http://127.0.0.1:7890`.

## 4. Host hardening (`deploy/setup.sh`, idempotent)

- Creates the `pulse` user; installs Docker (official repo), `unattended-upgrades` (security updates, auto
  reboot off — Greg reboots when convenient), `rclone`, `age`.
- **Tailscale** with **Tailscale SSH**: the box is reachable only from Greg's tailnet; public port 22 closed
  once Tailscale SSH works (the script refuses to close it before a Tailscale SSH session is confirmed).
  sshd: key-only, no root login.
- Firewall per §2 (ufw on xneelo; ufw + Oracle iptables fix on Oracle).
- Installs the backup systemd timer (§5).
- Deliberately **not** included: IDS, WAF, disk encryption, fail2ban (redundant without public SSH), honeypot.
  **CrowdSec** + Caddy bouncer is the upgrade path if logs ever show real noise.

## 5. Backups

- Daily systemd timer: `docker compose exec pulse-server pulse-app-server backup` → encrypt with **age**
  (recipient = Greg's public key; the server can't decrypt) → `rclone` to a **Cloudflare R2** bucket.
- Retention: 14 daily + 8 weekly; older objects pruned by the job.
- R2 credentials scoped to that one bucket, stored in `/srv/pulse/.env`.
- `deploy/restore.sh` (run from any trusted machine): download, decrypt, stop server, swap DB, start.
- **A real restore is performed once during setup** (into a scratch copy, then compared).
- Every `just deploy` takes a backup first.

## 6. CI and updates

- New workflow **`release-server`**: on push to `main` after `ci` passes → build the image on **native**
  runners for `amd64` and `arm64` (GitHub's free ARM runners for public repos; no QEMU) → push to GHCR as
  `:<sha>` and `:latest` → multi-arch manifest. Public image (no pull credentials). Uses `GITHUB_TOKEN` only.
- **`release-windows`** builds with `VITE_DEFAULT_SERVER=https://pulsechat.co.za`.
- **`just deploy [sha]`** (from any machine on the tailnet): backup → set image tag → `docker compose pull`
  → `up -d` → wait for `/health`. Migrations run at server start. Restart = a few seconds; clients reconnect;
  voice continues (media goes to LiveKit).
- **Rollback:** `just deploy <previous-sha>`; if a migration broke data, `restore.sh` the pre-deploy backup.
- **Optional:** a free external uptime check on `https://pulsechat.co.za/health` that emails Greg.

## 7. Secrets

| Secret | Lives | Notes |
|---|---|---|
| LiveKit API key/secret | server `.env` (600) | generated by `setup.sh`; regenerable |
| R2 access key (one bucket) | server `.env` | revocable in Cloudflare |
| age **public** key | server | encrypt-only |
| age **private** key | Greg's laptop + **NordPass** (synced to another device) | the only irreplaceable secret |
| Oracle first-login SSH key | NordPass | only until Tailscale SSH works |

No secrets in GitHub (deploys run from Greg's machine; image pushes use `GITHUB_TOKEN`).

## 8. Build order

1. **Code prep** (local, TDD): rate limits, 24 h invites, version handshake + "plz update :)", `backup`
   subcommand, default server URL, log audit, Dockerfile, `release-server` workflow, `deploy/` files, runbook.
2. **Server:** Oracle instance (fallback xneelo) → `setup.sh` → Tailscale → firewalls.
3. **First deploy:** DNS A records (`pulsechat.co.za`, `voice.pulsechat.co.za`, proxy off) → `compose up` →
   certificates → smoke test: two local profiles chatting + voice through the real server.
4. **Backups:** R2 bucket, age key, timer, **one real restore**.
5. **Windows test** with friends on the deployed server.
6. **Optimisation pass** with real traffic: data per message/minute of voice, latency, audio quality.

## 9. Lost or returned laptop

Nothing lives only on the laptop: code on GitHub, images on GHCR, server secrets on the server, server access
via Tailscale identity, backup key in NordPass. Recovery on a new machine: install Tailscale (personal login),
clone the repo, restore the age key from NordPass. **Stolen:** remove the device in the Tailscale admin console
(works from a phone); rotate the R2 key if paranoid. `deploy/README.md` carries this as a checklist.

## 10. Testing

- Unit/integration tests for every §3 change (rate-limit 429s, invite expiry, 4005 close, backup produces a
  readable consistent DB, XFF parsing incl. spoofed left-most entries).
- `deploy/` files: `docker compose config` validates; `caddy validate` on the Caddyfile in CI.
- Smoke script against the live server: `/health`, TLS cert valid for both names, register-with-invite,
  gateway connects, voice token issued, LiveKit reachable on 7882/udp (a real call from two profiles).
- Restore drill (§5).

## 11. Out of scope

TURN over TLS/443, CrowdSec, Litestream (continuous replication), staging environment, auto-update for clients,
multi-region, bitrate configurability (now unblocked by the hosting decision — its own small follow-up).
