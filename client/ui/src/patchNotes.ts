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
  id: '0.2.21',
  version: '0.2.21',
  date: '27 November 2026',
  new: [
    'Set your own typing text in Profile settings: instead of "is typing…", everyone sees "<you> is cooking", "is plotting" or whatever you like (pick a suggestion or write your own)',
    'Clips channels get a games panel on the right, like Builds: filter clips by game, with counts that follow your other filters',
    '"Hear yourself as others do" in the mic test: plays your voice back after the same compression everyone else hears, so you know exactly how you sound',
    'A "Light" noise suppression level, for mics that already clean up their own sound (Sonar, NVIDIA Broadcast and others)',
    'Voice settings warn you when your mic is clipping, and tell you where to turn it down',
  ],
  fixes: [
    'Voice: deep voices keep their low end (the mic no longer cuts bass twice)',
    'Voice: the start and end of words are no longer clipped when your mic opens and closes',
    'Streams: game and video audio is sent in proper stereo quality',
    'Builds: popping a build out twice no longer leaves the window saying the build is gone',
    'Builds: the pop-out\'s "On top" button is labelled, and hidden where your system can\'t keep windows on top',
    'Builds: big images show a loading state, and the next and previous ones load ahead',
    'Typing: someone typing straight after sending a message shows up again, and switching chats no longer leaves the old typing line behind',
  ],
  qol: [
    'One date picker everywhere: plans, scheduled messages and reminders, and it won\'t let you pick a day in the past',
    'Filter counts in Clips and Builds stay on screen while they update, instead of blinking',
  ],
  soon: [
    'Speaking rings: pick your own style and colour, some that move with your voice',
  ],
  message: 'Typing text and a clearer mic 🎙️\nSpeaking rings next 💫',
}

/** The release before `patchNotes` (#235), shown collapsed below them; `null` = none. */
export const previousPatchNotes: PatchNotes | null = {
  id: '0.2.20',
  version: '0.2.20',
  date: '25 November 2026',
  new: [
    'Builds channels: a new kind of channel for saving game builds and loadouts. Browse them as a gallery or a list, filter by game in the right panel, show or hide archived builds, and archive old builds in bulk by date',
    'Add a build in seconds by pasting a link or an image, or use the full form for screenshots, notes and the game',
    'Every build has its own page with comments, and its video plays right inside Pulse',
    'Star your favourite builds to keep them in your own Favourites row at the top (only you see your stars)',
    'Right-click a build to open, edit, archive or delete it, or Shift+click to select several and archive, delete or favourite them in one go',
    'Pin a build to the side to keep it in view, take a quick peek without leaving where you are, find builds from Ctrl+K with "builds:", or pop one out into its own small window that stays on top',
    'The right panel has a switcher: when members, voice chat, search and a pinned build all want the space, flip between them without closing any',
    'Voice clarity EQ (in Voice settings): cuts rumble and boominess from your mic and adds a little clarity to your voice',
    'Even out voice volumes (in Voice settings): brings quiet and loud people closer to the same volume, with each person\'s own volume still applied on top',
    'Record a mic sample (in Voice settings): saves 15 seconds of your mic, before and after processing, to attach to a bug report',
    '"Hide preview" while you\'re live takes your own tile off your grid (it keeps going for everyone watching); "Show my stream" brings it back',
    'Open your own stream in full view or full screen, like anyone else\'s',
  ],
  fixes: [
    'Voice: loud moments go through a proper limiter instead of crunchy clipping, on your mic and on the voices you hear',
    'Voice: when someone\'s audio briefly drops out, it fades instead of clicking, and deep voices keep more of their warmth',
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
