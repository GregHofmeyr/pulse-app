// The patch notes shown from the title bar (#207). Swap this whole object for each release: a new `id` brings the
// dot back on everyone's icon until they open the notes once. Empty sections are left out of the dialog.
// The repository is public, so no friends' names (or anything else private) go in here.

export type PatchNotes = {
  /** Unique per release; what "seen on this device" is remembered by. */
  id: string
  /** The release date as people read it. */
  date: string
  new: string[]
  fixes: string[]
  qol: string[]
  /** The sign-off at the bottom; may hold a line break. */
  message: string
}

export const patchNotes: PatchNotes = {
  id: '2026-11-05',
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
  message: "Thanks for poking at everything and sending in reports, it's already made Pulse so much better 💜\n…thanks for testing, doos.",
}
