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
    'Servers are now private: you only see servers you\'re in, and you join with an invite',
  ],
  fixes: [
    'Restoring a deleted channel now tells you when the server is at its channel limit, and deleted channels no longer count toward it',
  ],
  qol: [
    'Signing someone out of everything takes effect within seconds, even if their app is open',
  ],
  soon: [
    'A fresh profile editor with name effects',
    'A queue for watch parties, with Twitch',
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
