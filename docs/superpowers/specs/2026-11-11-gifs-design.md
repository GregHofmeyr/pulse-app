# GIFs and GIF search

Issue #58. Decided with the owner on 2026-11-11. The owner's own words and decisions are marked **(owner)**; everything else is the builder's call unless it says otherwise.

## What it does

- **Giphy (owner).** A **GIF button** in the composer (next to emoji) opens a GIF picker: a search box, trending GIFs when empty, a masonry grid of results, and a **Favourites** tab. Clicking a GIF sends it as its own message (or adds it to the queued message, builder's call matching Discord-like feel).
- **Favourites:** a star on hover in the picker and on any GIF in chat; stored per user on our server, so they follow you across devices.
- **In chat:** GIFs play inline (respect reduced motion: show the still and play on hover/click), sized sensibly, with the GIF's title as alt text. A Giphy link pasted into chat renders the same way.
- **Attribution:** "Powered by GIPHY" in the picker, per Giphy's terms.
- **Older clients (0.2.x):** the message's `content` is the Giphy URL, so they see a link/embed.

## How it gets made

- **We don't store GIFs.** Messages store the Giphy ID and media URLs; clients load media straight from Giphy's CDN, as Giphy's terms require (the builder checks the current API terms and attribution rules and follows them).
- **Search proxy:** the client never holds the API key. A server route (`/gifs/search?q=`, `/gifs/trending`, signed-in only, rate-limited per user, short in-memory cache) calls Giphy's API with `GIPHY_API_KEY` from the server's environment, with content rating `pg-13` by default (builder's call; say which), and returns only what the picker needs.
- **Key setup (owner does it):** create an app at developers.giphy.com → API key; add `GIPHY_API_KEY=…` to `/srv/pulse/.env` on the server (never in the repo or chat). Without the key, the GIF button shows "GIFs aren't set up on this server yet".
- **Tests** use a fake Giphy server; no real API calls in CI.
- Favourites table: next free migration number, foreign keys OFF at migration time.
- Look and feel matches the emoji picker exactly (same popover, tabs, search field, grid spacing); existing tokens.

## Out of scope

- Stickers (separate later), uploading GIFs to Giphy, Tenor.
