# Link embeds

Research: `docs/superpowers/research/link-embeds.md`, which has the sources, the per-platform details and the
Tauri constraints. Read it with this.

## Why

A friend asked for "link embeds Discord-style for Instagram/YouTube/TikTok etc. to watch without leaving the app".
Instagram is the one that fails on Discord. The owner agreed (2026-10-22). On 2026-11-02 he accepted every
recommendation below and asked how Instagram and TikTok in particular would work.

## Goals

- **A preview card** under a message that holds a link: title, site name, a short description and a thumbnail.
  The server builds it once, and everyone sees the same card.
- **Click-to-play** inside the message for YouTube, TikTok, Spotify, SoundCloud and Twitch clips. For Instagram
  posts and reels, too, if the spike shows they play; otherwise in a small pop-out window.
- **Privacy:** a friend's app contacts no third party until someone presses play. Only the server fetches
  previews.

## Non-goals

- Downloading or re-hosting other sites' videos.
- Inline players for arbitrary sites. Unknown sites get a card at most.
- X/Twitter and Reddit inline playback. They get a text card or a plain link.
- Third-party mirror services (fxtwitter, vxtiktok, Instagram mirrors). Owner decision: official endpoints only.

## Anti-goals

- **No SSRF:** the server's fetcher must never reach the server itself, the tailnet, cloud metadata or any private
  address.
- **No third-party HTML in the page.** No provider scripts (`embed.js` / `widgets.js`), no iframes through
  markdown or `{@html}`, and no remote content in the main window with Tauri command access.
- **Nothing plays by itself.**

## Owner decisions (2026-11-02, all as recommended)

1. **The server fetches previews.** Clients never fetch link metadata.
2. **Official endpoints only.** No mirror services; thinner Instagram and X cards are accepted.
3. **One public HTTPS embed page on our server** (`/embed/<provider>/<id>`) that the app iframes. It is what makes
   YouTube (error 153) and Twitch (`parent=`) work the same on Windows and Linux.
4. **Instagram:** first a short spike on Windows to see whether reels and posts play in Instagram's official embed
   iframe inside the app. If yes, they play inline like the others. If not, clicking opens a small pop-out window
   inside Pulse that has no access to Tauri commands, with "open in browser" as the last resort.
5. **Defaults:**
   - click-to-play only, never autoplay;
   - a per-user setting "Show link previews" (on by default);
   - the sender can hide the embeds on their own message;
   - at most 3 links per message get a card.

## Behaviour

- **Which links:** `http`/`https` links in the text, the first 3 that are distinct. Links in code blocks or wrapped
  as `<https://…>` (the Discord convention for "no preview") are skipped. Editing a message re-reads its links:
  new ones get cards, removed ones lose theirs.
- **When:** the message posts at once and the card follows when the fetch finishes (a short wait at most), arriving
  as a message update over the existing connection. A failed fetch means no card; the message is unchanged.
- **Card:**
  - a provider label and icon (YouTube, TikTok…), title, one or two lines of description, and the thumbnail;
  - for players, the thumbnail has a play button, and clicking it swaps in the player at a sensible size (16:9; 9:16
    for TikTok and reels);
  - one player plays at a time. Starting another, or scrolling the message far away, stops the first.
- **Hide:** the sender gets "Remove preview" on each card (a ✕ on hover) and in the message menu. Hidden previews
  stay hidden for everyone.
- **Setting off:** the "Show link previews" setting hides every card on that device (plain links only).
- **Per provider:**
  - **YouTube:** videos, Shorts and `youtu.be` links. The card comes from oEmbed. The player is
    `youtube-nocookie.com/embed/<id>` through our embed page, keeping the start time if the link has one.
  - **TikTok:** the card comes from TikTok's official oEmbed (title, author, thumbnail). The player is
    `tiktok.com/player/v1/<id>` through our embed page, in 9:16. Short links (`vm.tiktok.com`, `tiktok.com/t/`)
    are resolved on the server.
  - **Spotify, SoundCloud and Twitch clips:** official oEmbed or OG for the card, and their official players through
    the embed page.
  - **Instagram:**
    - The card comes from the tokenless oEmbed. It has no thumbnail, so it reads "Instagram · reel / post" plus the
      caption when available, with a neutral placeholder image.
    - Playback follows the spike outcome above.
  - **Everything else:** an OpenGraph card if the page offers one, else nothing. No player.

## How it gets made (builder's call where not stated)

- **Server unfurler:**
  - It runs asynchronously after a message is created or edited. Fetches are guarded per the research's SSRF list:
    - http/https only on ports 80 and 443;
    - our own DNS resolution, rejecting any non-public address, and connecting to the validated IP;
    - redirects followed by hand (at most 5), each one re-checked;
    - a 5 s timeout, at most 1–2 MB of HTML, and `text/html` only for page fetches;
    - an honest user agent (`PulseBot/1.0`).
  - A cache keyed by the normalised URL, valid for a day.
  - Concurrency is limited and each user is rate-limited.
- **Storage:** a `link_embeds` table (migration after search's), one row per message link: url, provider,
  provider id, title, description, site name, thumbnail media hash, kind (card or player), hidden flag, and
  fetched_at. It goes out to clients in the message payload; the protocol types are extended.
- **Thumbnails:** fetched by the server, size-capped, decoded and re-encoded to WebP through the existing media
  processing, stored as a new media kind, and served through `pulse-media`. The hourly media cleanup must treat
  thumbnails referenced by `link_embeds` as live, so a deleted message's thumbnails go away with it.
- **The embed page:**
  - `GET /embed/<provider>/<id>`: a tiny static HTML page that iframes the provider's official player.
  - Ids are validated per provider and URLs built from templates only; no open redirect.
  - Response headers: `Referrer-Policy: strict-origin-when-cross-origin`, plus `Content-Security-Policy` with
    `frame-ancestors` for the app origins (`http://tauri.localhost`, `https://tauri.localhost`, `tauri:`) and
    `frame-src` for the provider.
  - Caddy routes it like the rest of the server.
- **Client:**
  - A `LinkEmbed.svelte` under MessageItem, built from structured data, never through markdown or `{@html}`.
  - The app CSP gains one `frame-src` entry for the server's origin.
  - Iframe URLs come from the embed page only.
  - Players are `sandbox`ed with only what each provider needs.
- **Instagram pop-out (if needed):**
  - A separate `WebviewWindow` with no capabilities (so no IPC), loading the embed page or the post URL as a
    top-level page.
  - One pop-out at a time; closing it ends playback.
- **Settings:** "Show link previews", per device, in Settings → Chat.

## Settled after the builder check (2026-11-02, builder's calls confirmed by the coordinator)

- **Where players work:** only when the app is connected to a server over HTTPS. The embed page's origin is the
  server's own base URL, and the app CSP's `frame-src` is pinned to `https://pulsechat.co.za`, as voice is already
  pinned to `voice.pulsechat.co.za`. Against any other server (the http loopback test kit, dev), cards still show,
  and the play button opens the link in the browser instead.
  - The server learns its public origin from a new config value, `PULSE_PUBLIC_URL`, defaulting to unset, which
    disables the embed page. It is used for Twitch's `parent=` too.
  - So phase 1's error-153 spike and the owner's player tests happen against production after a deploy.
    Deploys still need the owner's OK.
- **Fallback if the embed page doesn't fix YouTube error 153, or Twitch refuses the app as an ancestor:** no
  Windows-only native Referer injection. That provider's play button uses the capability-free pop-out window
  instead (built in phase 1 in that case, rather than phase 3).
- **Retention:** a card and its thumbnail live as long as the message. They come from oEmbed/OpenGraph, not
  YouTube's Data API, so its storage limits don't apply. No TTL or refresh sweep.
- **Remove preview and edits:** hidden is kept per URL. A link the sender hid stays hidden through edits, and
  there's no un-hide.
- **Builder's own calls, recorded:**
  - quotes and clips messages get no link previews;
  - no backfill for messages sent before this ships;
  - new protocol fields are `serde(default)`, so old and new clients and servers interoperate;
  - soft delete removes the message's `link_embeds` rows.

## Phases (one design; the plan may split it)

1. The unfurler, thumbnails, cards, the embed page, and the YouTube player. Spike error 153 on WebView2 early.
2. TikTok, Spotify, SoundCloud, Twitch clips.
3. The Instagram spike, then inline or pop-out.

## Testing

- **Server:**
  - SSRF guards: private, loopback, link-local and metadata IPs are refused, DNS rebinding is blocked, redirects
    are re-checked, and size and time caps hold;
  - link extraction: code blocks, `<…>` suppression and the cap of 3;
  - provider id parsing for every URL form;
  - the embed page: valid ids only, correct headers, no redirect;
  - cleanup of thumbnails from deleted messages;
  - a fake provider server for the oEmbed and OG flows.
- **Client:**
  - cards render from data, never as HTML;
  - click-to-play swaps in the player;
  - one player at a time;
  - hide (sender only) and the device setting.
- **Manual (owner, Windows):**
  - a YouTube link plays inline, including a Short and a timestamped link;
  - a TikTok link plays in 9:16;
  - an Instagram reel and post, per the spike outcome;
  - a random news site shows a card;
  - a private-IP link gets no card;
  - hiding a preview;
  - the setting off.
