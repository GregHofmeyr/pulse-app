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
  id: '0.2.13',
  version: '0.2.13',
  date: '17 November 2026',
  new: [
    'Watch parties: hit "Start watch party" on any YouTube video while you\'re in voice and everyone in the call watches in sync. Your volume is yours, and you can leave the party without leaving voice',
    'Plan something: post an invite (📅 or /plan) with Going / Maybe / Can\'t, an end time, Add to calendar, the weather forecast and a reminder an hour before',
    'Custom status: set a line of text and an emoji from the status menu, with a timer to clear it',
    'Send later and Remind me: schedule a message from the clock button, or right-click any message to get reminded about it. Both live in new inbox tabs',
    'Forward a message, see a message\'s edit history, and Save messages to a private Saved list with notes',
    'Deadlock: post-match cards, your rank next to your name, squad stats on your profile card and a patch notes channel (pick it in Server settings › Deadlock)',
    'F1 race weekends: a countdown, results with constructors and sprints, and podium predictions with a season table (Server settings › F1)',
    'Steam: most hours played in the last two weeks on the Leaderboard, and one-click Join game when a friend is in a joinable lobby',
    'Servers: leave a server, remove someone, hand over ownership, see the Members list, and archive old channels so they stay readable',
    'View-once pictures, videos and sounds',
    'Listening to now works with YouTube Music, Apple Music, Tidal, Deezer and foobar2000 too (switches in Settings › Privacy)',
    'New badges: Touch Grass, Achievement Hunter and Podium Prophet. Completionist now has five tiers',
  ],
  fixes: [
    'Removed or departed members can no longer edit their old messages, polls or plans in that server',
    'Leaving a server cleans up your calls, streams, unreads, reminders and saved messages from it',
  ],
  qol: [
    'Pin DMs to the top, sort them into your own categories, and arrange a server\'s channels just for you',
    'Do Not Disturb for one person, and saved inbox filters (people, chats, only unread)',
    'Chats with lots unread open at the first unread message, and the inbox has a Threads tab',
    'Crop, rotate or draw on a picture before sending it',
    'Edit a poll after posting it, and give picker wheel options weights and colour themes',
    'Previews for .txt and .md files',
    'New keybinds: Leave call and Go Live',
    'People\'s current names show in system lines and search results',
  ],
  soon: [
    'Live PSL, URC and Premier League scores',
    'A redesigned quotes leaderboard',
    'A fresh profile editor',
  ],
  message: 'Big night. Grab the popcorn 🍿',
}

/** The release before `patchNotes` (#235), shown collapsed below them; `null` = none. */
export const previousPatchNotes: PatchNotes | null = {
  id: '0.2.12',
  version: '0.2.12',
  date: '16 November 2026',
  new: [
    'Game activity (Windows): Pulse shows what you\'re playing ("Playing Deadlock · 1h 12m") on your profile card, with the game\'s art, and behind your tile in voice calls. Steam games are found on any drive, and you can add other games by hand. Switch it off, or turn off the art or play time, in Settings › Privacy › Game activity',
    'Steam achievements: link your Steam account and your unlocks show up live in your server\'s Achievements channel, with the icon and how rare each one is. Finish a game and Pulse celebrates the 100%. Your completion rate appears on your profile card',
    'New badge: Completionist, for your first 100% game',
    'Auto clip drop: when NVIDIA or Game Bar saves a new recording, Pulse offers to share it to #clips',
  ],
  fixes: [
    'If something breaks while showing a screen, you now get a Reload button instead of a blank window',
    'Noise suppression steps back up on its own after a CPU spike forced it down',
  ],
  qol: [
    'The stream sound picker shows proper app names and a little level meter',
    'Echo cancellation explains itself when it switches off for headphones',
  ],
  soon: [
    'Deadlock: post-match cards, rank badges and squad stats',
    'A fresh profile editor: edit right inside your big profile card',
  ],
  message: 'GG. Pulse knows what you\'re playing now 🎮',
}
