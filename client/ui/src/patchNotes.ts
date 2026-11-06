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
  id: '0.2.1',
  version: '0.2.1',
  date: '6 November 2026',
  new: [
    "Notification settings now also live in Server settings, and in each channel's bell",
    "Save quote pictures: the download icon on the picture, or Save image in the line's menu",
    'Missed the last patch notes? They\'re now tucked under "Previous update" at the bottom',
  ],
  fixes: [
    'Badge progress in your trophy case now updates every time you open it (Ghost was counting the whole time, promise)',
    'One-of-a-kind badges now say 1 of 1, because they are',
    'On narrower windows, the hover bar on messages no longer covers the line above',
  ],
  qol: [
    'Drag a channel to the top or bottom of the list and it scrolls with you',
    'Smoother motion all round: the stream mini preview glides into its corner, sections fold smoothly, and menus grow from where you clicked',
    "Profile cards don't flash in early anymore, and a member's card sits beside them instead of covering faces",
    'The emoji picker opens quicker',
    'Voice chat tidy-ups: the Chat button shows unreads, and your search results wait for you while the chat is open',
    'Go Live and the Soundboard are now tidy icon buttons (hover for their names)',
  ],
  soon: [
    'More fresh and reworked designs on the way, hopefully with more taste!',
    'Pinned messages',
    'Polls',
    'A random picker wheel 🌈',
    'Streaming in DM and group calls',
  ],
  message: "If you're reading this, Pulse just updated itself. No zip files were harmed 💜\n…and the Doos is a slow burn on purpose. Patience, doos.",
}

/** The release before `patchNotes` (#235), shown collapsed below them; `null` = none. */
export const previousPatchNotes: PatchNotes | null = {
  id: '2026-11-06',
  version: '0.2.0',
  date: '6 November 2026',
  new: [
    'Pulse now updates itself. This is the last time you install by hand 🎉',
    'Text chat inside voice channels: open it from the chat icon on the channel, or the Chat button on the stage',
    "Choose your stream's sound when you go live: just that app, everything except Pulse, or none",
    'Spotify links get proper cards, and Jam invites become a "Join the Jam" card',
    'An Idea tab in Report a bug, for feature requests',
    'Some very special badges just got very special pictures 👀',
    'Hidden badges now leave riddles in your trophy case',
  ],
  fixes: [
    'You can hear everyone on every call again (people could go silent after your first call until you restarted)',
    "Streams no longer go out silent when Pulse couldn't tell which app's sound to share",
    'Spamming a soundboard sound no longer stacks it on top of itself',
    'Names with stacked accent marks stay inside their own row (YOU KNOW WHO YOU ARE)',
  ],
  qol: [
    'The voice stage fills up: tiles grow and rearrange smoothly as people join',
    "See which call you're in from anywhere: bold channel, purple speaker, marked server",
    'Go Live and the Soundboard sit right above your mute buttons, with a clearer soundboard icon',
    'Clearer join and leave sounds, plus a highlight when someone joins',
    'Volume control on clips in the clips wall',
  ],
  soon: [
    'More fresh and reworked designs on the way, hopefully with more taste!',
    'Pinned messages',
    'Polls',
    'A random picker wheel 🌈',
    'Streaming in DM and group calls',
  ],
  message: "Another day, another pile of fixes straight from your reports 💜 Keep them coming, and try the new Idea tab.\n…and yes, the Zalgo thing is real. He comes.",
}
