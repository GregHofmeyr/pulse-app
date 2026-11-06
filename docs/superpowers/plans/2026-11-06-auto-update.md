# Auto-update Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Friends' Pulse apps update themselves: checked on launch (an "Updating Pulse…" screen, passive install,
reopen), offered while running ("Restart to update", never during a call), and automatic on the forced
"update required" screen (#82).

**Architecture:** Tauri's official updater, driven from the Rust core (the UI calls our own commands, never the
plugin's JS API). The core asks `<signed-in server>/updates/latest.json` with the session token as a Bearer header; a
new authenticated server route serves `latest.json` and the signed NSIS installer from `/data/updates`. CI signs the
installer; `just publish-update <run-id>` downloads that run's artifact, checks the signature locally, uploads it,
writes the manifest last and prunes to two versions.

**Tech Stack:** `tauri` 2.12.2 (lockfile) + `tauri-plugin-updater` 2.x (docs.rs latest 2.14.1; see Task 4 step 1),
`@tauri-apps/cli` 2.12.4, axum 0.8 + `tower-http` 0.6 (`fs` feature), bash + jq + minisign for the publish script,
Svelte 5 + vitest.

**Spec:** `docs/superpowers/specs/2026-11-06-auto-update-design.md`

**Research notes (cited):**
- Tauri updater docs (v2.tauri.app/plugin/updater): `bundle.createUpdaterArtifacts: true` makes the NSIS build emit
  `<name>-setup.exe` + `<name>-setup.exe.sig`; signing reads `TAURI_SIGNING_PRIVATE_KEY` (content or path) and
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` from the environment (not `.env`); `plugins.updater.pubkey` is the key *content*;
  `plugins.updater.windows.installMode` `"passive"` (the default) = small progress window, no interaction. Static
  manifest: `{ version, notes?, pub_date? (RFC 3339), platforms: { "windows-x86_64": { url, signature } } }`, where
  `signature` is the `.sig` file's *content*. On Windows `install` exits the app (`std::process::exit(0)`) after
  `on_before_exit`.
- Plugin source (plugins-workspace `v2`, `plugins/updater/src/updater.rs`): builder `.header()`s are copied into
  `Update.headers` and sent on the **download** too; a non-2xx check response is skipped silently (ends as
  `Error::ReleaseNotFound`), 204 = no update; NSIS platform key tried as `windows-x86_64-nsis` then `windows-x86_64`;
  NSIS runs with `/UPDATE` and, since `restart_after_install` defaults to true, relaunches the app. A builder
  `.timeout()` is copied into `Update.timeout` and so also bounds the 80 MB download: **never set it**; bound the
  check with `tokio::time::timeout` instead.
- Tauri config reference (v2.tauri.app/reference/config): if top-level `version` is removed, "the version number from
  `Cargo.toml` is used". NSIS `installMode` default `currentUser` (no UAC), so passive updates need no elevation.

## Global Constraints

- Linux and debug builds never update (`updates_enabled() = cfg!(windows) && !cfg!(debug_assertions)`, and
  `PULSE_NO_UPDATE=1` turns it off on a test machine).
- "Restart to update" never interrupts and never restarts by itself; disabled with a tooltip while in a call or
  streaming. Copy: button "Restart to update", tooltip "Finish your call first", screen "Updating Pulse…".
- A failed update (no network, bad download, signature mismatch) starts Pulse normally on the old version, logs why,
  tries again next launch; the forced screen offers Retry.
- Updates are only fetched from the origin the session token belongs to; the token never goes to another host.
- Hosting: `pulsechat.co.za` serves `latest.json` and the installer from the server's data directory, signed-in only.
  Keep the current version and the one before; delete older.
- Versions are real semver, starting at **0.2.0**, from **one** place: `[workspace.package] version` in `Cargo.toml`.
- The server's `MIN_CLIENT_VERSION` (`server/src/gateway/socket.rs`, compared with `PROTOCOL_VERSION`) stays the forced
  switch; the release routine raises it only when a release truly needs it.
- The private key never enters the repo, a log or a chat; the owner generates it and sets the secrets himself.
- Public repo: test people are alex, sam, kim, jo; no real names, IPs, secrets.
- Cargo: `CARGO_BUILD_JOBS=4 flock ~/Projects/pulse-app/target/.cargo-serial nice -n 19 ionice -c3 cargo …`,
  tests `-- --test-threads=4`, worktree's own `target/`.

## Decisions (builder's calls, where the design left it open)

- **Server route, not Caddy.** Caddy can't check a Pulse session; the route reuses `AuthUser`. Files stream through
  `tower_http::services::ServeFile` (no 80 MB `Vec` under the container's 1 GB limit). Caddy is unchanged (it already
  proxies everything). `/data/updates` sits outside `/data/media`, so backups don't carry installers.
- **Endpoint = the stored session's server** (`<server>/updates/latest.json`, set at runtime with `.endpoints()`);
  `tauri.conf.json` carries `https://pulsechat.co.za/updates/latest.json` only as the config default. Signed-out =
  no check; the core checks again right after login.
- **Launch check starts in the core's `setup`** (before the webview loads) so it usually finishes before the UI asks;
  the UI waits at most `LAUNCH_WAIT` = 4 s for it, then boots normally. The check itself is bounded at 4 s.
- **While running:** checked after each login's first Ready and every 30 min; the core emits `pulse://update`.
- **Signature checked twice:** by `publish-update` (minisign, before anything is uploaded) and by the updater on
  install (the real gate).
- **Manifest writing, pruning and checks live in `deploy/publish-update.sh`** with a self-test against a fake server
  dir, like `backup.sh` / `backup-selftest.sh`.
- **No JS plugin, no `updater:default` capability:** the UI only invokes our commands.

## Review Focus

- A manifest whose `url` points at another host (tampered or a publishing slip): the core must refuse to download,
  so the token never leaves our origin → `refuses_foreign_download_url` (Task 4) and `refuses_foreign_url` in the
  publish self-test (Task 3).
- An expired / revoked session at launch: the check gets 401 and Pulse boots normally, no Updating screen →
  `unauthorized_check_boots_normally` (Task 5).
- The forced screen when the server raised the minimum but the new build isn't published yet: shows the old text and
  Retry, and re-checks by itself every 60 s → `forced_without_update_retries` (Task 7).
- Publishing the same or an older version (a rerun of an old CI run): refused before upload →
  `refuses_same_or_older` (Task 3).
- Path tricks on the download route (`../pulse.db`, `latest.json.tmp`, `Pulse_0.2.0_x64-setup.exe.sig`): 404, never
  a file outside the allowed names → `only_installer_names_are_served` (Task 2).

---

## Before you start (owner, by hand, once)

Run on your own machine, **outside the repo**. Nothing here is committed.

```bash
mkdir -p ~/.tauri && chmod 700 ~/.tauri
cd ~/Projects/pulse-app/client/ui
# prompts for a password twice; writes the private key and pulse-updater.key.pub
node node_modules/@tauri-apps/cli/tauri.js signer generate -w ~/.tauri/pulse-updater.key
chmod 600 ~/.tauri/pulse-updater.key
# the two GitHub secrets release-windows.yml reads (the first from the file, the second typed at the prompt)
gh secret set TAURI_SIGNING_PRIVATE_KEY --repo GregHofmeyr/pulse-app < ~/.tauri/pulse-updater.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --repo GregHofmeyr/pulse-app
# the public key, for Task 4 (safe to share)
cat ~/.tauri/pulse-updater.key.pub
```

Then save the private key file's contents **and** the password as one entry in your password manager. Lost key =
updates stop and everyone reinstalls once by hand (Risks). Also install `minisign` (`sudo pacman -S minisign`) for
`just publish-update`.

---

### Task 1: One version, 0.2.0, shown in patch notes

**Files:**
- Modify: `Cargo.toml` (`[workspace.package] version = "0.2.0"`), `Cargo.lock` (via cargo)
- Modify: `client/src-tauri/tauri.conf.json` (delete the top-level `"version"`)
- Modify: `client/ui/src/patchNotes.ts` (`version: string` on `PatchNotes`; `version: '0.2.0'`)
- Modify: `client/ui/src/components/PatchNotes.svelte` (heading shows `Pulse {notes.version} · {notes.date}`)
- Modify: `justfile` (recipe `bump-version`)
- Test: `client/ui/src/patchNotes.test.ts` (create), `client/ui/src/components/PatchNotes.test.ts`

**Interfaces:**
- Produces: `PatchNotes.version`; `just bump-version X.Y.Z` (rewrites the workspace version and `patchNotes.version`,
  runs `cargo update -w --offline`); the app version = `env!("CARGO_PKG_VERSION")` everywhere.

- [ ] **Step 1: Write the failing tests**

```ts
// patchNotes.test.ts
import cargo from '../../../Cargo.toml?raw'
it("patch notes name the app's version", () => {
  const v = /\[workspace\.package\][^[]*?\nversion = "([^"]+)"/.exec(cargo)?.[1]
  expect(patchNotes.version).toBe(v)
})
// PatchNotes.test.ts
it('the notes show the version', () => {
  render(PatchNotes, { notes: { ...notes, version: '0.2.0' }, onClose })
  expect(screen.getByText(/Pulse 0\.2\.0/)).toBeInTheDocument()
})
```

- [ ] **Step 2: Run `cd client/ui && pnpm test patchNotes PatchNotes`** — Expected: FAIL (`version` undefined).
- [ ] **Step 3: Implement** the files above; `bump-version` refuses anything not matching `^[0-9]+\.[0-9]+\.[0-9]+$`.
- [ ] **Step 4: Run the tests again, then `just check`** — Expected: PASS; `grep -c '"version"' client/src-tauri/tauri.conf.json` prints 0.
- [ ] **Step 5: Commit** — `git commit -m "Versions from Cargo.toml, start at 0.2.0, patch notes show it (#82)"`

### Task 2: Server serves updates to signed-in people

**Files:**
- Create: `server/src/updates.rs`
- Modify: `server/src/lib.rs` (`pub mod updates;`, `.merge(updates::router())`, create the dir at startup like
  `media_dir`), `server/src/config.rs` (`updates_dir: PathBuf` from `PULSE_UPDATES_DIR`, default `/data/updates`;
  test config: a temp dir), `server/Cargo.toml` (`tower-http` features `["trace", "fs"]`)
- Test: `server/tests/updates.rs`

**Interfaces:**
- Produces: `GET /updates/latest.json` (auth, `Content-Type: application/json`, `Cache-Control: no-store`, 404 if
  absent) and `GET /updates/{file}` (auth, only names matching `^Pulse_[0-9]+\.[0-9]+\.[0-9]+_x64-setup\.exe$`,
  `application/octet-stream`, streamed by `ServeFile`). `pub fn installer_name_ok(name: &str) -> bool`.
- On-disk layout (Task 3 writes it): `<updates_dir>/latest.json`, `<updates_dir>/Pulse_<v>_x64-setup.exe`.

- [ ] **Step 1: Write the failing tests** (`server/tests/updates.rs`, using `common` / `testing::spawn_with`)
  - `manifest_needs_a_session`: no header → 401; bad token → 401; registered alex's token → 200 with the file's exact
    bytes and `cache-control: no-store`.
  - `installer_is_streamed_to_signed_in_people`: a 3 MB file named `Pulse_0.2.0_x64-setup.exe` → 200, same bytes;
    without a token → 401.
  - `only_installer_names_are_served`: `..%2Fpulse.db`, `latest.json.tmp`, `Pulse_0.2.0_x64-setup.exe.sig`,
    `pulse.exe` → 404 even when such files exist in the dir.
  - `no_manifest_is_404`: empty dir → 404 (the updater treats non-2xx as "no update").
- [ ] **Step 2: Run** `cargo test -p pulse-server --test updates -- --test-threads=4` — Expected: FAIL (404 for all).
- [ ] **Step 3: Implement** `updates::router()`; the name check runs before any path is built; `AuthUser` first.
- [ ] **Step 4: Run the tests again, then `cargo clippy -p pulse-server --all-targets -- -D warnings`** — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -m "Server: signed-in-only /updates route (#82)"`

### Task 3: `just publish-update <run-id>`

**Files:**
- Create: `deploy/publish-update.sh`, `deploy/publish-update-selftest.sh`
- Modify: `justfile` (`publish-update $run_id` recipe; `deploy-check` runs the self-test),
  `.github/workflows/ci.yml` (run the self-test beside `backup-selftest.sh`; install `minisign` there),
  `deploy/README.md` (new section "Releasing an app update": the routine and the rollback note)

**Interfaces:**
- Consumes: Task 2's on-disk layout; `jq -r .plugins.updater.pubkey client/src-tauri/tauri.conf.json` (Task 4).
- Produces (functions, `PUBLISH_UPDATE_LIB=1 source deploy/publish-update.sh`):
  - `version_of FILE` → prints `X.Y.Z` from `Pulse_X.Y.Z_x64-setup.exe`, else fails.
  - `verify_sig FILE SIGFILE PUBKEY_B64` → base64-decodes both (Tauri wraps minisign files in base64) and runs
    `minisign -V -p … -x … -m FILE`.
  - `newer_than NEW OLD` → success only if NEW > OLD by `sort -V` (OLD empty = success).
  - `write_manifest VERSION URL SIGFILE NOTES OUT` → `{version, notes, pub_date (UTC RFC 3339), platforms:
    {"windows-x86_64": {url, signature}}}` via `jq -n`; refuses a URL whose origin isn't `https://$PULSE_DOMAIN`.
  - `prune DIR` → keeps the installer the manifest names and the next-highest version; deletes other `Pulse_*.exe`.
  - `remote CMD…` → `ssh pulse@pulse` normally; runs locally in `$PUBLISH_UPDATE_DIR` when set (self-test).
- Recipe flow: `gh run view $run_id --json workflowName,conclusion,headBranch` must say `release-windows`,
  `success`, `main` or a `v*` tag → `gh run download $run_id -n pulse-windows-installer` into a `mktemp -d` → exactly
  one `Pulse_*_x64-setup.exe` and its `.sig` → `verify_sig` → read the served version from
  `/srv/pulse/data/updates/latest.json` and require `newer_than` → `scp` the installer to `…/updates/.incoming/`,
  `mv` into place → write `latest.json.tmp`, `mv` over `latest.json` (manifest last, so it never names a missing
  file) → `prune` → checks: the on-disk installer's `sha256sum` equals the local one, the manifest parses with
  `jq -e '.platforms["windows-x86_64"].signature'`, and `curl https://pulsechat.co.za/updates/latest.json` without a
  token answers 401 (route live and guarded). Prints `published X.Y.Z`.

- [ ] **Step 1: Write the self-test** (fake server dir, stubbed `gh`/`ssh`/`scp`/`curl`, like `deploy-selftest.sh`)
  - `version_of`: `Pulse_0.2.1_x64-setup.exe` → `0.2.1`; `Pulse_0.2_x64-setup.exe`, `evil.exe` → refused.
  - `verify_sig`: a minisign key made in the test (`minisign -G -W`), sig and pubkey base64-wrapped as Tauri does →
    ok; one flipped byte in the file → refused; another key's pubkey → refused.
  - `tauri_signature_verifies` (Linux e2e, skipped with a note if `client/ui/node_modules` is absent): a throwaway
    key from `tauri.js signer generate --ci -p test -w $tmp/k`, `tauri.js signer sign -f $tmp/k -p test $tmp/dummy`
    → `verify_sig` accepts it with `$tmp/k.pub`.
  - `refuses_same_or_older`: served 0.2.1 → publishing 0.2.1 or 0.2.0 refused, 0.2.2 ok; nothing uploaded on refusal.
  - `refuses_foreign_url`: `write_manifest` with `https://example.com/…` → refused.
  - `manifest_shape`: written file has `version`, RFC 3339 `pub_date`, the `.sig` content (not a path) as `signature`.
  - `keeps_two_versions`: dir with 0.2.0, 0.2.1, 0.2.2 (manifest 0.2.2) → 0.2.1 and 0.2.2 remain.
  - `wrong_run_refused`: `gh run view` stub says `ci` or `failure` → refused before download.
- [ ] **Step 2: Run `bash deploy/publish-update-selftest.sh`** — Expected: FAIL (script missing).
- [ ] **Step 3: Implement** `publish-update.sh` and the recipe; shellcheck-clean.
- [ ] **Step 4: Run the self-test and `just deploy-check`** — Expected: all `ok`, exit 0.
- [ ] **Step 5: Commit** — `git commit -m "just publish-update: signed installer + manifest to the server (#82)"`

### Task 4: CI signs, core can check and install

**Files:**
- Modify: `client/src-tauri/Cargo.toml` (`tauri-plugin-updater`, desktop target only),
  `client/src-tauri/tauri.conf.json` (`bundle.createUpdaterArtifacts: true`; `plugins.updater: { pubkey: "<owner's
  .pub content>", endpoints: ["https://pulsechat.co.za/updates/latest.json"], windows: { installMode: "passive" } }`),
  `.github/workflows/release-windows.yml` (build step `env: TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.… }}`,
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.… }}`; "Assemble the download" also copies
  `target/release/bundle/nsis/*.exe.sig`; fail if there isn't exactly one `.exe` and one `.sig`)
- Create: `client/src-tauri/src/updater.rs`
- Modify: `client/src-tauri/src/lib.rs` (register `tauri_plugin_updater::Builder::new().build()`, manage `Updates`,
  start the launch check in `setup` from `Store::load()`, add the commands to `generate_handler!`)

**Interfaces:**
- Consumes: the stored session (`Store::load() -> Option<(server_url, token)>`); the core's current voice channel
  (the value `pulse://voice` reports as `channel_id`; streaming implies one, since a stream never outlives it).
- Produces:
  - `pub fn updates_enabled() -> bool`; `pub fn should_offer(current: &semver::Version, offered: &semver::Version) -> bool`
    (strictly newer only); `pub fn same_origin(server: &Url, download: &Url) -> bool`;
    `pub fn may_restart(in_call: bool) -> Result<(), RestartRefused>`.
  - `Updates` (managed): `launch: watch::Receiver<LaunchCheck>` with `enum LaunchCheck { Pending, Done(Option<Offer>) }`,
    plus `Mutex<Option<tauri_plugin_updater::Update>>`.
  - `#[derive(Serialize, Clone)] struct Offer { version: String }` (camelCase JSON).
  - Commands: `update_launch() -> Option<Offer>` (waits ≤ `LAUNCH_WAIT` 4 s for the launch check, else `None`);
    `update_check() -> Result<Option<Offer>, String>` (a fresh check now); `update_install(restart: bool) ->
    Result<(), String>` (`restart = true` from the title bar → `may_restart` against the voice state first; downloads
    with progress events, then installs; on Windows the process ends here).
  - Events: `pulse://update` `Offer` (a running check found one); `pulse://update-progress` `{ downloaded: u64,
    total: Option<u64> }`.
  - Background: check after the first Ready of each login and every 30 min (`RECHECK_EVERY`); `on_before_exit`
    calls `reports::crash::clear_running` and flushes the log so the update isn't reported as a crash.

- [ ] **Step 1: Pick the plugin version.** `cargo info tauri-plugin-updater` — use the newest 2.x whose `tauri`
  requirement the lockfile's 2.12.2 satisfies; don't bump `tauri` in this task. Note the chosen version in the commit.
- [ ] **Step 2: Write the failing unit tests** (in `updater.rs`)
  - `offers_only_newer`: current 0.2.0 → offered 0.2.1 true; 0.2.0 false; 0.1.9 false; 0.10.0 vs 0.9.0 true.
  - `refuses_foreign_download_url`: server `https://pulsechat.co.za` → `…/updates/Pulse_0.2.1_x64-setup.exe` true;
    `https://github.com/…`, `http://pulsechat.co.za/…`, `https://pulsechat.co.za.evil.example/…` false.
  - `restart_refused_in_call`: `may_restart(true)` is `Err`, `may_restart(false)` is `Ok`.
  - `launch_wait_gives_up`: a `LaunchCheck::Pending` that never resolves → `None` after `LAUNCH_WAIT` (paused tokio
    clock); a `Done(Some(offer))` → that offer immediately.
  - `disabled_on_linux_and_debug`: `updates_enabled()` is false under `cfg(test)` on Linux.
- [ ] **Step 3: Run** `cargo test -p pulse-client --lib updater:: -- --test-threads=4` — Expected: FAIL (not defined).
- [ ] **Step 4: Implement.** Build the updater with `app.updater_builder().endpoints(vec![server.join("updates/latest.json")])
  .header("Authorization", format!("Bearer {token}")).version_comparator(|cur, up| should_offer(&cur, &up.version))`;
  wrap `check()` in `tokio::time::timeout(4 s)`; **no builder `.timeout()`**. Before downloading, `same_origin` or
  refuse. Every failure logs a reason (`tracing::warn!`, no token) and returns `None`/`Err`. When
  `!updates_enabled()`, checks return `None` without a request.
- [ ] **Step 5: Run the tests again, `just check`** — Expected: PASS.
- [ ] **Step 6: Commit** — `git commit -m "Core updater: signed releases, launch + periodic checks, never mid-call (#82)"`

### Task 5: The Updating screen on launch

**Files:**
- Create: `client/ui/src/routes/Updating.svelte`, `client/ui/src/lib/updates.svelte.ts`
- Modify: `client/ui/src/lib/tauri.ts` (`api.updateLaunch`, `api.updateCheck`, `api.updateInstall`),
  `client/ui/src/lib/screen.ts` (`'updating'` screen, ahead of everything once set), `client/ui/src/App.svelte`
  (await `api.updateLaunch()` before `restoreSession`; on an offer show Updating and `updateInstall(false)`; on its
  failure fall through to the normal restore)
- Test: `client/ui/src/routes/Updating.test.ts`, `client/ui/src/App.test.ts`, `client/ui/src/lib/screen.test.ts`

**Interfaces:**
- Consumes: Task 4's commands and events.
- Produces: `updates` store `{ offer: Offer | null, installing: boolean, progress: { downloaded: number; total: number | null } | null, error: string | null }`
  fed by `pulse://update` / `pulse://update-progress`; `startUpdateListening(): Promise<() => void>`;
  `installUpdate(restart: boolean): Promise<void>` (sets `installing`, clears it and sets `error` on failure).
  `Updating.svelte` props: none (reads the store). Looks like the boot/Waiting screens: `--bg-0`, centred, a
  drag region, "Updating Pulse…", a thin progress bar (indeterminate while `total` is null).

- [ ] **Step 1: Write the failing tests**
  - `Updating.test.ts`: shows "Updating Pulse…"; progress 50/100 → bar `aria-valuenow="50"`; total null → no
    `aria-valuenow`.
  - `App.test.ts` `an update at launch shows Updating and installs`: `update_launch` mock → `{ version: '0.2.1' }` →
    "Updating Pulse…" shown, `update_install` invoked with `{ restart: false }`, `restore_session` not invoked.
  - `App.test.ts` `a failed launch update boots normally`: `update_install` rejects → `restore_session` runs and the
    shell appears.
  - `App.test.ts` `unauthorized_check_boots_normally`: `update_launch` → `null` → no Updating screen, normal boot.
  - `screen.test.ts`: `updating: true` wins over every other state.
- [ ] **Step 2: Run `cd client/ui && pnpm test Updating App screen`** — Expected: FAIL.
- [ ] **Step 3: Implement** the files above.
- [ ] **Step 4: Run the tests again, `pnpm check`** — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -m "Updating Pulse… on launch (#82)"`

### Task 6: "Restart to update" in the title bar

**Files:**
- Modify: `client/ui/src/routes/Shell.svelte` (button left of Patch notes, only while `updates.offer`),
  `client/ui/src/lib/updates.svelte.ts` (`restartBlocked(): boolean` = `voice.channelId !== null`)
- Test: `client/ui/src/routes/Shell.test.ts`

**Interfaces:**
- Consumes: `updates`, `installUpdate` (Task 5); `voice.channelId` from `lib/voice.svelte.ts`.
- Produces: `<button class="win update">Restart to update</button>`; disabled → `aria-disabled="true"` plus the
  app-wide tooltip "Finish your call first" (keep it hoverable, so not the `disabled` attribute). Click → the
  Updating screen and `installUpdate(true)`.

- [ ] **Step 1: Write the failing tests**
  - `no offer, no button`; `pulse://update` event → "Restart to update" appears beside Patch notes.
  - `in a call the button is disabled`: `voice.channelId = 'c1'` → `aria-disabled="true"`, tooltip "Finish your call
    first", click invokes nothing; leaving the call → enabled.
  - `click installs with restart`: → `update_install` invoked with `{ restart: true }`, Updating screen shown;
    rejection → back to the shell, button still there, error logged.
- [ ] **Step 2: Run `pnpm test Shell`** — Expected: FAIL.
- [ ] **Step 3: Implement**; style it as subtle as the other `.win` buttons (text, no accent fill).
- [ ] **Step 4: Run the tests again, `pnpm check`** — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -m "Restart to update in the title bar, never mid-call (#82)"`

### Task 7: The forced screen updates by itself

**Files:**
- Modify: `client/ui/src/routes/UpdateRequired.svelte`
- Test: `client/ui/src/routes/UpdateRequired.test.ts` (create)

**Interfaces:**
- Consumes: `api.updateCheck`, `installUpdate(false)`, `updates` (Task 5).
- Produces: on mount `updateCheck()` → an offer → the Updating view with progress (the process ends on install) ·
  none → the existing "plz update :)" card with "Pulse needs an update that isn't ready yet. It'll install as soon
  as it is." and a Retry button, re-checking every 60 s (`FORCED_RECHECK_MS`) · error → "Couldn't update: {error}" and
  Retry.

- [ ] **Step 1: Write the failing tests**
  - `forced_installs_automatically`: check → offer → `update_install` `{ restart: false }`, "Updating Pulse…".
  - `forced_without_update_retries`: check → null → Retry shown; fake timers +60 s → checked again; Retry click →
    checked again.
  - `forced_failure_offers_retry`: install rejects "signature mismatch" → message and Retry.
- [ ] **Step 2: Run `pnpm test UpdateRequired`** — Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the tests again, `just check`** — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -m "Forced update installs itself, with Retry (#82)"`

---

## The release routine (goes into `deploy/README.md` in Task 3)

1. `just bump-version 0.2.N`, update `patchNotes.ts` (new `id`, `version`, notes), merge to main.
2. `just deploy` (server first: the route must exist before any client asks). Raise `MIN_CLIENT_VERSION` only when
   the release truly needs it — and then publish the update **before** or right after the deploy, since forced
   clients sit on the forced screen re-checking every 60 s until it's there.
3. Run release-windows (push tag `v0.2.N`, or `gh workflow run release-windows.yml`), wait for green.
4. `just publish-update <run-id>`.
5. Rollback = publish a higher version with the fix (the updater never downgrades); the previous installer stays on
   the server for anyone who needs it by hand.

## Rollout (the first manual install)

1. Owner: "Before you start" (key, secrets, pubkey into Task 4, minisign).
2. Merge Tasks 1–7; `just deploy`; run release-windows on main → the artifact holds `Pulse_0.2.0_x64-setup.exe` and
   its `.sig` (check the name says 0.2.0: proves Tauri took the version from `Cargo.toml`).
3. `just publish-update <run-id>` (publishes 0.2.0; nobody is offered it yet, since it's the version they'll install).
4. Hand that installer to each friend **once**, as today (0.1.0 can't update itself). Installing over 0.1.0 keeps
   sessions and settings (same identifier `app.pulse.client`).
5. Owner's manual test (Windows): on 0.2.0, publish 0.2.1 → open Pulse → Updating… → reopens on 0.2.1 with the patch
   notes dot. Join a call, publish 0.2.2 → within 30 min "Restart to update" appears, disabled with the tooltip, and
   enables when you leave.

## Risks

- **Lost private key or password:** no new update can be signed for installed apps; generate a new key, ship a build
  with the new pubkey, everyone reinstalls once by hand. Mitigation: the password-manager copy.
- **Leaked private key:** someone with it *and* a way to serve our origin could push code; rotate the same way and
  revoke sessions. The signed-in-only route narrows who can even see the manifest.
- **Missing secrets break release builds:** with a pubkey and `createUpdaterArtifacts`, `tauri build` fails without
  `TAURI_SIGNING_PRIVATE_KEY`. Intended (no unsigned releases); forks can't build releases.
- **Plugin vs `tauri` 2.12 compatibility:** Task 4 step 1 picks a plugin version the lockfile satisfies.
- **`version.workspace = true`:** if the Tauri CLI didn't resolve the inherited version the artifact would be misnamed;
  Rollout step 2 checks the name; the fallback is putting the literal version back in `tauri.conf.json` and making
  `bump-version` edit both (plus a test that they agree).
- **Windows `exit(0)` on install:** skips normal shutdown; `on_before_exit` clears the crash marker and flushes logs;
  the in-call gate means no call is cut.
- **Signed-out people don't update** until they log in (then the post-login check runs). Accepted.
- **Disk / bandwidth:** ~2 installers on disk; each friend downloads ~80 MB per release over the server's link.

## Fallback: GitHub Releases

If self-hosting hits a real snag (say so in #82 first): release-windows gets `contents: write` and creates a release
with the `.exe`, `.sig` and a `latest.json` (from `write_manifest`, URL = the release asset); the client endpoint
becomes `https://github.com/GregHofmeyr/pulse-app/releases/latest/download/latest.json` **with the Authorization
header removed** (it would otherwise be sent to GitHub, see Research notes) and `same_origin` checks against
`github.com` / `objects.githubusercontent.com`. Installers become public (the repo already is). Switching is itself a
client release: deliver it through the self-hosted path while that still works, else by hand once.
