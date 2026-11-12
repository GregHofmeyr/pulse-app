// Quick actions in the quick switcher (#309): type `>` (or a matching word: they also list under the places). The one
// registry the switcher lists from and runs.
//
// To add an action, add one source to SOURCES below (`settings` is the simplest to copy): a function from the
// context to the actions it offers right now. If `run` needs something new from the app, add it to ActionContext and
// pass it where the shell builds it (`quickContext` in Shell.svelte).

import type { IconName } from '../components/Icon.svelte'
import type { Section } from '../components/Settings.svelte'
import type { Sound } from './protocol/Sound'
import { matchScore, WORD_START } from './quickSwitch'

/** What an action can see and do. The shell provides it. */
export interface ActionContext {
  /** The call you're in (connected or joining), or null. */
  call: { channelId: string; name: string; connected: boolean } | null
  muted: boolean
  deafened: boolean
  /** Voice channels you could join (servers you're in), with their server's name. */
  voiceChannels: { id: string; name: string; server: string }[]
  /** Sounds you can play into a call (servers you're in), favourites first. */
  sounds: Sound[]
  /** Why Go Live is off here (Linux, not connected yet), or null when it isn't. */
  goLiveOff: string | null
  join: (channelId: string) => void
  play: (sound: Sound) => void
  toggleMute: () => void
  toggleDeafen: () => void
  leave: () => void
  goLive: () => void
  openSettings: (section: Section) => void
  /** Open your notes to self (#308), making them; only until they exist (then they're listed as a place). */
  openNotes?: () => void
  /** Open the inbox (#306). */
  openInbox: () => void
}

export interface QuickAction {
  /** Unique across all actions shown at once. */
  id: string
  /** What the row says, e.g. `Join Lounge`. */
  label: string
  /** What sets it apart from its family, when the label starts with a shared verb (`Lounge` for `Join Lounge`): a
   *  plain query matches this, not the verb, so `j` doesn't list every Join under the places. */
  name?: string
  /** Shown faintly after it (the server, the settings page). */
  context?: string
  icon: IconName
  /** Other words it's found by, beside its label. */
  keywords?: string[]
  run: (ctx: ActionContext) => void
}

type Source = (ctx: ActionContext) => QuickAction[]

const SETTINGS: { section: Section; label: string; keywords?: string[] }[] = [
  { section: 'profile', label: 'Profile' },
  { section: 'appearance', label: 'Appearance', keywords: ['theme'] },
  { section: 'voice', label: 'Voice & Audio', keywords: ['microphone', 'mic', 'audio', 'output'] },
  { section: 'keybinds', label: 'Keybinds', keywords: ['shortcuts', 'hotkeys'] },
  { section: 'chat', label: 'Chat' },
  { section: 'notifications', label: 'Notifications' },
  { section: 'privacy', label: 'Privacy' },
  { section: 'advanced', label: 'Advanced' },
]

export const SOURCES: Source[] = [
  // the call you're in: mute, deafen, Go Live, leave
  (ctx) => {
    if (!ctx.call) return []
    const out: QuickAction[] = [
      { id: 'mute', label: ctx.muted ? 'Unmute' : 'Mute', icon: ctx.muted ? 'mic' : 'micOff', keywords: ['microphone', 'mic'], run: (c) => c.toggleMute() },
      { id: 'deafen', label: ctx.deafened ? 'Undeafen' : 'Deafen', icon: ctx.deafened ? 'headphones' : 'headphonesOff', run: (c) => c.toggleDeafen() },
    ]
    if (!ctx.goLiveOff) out.push({ id: 'golive', label: 'Go Live', context: ctx.call.name, icon: 'screen', keywords: ['stream', 'share screen'], run: (c) => c.goLive() })
    out.push({ id: 'leave', label: 'Leave call', context: ctx.call.name, icon: 'hangup', keywords: ['disconnect', 'hang up'], run: (c) => c.leave() })
    return out
  },
  // join any voice channel but the one you're in
  (ctx) =>
    ctx.voiceChannels
      .filter((v) => v.id !== ctx.call?.channelId)
      .map((v) => ({ id: `join:${v.id}`, label: `Join ${v.name}`, name: v.name, context: v.server, icon: 'speaker' as const, keywords: ['voice'], run: (c: ActionContext) => c.join(v.id) })),
  // a soundboard sound, into the call you're in
  (ctx) =>
    !ctx.call
      ? []
      : ctx.sounds.map((s) => ({ id: `play:${s.id}`, label: `Play ${s.name}`, name: s.name, context: 'Soundboard', icon: 'soundboard' as const, keywords: ['sound'], run: (c: ActionContext) => c.play(s) })),
  // your notes to self (#308), before they're a place
  (ctx) => (ctx.openNotes ? [{ id: 'notes', label: 'Notes', icon: 'pencil', keywords: ['notes to self', 'yourself'], run: (c) => c.openNotes?.() }] : []),
  // the inbox (#306): your mentions and replies
  () => [{ id: 'inbox', label: 'Inbox', icon: 'inbox', keywords: ['mentions', 'replies', 'notifications'], run: (c) => c.openInbox() }],
  // settings, and each of its pages
  () => [
    { id: 'settings', label: 'Open Settings', icon: 'gear', keywords: ['preferences', 'options'], run: (c) => c.openSettings('voice') },
    ...SETTINGS.map((p) => ({ id: `settings:${p.section}`, label: `Settings: ${p.label}`, name: p.label, icon: 'gear' as const, keywords: p.keywords, run: (c: ActionContext) => c.openSettings(p.section) })),
  ],
]

/** Every action on offer now, in registry order. */
export function allActions(ctx: ActionContext, sources: readonly Source[] = SOURCES): QuickAction[] {
  return sources.flatMap((s) => s(ctx))
}

/** The actions matching `query` (all of them for an empty one, in registry order), best first, at most `limit`.
 *  `close`: the actions listed under the places, which take only a word starting with the query, and only in what
 *  sets the action apart (its `name`, not the verb every Join or Play row starts with). */
export function matchActions(actions: readonly QuickAction[], query: string, limit = 50, close = false): QuickAction[] {
  if (!query.trim()) return actions.slice(0, limit)
  const min = close ? WORD_START : 1
  const scored: { a: QuickAction; score: number }[] = []
  actions.forEach((a, i) => {
    const own = matchScore(query, close ? (a.name ?? a.label) : a.label)
    // a family's shared words (`voice`, `sound`) would list the whole family under the places: only after `>`
    const keys = close && a.name ? [] : (a.keywords ?? [])
    const score = Math.max(own, ...keys.map((k) => matchScore(query, k) * 0.95))
    // a later entry loses a tie: registry order is the tiebreak
    if (score >= min) scored.push({ a, score: score - i * 1e-6 })
  })
  return scored.sort((x, y) => y.score - x.score).slice(0, limit).map((x) => x.a)
}
