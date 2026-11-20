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
  id: '0.2.19',
  version: '0.2.19',
  date: '20 November 2026',
  new: [],
  fixes: [
    'Updates: if an update doesn\'t install while Pulse is left alone, it waits a while before trying again instead of downloading it every few minutes',
    'Updates: waking your computer from sleep no longer counts as having left Pulse alone, so an update doesn\'t start the moment you\'re back',
    'Updates: if an update doesn\'t install, your clips carry on as before without a restart',
    'Very old versions of Pulse are only asked to update when there\'s an update for them to install',
  ],
  qol: [
    'Updates never install by themselves while you\'re playing a game or something is full screen, or over a message edit you haven\'t saved',
    'A message that failed to send no longer holds updates back (it\'s still there after the update)',
  ],
  message: 'Update fixes 🔧',
}

/** The release before `patchNotes` (#235), shown collapsed below them; `null` = none. */
export const previousPatchNotes: PatchNotes | null = {
  id: '0.2.18',
  version: '0.2.18',
  date: '20 November 2026',
  new: [
    'Updates install by themselves: when one is waiting and you\'ve left Pulse alone for 10 minutes, it updates and reopens. Never during a call, a stream, an upload or a message you haven\'t sent',
    'Signed out, the login screen says when an update is available (signing in gets it)',
  ],
  fixes: [
    'Updates: a server hiccup during an update check no longer hides an update that was already found, and the check tries again',
    'Updates: if an update doesn\'t install, Pulse says so with a link to download it, and stops downloading it again at every launch',
    'Very old versions of Pulse are now asked to update when they connect, so nobody gets stuck on one',
  ],
  qol: [
    'Pulse starts faster: it waits less for the update check, and shows its logo instead of a blank window while it gets going',
    'Old update installers no longer pile up in your temporary files',
  ],
  message: 'Smoother updates 🔄',
}
