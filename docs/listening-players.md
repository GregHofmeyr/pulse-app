# "Listening to": which players count (#322)

"Listening to" (#275) reads the OS media session: the system media transport controls (GSMTC) on Windows, MPRIS on
Linux. Every app that plays sound can show up there, including a browser tab playing any video. So Pulse reads only
sessions on a strict allowlist and ignores everything else. The code is `client/src-tauri/src/activity/parse.rs`
(`classify_gsmtc`, `classify_mpris`, `now_playing_*`), and its tests double as fixtures.

All of it sits behind **Read media activity**. Under it, two switches (Settings → Privacy → Activity), both on until
turned off:

| Group | Switch | Players |
| --- | --- | --- |
| Spotify | (Read media activity itself) | Spotify app |
| Players | Share music from other players | YouTube Music app, Apple Music, Tidal, Deezer, foobar2000 |
| Browsers | Share music from a browser | YouTube Music installed from a browser; music in a browser tab |

When several play at once, Spotify wins, then a desktop player, then a browser.

## Windows (GSMTC `SourceAppUserModelId`, case-insensitive)

| Player | App ids |
| --- | --- |
| Spotify | `Spotify.exe`; `SpotifyAB.SpotifyMusic_*` (Store) |
| YouTube Music (app) | `com.github.th-ch.youtube-music`, `YouTube Music.exe` (th-ch/youtube-music) |
| YouTube Music (installed from a browser) | `Chrome._crx_cinhimbnkkaeohfgghhklpknlkffjgod`, same with `MSEdge.` / `Brave.` |
| Apple Music | `AppleInc.AppleMusicWin_*` (Store) |
| Tidal | `TIDAL.exe`, `com.squirrel.TIDAL.TIDAL` |
| Deezer | `Deezer.exe`, `com.deezer.deezer-desktop`, `Deezer.62021768415AF_*` (Store) |
| foobar2000 | `foobar2000.exe`, `foobar2000` |
| Browser tab | `Chrome`, `MSEdge`, `Brave`, `Vivaldi`, `Opera` (also with a `.<suffix>` per install, or as `<name>.exe`); Firefox's default-folder ids `308046B0AF4A39CB` (64-bit) and `E7CF176E110C211B` (32-bit) |

Other installed web apps (`*._crx_<other id>`, YouTube's included) never count. Neither do the Windows Media Player,
Films & TV, VLC, Discord or anything else not listed.

## Linux (MPRIS bus name `org.mpris.MediaPlayer2.<app>[.instance…]`)

| Player | `<app>` |
| --- | --- |
| Spotify | `spotify` (not `spotifyd`) |
| YouTube Music (app) | `youtube-music`, `YoutubeMusic` |
| Apple Music | `cider`, `Cider` (Cider) |
| Tidal | `tidal-hifi` |
| Deezer | `deezer`, `Deezer` |
| Browser | `chromium`, `chrome`, `google-chrome`, `brave`, `vivaldi`, `opera`, `edge`, `msedge`, `firefox`, `librewolf`, `plasma-browser-integration` |

`playerctld`, KDE Connect's phone players, mpv, VLC and the rest never count.

## Browsers: only when it's clearly music

A browser plays anything, and GSMTC gives only a tab's **title, artist, album title** (plus album artist, track number,
genres and a thumbnail). It gives no page address or site name. So a browser session counts only when:

- **Windows:** it has an artist **and** an album. YouTube Music, Spotify's web player and the other web players set an
  album in their media metadata. Video sites, YouTube's included, set the channel as the artist and no album, so a
  video's title never shows. The label is "a browser", since Windows doesn't say which site.
- **Linux:** when the browser gives the page's address (`xesam:url`, as Plasma's browser integration does), only
  `music.youtube.com`, `open.spotify.com`, `music.apple.com`, `listen.tidal.com` and `deezer.com` count, each labelled
  as its player; any other address never counts. Without an address, the Windows rule applies.

## What others see

`Person.listening` keeps its Spotify shape and adds `player` (absent means Spotify, as older servers and clients
expect). Only Spotify's activity has a Spotify id, album art, the "Listen on Spotify" link-out, "Share to stage", or a
Jam's "Now playing". Every other player shows a generic music icon with its name ("Listening on Tidal", "Listening in a
browser"). Clients from before #322 show another player's song as if from Spotify, with an "Open Spotify" search link
and no art, until they update.
