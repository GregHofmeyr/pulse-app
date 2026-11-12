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
  id: '0.2.9',
  version: '0.2.9',
  date: '12 November 2026',
  new: [
    'Inbox: every @mention and reply to you, across every server and DM, in one place (top bar). All and Mentions tabs, click to jump',
    'Listening to: show what you\'re playing on Spotify on your profile, and a Jam on the stage shows the current song and art. Off until you turn it on in Settings › Privacy',
    'Notes to self: a private chat with just you, pinned at the top of your DMs',
    'Add to quotes from any message\'s menu',
  ],
  fixes: [],
  qol: [
    'Jumped away to read a reply? "Back to where you were" takes you back before jumping to the present',
    'The quick switcher now searches messages too',
    'Bind a global key to open the quick switcher from any app (Settings › Keybinds)',
  ],
  soon: [
    'A fresh profile editor: edit right inside your big profile card',
    'More fresh and reworked designs on the way',
  ],
  message: 'Your inbox is now in Pulse. Sorry 📬',
}

/** The release before `patchNotes` (#235), shown collapsed below them; `null` = none. */
export const previousPatchNotes: PatchNotes | null = {
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
