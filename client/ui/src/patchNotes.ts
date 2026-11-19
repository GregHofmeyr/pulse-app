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
  id: '0.2.17',
  version: '0.2.17',
  date: '20 November 2026',
  new: [
    'Favourite stickers: star a sticker in the GIF picker or in chat, and find it again under Favourite stickers, a list of its own beside your favourite GIFs',
    'Going live survives a short network drop: your stream shows Reconnecting and carries on by itself, with no need to pick the screen again',
  ],
  fixes: [
    'The opened server card from an invite pack is sharp again (its text and icon were slightly blurry), and "✦ MYTHICAL PULL ✦" is easier to read in every theme',
    'You can\'t schedule a message in a DM with someone you\'ve blocked any more (it used to wait until its time, then fail)',
    'Voice: when your connection drops for a moment and comes back, nobody else hears you leave and rejoin',
  ],
  qol: [
    'In a full profile\'s Name Archive, click who made a change to see their profile, or right-click them for their menu',
    'A watch party opened in a window moves on with the party to its next video, and closes when you leave the party',
    'Making a poll anonymous in a DM or a small group now warns that people can still work out a vote from the counts changing',
  ],
  message: 'Polish 🧽',
}

/** The release before `patchNotes` (#235), shown collapsed below them; `null` = none. */
export const previousPatchNotes: PatchNotes | null = {
  id: '0.2.16',
  version: '0.2.16',
  date: '19 November 2026',
  new: [
    'Servers are now private: you only see servers you\'re in, and you join with an invite',
    'Server icons: anyone in a server can give it a picture, shown on the server bar, invites and everywhere else it appears',
    'Server settings › Overview: the server\'s icon, banner and name in one place, shown together the way its invites show them',
    'My status here: right-click a server to be Online, Do Not Disturb or Invisible in that server only. Its members see it there, everyone sees your usual status everywhere else, and a dot on the server marks it',
    'Server invites arrive as a sealed card pack in the server\'s colours: drag across the top to tear it open and pull a mythical server card (once opened, it stays open)',
    'Block someone from their right-click menu or Settings › Privacy: their DMs, calls, invites, pings and sounds stop reaching you, and they see you offline. Pick whether their messages show as a \'Blocked message\' line or vanish completely',
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
    'A DM with someone you\'ve blocked says so where you\'d type, and Hide completely leaves their reactions out of the counts',
  ],
  message: 'Your servers, your people 🔒',
}
