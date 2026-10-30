# Link embeds: research

Date: 2026-10-30. Input for a brainstorm and spec, not a decision. The request came from a friend: "link embeds
Discord-style for Instagram/YouTube/TikTok etc. to watch without leaving the app". Instagram is the one that fails
on Discord. Pulse has 5 users.

Confidence labels used below: **[doc]** = the platform's own docs say so; **[report]** = community or third-party
write-ups; **[untested]** = our inference, needs a spike in the real app.

## TL;DR

- Do **server-side unfurling** (the axum server fetches OpenGraph/oEmbed and stores the thumbnail in our media store),
  so clients contact nobody until someone presses play.
- **Inline players are click-to-play iframes** (a "facade"): the thumbnail is shown first, and the platform's iframe
  loads only on click. YouTube, TikTok, Spotify, SoundCloud and Twitch clips all have official, tokenless iframe
  players.
- The Tauri-specific trap is **YouTube error 153**: YouTube now requires a valid `Referer`, and desktop webviews often
  send none. The cleanest fix for every platform is a **tiny HTTPS "embed wrapper" page on our own server**, which the
  app iframes. The platforms then see a normal `https://<our domain>` embedder. This also solves Twitch's `parent=`
  rule.
- **Instagram is possible but fragile.** Meta made oEmbed tokenless in June 2026, but it returns no thumbnail. Instagram
  also blocks server fetches of its pages. A click-to-load `instagram.com/p|reel/<code>/embed/` iframe is the realistic
  option, and it needs a spike.
- **Don't build on mirror services** (ddinstagram, fxtwitter, vxtiktok). The best-known Instagram one, InstaFix, was
  archived in April 2026.

## 1. How Discord does it, and why Instagram fails

- **The crawler.** When a link is posted, Discord's servers fetch the page on demand with a user-agent containing
  `Discordbot` (e.g. `Mozilla/5.0 (compatible; Discordbot/2.0; +https://discordapp.com)`). They read the HTML
  `<meta>` tags: OpenGraph (`og:title`, `og:image`, `og:video`), Twitter cards and oEmbed discovery. The crawl
  happens on demand, not as continuous indexing **[report]** (usehall, Discord support article). The result is stored
  as an "embed" object on the message.
- **The image proxy.** The embed object carries both the original URL and a `proxy_url` for images, thumbnails and
  videos **[doc]** (Discord developer docs, embed object). Clients load media from Discord's own media proxy
  (`media.discordapp.net` / `images-ext-*.discordapp.net`), never from the third party. So the third party never
  sees a client IP, and the image still works when it is hotlink-protected.
- **Inline players.** For a known set of providers (YouTube, Twitch, Spotify, SoundCloud and others), Discord swaps
  the card for the provider's official iframe player when you click play. Sites that expose a direct `og:video` mp4
  play in Discord's own `<video>` element. That is why the "fixer" sites exist: they serve `og:video` tags pointing
  at raw mp4s **[report]** (tiktok-embeds README notes a `.mp4` URL is needed).
- **Why Instagram fails.** Instagram serves Discordbot (and most datacenter IPs) a login wall or a rate-limited page,
  with missing or partial meta tags, and it does not expose a playable `og:video`. Discord's own forum has years of
  "Instagram embeds intermittently broken" threads. Users have proposed that Discord get whitelisted, and Discord
  staff have only called it "a bug" **[report]**. Meta's sanctioned route is its oEmbed API. That needed an app,
  a token and App Review until June 2026, and it has returned no thumbnails since 2025-11-03 **[doc]**. Discord has
  never used it for Instagram. This is a platform-policy problem, not something Discord can fix by better parsing.

## 2. Per-platform options for an inline player

| Platform | Metadata (server) | Inline player | Auth / review | Gotchas |
|---|---|---|---|---|
| **YouTube** | `https://www.youtube.com/oembed?url=…&format=json` (title, author, thumbnail), or OG tags | `https://www.youtube-nocookie.com/embed/<id>` | None | **Referer required** (error 153); min viewport 200×200; don't overlay or modify the player |
| **TikTok** | `https://www.tiktok.com/oembed?url=…` (title, author, thumbnail) | `https://www.tiktok.com/player/v1/<id>` with `autoplay`, `rel`, `music_info`, etc., plus a postMessage API | None | Usage limits per embedding domain; short links (`vm.tiktok.com`) need a server-side redirect resolve to get the numeric id |
| **Instagram** (posts, reels) | Tokenless oEmbed since June 2026: `graph.facebook.com/v25.0/instagram_oembed?url=…`. Returns `html` and `width`, **no thumbnail and no author**. OG fetch is usually login-walled | `https://www.instagram.com/p/<code>/embed/` or `/reel/<code>/embed/`, which is what `embed.js` builds | None for the tokenless call. App Review gives higher rate limits | Public content only; Meta Platform Terms apply; whether reels play inline in the iframe inside WebView2 is **[untested]** |
| **X / Twitter** | `https://publish.twitter.com/oembed?url=…` is documented as no-auth, but returns a `<blockquote>` plus `widgets.js` | No official plain iframe. `platform.twitter.com/embed/Tweet.html?id=` is undocumented | None (oEmbed) | X blocks most bot fetches of x.com pages; `widgets.js` would need script-src in our CSP. Render a native text card instead |
| **Twitch clips** | Helix API (needs a client id) or OG tags | `https://clips.twitch.tv/embed?clip=<slug>&parent=<domain>` | None for the iframe | **`parent` must name the embedding domain(s)**, and Twitch assumes HTTPS sites; min 400×300 |
| **Reddit** | Unreliable: logged-out `.json` returns 403 since May 2026 even with a browser UA; official API signup is closed | `embed.reddit.com` exists but is **[untested]** | OAuth for the API | Treat as a plain link or a best-effort OG card |
| **Spotify** | `https://open.spotify.com/oembed?url=…` | `https://open.spotify.com/embed/<type>/<id>` | None | 30-second previews unless the viewer is logged into Spotify in the webview |
| **SoundCloud** | `https://soundcloud.com/oembed?url=…&format=json` | `https://w.soundcloud.com/player/?url=<track url>` | None | Straightforward |
| **Generic sites** | OG / Twitter-card tags; oEmbed via `<link rel="alternate" type="application/json+oembed">` | **None.** Never iframe arbitrary sites | n/a | A card only (title, description, thumbnail, site name) |

Notes on terms of service:
- **YouTube:** the Required Minimum Functionality rules (Referer identity, min size, no hiding or covering the player)
  apply to anyone using the embedded player **[doc]**. Do not download, proxy or re-host the video stream itself;
  only the official player is allowed. The YouTube API policies also limit how long API data may be stored, so give
  cached metadata and thumbnails a TTL.
- **TikTok:** the Developer Terms apply, and TikTok throttles embedding domains that send heavy traffic **[doc]**.
  At 5 users that is irrelevant.
- **Meta:** the tokenless oEmbed is for public content and falls under the Meta Platform Terms. Meta tells you to
  render its embed HTML rather than build your own card from it **[doc]**.

## 3. Unfurling metadata: where it runs, privacy and safety

**Server-side vs client-side.**

| | Server fetches (recommended) | Each client fetches |
|---|---|---|
| Who sees the user's IP | Nobody until they press play (then the platform, which is unavoidable for a player) | Every linked site sees every viewer's IP as soon as the message renders. A sender can post a link to their own server to log friends' IPs |
| Consistency | One fetch; everyone sees the same card; stored with the message | N fetches; cards differ; work is repeated on every client |
| Bot blocking | One datacenter IP (Instagram and X will often block it) | Residential IPs get through more often (this is the only real plus) |
| Cost | Rust work in the server, plus SSRF hardening | WebView CORS blocks it, so it would need Rust in the client anyway |

**SSRF hardening for a server-side fetcher** (OWASP "case 2: arbitrary URLs"):
- Only `http`/`https`, ports 80/443. Reject URLs whose host is ambiguous between parsers (userinfo, backslashes).
- Resolve DNS yourself, reject if **any** A/AAAA record is non-public: `127.0.0.0/8`, `0.0.0.0/8`, `10/8`,
  `172.16/12`, `192.168/16`, `169.254/16` (cloud metadata), `100.64/10`, `::1`, `fc00::/7`, `fe80::/10`, multicast.
  Then **connect to the validated IP** (pin it; this stops DNS rebinding). In reqwest that means a custom
  `dns::Resolve` that filters addresses, so every connection goes through the check.
- Turn off automatic redirects and follow them by hand: at most 3–5, re-validating each hop.
- Caps: about 5 s total timeout, and read at most 1–2 MB of HTML (stop after `</head>`). Only parse `text/html`.
  Images capped at about 8 MB and passed through the existing `media::process` (decode, resize and re-encode to
  WebP), so we never serve a third party's bytes as-is.
- A concurrency limit (reuse the media slot semaphore), a per-user rate limit, and a cache keyed by the normalised
  URL with a TTL of hours to days.
- An honest user-agent such as `PulseBot/1.0 (+https://…)`. Some sites only serve OG tags to known bot UAs. Faking
  `Discordbot` works on some sites, but it is spoofing; don't.

**Thumbnails: cache vs hotlink.** Hotlinking puts third-party URLs into `img-src`, leaks viewer IPs, and breaks on
hotlink protection and expiring signed URLs (Instagram's and TikTok's CDN URLs expire). Caching through our media
store fits what Pulse already has: content-addressed WebP on disk, served via the `pulse-media` scheme that is
already in `img-src`. It would be a new `MediaKind::LinkThumb` with a cleanup TTL.

**Sites that block bot fetches and the mirror workarounds.**
- Instagram and X block datacenter fetches of post pages; Reddit now 403s logged-out access almost entirely.
- The mirrors: **ddinstagram/InstaFix** (the GitHub repo was **archived on 2026-04-02**), kkinstagram and similar
  short-lived successors, **FxEmbed/fxtwitter** (`api.fxtwitter.com`, a free keyless JSON API, still maintained),
  and **vxtiktok / fxTikTok (tnktok)** (open source, self-hostable).
- **Verdict:** usable as a *best-effort, optional* source of card data. They are not a foundation. They are
  third-party scrapers that break whenever the platform changes, and the Instagram ones die often. Using them also
  sends every link posted in Pulse to a stranger's server, and they may breach the platforms' terms. If we use one,
  put it behind a per-provider switch and fall back to a plain link. fxtwitter is the most durable of them.
  Self-hosting InstaFix only moves the blocking onto our server's IP.

## 4. Tauri-specific constraints

**CSP.** The current CSP has no `frame-src`, so it falls back to `default-src 'self' …` and every third-party iframe is
blocked. Embeds need either:
- (a) a wrapper: `frame-src https://<our-server-domain>`, a single origin we control. Recommended.
- or (b) a direct list: `https://www.youtube-nocookie.com https://www.tiktok.com https://www.instagram.com
  https://open.spotify.com https://w.soundcloud.com https://clips.twitch.tv`.

`img-src` needs no change if thumbnails go through `pulse-media`.

Embeds must **not** go through `markdown.ts` / `{@html}`. The DOMPurify allow-list rightly has no `iframe`. Render
embeds as a separate Svelte component (`LinkEmbed.svelte` under `MessageItem`) from structured data the server
attaches to the message, and build iframe URLs only from an allow-list of provider templates plus a validated id.

**App origins.** In Tauri 2 the page origin is `http://tauri.localhost` on Windows (WebView2) and `tauri://localhost`
on Linux (WebKitGTK). `app.windows.useHttpsScheme` switches Windows to `https://tauri.localhost`, but it changes the
origin, so localStorage and IndexedDB are lost.

**YouTube error 153 and Referer.** YouTube requires the embedder to identify itself through the `Referer` header. In
webviews the header is empty by default, and for desktop apps Google says to set it explicitly, e.g. via WebView2's
`CoreWebView2HttpRequestHeaders.SetHeader`. The value should be an HTTPS URL whose host is the app's ID, e.g.
`https://app.pulse.client` **[doc]**. Error 153 ("embedder.identity.missing.referrer") is the result when the header
is missing. `file://` and custom-scheme origins hit it (ARGUS, TiddlyWiki, Simon Willison), and `referrerpolicy=
"strict-origin-when-cross-origin"` plus `youtube-nocookie` fixes many browser cases **[report]**.

How each webview behaves **[untested, spike needed]**:
- **WebView2 (Windows, the main platform):** `http://tauri.localhost` is a real http origin, so with the default
  policy the iframe probably sends `Referer: http://tauri.localhost/`. YouTube may or may not accept that. The
  documented fix is a `WebResourceRequested` filter with source kind "all" (so iframe requests are covered) that sets
  `Referer: https://app.pulse.client/` on `*.youtube*.com`. This is reachable through Tauri's
  `WebviewWindow::with_webview` and the `webview2-com` crate.
- **WebKitGTK (Linux, dev only):** `tauri://localhost` is a custom scheme. WebKit treats custom-scheme loads as non-HTTP
  and probably sends no Referer, so expect error 153. WebKitGTK has no easy per-subframe header hook. (H.264
  playback also depends on GStreamer plugins being installed.)

**Options, in order of preference:**
1. **An HTTPS embed wrapper on our server** (e.g. `GET /embed/<provider>/<id>` on the axum server). It returns a tiny
   static page that iframes the provider. It sends `Referrer-Policy: strict-origin-when-cross-origin` and
   `Content-Security-Policy: frame-ancestors tauri: http://tauri.localhost https://tauri.localhost`, plus a strict
   frame-src. The app iframes the wrapper. YouTube then sees `Referer: https://<our domain>/`, and Twitch gets
   `parent=<our domain>`, a real HTTPS domain. It works the same on WebView2 and WebKitGTK, needs one entry in the
   app CSP, and puts the provider allow-list on the server, where we can change it without shipping a client. The
   costs: one extra hop, and an endpoint we must keep locked down (provider and id validated, no open redirect).
   Twitch may check the whole ancestor chain, so it might also need `parent=tauri.localhost` **[untested]**.
2. **WebView2 header injection** (`https://app.pulse.client` Referer). This is Google's documented way for desktop
   apps, but it only works on Windows and only fixes YouTube.
3. **A pop-out `WebviewWindow`** loading the wrapper or watch URL as a top-level page. This avoids every iframe rule
   and is the fallback for Instagram if its iframe misbehaves. It is "in the app" but a separate window. That window
   must get no IPC capabilities: remote content gets no Tauri commands.

Don't use `useHttpsScheme` just for this (it loses storage), and don't load remote pages into the main window.

## 5. Recommendation for Pulse

**Phase 1: cards plus YouTube (cheap, reliable).**
- **Server unfurler.** On message create, detect the first 1–3 URLs (skip any inside code blocks or `<…>`
  suppression, as Discord does). Fetch them async with SSRF guards. Store a `link_embeds` row: url, provider, title,
  description, site name, thumbnail media hash, provider id, and fetch time. Push it to clients as a message update
  over the existing WebSocket. Use oEmbed for known providers and OG for everything else. Size **M**.
- **Thumbnails through the media store** (`MediaKind::LinkThumb`, WebP, TTL cleanup). Size **S**.
- **Client `LinkEmbed.svelte` card** (title, site name, description, thumbnail, and a hide-embed control for the
  sender). Size **S**.
- **YouTube click-to-play** via the HTTPS wrapper plus the `frame-src` CSP entry. Spike error 153 in WebView2 first.
  Size **S–M**.

**Phase 2: more players (cheap once the wrapper exists).** Each one is a provider template plus an id parser, size
**S**:
- Spotify, SoundCloud and Twitch clips (`parent` rule; spike it).
- TikTok `player/v1` (vertical 9:16 layout, and a server-side resolve of `vm.tiktok.com` short links).

**Phase 3: Instagram (harder, lower confidence).** Card from tokenless oEmbed plus click-to-load
`/p|reel/<code>/embed/` iframe. With no thumbnail, the card is "Instagram · reel" plus the caption if we can get it.
Spike first: do reels actually play inside the iframe in WebView2, or demand a login? What are the tokenless rate
limits from our server IP? Size **M**, and it may end at "card plus pop-out window" if inline play fails.

**Avoid:**
- Iframing arbitrary sites.
- Proxying or downloading video streams (yt-dlp style).
- `widgets.js` / `embed.js` script tags in the app.
- Faking `Discordbot`.
- Making any mirror service a hard dependency.
- X inline playback: give X a text card, optionally from fxtwitter. Reddit gets a link or best-effort card.

**Decisions for the owner:**
1. **Privacy model.** Server-side unfurling (recommended: no client IP leaks; the server's IP and the URLs posted are
   seen by third parties) vs client-side (each viewer's IP goes to every linked site)?
2. **Mirror services.** Allow fxtwitter / vxtiktok / Instagram mirrors as optional, switchable sources of card data,
   knowing each one sees every link of that type posted in Pulse and can vanish without notice? Or official
   endpoints only, accepting thinner Instagram and X cards?
3. **Wrapper page on our server** (`/embed/...`, public HTTPS) vs WebView2-only header injection. The wrapper is
   recommended; it means one public, unauthenticated route on the server.
4. **Instagram scope.** Is "card plus click-to-load iframe, may fall back to a pop-out window" good enough? The friend
   should know that inline reels aren't guaranteed.
5. **Defaults.** Autoplay off (click-to-play only)? A per-user "don't load embeds" setting? A per-message
   "suppress embeds" control for the sender? How many links get embedded per message (Discord shows up to 5 or so)?

## Sources

- Discord: link previews / Discordbot (support article): https://support.discord.com/hc/en-us/articles/42500550752919
- Discordbot user-agent: https://usehall.com/agents/discordbot
- Discord developer docs, embed object (`proxy_url`): https://discord.com/developers/docs/resources/message#embed-object
- Discord community, Instagram embeds broken: https://support.discord.com/hc/en-us/community/posts/360052124033 and
  https://support.discord.com/hc/en-us/community/posts/360068125232
- TikTok OG-embed fixer (notes on Discord's mp4 requirement): https://github.com/dustinrouillard/tiktok-embeds
- YouTube Required Minimum Functionality (Referer identity, WebView guidance, min size):
  https://developers.google.com/youtube/terms/required-minimum-functionality
- YouTube error 153 write-ups: https://til.simonwillison.net/youtube/fixing-153-embed ,
  https://github.com/simonw/simonwillisonblog/pull/562 ,
  https://talk.tiddlywiki.org/t/youtube-error-153-when-using-utube-simple-annotations/15442 ,
  https://teamdynamix.umich.edu/TDClient/30/Portal/KB/Article/14491/Fixing-YouTube-Player-Error-153-with-Referrer-Policy-Settings
- ARGUS desktop app (custom scheme rejected; fixed by serving over http): https://github.com/0xhav0c/ARGUS/releases/tag/v1.0.1
- YouTube in WKWebView still failing: https://github.com/openai/openai-apps-sdk-examples/issues/161
- TikTok embed player v1: https://developers.tiktok.com/doc/embed-player/ ; TikTok oEmbed: https://developers.tiktok.com/doc/embed-videos/
- Meta tokenless oEmbed (2026-06-15): https://developers.facebook.com/blog/post/2026/06/15/tokenless-access-to-meta-oembed-apis/
- Meta oEmbed updates, thumbnail/author removal from 2025-11-03: https://developers.facebook.com/blog/post/2025/04/08/oembed-updates/ ,
  https://iframely.com/updates/193071-facebook-and-instagram-oembed-thumbnail-deprecation
- Instagram oEmbed docs: https://developers.facebook.com/docs/instagram-platform/oembed
- Discourse, Instagram app-token setup (old App Review flow): https://meta.discourse.org/t/configure-an-app-token-for-instagram-oneboxes/223548
- X oEmbed API: https://developer.x.com/en/docs/twitter-for-websites/oembed-api
- Twitch video and clip embeds (`parent`, min size): https://dev.twitch.tv/docs/embed/video-and-clips/ ; parent pitfalls:
  https://meta.discourse.org/t/twitch-clips-embed-problem/344678
- Spotify embeds and oEmbed: https://developer.spotify.com/documentation/embeds
- SoundCloud oEmbed: https://developers.soundcloud.com/docs/oembed
- Reddit logged-out 403s (2026): https://bigideasdb.com/reddit-json-403-forbidden ,
  https://dev.to/tonywangca/why-reddit-blocked-unauthenticated-json-in-2026-and-how-to-still-get-reddit-data-58b9
- InstaFix / ddinstagram (archived 2026-04-02): https://github.com/Wikidepia/InstaFix
- FxEmbed / fxtwitter: https://github.com/FxEmbed/FxEmbed , https://docs.fxtwitter.com/en/latest/api/about.html
- vxtiktok: https://github.com/dylanpdx/vxtiktok ; fxTikTok (tnktok): https://github.com/okdargy/fxTikTok
- OWASP SSRF Prevention Cheat Sheet: https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html
- WebKit: custom-scheme loads are not HTTP: https://bugs.webkit.org/show_bug.cgi?id=203273
- Tauri 2 config reference (`useHttpsScheme`, remote capabilities): https://v2.tauri.app/reference/config/
