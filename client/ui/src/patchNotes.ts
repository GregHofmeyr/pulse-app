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
  id: '0.2.11',
  version: '0.2.11',
  date: '16 November 2026',
  new: [
    'Idle status: after 30 minutes away from your PC (Windows), you show an amber moon until you\'re back',
    'Invisible can end on a timer, just like Do Not Disturb',
    'Spoilers: mark images and files as spoilers before sending, and they stay blurred until someone clicks',
    'Drag your servers into your own order in the server rail (or Alt+Left/Right), on every device',
    'Give someone their own message sound: right-click them and pick Message sound',
    '"Show in folder" after you download a clip, image or file',
    'Share a friend\'s song to the stage straight from their Listening to',
    'Removed a link preview by accident? "Show previews again" is in your message\'s menu',
    'A new badge for quotes from the small hours: 3 am Thoughts',
  ],
  fixes: [
    'Watching a stream in another channel no longer silently ends your own stream: Pulse asks first (and the same for going live while you watch elsewhere)',
    '60 fps streams really stream at 60 now (they were capped at 30)',
    'No mic, or mic blocked? You can still join voice and listen; a No mic chip shows and Pulse picks the mic up once it works',
    'When a call ends on its own, Pulse says why: another device, lost connection (with Rejoin), removed, or channel deleted',
    'Muting from someone\'s menu in a DM no longer mutes the wrong conversation',
    'A caption that looked like a file name no longer disappears, and (edited) shows on file-only messages',
    'Quick switcher searches no longer run out of steam after a few tries',
    'Stopping a stream ends the video straight away',
    'YouTube videos that refuse to play inside Pulse open in a window instead',
  ],
  qol: [
    'Drop several folders at once and Pulse offers to zip them all',
    'Big uploads go faster, and a stalled part retries with a fresh link',
    'Ghost watches come back on their own after a server hiccup',
    'Screen readers announce channel and category moves',
    'Ghost and Early Bird badges are fairer: quick reconnects don\'t count, and Early Bird needs someone joining you later',
  ],
  soon: [
    'A fresh profile editor: edit right inside your big profile card',
    'More fresh and reworked designs on the way',
  ],
  message: 'Less waiting, more knowing. Pulse tells you what\'s going on now 🌙',
}

/** The release before `patchNotes` (#235), shown collapsed below them; `null` = none. */
export const previousPatchNotes: PatchNotes | null = {
  id: '0.2.10',
  version: '0.2.10',
  date: '13 November 2026',
  new: [
    'Profile cards in your own colours: light colours finally look like themselves, text switches dark or light to stay readable, and there\'s a Card glow switch in Settings › Profile',
    'Pin quotes and clips: right-click a quote or use a clip\'s ⋯ menu, and find them under Pins in the channel header',
    'The quick switcher searches messages everywhere you are, not just the server you\'re in',
    'Downloads show progress: clips, images and files get a progress ring, you can cancel, and they say Saved when done',
  ],
  fixes: [
    'Coming back after a busy spell opens where you stopped reading, with nothing silently skipped',
    'Reactions, edits and deletes from while you were offline now show on older messages too',
    'The delete confirm on the bottom message or quote no longer hides under the message box',
    'Settings no longer flashes while you pick colours (Linux)',
    'Server names in the quick switcher line up neatly',
    'Unread counts in the Inbox stay right after deletes, mutes and reconnects',
  ],
  qol: [
    'Right-click a person almost anywhere for their menu: group members, @mentions, reply names, quote speakers, search results and the Inbox. Remove from group lives there now too',
    'Small profile cards are a little bigger',
    'A message you jump to now gets a clearer highlight',
    'Clips and clip replies in the Inbox open the clip',
    'The Inbox count catches up on its own when a timed mute runs out',
  ],
  soon: [
    'A fresh profile editor: edit right inside your big profile card',
    'More fresh and reworked designs on the way',
  ],
  message: 'Pin it, find it, download it. Pulse has a filing cabinet now 🗂️',
}
