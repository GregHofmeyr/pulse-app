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
  id: '0.2.15',
  version: '0.2.15',
  date: '18 November 2026',
  new: [
    'Check for updates yourself in Settings › Advanced › Updates',
    'Invite people to a server with a link, or straight from their right-click menu',
    'Invites show as cards in chat, with a Join button',
    'Server owners can ban someone from their right-click menu, and see who is banned in Server settings › Bans',
    'A new profile card theme, Dusk: purple to deep rose',
    'Invite links work for friends without Pulse too: the link opens a page with Open in Pulse and a Windows download, and a server owner\'s link signs them up straight into the server',
    'A fresh profile editor: tabs for About you, Card, Name style and Badges, with your card always in view. Hover your avatar or banner on it to change them, and drag your badges into the order you like',
    'Name effects: give your name Neon, Toon, Pop, Gummy, Prism, Shimmer, Glitch or Ember in Settings › Profile. They move when someone hovers your name or opens your profile',
    'A queue for watch parties: anyone in the call can paste or drop in YouTube links from the Queue button, and the party moves on to the next video for everyone. Right-click a queued video to play it now, move it or remove it',
    'Twitch in watch parties: VODs stay in sync like YouTube, live channels play and pause together until someone skips, and clips start together. Twitch cards for VODs and channels play in chat too',
  ],
  fixes: [
    'Restoring a deleted channel now tells you when the server is at its channel limit, and deleted channels no longer count toward it',
    'Do Not Disturb for someone now also covers their calls (no ringing) and their going live (no notification)',
  ],
  qol: [
    'Drag a conversation onto one of your categories, Pinned or the main list to move it there',
    'Signing someone out of everything takes effect within seconds, even if their app is open',
    'The Sunset card theme\'s orange is brighter, and secondary text on dark cards is a little lighter',
    'A tidier message box: attach files, wheels, polls, plans, game invites and Send later all live in the + menu',
    'Emoji and GIFs share one picker with tabs, and it opens on the one you used last',
    'Go Live\'s sound picker is a proper dropdown that shows the app by name and icon, with a live level meter',
    'Streaming "everything" leaves out Discord as well as Pulse, so a call there isn\'t carried into your stream',
    'Right-click a pinned message or an incoming call for that person\'s menu',
  ],
  soon: [
    'Private servers you join by invite',
  ],
  message: 'More to come ✨',
}

/** The release before `patchNotes` (#235), shown collapsed below them; `null` = none. */
export const previousPatchNotes: PatchNotes | null = {
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
