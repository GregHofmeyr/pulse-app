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
- **Only while something is bound.** The hooks are installed only when the **Global keybinds** switch is on *and* at
  least one keybind is set. Nothing is bound by default, so a fresh install runs no hooks. Adding the first bind
  installs them; removing the last one removes them. Recording a bind in Settings doesn't need them: Settings reads
  the keys from its own window.
- **How events are used.** Each event is checked against the user's saved binds and then always passed on unchanged.
  Pulse does not block or alter input.
  - Key presses are never stored, logged or sent anywhere.
  - The mouse hook only acts on the middle button and the two side buttons. Moves and left/right clicks pass through
    untouched.
- **Elevated windows.** While the hooks are installed, the client checks whether they can see input at the moment.
  Part of that check reads the elevation flag of the process that owns the foreground window. That flag is the only
  thing it reads from that process.
- **Switching it off.** The switch is on by default. Settings → Keybinds → **Global keybinds** turns it off. When it is
  off, the hooks are removed, not just ignored. The setting is kept, so no hooks are installed at the next start.
- **Linux.** The client installs no hooks. Global binds come from the desktop's own shortcut settings, which run the
  Pulse command line (see *Local IPC* below).

**Screen and window capture: only when the user clicks Go Live.**
- **Choosing what to share.** Capture uses the webview's standard screen-capture API. The user chooses a screen or
  window in the system picker, and the capture stops when they stop sharing.
- **Preview image.** While live, the client uploads a small preview image of the shared content (480×270) every 15
  seconds. The image is WebP, or JPEG where the webview can't encode WebP. Other members who can see the channel see
  it before they open the stream.
  - The server decodes each upload and re-encodes it as a new WebP of at most 480×270. Only the re-encoded image is
    kept and served, so the upload's metadata never reaches viewers.
  - It accepts WebP, PNG or JPEG of at most 100 KB and 1920×1920. The bytes must match the declared type. Animated
    WebP is refused; an animated PNG keeps only its still default image. An upload that can't be decoded within
    these limits is refused, and the previous preview stays.
  - The server keeps only the latest image, in memory, and drops it when the stream ends.
  - Uploads are limited per user to a burst of 6, then one every 5 seconds.

**Process audio and process listing: Windows, only during Go Live with audio turned on.**
- **Window share.** The client captures only the audio of the process that owns the shared window, using WASAPI process
  loopback. If no single process can be identified, it captures no audio and publishes no audio track.
- **Screen share.** The client captures the machine's audio output except Pulse's own.
- **Process list.** To choose these processes, the client lists the running processes and the processes with an active
  audio session. It reads process IDs and executable names.
  - For a window share, it finds the window's process by going through the visible top-level windows and comparing
    their titles with the shared window's title. Pulse's own windows are skipped. If windows of two or more processes
    have that title, it doesn't guess.
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
| TURN relay | UDP 3478 | LiveKit's built-in TURN, for clients on restrictive networks. It also serves as the clients' STUN server. |

- **No peer-to-peer media.** Media goes from each client to the SFU and from the SFU to the other members. Clients never
  exchange media directly.
- **No third-party STUN.** The only ICE server clients are given is the Pulse server's own TURN
  (`turn:<server IP>:3478`).
  - The client configures no ICE servers of its own. It uses the list LiveKit sends when it joins.
  - LiveKit adds public STUN servers (Twilio's and Google's) to that list only when it has no UDP TURN and no STUN
    servers configured. Ours has UDP TURN and no `stun_servers` (LiveKit v1.13.10, `pkg/service/roommanager.go`,
    `iceServersForParticipant`).
  - A test checks both LiveKit configurations (production and development) for this. Another test, run against a real
    LiveKit, checks that the ICE servers in the join response are only our own.
- **Webview limits.** The UI's Content Security Policy (section 4) also limits where the webview itself may connect.

**No listening network ports.** The client opens no listening TCP or UDP port that another machine can connect to.
WebRTC uses ephemeral UDP sockets for its own connection to the SFU.

**Local IPC.** Two local mechanisms exist. Neither is reachable over the network.
- **Linux command line.** The client listens on a Unix domain socket with mode `0600`. The socket is in the user's
  runtime directory (`$XDG_RUNTIME_DIR`). If that isn't set, it is in `pulse-app-<uid>` in the system temp directory,
  a directory with mode `0700`. If that directory already exists and isn't a real directory owned by the user with no
  group or other access (for example, a symlink), the client doesn't open the socket and logs a warning.
  The `pulse-app --toggle-mute` / `--toggle-deafen` command uses it, and it accepts only those two commands. The
  command follows the same rules and refuses a socket that isn't owned by the user.
- **Second launch.** When the app is started a second time, the single-instance plugin tells the copy already running
  to show its window.

## 3. Data on the user's machine

- **Session token.**
  - **Where it is kept.** The token goes in the OS credential store: Windows Credential Manager, or the Secret Service
    on Linux. If no credential store is available (for example, on some Linux desktops), it falls back to a
    `session.json` file in the app's data folder. That file is owner-only. On Linux its mode is `0600`. On Windows it
    is created with a protected access list (nothing inherited from the folder) that grants only the current user, so
    it is never readable through the folder's permissions. A `session.json` left on disk is deleted the next time Pulse
    starts and finds its session in the credential store. Otherwise Pulse reads the file at startup, and on Windows
    tightens it to the same owner-only access list when it reads it (also when the file turns out to be unusable).
  - **Windows roaming.** On Windows the credential is stored with Windows' enterprise persistence. Where roaming
    profiles or credential roaming are enabled, it roams to the user's other machines. The `session.json` fallback is
    in roaming AppData (`%APPDATA%\app.pulse.client`), but it is almost never used on Windows, because Credential
    Manager is always present.
  - **Who can use it.** The token is held by the Rust core and never passed to the web UI.
  - **What it does not hold.** Passwords are never stored on the machine.
- **Two-factor material.**
  - **Login ticket.** When an account has 2FA, the password step returns a ticket (section 4). The Rust core keeps it in
    memory only: it is never written to disk, never passed to the web UI and never logged. It is dropped when the code
    is accepted or the ticket is reported gone, and when the user goes back to the password step, registers, signs out
    or starts another login. A wrong code leaves it in place, so the code can be re-entered until the server's own
    limits end it.
  - **Setup secret and QR code.** The `otpauth://` link that carries the secret stays in the Rust core. The core draws
    the QR code itself from the link, locally, and gives the web UI only a grid of dark and light squares to paint.
    The web UI also receives the key as text, for typing in by hand.
  - **Recovery codes.** They are shown once after turning on 2FA or making new codes, and are kept only in the
    Settings screen's memory until the user clicks **I've saved these**, starts another action or leaves the screen.
    Nothing is stored locally. **Copy** puts them on the clipboard only when the user clicks it.
- **Logs.**
  - **Where and how much.** Logs are written to the app's log folder (`%LOCALAPPDATA%\app.pulse.client\logs` on
    Windows). There is one file per day, the last 7 are kept, and each day is capped at 50 MB.
  - **No tokens or passwords.** Pulse never writes session tokens or passwords to the log. They are never given to the
    logger, and a failed request is logged with its URL path only (no host, no query string). There is no separate
    redaction filter. By default the LiveKit and WebRTC libraries log at warning level and above; everything else logs
    at info level and above. A local `PULSE_LOG` environment variable can change these levels, but it can't raise
    the HTTP, WebSocket and TLS libraries (`tungstenite`, `tokio_tungstenite`, `hyper`, `hyper_util`, `h2`, `reqwest`,
    `rustls`, `tower_http`) above info. Their debug and trace output can include request and frame contents.
  - **No home paths.** Paths under the user's home folder are logged with the home part replaced by `~`.
  - **Crash files.** The log folder also holds:
    - a `running` marker, naming the server the current session belongs to;
    - `last-server`, the server address this profile last signed in to;
    - `crash.json`, written on a crash;
    - `pending-report.json`, a crash report not yet sent, including its log tail.
  - **Report log tails.** A report's log tail is at most the last 256 KB of the newest log file, and it starts at the
    current session (the last app start, sign-in or restore, marked by a line that `PULSE_LOG` and the daily size cap
    never drop). Older sessions in the same file are left out.
- **Image cache.** Custom emoji images are fetched from the server once and cached as WebP files, keyed by their
  SHA-256 hash.
  - **Where.** `%LOCALAPPDATA%\app.pulse.client\media\` on Windows.
  - **Separate per login.** Each account on each server has its own subfolder.
  - **Integrity.** A file whose contents don't match its hash is fetched again.
  - **Sign-out.** Signing out deletes that account's cache, along with the webview's own copies.
- **Other local files.** The client also stores:
  - keybinds and the global-keybinds switch (`keybinds.json`);
  - the crash-report setting (`reports.json`);
  - unsent messages, in the webview's local storage (see below);
  - a few UI preferences in the webview's local storage;
  - the webview's own profile data (under `%LOCALAPPDATA%` on Windows).

  `keybinds.json`, `reports.json` and the `session.json` fallback are in the app's data folder, which on Windows is
  roaming AppData (`%APPDATA%\app.pulse.client`).
- **Unsent messages.** A message that failed to send, or is still queued, is kept in the webview's local storage for
  that account (its text and the message it replies to). That lets the user retry it after signing back in, so it is
  kept across sign-out. It stays until that account signs in and sends or discards it. Other than these, the client
  does not write message history to disk.
- **Bug and crash reports.** Reports go only to the Pulse server. They are stored in its database for the operator to
  read. No third-party crash or analytics service is used.
  - **Bug reports.** The user writes a bug report and sends it, or saves it to a file instead. It is sent only when
    the user presses Send.
  - **Crash reports.** After a crash or an unexpected exit, the next launch saves a report on the machine.
    - **Which server.** A report goes only to the server the crashed session was signed in to. If the session crashed
      before signing in, that is the server this profile last signed in to. If the user next signs in to a different
      server, or the profile has never signed in to any server, the report is deleted unsent.
    - **The setting.** Settings → Advanced → **Send crash reports automatically** is on by default. When it is on, the
      report is sent at the next sign-in, and the UI says so.
    - **When it is off.** Nothing is sent automatically: not a new report, and not a retry of an earlier one. The
      setting is checked again before every attempt. The report is held, and the user is asked **Send** or **Don't
      send** in the main window. Don't send deletes it. Turning the setting back on does not release a held report,
      so a held report is only ever sent when the user presses Send.
    - **Settings file.** The setting is stored in `reports.json`. If that file, or the setting in it, is missing, the
      setting is on. If the file exists but can't be read or parsed, the setting counts as off, and the local log says
      why.
    - **Expiry.** A report older than 7 days is deleted instead of sent.
  - **What a report contains.** Each one contains:
    - the user's text, if any;
    - the app version and build commit;
    - the OS name and version;
    - the server address;
    - the connection state;
    - the voice channel, the mute and deafen state, and the names of the audio devices chosen, if set;
    - the panic message and its location, for crashes (the message capped at 2 KB);
    - the last 256 KB of the local log.

## 4. Server-side protections

**Accounts.**
- **Joining.** Registration needs an invite code.
- **Passwords.** Passwords are 8 to 256 bytes long (UTF-8) and are hashed with Argon2id (the `argon2` crate's default
  parameters: 19 MiB memory, 2 passes, 1 lane, random 16-byte salt). At most two hashes are computed at once, which
  limits the CPU and memory an attacker can make the server use.
- **Unknown usernames.** A login for a username that doesn't exist still runs a hash, so response timing doesn't reveal
  whether the account exists.
- **Changing the password.** Settings → Advanced → **Change password** asks for the current password, the new one twice,
  and a code (TOTP or recovery code) if the account has 2FA on. The new password follows the same 8 to 256 byte rule. It
  is checked under the two-factor limits (a wrong current password or code keeps its count). Setting the new hash,
  **signing out every other device** and deleting pending sign-in tickets are one database transaction. The other
  devices' real-time connections are then closed and their calls ended, as with **Sign out all other devices**. The
  device doing it stays signed in. A sign-in that checked the old password but had not finished when the change
  committed gets no session or sign-in ticket: issuing one re-checks, in the same transaction, that the stored hash is
  still the one the password was checked against. The same holds after the owner's `reset-password`. The client passes
  the passwords straight to the server and never stores or logs them.
- **Owner tools.** The server's owner can run these on the host, against the database:
  - `reset-password <username>` sets a new password, read as one line from standard input (never a command-line
    argument, so it doesn't show up in the process list or shell history). It signs out all of the account's sessions
    and deletes its pending sign-in tickets. It leaves 2FA alone.
  - `revoke-sessions <username>` deletes all of the account's sessions and pending sign-in tickets. The password is
    not changed.
  - Both work with the server stopped or running. A running server is not told: its next request with a deleted
    session fails, and a new real-time connection is refused, but real-time connections and calls already open stay
    up until they drop. Restarting the server closes them.

**Two-factor authentication.** Optional, per account, and off until the member turns it on in Settings → Advanced.
- **Codes.** Time-based one-time passwords (TOTP): SHA-1, 6 digits, 30-second steps, with the previous and next step
  also accepted (to allow for clock drift). A code can only be used once: the server remembers the newest step it
  accepted for the account and refuses that step or any older one. The check is atomic, so two requests racing with the
  same code cannot both pass.
- **The secret at rest.** Each account's 20-byte secret is encrypted before it is stored, with ChaCha20-Poly1305 under
  a 32-byte key from the `PULSE_TOTP_KEY` setting (64 hex characters), with a fresh random 12-byte nonce each time. The
  account's id is bound in as associated data, so a stored secret copied to another account does not decrypt.
  - **What this protects.** The database file and its backups, which hold only the encrypted secret. The key is not
    in the backups. Backups are already encrypted with `age`, so for them the key is a second layer.
  - **What it does not protect.** The key sits in the server's `.env` file, on the same host as the database. Someone
    who can read both can decrypt every secret. It is a defence for stolen database files and backups, not for a
    compromised host.
  - **Not encrypted.** Whether 2FA is on, and the newest accepted step number, are stored in clear. Recovery codes
    and login tickets are stored only as SHA-256 hashes (below).
- **The server will not start with the wrong key.** If any account has 2FA turned on, the server refuses to start
  when the key is missing, or when any of those accounts' secrets does not decrypt with it, and says so in the log
  (counts only, never the key or a secret). Setups that were never finished are not checked. With no 2FA accounts, a
  missing key just means 2FA cannot be set up. A key that is set but is not exactly 64 hex characters stops every
  server command, not only `serve`. The deploy script refuses to go on without a well-formed key, and provisioning
  fills the key in only when it is missing or blank, never replacing a value.
- **Turning it on.** Settings asks for the account password, then shows a QR code and the key to type in. 2FA turns on
  only when the member enters one current code from the app, which proves the app works; a recovery code is not accepted
  for this. Starting again before then replaces the unfinished secret (and is what the member is told to do if that
  secret can no longer be decrypted). Turning it on **signs out every other device**, the same way **Sign out all other
  devices** does (their real-time connections are closed and their calls ended, with the limits described under *Signing
  out ends calls too*); turning 2FA on and ending those sessions are one database transaction, so neither happens
  without the other. The device doing it stays signed in, so anyone who had only the password is out. Sessions are not
  touched otherwise: signing in with a code, turning 2FA off, regenerating recovery codes and the owner reset leave
  existing sessions alone.
- **Recovery codes.** Turning 2FA on gives 10 recovery codes, shown once. Each is 16 characters from a 31-character
  alphabet (about 79 bits), written `xxxx-xxxx-xxxx-xxxx`; spaces, dashes and case are ignored when one is entered.
  Only their SHA-256 hashes are stored. A code works once and is then deleted. Making new codes needs a current TOTP
  code and replaces all the old ones. Signing in, turning 2FA off and changing the password accept either a TOTP code
  or a recovery code. In those three, anything that is not 6 digits is tried as a recovery code.
- **Signing in.** A wrong password gets the same answer whether or not the account has 2FA. With the right password
  and 2FA on, the server returns no session, only a login ticket:
  - 256 random bits, valid for 5 minutes. Only its SHA-256 hash is stored; expired tickets are pruned whenever a
    new one is issued.
  - Spent by one successful code. The ticket is deleted when it succeeds, and also when the 5th wrong code is entered
    on it. Unknown, expired and used-up tickets all get the same "gone" answer.
  - Turning 2FA off, the owner reset, changing the password and the owner's `reset-password` delete the account's
    tickets.
- **Limits on guessing.** Every 2FA check (the sign-in code, the password or code asked for when turning it on or
  off or making new recovery codes, and the current password and code asked for when changing the password) is counted against two limits at once: 5 per minute per IP address and 10 per
  hour per account. The count is taken before the check, so parallel guesses cannot all slip past. A wrong password
  or code keeps its count, as does a guess at an unknown or dead ticket (that one counts against the address only);
  every other outcome is refunded, and a refused attempt costs nothing. A refused attempt gets 429. Like the other
  limits, these are in memory.
- **Lost app and recovery codes.** There is no self-service way back in. The server's owner turns 2FA off for the
  account from the host (`reset-2fa`, run in a one-off server container). It deletes the account's secret, recovery
  codes and tickets, never uses the key, so any well-formed `PULSE_TOTP_KEY` in `.env` will do (it still works after
  the key is lost, once a new one is set), and leaves sessions alone.
- **Who can see it.** Only the account itself can ask whether its 2FA is on, and how many recovery codes are left.
  Nothing in the member list, the real-time events or any other endpoint carries it. Someone who already has the
  password does learn it at sign-in, from the extra step.
- **Never logged.** Secrets, codes, recovery codes and tickets are not put in logs or error messages.
- **Client.** See section 3 for what the desktop client keeps. It makes no calls beyond the Pulse server for this.

**Sessions.**
- **Format.** Sessions are 256-bit random opaque tokens. The server stores only their SHA-256 hash.
- **Expiry.** A session expires after 90 days without use. Each use pushes the expiry back; the stored expiry is updated
  at most once an hour. There is no absolute maximum lifetime, so a session used regularly never expires.
- **Sign-out.** Signing out deletes the session. Settings → Advanced → **Sign out all other devices** deletes every
  other session of the account and closes those devices' real-time connections at once. Changing the password does
  the same. The owner's `revoke-sessions` and `reset-password` delete all of an account's sessions (see *Owner tools*
  above for what that leaves open on a running server).
- **Sending the token.** REST calls send the token in the `Authorization` header. The real-time connection sends it in
  its first message. It never appears in a URL.

**Signing out ends calls too.** Voice and screen share go through LiveKit, which has its own short-lived tokens. Pulse
ties each of those tokens to the session that asked for it:
- **Grant IDs.** Every voice or stream token carries a random grant ID (a participant attribute participants can't
  change). The server records which session each grant ID belongs to. The token never contains the session token or
  its hash.
- **On sign-out.** When a session is signed out, directly, by **Sign out all other devices** or by changing the
  password, the server asks LiveKit for the participants in the rooms that session holds grants for. It then removes
  every participant carrying one of that session's grant IDs, from voice and screen share alike.
  - This is best-effort and takes at most 2 seconds. If LiveKit can't be reached in that time, the sign-out still
    succeeds and the miss is logged. That device then stays in its call until it leaves.
- **Rejoining.** Removing a participant does not invalidate the LiveKit token it holds (LiveKit v1.13.10 has no
  working way to revoke one). So every join is checked: LiveKit tells the server about each join, and a participant is
  removed again unless its grant ID belongs to a live session and was issued for that exact room and identity. This
  covers a signed-out device rejoining with a token it kept, and an expired session.
  - **Limits of that check.** The removal happens after LiveKit reports the join, in the background, and takes at
    most 2 seconds. Until then the refused device is briefly in the call, and its join response has already given it
    TURN credentials. If LiveKit can't be reached for the removal, the miss is logged; if the join notification itself
    is lost, nothing is removed and nothing is logged. Removal is by identity, so a valid device of the same user that
    joins in the same instant (one LiveKit round trip) could be removed instead.
  - **After a server restart.** LiveKit keeps running when the Pulse server restarts, and doesn't repeat the join
    notifications. So at startup the server asks LiveKit who is connected, in voice and screen-share rooms, and runs
    the same check on every participant carrying a grant ID: one that fails is removed (in the background, as above)
    and isn't shown as in the call. Grants whose connection LiveKit no longer reports are marked as ended (their leave
    notification was missed). Participants without a grant ID, connected with a token from before grant IDs existed,
    are left alone. If LiveKit can't be reached at startup, the server retries 12 times, about 5 to 10 seconds apart
    (one to two minutes in all); if every try fails, this check doesn't happen for that restart.
- **Lifetime.** Voice and stream tokens are valid for 10 minutes. LiveKit keeps refreshing the token of a participant
  that stays connected, so token expiry alone does not end a call. The join check above, not expiry, is what keeps a
  signed-out device out.

**Calls follow access.** A LiveKit token is only as good as the access behind it, checked again at every join:
- **Joins.** The join check above also needs current access. A voice join needs the user to still be allowed in that
  call: a member of the DM or group, or of the server for a server voice channel (the same rule as getting a token). A
  screen-share join needs that, and the user in that channel's voice call. A join that fails is removed like any other
  refused join, with the same limits.
  - **In the call** means the server's list of who is in voice, or else a voice connection the server admitted whose
    leave hasn't been reported. That covers the list lagging behind LiveKit (just after a restart, or a screen-share
    join reported before its voice join). A voice token that was fetched but never used doesn't count.
- **Removed from a group.** Removing someone from a group (or leaving it) ends their part in its call. Their LiveKit
  tokens for it are forgotten, LiveKit is asked to remove them (best-effort, at most 2 seconds, as on sign-out) and
  they are taken off the call's member list. A rejoin with a token they kept fails the join check. Their client also
  leaves the call when it is told the group is gone.
- **Leaving voice.** When someone leaves a server voice channel's call (not a reconnect), LiveKit is asked to remove
  their screen-share connections there too, in the background. This is skipped if they are back in the call by then.
  Removal is by identity, so a new screen-share connection made in the same instant could be removed.
- **No data messages.** Voice tokens can't send LiveKit data messages, like stream tokens. Calls carry the microphone
  only.

**Rate limits.** These are in-memory token buckets. Per-IP limits count an IPv6 client by its /64 prefix (one client can hold a whole /64), and an IPv4-mapped IPv6 address as its IPv4 address. Each limiter tracks at most 10,000 keys; past that, the least recently used bucket is dropped.

| What | Limit |
|---|---|
| Failed logins | 5/minute per IP, 10/hour per username from one IP, and 100/hour per username from all IPs together. Once a username's 100/hour is used up, an IP that has failed for it recently is refused, but an IP with no recent failures for it can still try (see section 7) |
| Failed registrations (wrong, used or expired invite) | 5/hour per IP |
| Two-factor checks (sign-in code, the password or code for turning it on or off or making new recovery codes, and the current password and code for changing the password) | 5/minute per IP, and 10/hour per account; spent up front, kept only for a wrong password or code or a bad ticket |
| Real-time connection attempts | 20/minute per IP (refunded once the connection authenticates) |
| Concurrent real-time connections | 8 per user; further connections are refused, and the client tells the user why |
| New messages | burst of 20, then 5/second per user |
| Message edits and deletes, reaction adds and removes, Go Live and watching a stream (together) | burst of 30, then 2/second per user (stopping a stream and unwatching are never limited) |
| Typing notifications | 1/second per user (extras dropped) |
| Mute and deafen updates sent to others | burst of 10, then 2/second per user. Every update is still stored; past the limit, others get one update with the latest state once the limit allows. An update that changes nothing is not sent on. |
| New servers, server channels, DMs and groups, and members added to a group (together) | 20/hour per user (opening a DM that already exists is free) |
| Invite codes | 10/hour per user |
| Bug / crash reports | 5/hour per user |
| Custom emoji uploads | 30/hour per user |
| Custom emoji creates, renames and deletes (together) | 30/hour per user |
| Go Live preview uploads | burst of 6, then one every 5 seconds per user |
| Voice and stream tokens (together) | burst of 60, then 1/second per user |

Real-time frames from the client are capped at 64 KB, and messages at 4,000 characters.

**Caps.** These hard limits on stored rows don't reset. Going over one is refused with an error.
- Servers: 10 created per user.
- Channels: 100 per server, text and voice together.
- Groups: a user can be in 200 groups (1:1 DMs don't count). Creating a group or adding people fails if it would put
  anyone over.
- Invite codes: 20 unused, unexpired codes per user.
- Custom emoji: 100 per server at a time, and 500 ever added per server (deleted emoji still count, because their
  names are kept for old messages).

**Database cleanup.** Once an hour the server deletes, one statement per table and logging the counts:
- sessions past their expiry (their LiveKit grants go with them);
- invite codes older than their 24 hours, used or not (nothing refers to an invite row, and an expired code is already
  refused as unknown);
- expired two-factor login tickets;
- bug and crash reports older than 90 days (the report retention).

Unused media is cleaned up in the same hourly pass (see "Uploaded images").

**Low disk.** Sending a message, submitting a report and adding a custom emoji are refused with a 507 "server storage
is full" when the free space on the database's volume or the media volume is below the configured minimum (2 GB by
default). Image uploads were already refused the same way. The check comes after sign-in and the rate limit, before
any work.

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
- **Go Live previews** are image uploads too. They use the same decoding memory limit and the same WebP encoder, with
  their own size limit and fitting:
  - at most 100 KB per upload;
  - at most 1920×1920, checked from the header before any decoding;
  - WebP, PNG or JPEG matching the declared type;
  - animated WebP is refused; an animated PNG keeps only its still default image;
  - the output is a still WebP of at most 480×270;
  - processed in the same two slots as other image uploads;
  - per user, a burst of 6, then one every 5 seconds.

  The Go Live video itself goes through LiveKit as video and is not re-encoded by the server.

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
- **Tauri capabilities.** The main window gets an explicit list, not `core:default`. It is exactly:
  - `core:event:allow-listen` and `core:event:allow-unlisten` (the UI listens to events from the Rust core);
  - `core:window:allow-is-focused` (focus state);
  - `core:window:allow-minimize`, `core:window:allow-toggle-maximize` and `core:window:allow-close` (the title-bar
    buttons);
  - `core:window:allow-start-dragging` and `core:window:allow-internal-toggle-maximize` (dragging the title bar, and
    double-clicking it to maximise).
  - The page cannot emit events, use the app, path, image, menu or tray APIs, or open devtools. It reaches the Rust core
    only through the app's own commands.
  - No file-system, shell, HTTP or opener plugins are included. There is one capability file and one window (`main`).
  - A test (`client/src-tauri/tests/capabilities.rs`) fails if the list changes.
- **DNS prefetch.** `index.html` sets `x-dns-prefetch-control: off`, so the webview does not resolve hostnames it has
  not been told to connect to.
- **Where network access happens.**
  - Most network and file access goes through the Rust core's own commands.
  - The exception is screen-share video. The webview publishes and watches it directly with the LiveKit JavaScript SDK
    over WebRTC. That is why the CSP allows connections to the voice server.

## 5. Infrastructure

- **TLS.** Caddy terminates TLS with automatically renewed certificates.
- **Internal services.** The Pulse server listens on loopback only. LiveKit's signalling port (7880) also listens on
  loopback only, so it is reachable only through Caddy; the firewall keeps 7880 closed as well. LiveKit's media and
  TURN ports are public (see *Firewall*). The LiveKit webhook path is refused at the proxy. So is LiveKit's server API
  (`/twirp/*`, room and participant admin); the Pulse server calls it over loopback.
- **Caddy admin API.** Caddy's admin API listens on a unix socket in Caddy's own config volume, not on TCP. The
  containers share the host's network, so a TCP admin port on loopback would be open to all of them.
- **Provisioning.** The setup script installs Docker and Tailscale from their vendors' signed apt repositories. It
  checks each repository's signing key against a full fingerprint pinned in the script and stops if it differs. No
  vendor install script is piped into a shell.
- **Firewall.** It allows only TCP 80 and 443 (HTTPS, and redirects/certificate issuance), TCP 7881, UDP 7882 and
  UDP 3478 (media and TURN), and the port of the private VPN used for administration.
- **SSH.**
  - The OpenSSH server is key-only, with root login disabled.
  - The provisioning procedure closes public SSH. After that, SSH is reachable only over the owner's private VPN
    (Tailscale, using Tailscale's own SSH), so who may log in is decided by that VPN's access policy. That policy is
    managed in the VPN, not in this repository.
- **Updates.** Unattended security updates are enabled on the host, but it does not reboot by itself, so kernel and
  libc fixes only apply after the operator reboots it. Docker and Tailscale come from their vendors' apt
  repositories, which are not in the automatic update origins: the operator updates them by hand. Container images
  are not covered by host updates (they are pinned and change only on a deploy).
- **Client address.** The server takes the client's IP from the right-most `X-Forwarded-For` entry, the one Caddy
  writes. It must therefore be reachable only through Caddy. It binds `127.0.0.1` for that reason.
- **Containers.**
  - All three containers (Pulse server, LiveKit, Caddy) run as a dedicated host user, not root. The server image also
    defaults to a non-root user.
  - Every container drops all Linux capabilities and has a process-count and memory ceiling. Caddy keeps one: binding
    ports 80 and 443. The server and LiveKit also run with `no-new-privileges`. Caddy can't: it binds those ports
    through a file capability on its binary, which `no-new-privileges` would ignore.
  - Root filesystems are not read-only: each image's write paths have not been checked.
  - That host user is in the `docker` group so it can run deploys and backups, which makes it root-equivalent on the
    host (see the known gaps).
  - Images are pinned by digest. LiveKit and Caddy are pinned in the compose file. The Pulse server image is pinned to
    the digest pulled at deploy time, so a moved tag cannot change what runs.
- **Secrets.** The server's secrets live in a mode-`0600` file owned by the service user.
- **Backups.** Provisioning leaves backups off until the operator sets the encryption key and object-store details.
  Once configured, every night and before each deploy, the database and uploaded media are encrypted with `age` and
  stored in an S3-compatible object store. That object store is with a different provider from the VM. The
  decryption key is not on the server, and the plaintext snapshot is deleted as soon as it has been encrypted.
- **LiveKit's public address.** LiveKit is given the server's public IPv4 address in the deployment's configuration.
  It does not discover it, so it contacts no outside STUN server at start-up. A deploy stops before changing anything
  if that address is missing or malformed, or is a private, loopback, link-local, carrier-grade NAT, multicast or
  reserved address. (Documentation-only ranges such as TEST-NET pass this check.)
- **TURN relay limits.** Pulse sets no TURN relay options of its own, so the defaults of the pinned LiveKit release
  (v1.13.10) apply. The following is LiveKit's code, not Pulse's (`pkg/service/turn.go` and
  `pkg/service/turnrelay.go` at tag `v1.13.10`), and it is not tested in this repository.
  - **Restricted peers are refused.** The TURN permission check refuses relaying to loopback, link-local unicast and
    multicast, multicast, private (RFC 1918 and IPv6 unique-local) and unspecified addresses. It also refuses
    `0.0.0.0/8`, `100.64.0.0/10` (carrier-grade NAT, which includes the VPN's addresses), `198.18.0.0/15`,
    `240.0.0.0/4` and the NAT64 prefixes. Our configuration sets no allow-list (`allow_restricted_peer_cidrs`), so
    none of these are let through.
  - **The host itself only on media ports.** Relayed packets to the host's own addresses (its interfaces and the
    public IP) are dropped unless they go to the SFU's media port (UDP 7882).
  - **No TCP relays.** TCP relay allocations (RFC 6062) are refused.
  - **Per-participant quota.** One participant's credentials may hold at most 12 relay allocations at once.
  - **What it does allow.** TURN credentials are issued per participant by LiveKit, so only someone in a call has
    them. A participant can relay UDP to public internet addresses outside the ranges above. That is how TURN works.

## 6. Build and supply chain

- **Builds.** GitHub Actions builds the Windows installer and the server image from this repository on GitHub-hosted
  runners.
  - **Permissions.** The workflows run with read-only repository permissions by default.
  - **Secrets.** The only credential they use is the automatic `GITHUB_TOKEN`. The server-image workflow is also granted
    `packages: write`, to publish the image to GitHub's container registry. No other secrets are configured or used.
  - **Who can publish.** Only a successful CI run of a push to `main` in this repository, or a manual run by someone with write access,
    publishes a server image. Pull requests from forks cannot.
- **Pinned build inputs.**
  - **Actions.** Every GitHub Action is pinned to a full commit SHA.
  - **Base images.** The server image's base images are pinned to an exact version and digest (`rust:1.98.1-bookworm`,
    `debian:bookworm-20261020-slim`). A pull request that changes the Dockerfile (a Dependabot bump included) gets a
    CI build of the image (amd64, not published) on the pull request.
  - **Rust.** The toolchain is pinned to 1.98.1 in `rust-toolchain.toml`. CI and the release builds install exactly
    that version.
  - **Tauri CLI.** The Windows build uses the Tauri CLI at an exact version (2.12.1), installed from the UI's lockfile.
  - **Node.js and pnpm.** CI and the Windows build use exact versions (Node.js 22.23.4, pnpm 11.28.8). Dependabot
    doesn't update these; `deploy/README.md` says how to bump them.
  - **LLVM (CI only).** LLVM comes from apt.llvm.org's signed package repository. Before adding it, CI checks the
    repository key's fingerprint against a pinned value and fails if it differs. Nothing installed this way ships.
- **Verified downloads.**
  - **libwebrtc.** The client links a prebuilt `libwebrtc` that the build downloads from LiveKit's GitHub releases
    (tag `webrtc-89d790b`). Before extracting it, the build checks the archive's SHA-256 against a value pinned in the
    repository (`vendor/webrtc-sys/libwebrtc_download.rs`, for Linux x64 and Windows x64). It fails on a mismatch, and
    for any target without a pin. A libwebrtc built locally from source (`LK_CUSTOM_WEBRTC`) skips the download.
  - **Test kit.** The build output is a test kit zip containing the installer. The kit also includes a LiveKit server
    binary (v1.13.10), for running a local test server. The release job checks the downloaded archive against a
    pinned SHA-256 before extracting it, and fails on a mismatch. The installer does not install it.
  - **Test kit LiveKit.** The kit's LiveKit config keeps signalling, UDP media and TURN on loopback, and its own
    TURN means clients are given no third-party STUN server. LiveKit's ICE-TCP port (7881) still listens on every
    interface; LiveKit gives no way to restrict it. A test checks this config like the production and dev ones.
- **Rust dependencies.** `cargo-deny` checks advisories, licences, wildcard version requirements and sources
  (`deny.toml`). Only crates.io and one pinned git dependency are allowed as sources.
  - **Ignored advisories.** Each ignored advisory is listed there with its reason.
  - **Licences.** Licences must be on an explicit allow-list.
- **UI dependencies.** `pnpm audit --prod --audit-level high` checks the UI's production dependencies, and
  `pnpm audit --audit-level high` all of them, build tools included (they run on CI and release machines).
- **When the checks run.** Both checks run in CI whenever dependencies change, and weekly.
- **Dependabot.** It opens weekly update pull requests for Cargo, npm, GitHub Actions, the Rust toolchain, the server
  Dockerfile's base images and the compose images (production `deploy/compose.yml` and the dev `docker-compose.yml`,
  whose LiveKit image CI requires to be identical to production's). This keeps the pins above from going stale.

## 7. Known gaps

- **Not end-to-end encrypted.** Voice and screen share are encrypted in transit (DTLS-SRTP) but not end to end: the SFU
  on the server can access the media. Messages are encrypted in transit (TLS) and stored unencrypted in the server's
  database. Backups are encrypted.
- **Two-factor authentication is optional.** Nothing requires a member to turn it on, and an account that hasn't is
  protected by its password only.
- **Two-factor codes can be phished.** A fake sign-in page can collect the password and a code and pass both on at
  once, while the code and the 5-minute ticket are still valid. There is no WebAuthn or passkey support.
- **The two-factor key is on the database's host.** `PULSE_TOTP_KEY` lives in the server's `.env`, next to the
  database. It protects the database files and backups, not a compromise of the host (section 4). If the key is lost
  and the secrets can't be decrypted, the owner has to reset each affected account by hand. Restoring a backup onto a
  new host needs the same key too, or every 2FA account has to be reset.
- **Deploy user is root-equivalent.** The host user that runs the containers is in the `docker` group, which makes it
  root-equivalent on the host. Mitigation: public SSH is closed, so it can only be reached over the owner's private VPN
  (section 5).
- **Floating build inputs.** These are not pinned:
  - the GitHub runner images (`ubuntu-24.04`, `ubuntu-24.04-arm`, `windows-latest`): GitHub updates them, and only
    the OS release is fixed (none for `windows-latest`);
  - the versions of apt packages installed on the CI runners (system build dependencies, and LLVM 21's patch level);
  - `ca-certificates` in the server image's runtime stage, installed with apt on top of the pinned base image;
  - the BuildKit (buildx) version used to build the server image, and the `just` version in CI.
- **Backups can be destroyed from the host.** The server's object-store token can delete and overwrite backups (it
  prunes the old ones), and the bucket has no object lock or retention. Someone who takes over the host could destroy
  the backups. The fix is R2 bucket lock and lifecycle rules, which the operator sets in Cloudflare, not this
  repository.
- **The server image is not signed.** It has no signature and no build provenance. The deploy pins the digest it
  pulled, so a moved tag can't change what runs, but nothing proves where that image was built.
- **Code signing.** The Windows installer is not code-signed yet, so SmartScreen and endpoint tools will see an unsigned
  publisher.
- **Updates.** There is no auto-updater. New versions are installed by hand.
- **Accounts.**
  - Any member can create invite codes.
  - Rate limits, including the two-factor limits, are held in memory, so they reset when the server restarts.
  - Login limits trade guessing against lockout. Guesses at one username from many IPs can use up its 100/hour, but
    that doesn't lock its owner out: an IP with no recent failed logins for that username can still sign in. The cost
    is that past 100/hour, each further IP still gets a guess, about one every 6 minutes per IP. An owner who shares
    an IP with the attacker (the same network or NAT) can be refused until either limit refills.
- **Server design.**
  - Any member of a server can rename or delete any custom emoji in it.
  - Every account can read the history of server channels.
  - There are no server roles, kicks or bans.
- **No TURN over TLS.** There is no TURN over TLS/TCP 443, so voice may not work on networks that block UDP and allow
  nothing but 443.

## 8. Reporting a vulnerability

Please report security issues privately. Go to this repository's **Security** tab and choose **Report a
vulnerability** (GitHub private vulnerability reporting). Please don't open a public issue for a security problem.
