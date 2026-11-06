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

/** The release before `patchNotes` (#235), shown collapsed below them; `null` = none. */
export const previousPatchNotes: PatchNotes | null = {
  id: '2026-11-05',
  version: '0.1.0',
  date: '5 November 2026',
  new: [
    'Channel categories: group text and voice channels, and drag channels and categories into any order',
    'Notification settings for each server and channel: All messages, Only @mentions or Nothing',
    "Download clips, from the clip's menu or the icon on the video",
    'Patch notes, right here 👋',
  ],
  fixes: [
    'Dragging the crop circle on a new avatar moved it the wrong way',
    'Menus now close when you open another one or click away',
    'Typing fast no longer sends your first letter to the end',
    'Numbers in the red unread badges sit dead centre',
    'Quote lines with nothing to do no longer open an empty menu',
    'Custom emoji show properly in the DM list preview',
  ],
  qol: [
    "Right-click someone's name in a DM or group for their menu",
    'Add people straight from Settings → Privacy → Appear offline to',
    'Mute times open as a hover list in DM menus',
    'The Combo badge shows clear progress toward your next tier',
    'Clicking a channel or DM puts you straight in the message box',
  ],
  soon: [],
  message: "Thanks for poking at everything and sending in reports, it's already made Pulse so much better 💜\n…thanks for testing, doos.",
}
