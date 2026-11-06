# Auto-update

Issue #82. Decided with the owner on 2026-11-06. The owner's own words and decisions are marked **(owner)**; everything else is the builder's call unless it says otherwise.

## Why

Every release today means the owner downloads the installer from GitHub Actions and hands it to each friend. Instead: deploy, and friends' apps update themselves **(owner)**.

## What friends see

- **On launch:** Pulse checks for a newer version. If there is one, a small **"Updating Pulse…"** screen (in the app's own look, like the existing loading screen) shows progress; the update installs quietly and Pulse reopens on the new version **(owner)**.
- **While running:** if a newer version appears, a subtle **"Restart to update"** button appears in the title bar near Patch notes. It never interrupts, never restarts by itself, and is disabled with a tooltip while you're in a call or streaming (#82 comment: never interrupt a call) **(owner)**.
- **Required updates:** the existing forced "update required" screen (the server's minimum-version check) now updates automatically instead of telling people to fetch an installer **(owner)**.
- After updating, the patch notes dot lights up as usual, and the notes show the version.
- If an update fails (no network, bad download, signature mismatch), Pulse starts normally on the old version, logs why, and tries again next launch; the forced-update screen offers Retry.

## How it gets made

- **Tauri's official updater** (`tauri-plugin-updater`) with the existing NSIS installer, in passive install mode on Windows. Linux dev builds skip updating.
- **Signing:** a Tauri updater keypair. The public key goes in `tauri.conf.json` (public is fine). The private key and its password live as GitHub Actions secrets used by `release-windows.yml`; the owner keeps one backup copy in a password manager **(owner)**. If the key were lost, updates stop and everyone reinstalls once by hand. The builder generates the key **outside the repo** and gives the owner the exact commands to store it (the owner sets the GitHub secrets himself).
- **Hosting on our own server (owner):** `pulsechat.co.za` serves the update manifest (`latest.json`: version, notes, signature, URL) and the installer from the server's data directory, through Caddy or a server route. The builder picks; a server route can **require a signed-in session** (the updater sends the token as a header), so only friends can download. Keep the current version and the one before; delete older.
  - **Fallback (owner):** if self-hosting hits a real snag, host on GitHub Releases instead and say so.
- **Publishing a release:** CI builds and signs the installer as today. A `just` recipe (e.g. `just publish-update <run-id>`) downloads that run's signed artifact, uploads it to the server, writes `latest.json`, and checks the signature and that the manifest is served. It's part of the deploy routine: merge → deploy → Windows build → publish-update.
- **Versions:** every release bumps a real version (0.2.0, 0.2.1, …) in the places Tauri and Cargo read it; the release routine does the bump; the patch notes show it. Start at 0.2.0.
- **Minimum version:** the server's existing minimum-version check stays the switch for forced updates; the release routine only raises it when a release truly requires it.

## Rollout

- The first build with the updater has to be installed by hand once (the old app can't update itself). Every build after that updates itself.

## Testing

- Unit tests: the update-check decision (newer / same / older / forced), the in-call gate on Restart to update, the manifest writer, the publish recipe's checks (with a fake server dir).
- UI tests: the Updating screen, the title-bar button and its disabled-in-call state, the forced-update screen's automatic path and Retry.
- An end-to-end check on Linux of the manifest and signature flow if feasible (sign a dummy file with a test key, verify).
- **Manual (owner, Windows):** install 0.2.0 by hand; publish 0.2.1; open Pulse → Updating → reopens on 0.2.1; while in a call, publish 0.2.2 → Restart to update appears but is disabled until you leave.

## Out of scope

- Delta or background downloads, update channels (beta/stable), macOS.
