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
  id: '0.2.8',
  version: '0.2.8',
  date: '11 November 2026',
  new: [
    'Quick switcher: press Ctrl+K (or / when you\'re not typing) to jump anywhere: servers, channels, DMs, groups. Type > for quick actions like joining voice, playing a sound or muting',
  ],
  fixes: [
    'Images in chat actually show up now 😅',
  ],
  qol: [],
  soon: [
    'A fresh profile editor: edit right inside your big profile card',
    'More fresh and reworked designs on the way',
  ],
  message: 'Ctrl+K. You\'re welcome.',
}

/** The release before `patchNotes` (#235), shown collapsed below them; `null` = none. */
export const previousPatchNotes: PatchNotes | null = {
  id: '0.2.7',
  version: '0.2.7',
  date: '11 November 2026',
  new: [
    'Images and files: paste, drop or 📎. Screenshots shrink automatically, there\'s a full-size viewer, and video and audio play inline',
    'Big files up to 2 GB as temporary sends (1 or 7 days). Drop a folder and Pulse zips it for you',
    'GIFs: search, trending and favourites, powered by GIPHY',
    'Delete a server (owner only, with 7 days to change your mind)',
  ],
  fixes: [
    'The chat stays at the bottom when link cards or images load',
    'Mark as read now clears mentions in quote and clip replies',
    'YouTube cards come back after playing, and a ✕ closes the player',
    'Reordering the sidebar at the same time as someone else no longer jumps twice',
  ],
  qol: [
    'Spotify on the stage: the pill grows into the card, a "Share to the stage?" nudge when you paste a link, and shares post the link so everyone can join',
    'See your own stream in DM calls, with a LIVE mark in your DM list',
    '"Spotify" shows on a voice channel while something\'s on its stage',
  ],
  soon: [
    'A fresh profile editor: edit right inside your big profile card',
    'More fresh and reworked designs on the way',
  ],
  message: 'Send memes properly at last. Screenshots, GIFs, the lot 📎',
}
