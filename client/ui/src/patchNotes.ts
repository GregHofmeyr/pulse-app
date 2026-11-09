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
  id: '0.2.3',
  version: '0.2.3',
  date: '9 November 2026',
  new: [
    'Spin quietly: everyone watches the wheel spin live, but nothing is saved in the chat',
    'Just me wheels are now private. Only you see their spins',
    'A chime when the wheel lands, so you know even if you tabbed out (volume mixer › Picker wheel result)',
  ],
  fixes: [
    'The tick on poll options sits in the middle of its box now',
  ],
  qol: [
    'The result now shows big over the wheel, and the card waits for you to see it before folding away (✕ to dismiss it sooner)',
    'Starting a spin takes you to the bottom of the chat so you can watch it',
    'Wheel options can differ by case only: PIZZA and pizza are two different options 👀',
    'Polls allow the same option twice (we see you)',
    'A tidier wheel and poll composer: options first, a + for new wheels, more breathing room',
  ],
  soon: [
    'Streaming in DM and group calls',
    'Share a Spotify Jam or song to the voice stage',
    'A fresh profile editor: edit right inside your big profile card',
    'Custom join sounds',
    'Stream fullscreen, with chat as a side strip',
  ],
  message: 'A quick one: the wheel got some love from our resident UI/UX designer 💜',
}

/** The release before `patchNotes` (#235), shown collapsed below them; `null` = none. */
export const previousPatchNotes: PatchNotes | null = {
  id: '0.2.2',
  version: '0.2.2',
  date: '9 November 2026',
  new: [
    'Pin messages for an hour, a day, a week or forever. The 📌 in the channel header lists them all, with Jump',
    'The picker wheel 🎡: spin together and land on the same slice, save Server or Just me wheels, or type /wheel pizza, sushi, braai',
    "Polls 📊: multiple choice, anonymous, and an end time. See who voted for what, unless it's anonymous",
    'Type / in the message box for commands, and press Tab to finish one',
  ],
  fixes: [
    'Clips replay instantly after pausing or finishing, with no reloading',
    'Menus on Linux no longer flash when they open',
    'Escape and clicking away now close the suggestion lists and wheel edits sensibly instead of the whole window',
  ],
  qol: [
    'A double ding when you talk while muted. Turn it down or off in the volume mixer',
    'The soundboard is now properly mashable 🔨',
    'Quotes get a 🏆 award button on each line, and Copy quote copies the whole thing',
    'The bell on a muted channel lets you change notifications without unmuting',
    "Escape in Settings › Wheels only closes what you're editing, and asks before throwing away changes",
  ],
  soon: [
    'Streaming in DM and group calls',
    'Share a Spotify Jam or song to the voice stage',
    'A fresh profile editor: edit right inside your big profile card',
    'Custom join sounds',
    'Stream fullscreen, with chat as a side strip',
  ],
  message: 'Pins, polls AND a wheel? The Coming soon list got absolutely cooked 🌈\nSpin responsibly.',
}
