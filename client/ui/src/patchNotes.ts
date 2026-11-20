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

/** The release before `patchNotes` (#235), shown collapsed below them; `null` = none. */
export const previousPatchNotes: PatchNotes | null = {
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
    'A stream whose streamer is still connected but sending nothing now ends after a minute, instead of staying live on a black screen',
    'Updates: a slow update check at launch no longer gets dropped (the update is offered once it\'s found), a failed check tries again within minutes instead of half an hour, and a dropped download tries again by itself',
    'With animations turned off (Windows "Animation effects"), a server invite still arrives as a sealed pack: click it, press Enter or give it a short drag and the card is just there, with no animation',
  ],
  qol: [
    'In a full profile\'s Name Archive, click who made a change to see their profile, or right-click them for their menu',
    'A watch party opened in a window moves on with the party to its next video, and closes when you leave the party',
    'Making a poll anonymous in a DM or a small group now warns that people can still work out a vote from the counts changing',
    'Linux: you show as Idle after 30 minutes away on Hyprland, sway and other wlroots desktops too',
    'In a call, the title bar says "Update ready" when a new version is waiting: it applies when you restart Pulse, never mid-call, and "Restart to update" comes back when the call ends',
  ],
  message: 'Polish 🧽',
}
