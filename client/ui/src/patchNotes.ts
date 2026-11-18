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
  id: '0.2.14',
  version: '0.2.14',
  date: '17 November 2026',
  new: [
    'Invite to game: right-click someone (or hit the 🎮 button in a DM) to send a Join card for the game you\'re in. Only the people in that chat can join, and it works even when your game activity is hidden',
    'Hide what I\'m playing: right-click someone to stop them seeing the game you\'re in (and joining it). They still see you online. Manage the list in Settings › Privacy › Game activity',
    'A redesigned quotes leaderboard that\'s easier to read and counts properly',
  ],
  fixes: [
    'Updates no longer get missed at launch: if Pulse checks quickly, it now installs the new version instead of starting on the old one',
    'Clip drop works on its own now, even with Game activity off (it moved to Settings › Chat)',
    'Very light, dark or grey custom accent colours no longer leave buttons and switches faint',
    'The Sunset profile card stays orange instead of turning brown, and the full profile card matches the quick one',
    'The inbox tabs, delete prompts and dialog buttons no longer spill off the edge of their box or the screen',
    'Watch parties pause properly now: a pause stays paused for everyone instead of playing again, and clicking the video pauses it',
    'Plans no longer guess a far-away place for the weather: a place it can\'t find nearby just shows no forecast',
  ],
  qol: [
    'Edit history is in the message menu too, not only behind "(edited)"',
    'Someone removed from a server three times in two days needs the owner to let them back in (Server settings › Members › right-click › Allow back)',
    'Your achievement and Deadlock cards skip servers that have someone you hide your games from',
    'Every dropdown is a Pulse one now (channel pickers, F1 predictions, Clear after and more): no more plain system menus, and you can type to jump to an option',
    'Right-click the F1 countdown strip to hide it for this weekend or turn it off (bring it back in Settings › Chat)',
  ],
  soon: [
    'Private servers you join by invite',
    'A fresh profile editor',
  ],
  message: 'Game night, your way 🎮',
}

/** The release before `patchNotes` (#235), shown collapsed below them; `null` = none. */
export const previousPatchNotes: PatchNotes | null = {
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
