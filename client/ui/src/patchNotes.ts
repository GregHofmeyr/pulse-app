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
  id: '0.2.20',
  version: '0.2.20',
  date: '25 November 2026',
  new: [
    'Builds channels: a new kind of channel for saving game builds and loadouts. Browse them as a gallery or a list, filter by game in the right panel, show or hide archived builds, and archive old builds in bulk by date',
    'Add a build in seconds by pasting a link or an image, or use the full form for screenshots, notes and the game',
    'Every build has its own page with comments, and its video plays right inside Pulse',
    'Pin a build to the side to keep it in view, take a quick peek without leaving where you are, find builds from Ctrl+K with "builds:", or pop one out into its own small window that stays on top',
    'The right panel has a switcher: when members, voice chat, search and a pinned build all want the space, flip between them without closing any',
    'Voice clarity EQ (in Voice settings): cuts rumble and boominess from your mic and adds a little clarity to your voice',
    'Even out voice volumes (in Voice settings): brings quiet and loud people closer to the same volume, with each person\'s own volume still applied on top',
    'Record a mic sample (in Voice settings): saves 15 seconds of your mic, before and after processing, to attach to a bug report',
    '"Hide preview" while you\'re live takes your own tile off your grid (it keeps going for everyone watching); "Show my stream" brings it back',
    'Open your own stream in full view or full screen, like anyone else\'s',
  ],
  fixes: [
    'Voice: loud moments go through a proper limiter instead of crunchy clipping',
    'Voice: noise suppression no longer garbles your first words after a silence',
    'Voice: your mic is sent as mono at a sensible bitrate, for clearer speech',
    'Voice: when your network changes, Pulse rejoins the call straight away, and voices that stalled come back by themselves',
    'Voice: echo cancellation stays on in Auto, whatever you\'re listening through',
    'Voice: steadier audio, and a problem in the mic\'s processing no longer leaves you silent for the rest of the call',
    'Picking a window to stream no longer minimises Pulse',
    'Small stream previews are sharper',
    'Stream tiles show the normal cursor instead of a magnifying glass',
    'Chat: scrolling up to read older messages no longer snaps you back to the bottom',
    'Reconnecting after a network change or a server update is quicker and smoother',
  ],
  qol: [
    'When your mic already cleans up its own sound (SteelSeries Sonar, NVIDIA Broadcast and others), Voice settings suggest a lighter noise suppression',
    'Voice: a little less delay in calls',
    'Bug reports now include the voice settings in use, your audio devices\' formats, system info and a masked summary of your network, plus mic stats in the log, so problems are quicker to pin down',
    'Pulse uses less memory over long sessions',
  ],
  message: 'Builds are here 🛠️\nAnd a clearer mic 🎙️',
}

/** The release before `patchNotes` (#235), shown collapsed below them; `null` = none. */
export const previousPatchNotes: PatchNotes | null = {
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
