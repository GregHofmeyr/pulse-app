# Running Pulse on the internet

The production setup for `pulsechat.co.za`. The design and the reasons behind it are in
`docs/superpowers/specs/2026-10-09-deploy-design.md`; this file is the how-to.

## 1. What runs where

One small Ubuntu server, three containers (Docker Compose, all on host networking, all running as the
`pulse` user):

| Container | Does | Listens |
|---|---|---|
| `caddy` | HTTPS certificates (Let's Encrypt) and routing by name | 80, 443 (public) |
| `pulse-server` | REST API, gateway, SQLite | 127.0.0.1:7890 only |
| `livekit` | voice | 7880 (signalling, firewalled; reached through Caddy), 7881/tcp, 7882/udp, 3478/udp (public) |

- `https://pulsechat.co.za` → Caddy → pulse-server. `/livekit/webhook` is answered with 404 by Caddy; LiveKit
  posts there over loopback.
- `wss://voice.pulsechat.co.za` → Caddy → LiveKit signalling. Audio goes straight to LiveKit over UDP.
- Firewall allow-list: `80/tcp 443/tcp 7881/tcp 7882/udp 3478/udp 41641/udp` (41641 = Tailscale). SSH only
  over Tailscale.

On the server everything lives in `/srv/pulse`:

```
compose.yml  Caddyfile  livekit.yaml  backup.sh  .env (600: settings + secrets)
data/pulse.db                         the database
data/media/                           uploaded emoji
caddy/                                certificates and Caddy state
```

## 2. Accounts

All on your **personal** email, never a work one (losing a work laptop or job must not lock you out):
GitHub, Oracle Cloud (or xneelo), Cloudflare (DNS + R2), Tailscale, UptimeRobot.

## 3. Your machine

1. Tailscale: `sudo pacman -S tailscale && sudo systemctl enable --now tailscaled && sudo tailscale up`
   (log in with the personal account). MagicDNS is on by default, so the server is just `pulse`.
2. Tools for backups: `sudo pacman -S age sqlite`.
3. `just`, `ssh`, `scp`, `curl` and `dig` are all the other commands use.

## 4. A new server

1. Create a VM running **Ubuntu 24.04** with a public IPv4 address.
   - Oracle: shape `VM.Standard.A1.Flex` in Johannesburg; make the network with the VCN wizard
     ("Create VCN with Internet Connectivity") and put the VM in its public subnet.
   - xneelo Cloud: the smallest instance is plenty.
2. Cloud firewall: allow inbound from `0.0.0.0/0` on TCP 80, 443, 7881 and UDP 7882, 3478, 41641 (plus TCP 22
   for now). Oracle: VCN → public subnet → Default Security List → Add Ingress Rules. xneelo: security groups.
3. From the repo: `just provision ubuntu@<public-ip> <path to the VM's ssh key>`. It installs Docker, automatic
   security updates, the firewall (Oracle's built-in iptables rules included), the UDP buffer sysctl
   LiveKit wants (`/etc/sysctl.d/60-pulse-udp.conf`), the `pulse` user, `/srv/pulse` with a fresh LiveKit
   secret, the nightly backup timer, and Tailscale.
4. When it prints a Tailscale login URL, open it and approve the machine.
5. Check: `ssh ubuntu@pulse hostname` prints `pulse`.
6. **Turn off key expiry for the server**: Tailscale admin console → Machines → `pulse` → ⋯ → *Disable key
   expiry*. Tailscale keys expire after 180 days by default; once public SSH is closed, an expired key
   would lock you out (the only way back in is the cloud provider's serial console).
7. `just close-public-ssh` closes port 22 on the server itself; then delete the TCP 22 rule from the cloud
   firewall too. From now on the server is only reachable for SSH over Tailscale.

## 5. DNS (Cloudflare)

Two A records, both **DNS only (grey cloud)**: `@` → the public IP, `voice` → the public IP.
The orange-cloud proxy can't carry voice; never turn it on for these.

## 6. First deploy

1. Make the image public, once: GitHub → your profile → Packages → `pulse-server` → Package settings →
   Change visibility → Public.
2. `ssh -t pulse@pulse 'TERM=xterm-256color nano /srv/pulse/.env'` → set `ACME_EMAIL` (your personal address).
3. `just deploy` → ends with `healthy: latest`. Caddy fetches the certificates on the first request.
4. `just smoke` → every line `ok`.
5. `just invite-prod` → a code for the first account.

## 7. Deploying and rolling back

- Every push to `main` that passes CI builds `ghcr.io/greghofmeyr/pulse-server:<7-char commit>` and `:latest`
  (`.github/workflows/release-server.yml`, about ten minutes).
- `just deploy` runs `latest`; `just deploy <sha7>` runs a specific build. Each deploy takes a backup first,
  pulls (a mistyped sha fails here and changes nothing), pins the image (below), restarts in a few seconds, and prints the commit
  that's now live. Right after a merge, `latest` is still the previous build until the image workflow
  finishes (about ten minutes): check the printed commit. Clients reconnect on their own and voice keeps going.
- Images are pinned by digest, so what runs is exactly what was pulled, even if a tag moves. `deploy.sh` looks up
  the digest of the pulled `pulse-server:<tag>` and writes `PULSE_IMAGE=…:<tag>@sha256:…` into `/srv/pulse/.env`
  (`PULSE_TAG` stays, for humans; an old `.env` without `PULSE_IMAGE` just gets the line appended). It prints
  `pinned: …` and the digest on the final `healthy:` line. LiveKit and Caddy are pinned in `compose.yml`.
- Bumping LiveKit or Caddy: Dependabot opens a PR for new tags (`.github/dependabot.yml`). To do it by hand,
  take the digest of the multi-arch **index**, not of one platform: `docker manifest inspect <image>:<tag> -v`, or
  `HEAD /v2/<repo>/manifests/<tag>` on the registry with the OCI index and manifest-list `Accept` headers and read
  `Docker-Content-Digest`. Check that the index lists `linux/arm64` (the server is ARM64). Then update
  `compose.yml` (CI and `just deploy-check` read the Caddy image from it), run `just deploy-check`, `just deploy`.
- Order for a release: deploy the server first, then send friends the new installer. Clients older than the
  server see "plz update :)".
- Building a branch: running `release-server` by hand (workflow dispatch) also tags it `:latest`, as every
  CI run on `main` does. Deploy a branch build only as `just deploy <sha7>`, never `just deploy`, and merge
  that branch before anything else lands on `main`: the next `main` build moves `latest` back to code
  without it.
- Roll back: `just deploy <previous sha7>`. If a database migration went wrong, also `just restore <backup>`
  (section 8) with the backup the deploy made.
- A deploy that ran a new migration can't be rolled back by image alone: an older image refuses to start
  against a database migrated past what it knows (e.g. `0004_message_nonce`). Roll back with
  `just restore <the backup that deploy made>` and then `just deploy <previous sha7>`.
- `just logs` (or `just logs caddy` / `just logs livekit`) follows the live logs.
- After changing `livekit.yaml`: `just deploy`, then `ssh pulse@pulse 'cd /srv/pulse && docker compose restart livekit'`.
  The codec list needs a LiveKit restart to apply; `just deploy` copies the file but does not restart LiveKit.

## 8. Backups

Nightly at about 03:00 (and before every deploy): a consistent snapshot of the database and a tarball of the
uploaded media (`data/media`, e.g. custom emoji; skipped until something has been uploaded), each encrypted with
**age** for your key and uploaded to **Cloudflare R2**. 14 daily and 8 weekly copies of each are kept. The server can
encrypt but never decrypt. A plaintext copy never stays on the server, even if a step fails.

Setup, once:

1. Cloudflare → R2 → create the bucket `pulse-backups`.
2. R2 → Manage API tokens → Create: permission **Object Read & Write**, only for `pulse-backups`. Note the
   Access Key ID, the Secret Access Key and your account ID.
3. On your machine: `mkdir -p ~/.config/pulse && age-keygen -o ~/.config/pulse/backup-age.key`.
   **Copy the whole file into NordPass now** and check it syncs to a second device. Without it the backups
   can't be opened. The line `# public key: age1…` is the public half.
4. `ssh -t pulse@pulse 'TERM=xterm-256color nano /srv/pulse/.env'` → fill `AGE_RECIPIENT` (the `age1…` key), `R2_ACCOUNT_ID`,
   `R2_BUCKET=pulse-backups`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`.
5. `just backup-now` → `backed up pulse-<time>.db.age`.
6. `just restore-drill` → downloads the latest, decrypts it here, checks it: `integrity ok: N users, M messages`.
   Do this now and then; an untested backup is a hope. `RCLONE_CONFIG_R2_NO_HEAD` in `backup.sh` drops rclone's post-upload check, so the drill is the integrity check of what reached R2.

Restoring for real: `just restore daily/pulse-<time>.db.age`. The server stops for a few seconds; the old
database is kept next to it as `data/pulse.db.pre-restore-<time>`.

Restoring the media (the database restore doesn't touch it; do this after a lost or rebuilt server, or to roll
emoji back): `just restore-media-drill` downloads the newest media backup, decrypts it here and lists it
(`archive ok: N media files`; it also refuses an archive with paths outside `media/`). Then
`just restore-media daily/pulse-media-<time>.tgz.age` unpacks it over `data/media` (the server keeps running): files in the backup come back, newer uploads it doesn't have stay. The newest name is also
`ssh pulse@pulse /srv/pulse/backup.sh latest-media`, which (like `latest`) says so and exits non-zero if there are none.
Files restored without matching database rows (e.g. rolling emoji back past a database restore) are never removed by
the row-driven hourly cleanup: delete those under `data/media` by hand, or restore the database from the same time.
Backups of a media directory that changes mid-archive (an upload during the backup) only warn; the database backup
and the rest of the media are kept.

## 9. Uptime monitor

UptimeRobot (free) → New monitor → type **Keyword**, URL `https://pulsechat.co.za/health`, keyword `ok`,
interval 5 minutes, alert by email. Pause and resume it once to see the emails arrive.

## 10. Lost, stolen or returned laptop

Nothing lives only on the laptop: the code is on GitHub, images on GHCR, server secrets on the server, server
access through your Tailscale identity, and the backup key in NordPass.

- **New machine:** install Tailscale (section 3), clone the repo, restore `~/.config/pulse/backup-age.key`
  from NordPass. Everything above works again.
- **Stolen:** Tailscale admin console (works from a phone) → Machines → remove the laptop. It loses all
  access to the server at once. If you're worried, also delete and re-create the R2 token (section 8 step 2)
  and update `.env`.

## 11. Moving to another host

1. Section 4 on the new VM (it gets its own Tailscale name; use `ubuntu@<name>` until the old one is gone).
2. `just backup-now` on the old server.
3. Deploy to the new one and restore onto it, pointing the recipes at it:
   `just --set prod pulse@<new-name> deploy`, then
   `PULSE_PROD=pulse@<new-name> just restore <that backup>`.
4. Switch the two Cloudflare A records to the new IP (they take effect within minutes).
5. Delete the old VM and remove it from Tailscale.
