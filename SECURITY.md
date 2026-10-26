# Security

Pulse is a small, invite-only voice and text chat. It has three parts:

- a server (Rust: axum and SQLite);
- a LiveKit SFU that carries voice and screen share;
- a desktop client (Tauri 2 with a Rust core and a Svelte UI). Windows is the main platform; Linux also builds.

Production is a single VM. Caddy sits in front, and the services run with Docker Compose.

This page is for someone deciding whether the client may be installed on managed machines. It covers what the client
touches, what it sends and to where, what it stores, and the known gaps. Everything here describes the code in this
repository.

## 1. What the client accesses, and when

**Keyboard and mouse: global keybinds, Windows only.**
- **What it installs.** Mute and deafen work while another app has focus. For this, the client installs low-level
  keyboard and mouse hooks (`WH_KEYBOARD_LL`, `WH_MOUSE_LL`).
- **How events are used.** Each event is checked against the user's saved binds and then always passed on unchanged.
  Pulse does not block or alter input.
  - Key presses are never stored, logged or sent anywhere.
  - The mouse hook only acts on the middle button and the two side buttons. Moves and left/right clicks pass through
    untouched.
- **Elevated windows.** The client checks whether the hooks can see input at the moment. Part of that check reads the
  elevation flag of the process that owns the foreground window. That flag is the only thing it reads from that process.
- **Switching it off.** The hooks are on by default. Settings → Keybinds → **Global keybinds** turns them off. When it
  is off, the hooks are removed, not just ignored. Pulse also stays off at the next start, so no hooks are installed.
- **Linux.** The client installs no hooks. Global binds come from the desktop's own shortcut settings, which run the
  Pulse command line (see *Local IPC* below).

**Screen and window capture: only when the user clicks Go Live.**
- **Choosing what to share.** Capture uses the webview's standard screen-capture API. The user chooses a screen or
  window in the system picker, and the capture stops when they stop sharing.
- **Preview image.** While live, the client uploads a small preview image of the shared content (480×270 WebP) every
  15 seconds. Other members who can see the channel see it before they open the stream. The server keeps only the
  latest image, in memory, and drops it when the stream ends. The server does not re-encode it: it checks only the
  declared content type (`image/webp`) and a 100 KB size cap, and passes the bytes on as they are (section 7).

**Process audio and process listing: Windows, only during Go Live with audio turned on.**
- **Window share.** The client captures only the audio of the process that owns the shared window, using WASAPI process
  loopback.
- **Screen share.** The client captures the machine's audio output except Pulse's own.
- **Process list.** To choose these processes, the client lists the running processes and the processes with an active
  audio session. It reads process IDs and executable names.
  - For a window share, it finds the window's process by going through the top-level windows and comparing their
    titles with the shared window's title.
  - For a screen share, it repeats the process and audio-session scan every 2 seconds while the share lasts, to follow
    apps that start or stop playing audio.
  - All of this is used only on the machine.
  - Process IDs may appear in the local log, and so in a bug or crash report (section 3).
  - Executable names and window titles are not logged.

**Microphone.** The microphone is opened when the user joins a voice channel or runs the microphone test in Settings.
Audio goes to the SFU (section 2).

## 2. Network

The client makes outbound connections only, all to the Pulse server and its LiveKit SFU:

| Purpose | Protocol / port | Notes |
|---|---|---|
| API and real-time events | HTTPS and WSS, TCP 443 | TLS. The client refuses plain `http://` for any server other than this machine (`localhost`). |
| Voice / screen-share signalling | WSS, TCP 443 (`voice.` subdomain) | Through the same TLS reverse proxy. |
| Voice / screen-share media | UDP 7882, or TCP 7881 as a fallback | WebRTC (DTLS-SRTP) to the SFU. |
| TURN relay | UDP 3478 | LiveKit's built-in TURN, for clients on restrictive networks. |

- **No peer-to-peer media.** Media goes from each client to the SFU and from the SFU to the other members. Clients never
  exchange media directly.
- **Public STUN.** The deployed LiveKit configuration names no STUN servers of its own. With that setting, LiveKit's
  default also offers clients public STUN servers (Google's, UDP 19302). The client may send STUN binding requests there
  to learn its own public address. No media or account data goes to them.
- **Webview limits.** The UI's Content Security Policy (section 4) also limits where the webview itself may connect.

**No listening network ports.** The client opens no listening TCP or UDP port that another machine can connect to.
WebRTC uses ephemeral UDP sockets for its own connection to the SFU.

**Local IPC.** Two local mechanisms exist. Neither is reachable over the network.
- **Linux command line.** The client listens on a Unix domain socket with mode `0600`. The socket is in the user's
  runtime directory (`$XDG_RUNTIME_DIR`), or, if that isn't set, at a per-user path in the system temp directory.
  The `pulse-app --toggle-mute` / `--toggle-deafen` command uses it, and it accepts only those two commands.
- **Second launch.** When the app is started a second time, the single-instance plugin tells the copy already running
  to show its window.

## 3. Data on the user's machine

- **Session token.**
  - **Where it is kept.** The token goes in the OS credential store: Windows Credential Manager, or the Secret Service
    on Linux. If no credential store is available (for example, on some Linux desktops), it falls back to a
    `session.json` file in the app's data folder. On Linux that file is owner-only (`0600`). On Windows it would inherit
    the user profile's permissions.
  - **Who can use it.** The token is held by the Rust core and never passed to the web UI.
  - **What it does not hold.** Passwords are never stored on the machine.
- **Logs.**
  - **Where and how much.** Logs are written to the app's log folder (`%LOCALAPPDATA%\app.pulse.client\logs` on
    Windows). There is one file per day, the last 7 are kept, and each day is capped at 50 MB.
  - **No tokens or passwords.** Pulse never writes session tokens or passwords to the log. They are never given to the
    logger, and a failed request is logged with its URL path only (no host, no query string). There is no separate
    redaction filter. By default the LiveKit and WebRTC libraries log at warning level and above; everything else logs
    at info level and above. A local `PULSE_LOG` environment variable can change these levels.
  - **Crash files.** The log folder also holds:
    - a `running` marker;
    - `crash.json`, written on a crash;
    - `pending-report.json`, an automatic report not yet sent, including its log tail.
- **Image cache.** Custom emoji images are fetched from the server once and cached as WebP files, keyed by their
  SHA-256 hash.
  - **Where.** `%LOCALAPPDATA%\app.pulse.client\media\` on Windows.
  - **Separate per login.** Each account on each server has its own subfolder.
  - **Integrity.** A file whose contents don't match its hash is fetched again.
  - **Sign-out.** Signing out deletes that account's cache, along with the webview's own copies.
- **Other local files.** The client also stores:
  - keybinds and the global-keybinds switch (`keybinds.json`);
  - a few UI preferences in the webview's local storage;
  - the webview's own profile data.

  The client does not write message history to disk.
- **Bug and crash reports.** Reports go only to the Pulse server. They are stored in its database for the operator to
  read. No third-party crash or analytics service is used.
  - **Bug reports.** The user writes a bug report and sends it, or saves it to a file instead.
  - **Crash reports.** After a crash or an unexpected exit, a report is sent automatically at the next sign-in, and the
    UI says so.
  - **What a report contains.** Each one contains:
    - the user's text, if any;
    - the app version and build commit;
    - the OS name and version;
    - the server address;
    - the connection state;
    - the voice channel and the names of the audio devices chosen, if set;
    - the panic message and its location, for crashes;
    - the last 256 KB of the local log.

## 4. Server-side protections

**Accounts.**
- **Joining.** Registration needs an invite code.
- **Passwords.** Passwords are 8 to 256 bytes long (UTF-8) and are hashed with Argon2id (the `argon2` crate's default
  parameters: 19 MiB memory, 2 passes, 1 lane, random 16-byte salt). At most two hashes are computed at once, which
  limits the CPU and memory an attacker can make the server use.
- **Unknown usernames.** A login for a username that doesn't exist still runs a hash, so response timing doesn't reveal
  whether the account exists.

**Sessions.**
- **Format.** Sessions are 256-bit random opaque tokens. The server stores only their SHA-256 hash.
- **Expiry.** A session expires after 90 days without use. Each use pushes the expiry back; the stored expiry is updated
  at most once an hour. There is no absolute maximum lifetime, so a session used regularly never expires.
- **Sign-out.** Signing out deletes the session. Settings → Advanced → **Sign out all other devices** deletes every
  other session of the account and closes those devices' real-time connections at once.
- **Sending the token.** REST calls send the token in the `Authorization` header. The real-time connection sends it in
  its first message. It never appears in a URL.

**Rate limits.** These are in-memory token buckets:

| What | Limit |
|---|---|
| Failed logins | 5/minute per IP, and 10/hour per username |
| Failed registrations (wrong, used or expired invite) | 5/hour per IP |
| Real-time connection attempts | 20/minute per IP (refunded once the connection authenticates) |
| Concurrent real-time connections | 8 per user; further connections are refused |
| New messages | burst of 20, then 5/second per user (edits, deletes and reactions are not rate-limited) |
| Typing notifications | 1/second per user (extras dropped) |
| Bug / crash reports | 5/hour per user |
| Image uploads | 30/hour per user |

Real-time frames from the client are capped at 64 KB, and messages at 4,000 characters.

**Uploaded images.**
- **Who can upload and read them.** Upload and download both need a signed-in session.
- **What is accepted.**
  - Formats: PNG, JPEG, GIF or WebP only.
  - Size: at most 2 MB.
  - Dimensions: at most 4096×4096, checked from the header before any decoding.
- **Decoding limits.** Decoding is bounded:
  - at most 256 MB of memory per image;
  - at most 300 frames;
  - at most 100 megapixels decoded per animation;
  - two images processed at a time.
- **Re-encoding.** Every image upload (custom emoji) is decoded and re-encoded server-side to a WebP of at most 128×128.
  The original bytes are never stored or served.
- **Exception: Go Live previews.** These are not image uploads in this sense and are not re-encoded (section 1).

**Message rendering.**
- **Markdown.** Markdown is rendered in the client and then sanitised with DOMPurify, using a strict allow-list of tags
  and attributes.
- **Raw HTML.** HTML typed in a message is shown as text.
- **Remote images.** Remote images are never loaded. Only bundled emoji and the client's own emoji cache can be images.
- **Links.** Links must be `http(s):` or `mailto:`.

**Client webview lockdown.**
- **Content Security Policy.** The UI runs under a strict CSP:
  - `default-src 'self'`;
  - no `unsafe-eval` and no inline scripts (inline styles are allowed: `style-src 'self' 'unsafe-inline'`);
  - images only from the app itself, `data:` and the app's emoji cache;
  - `connect-src` limited to the app's own IPC and the voice server.
- **Tauri capabilities.** The capability set is the Tauri core defaults plus four window controls (minimize, maximize,
  close, drag). No file-system, shell, HTTP or opener plugins are included.
- **Where network access happens.**
  - Most network and file access goes through the Rust core's own commands.
  - The exception is screen-share video. The webview publishes and watches it directly with the LiveKit JavaScript SDK
    over WebRTC. That is why the CSP allows connections to the voice server.

## 5. Infrastructure

- **TLS.** Caddy terminates TLS with automatically renewed certificates.
- **Internal services.** The Pulse server listens on loopback only. LiveKit's signalling port is reachable only through
  Caddy. The LiveKit webhook path is refused at the proxy.
- **Firewall.** It allows only TCP 80 and 443 (HTTPS, and redirects/certificate issuance), TCP 7881, UDP 7882 and
  UDP 3478 (media and TURN), and the port of the private VPN used for administration.
- **SSH.**
  - Key-only, with root login disabled.
  - The provisioning procedure closes public SSH. Administration happens over the private VPN.
- **Updates.** Unattended security updates are enabled on the host.
- **Containers.**
  - All three containers (Pulse server, LiveKit, Caddy) run as a dedicated host user, not root. The server image also
    defaults to a non-root user.
  - That host user is in the `docker` group so it can run deploys and backups, which makes it root-equivalent on the
    host.
  - Images are pinned by digest. LiveKit and Caddy are pinned in the compose file. The Pulse server image is pinned to
    the digest pulled at deploy time, so a moved tag cannot change what runs.
- **Secrets.** The server's secrets live in a mode-`0600` file owned by the service user.
- **Backups.** Every night, and before each deploy, the database and uploaded media are encrypted with `age` and stored
  in an S3-compatible object store. That object store is with a different provider from the VM. The decryption key is
  not on the server, and the plaintext snapshot is deleted as soon as it has been encrypted.
- **TURN relay limits.** Pulse configures no TURN relay restrictions of its own. It relies on the relay policy built
  into the pinned LiveKit release (v1.13.10), which is intended to limit relayed traffic to the SFU itself. This is
  upstream behaviour and is not verified by tests in this repository.

## 6. Build and supply chain

- **Builds.** GitHub Actions builds the Windows installer and the server image from this repository on GitHub-hosted
  runners.
  - **Permissions.** The workflows run with read-only repository permissions by default.
  - **Secrets.** The only credential they use is the automatic `GITHUB_TOKEN`. The server-image workflow is also granted
    `packages: write`, to publish the image to GitHub's container registry. No other secrets are configured or used.
  - **Who can publish.** Only a successful CI run of a push to `main` in this repository, or a manual run by someone with write access,
    publishes a server image. Pull requests from forks cannot.
- **Build inputs fetched at build time.** The client links a prebuilt `libwebrtc` that the build downloads from
  LiveKit's GitHub releases. The download is not checksum-verified.
  - The Windows build gets the Tauri CLI through a version range (`@tauri-apps/cli@^2`) and uses the current stable Rust
    toolchain.
  - The build output is a test kit zip containing the installer. The kit also includes a LiveKit server binary,
    downloaded without hash verification, for running a local test server. The installer does not install it.
- **Rust dependencies.** `cargo-deny` checks advisories, licences, wildcard version requirements and sources
  (`deny.toml`). Only crates.io and one pinned git dependency are allowed as sources.
  - **Ignored advisories.** Each ignored advisory is listed there with its reason.
  - **Licences.** Licences must be on an explicit allow-list.
- **UI dependencies.** `pnpm audit --prod --audit-level high` checks the UI's production dependencies.
- **When the checks run.** Both checks run in CI whenever dependencies change, and weekly.
- **Dependabot.** It opens weekly update pull requests for Cargo, npm, GitHub Actions, the server Dockerfile and the
  compose images.

## 7. Known gaps

- **Code signing.** The Windows installer is not code-signed yet, so SmartScreen and endpoint tools will see an unsigned
  publisher.
- **Updates.** There is no auto-updater. New versions are installed by hand.
- **Signing out does not end an active call.** Revoking a session immediately stops REST access and closes the real-time
  connection. It does not remove a device that is already in a voice channel or screen share: that media continues until
  the device leaves or LiveKit disconnects it. Voice and stream tokens are valid for 10 minutes, but they only control
  joining, not staying connected.
- **Not end-to-end encrypted.** Voice and screen share are encrypted in transit (DTLS-SRTP) but not end to end: the SFU
  on the server can access the media. Messages are encrypted in transit (TLS) and stored unencrypted in the server's
  database. Backups are encrypted.
- **Crash reports.** Crash and unexpected-exit reports are sent automatically, without an opt-out. They go only to the
  Pulse server, and they include the last 256 KB of the local log.
- **Accounts.**
  - There is no multi-factor authentication.
  - Any member can create invite codes.
  - Rate limits are held in memory, so they reset when the server restarts.
- **Pinning and verification.**
  - GitHub Actions in the workflows are referenced by version tag, not commit SHA.
  - The server image's base images are referenced by tag. The image that is actually deployed is pinned by digest.
  - The prebuilt `libwebrtc` and the test kit's LiveKit binary are downloaded without checksum verification.
  - The Windows build takes the Tauri CLI from a version range and Rust from the current stable toolchain.
- **Preview images are relayed unmodified.** Go Live preview images are passed from the streamer to viewers byte for
  byte. The server checks only the declared type and a 100 KB size cap, so viewers' webviews decode whatever the
  streamer's client sent.
- **No TURN over TLS.** There is no TURN over TLS/TCP 443, so voice may not work on networks that block UDP and allow
  nothing but 443.

## 8. Reporting a vulnerability

Please report security issues privately. Go to this repository's **Security** tab and choose **Report a
vulnerability** (GitHub private vulnerability reporting). Please don't open a public issue for a security problem.
