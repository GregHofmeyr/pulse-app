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
  id: '0.2.5',
  version: '0.2.5',
  date: '10 November 2026',
  new: [
    'Custom join sounds: right-click a soundboard sound → Set as my join sound. Everyone hears it when you join (yes, all 6 seconds)',
    'Spotify on the stage: share a Jam or song to the voice stage so everyone can hop in. Minimise it to a corner pill or hide it for yourself',
    'Watch a stream properly full screen, with the chat as a strip beside it',
    'Per-channel audio quality: right-click a voice channel → Channel settings… and turn the bitrate up',
    'Strong (low delay) noise suppression in Voice settings: less delay, more CPU. It downloads the first time you pick it',
  ],
  fixes: [
    'Leaving a call while others stay no longer leaves the old connection running in the background',
    "A dodgy connection no longer replays your join sound for everyone mid-call",
    'Spotify Jam links now show a proper Jam card in chat',
  ],
  qol: [
    'The soundboard plays the instant you click, and sounds load about 200× faster',
    'Switching channels and servers, and opening Settings, is snappier',
    "Undo a server mute or deafen for someone who's left voice, from their right-click menu",
    'Voice settings now tells you what echo cancellation Auto decided',
    'Mute someone\'s join sound just for you, from their right-click menu',
    'Link cards lost the coloured stripe down the side',
  ],
  soon: [
    'Streaming in DM and group calls',
    'A fresh profile editor: edit right inside your big profile card',
    'More fresh and reworked designs on the way',
  ],
  message: 'Voice day! Join sounds are live: choose wisely, everyone will hear it 🔊',
}

/** The release before `patchNotes` (#235), shown collapsed below them; `null` = none. */
export const previousPatchNotes: PatchNotes | null = {
  id: '0.2.4',
  version: '0.2.4',
  date: '9 November 2026',
  new: [
    'Wheel results now fold into a little result card: the landed wheel, the winner big, and Spin again, Remove winner and Save as wheel when you hover',
  ],
  fixes: [],
  qol: [],
  soon: [
    'Streaming in DM and group calls',
    'Share a Spotify Jam or song to the voice stage',
    'A fresh profile editor: edit right inside your big profile card',
    'Custom join sounds',
    'Stream fullscreen, with chat as a side strip',
  ],
  message: 'Tiny one. The wheel just wanted to look pretty in the chat 🎡',
}
