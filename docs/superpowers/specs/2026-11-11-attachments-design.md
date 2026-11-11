# Images and files in chat

Issue #57. Decided with the owner on 2026-11-11. The owner's own words and decisions are marked **(owner)**; everything else is the builder's call unless it says otherwise.

## Why

Friends can't post a screenshot or share a file today. Discord-style attachments are the biggest everyday gap. Some shares are big one-offs (a Minecraft world, a mods pack) that nobody needs to keep **(owner)**.

## What it does

- **Where:** server text channels, voice channels' chats, DMs and groups. Not quotes or clips channels (they have their own media).
- **Adding files:** paste (screenshots straight from the clipboard), drag and drop onto the chat, or a "+" / paperclip button in the composer that opens the file picker. The message box shows the queued files as removable chips/thumbnails with sizes; the text is the message's caption. Up to **10 files per message (owner)**.
- **Folders (owner):** a folder can't be sent as-is (like Discord); individual files are fine. If someone drops a folder, Pulse offers **"Zip and send"**, zipping it in the app before upload (builder's call whether to ship this in the first cut; at minimum, refuse folders with a clear "Zip the folder first" message).
- **Normal attachments: up to 50 MB each, kept forever (owner).**
  - **Images** are converted to **WebP** on upload (high quality, longest side capped at 4096 px; animated GIF/WebP stay animated) **(owner)**, shown inline (one large, or a grid for several), click for a full-size viewer (zoom, next/previous, save image, open original size).
  - **Videos** (mp4/webm/mov): inline player with poster frame; not re-encoded.
  - **Audio:** inline compact player.
  - **Other files:** a file card (icon, name, size, Download). Files that can run as programs (.exe, .msi, .bat, .cmd, .ps1, .scr, .sh, .AppImage, .jar, …) get a small "This file can run programs on your computer" warning on Download. Pulse never opens files by itself.
- **Big temporary sends (owner): above 50 MB, up to 2 GB per file.** The sender picks **expires in 1 day or 7 days**. The card shows the file, size, "expires in 6 days" and Download; after expiry it shows "Expired" and the bytes are deleted from storage. A **server-wide cap of 20 GB of live temp files** refuses new big sends with a clear message when full. Big uploads go up in chunks and resume after a dropped connection; progress shows in the message box and the message posts when the upload finishes.
- **Any file type (owner).**
- **Deleting** a message deletes its attachments from storage (after the existing message-delete rules). Editing a message can't change its files (builder may allow removing one).
- **Search:** a `has:media` / `has:file` filter (#57 mentions it) if the search supports filters cheaply; otherwise a follow-up issue.
- **Notifications and previews** show "📎 2 images" / "📎 report.pdf" when a message has no text.
- **Older clients (0.2.x)** must not break: a message with attachments still has readable `content`; if it has none, include a short plain-text fallback such as "📎 image.webp" so it isn't a blank row.

## How it gets made

- **Storage:** Cloudflare R2, the same bucket and presigned-upload pattern clips use (`server/src/clips/`, `server/src/media/r2.rs`). The client uploads directly to R2 with a presigned PUT (or multipart presigned parts for big files); the server never proxies the bytes. Downloads use short-lived presigned GETs, reusing the clip link-reuse and cache-control work from #268.
- **WebP conversion:** done on the client before upload (the Rust core already has image tooling for emoji/avatars; check what's there) so the server never decodes untrusted images; the server still validates type/size from the upload's metadata and the stored object.
- **Server:** an attachments table (message, R2 key, name, type, size, width/height/duration for media, expires_at for temp sends), routes to start an upload (validates size/type/limits/caps, returns presigned URL(s)), complete it, and attach to a message when sending; a sweep that deletes expired temp files and orphaned uploads that never got attached (e.g. after 24 h); everything counted for the 20 GB temp cap. Next free migration number; foreign keys OFF at migration time (`server/src/db.rs`).
- **A `just storage` recipe** prints R2 usage by kind (attachments, temp, clips, sounds, emoji) like `just clips-check`.
- Look and feel matches existing message cards, embeds (#40) and the clip player exactly; existing tokens; icon sizes 14/16/18; reduced motion respected.

## Testing

- Server: limits (count, 50 MB, 2 GB, 20 GB cap), membership checks, expiry sweep, orphan sweep, delete removes objects, presigned URL scoping.
- Client: paste/drop/pick, chips, WebP conversion (size and dimensions), upload progress and resume, inline image grid and viewer, video/audio players, file card and the run-programs warning, temp-send expiry picker and Expired state, 0.2.x fallback text.
- **Manual (owner, two accounts):** paste a screenshot, drop 3 images and a PDF, send a 300 MB zip as a 1-day temp send and download it on the other account.

## Out of scope

- Image editing/cropping before send, spoiler blur (follow-up issue if wanted), per-user storage quotas.
