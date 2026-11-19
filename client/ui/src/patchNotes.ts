// The patch notes shown from the title bar (#207), with the release before collapsed beneath them (#235: two releases
// can land before someone opens the notes). At each release:
//   1. move the current notes to previous: replace the whole `previousPatchNotes` object with `patchNotes` as it
//      stands (its version and all; the older notes drop off);
//   2. `just bump-version X.Y.Z` (it only touches `patchNotes`), then write the new notes in `patchNotes`.
// A new `id` brings the dot back on everyone's icon until they open the notes once. Empty sections are left out of the
// dialog; the previous release shows only its New / Bug fixes / Quality of life (its roadmap and sign-off are stale).
// The repository is public, so no friends' names (or anything else private) go in here.

export type PatchNotes = {
  /** Unique per release; what "seen on this device" is remembered by. */
  id: string
  /** The app version these notes are for: `[workspace.package] version` in Cargo.toml (`just bump-version` sets both). */
  version: string
  /** The release date as people read it. */
  date: string
  new: string[]
  fixes: string[]
  qol: string[]
  /** Coming soon (#224): a light roadmap, not a promise. Up to 5 bullets; more are cut off. Optional, hidden when empty. */
  soon?: string[]
  /** The sign-off at the bottom; may hold a line break. */
  message: string
}

export const patchNotes: PatchNotes = {
  id: '0.2.16',
  version: '0.2.16',
  date: '19 November 2026',
  new: [
    'Servers are now private: you only see servers you\'re in, and you join with an invite',
    'Server icons: anyone in a server can give it a picture, shown on the server bar, invites and everywhere else it appears',
    'Server settings › Overview: the server\'s icon, banner and name in one place, shown together the way its invites show them',
  ],
  fixes: [
    'The Invite dialog keeps one link while it\'s open: changing the button text, expiry or uses no longer swaps the link (or breaks Copy)',
  ],
  qol: [
    'Rename a server in place, from the name at the top of its channel list or from right-click on its icon',
    'Settings › About: which version you\'re on, the server you\'re connected to, Check for updates, and this version\'s patch notes',
    'Right-click a channel › Move to category; Move up and Move down are gone, since dragging reorders',
    'Anyone in a server can change its icon, banner and name, not just its owner',
    'Your profile, online status and what you\'re up to reach only people you share a server, a DM or a group with',
  ],
  message: 'Your servers, your people 🔒',
}

/** The release before `patchNotes` (#235), shown collapsed below them; `null` = none. */
export const previousPatchNotes: PatchNotes | null = {
  id: '0.2.15',
  version: '0.2.15',
  date: '18 November 2026',
  new: [
    'Give a channel or group its own message sound from its right-click menu (someone\'s own message sound still wins)',
    'Stickers: the GIF picker has a Stickers rail with GIPHY\'s stickers, which sit right on the chat',
    'Check for updates yourself in Settings › Advanced › Updates',
    'Invite people to a server with a link, or straight from their right-click menu',
    'Invites show as cards in chat, with a Join button',
    'Server owners can ban someone from their right-click menu, and see who is banned in Server settings › Bans',
    'A new profile card theme, Dusk: purple to deep rose',
    'Invite links work for friends without Pulse too: the link opens a page with Open in Pulse and a Windows download, and a server owner\'s link signs them up straight into the server',
    'A fresh profile editor: tabs for About you, Card, Name style and Badges, with your card always in view. Hover your avatar or banner on it to change them, and drag your badges into the order you like',
    'Name effects: give your name Neon, Toon, Pop, Gummy, Prism, Shimmer, Glitch or Ember in Settings › Profile. They move when someone hovers your name or opens your profile',
    'A queue for watch parties: anyone in the call can paste or drop in YouTube links from the Queue button, and the party moves on to the next video for everyone. Right-click a queued video to play it now, move it or remove it',
    'Twitch in watch parties: VODs stay in sync like YouTube, live channels play and pause together until someone skips, and clips start together. Twitch cards for VODs and channels play in chat too',
    'Server banners: a server\'s owner can give it a banner in Server settings › Banner, and its invites show it',
  ],
  fixes: [
    'Restoring a deleted channel now tells you when the server is at its channel limit, and deleted channels no longer count toward it',
    'Do Not Disturb for someone now also covers their calls (no ringing) and their going live (no notification)',
  ],
  qol: [
    'Right-click a clip\'s poster for their menu, in the list and on the wall',
    'Drag a conversation onto one of your categories, Pinned or the main list to move it there',
    'Signing someone out of everything takes effect within seconds, even if their app is open',
    'The Sunset card theme\'s orange is brighter, and secondary text on dark cards is a little lighter',
    'A tidier message box: attach files, wheels, polls, plans, game invites and Send later all live in the + menu',
    'Emoji and GIFs share one picker with tabs, and it opens on the one you used last',
    'Go Live\'s sound picker is a proper dropdown that shows the app by name and icon, with a live level meter',
    'Streaming "everything" leaves out Discord as well as Pulse, so a call there isn\'t carried into your stream',
    'Right-click a pinned message or an incoming call for that person\'s menu',
    'A plan\'s forecast now gives the temperature for when it starts, not just the day\'s range',
    'Deadlock rank movers now post every Monday morning, instead of a week after the first run',
  ],
  soon: [
    'Private servers you join by invite',
  ],
  message: 'More to come ✨',
}
