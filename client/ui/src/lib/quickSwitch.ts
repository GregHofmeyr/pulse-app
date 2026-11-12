// The quick switcher (#309): Ctrl+K anywhere, or `/` with no text field focused, opens a launcher to jump to any
// server, channel, DM or group, and (with `>`) run a quick action (lib/quickActions.ts). The pure parts live here:
// when a key opens it, the places it lists, how a query ranks them, and the recents (per device).
import type { AppState } from './state'
import { conversationName } from './conversations'
import { displayName, namesOf } from './selectors'
import { matchActions, type QuickAction } from './quickActions'
import type { SearchHit } from './protocol/SearchHit'

export type PlaceKind = 'server' | 'text' | 'voice' | 'quotes' | 'clips' | 'dm' | 'group'

/** Somewhere the switcher can take you. `key` is `s:<server id>` or `c:<channel id>` (what recents remember). */
export interface Place {
  key: string
  kind: PlaceKind
  /** The server's id, or the channel's. */
  id: string
  name: string
  /** The server a channel is in (shown beside it); null for servers, DMs and groups. */
  serverId: string | null
  context: string | null
  /** Lower-cased names it's found by, the shown name first (a DM: every name you have for the other person). */
  keys: string[]
}

// ---- opening ----

/** Where typing goes: a `/` there is a character (or the composer's slash commands, #262), never the switcher. A menu
 *  or list keeps its own keys too. */
const TYPING = 'input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="textbox"], [role="combobox"], [role="menu"], [role="listbox"], [role="slider"]'

export function isTyping(target: EventTarget | null): boolean {
  const el = target as Element | null
  if (!el || typeof el.closest !== 'function') return false
  const field = el.closest(TYPING)
  if (!field) return false
  // a range, checkbox or button input takes no text
  return !(field instanceof HTMLInputElement && ['range', 'checkbox', 'radio', 'button', 'submit', 'reset', 'color', 'file'].includes(field.type))
}

type KeyLike = Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey' | 'repeat' | 'defaultPrevented' | 'target'>

/** What a key press anywhere in the app does to the switcher: Ctrl+K (Cmd+K) toggles it from anywhere, even a text
 *  field; `/` opens it only when nothing that takes text has focus, and nothing else claimed the key. `open`: it's
 *  showing (a `/` then is just typed into its own field); `blocked`: another dialog is open (it keeps the keys). */
export function switcherKey(e: KeyLike, s: { open: boolean; blocked: boolean }): 'toggle' | 'open' | null {
  if (e.repeat) return null
  const mod = e.ctrlKey || e.metaKey
  if (mod && !e.altKey && !e.shiftKey && (e.key.toLowerCase() === 'k' || e.code === 'KeyK')) return s.blocked && !s.open ? null : 'toggle'
  if (e.key === '/' && !mod && !e.altKey && !s.open && !s.blocked && !e.defaultPrevented && !isTyping(e.target)) return 'open'
  return null
}

/** How close an in-app toggle and the global keybind's event are to be one press. */
export const SAME_PRESS_MS = 500

/** The global keybind fired (#319; the window is already brought up): whether the switcher shows next. Pressed again
 *  while Pulse was in front with it open, it closes, as a launcher's key does; from another app it opens (or stays
 *  open, now in front). Another dialog open (`blocked`) keeps its place. Bound to the in-app keys themselves (Ctrl+K),
 *  one press reaches both: with Pulse in front and an in-app toggle just now (`sinceLocalMs`), that toggle stands. */
export function globalSwitch(s: { open: boolean; wasFocused: boolean; blocked: boolean; sinceLocalMs?: number }): boolean {
  if (s.wasFocused && (s.sinceLocalMs ?? Infinity) < SAME_PRESS_MS) return s.open
  if (s.open) return !s.wasFocused
  return !s.blocked
}

// ---- places ----

const lower = (xs: (string | null | undefined)[]) => [...new Set(xs.filter((x): x is string => !!x).map((x) => x.toLowerCase()))]
const CHANNEL_KINDS: PlaceKind[] = ['text', 'voice', 'quotes', 'clips']
const KIND_ORDER: Record<PlaceKind, number> = { server: 0, text: 1, voice: 2, quotes: 3, clips: 4, dm: 5, group: 6 }

/** Everything the switcher can take you to: servers, their channels (text, voice, quotes, clips), and your DMs and
 *  groups (not the ones you closed: they're out of your list too). */
export function placesFrom(s: AppState): Place[] {
  const out: Place[] = []
  const serverName = new Map(s.servers.map((x) => [x.id, x.name]))
  for (const srv of s.servers) out.push({ key: `s:${srv.id}`, kind: 'server', id: srv.id, name: srv.name, serverId: null, context: null, keys: lower([srv.name]) })
  const me = s.me?.id
  for (const ch of Object.values(s.channels)) {
    if (ch.server_id) {
      const sn = serverName.get(ch.server_id)
      if (sn === undefined || !CHANNEL_KINDS.includes(ch.kind)) continue
      const name = ch.name ?? ''
      out.push({ key: `c:${ch.id}`, kind: ch.kind, id: ch.id, name, serverId: ch.server_id, context: sn, keys: lower([name]) })
    } else if ((ch.kind === 'dm' || ch.kind === 'group') && !s.hidden[ch.id]) {
      const others = (s.dmMembers[ch.id] ?? []).filter((u) => u !== me)
      const names = others.flatMap((u) => [displayName(s, null, u), ...namesOf(s, null, u), s.people[u]?.user.username])
      out.push({ key: `c:${ch.id}`, kind: ch.kind, id: ch.id, name: conversationName(s, ch.id), serverId: null, context: ch.kind === 'group' ? 'Group' : null, keys: lower([conversationName(s, ch.id), ...names]) })
    }
  }
  return out
}

// ---- matching ----

const norm = (x: string) => x.toLowerCase().trim().replace(/\s+/g, ' ')
const WORD = /[\s\-_.,:/#()[\]]+/
/** The lowest score of a query starting one of a name's words (matchScore's third band, at its loosest). */
export const WORD_START = 700

/** How well `query` matches `text` (both any case): 0 for no match. Bands, best first: the whole name, a prefix of
 *  it, the start of one of its words (or each query word starting a word, in order), anywhere inside it, then the
 *  letters in order (fuzzy). Within a band a shorter name wins. */
export function matchScore(query: string, text: string): number {
  const q = norm(query)
  const t = norm(text)
  if (!q || !t) return 0
  const tight = (base: number) => base - Math.min(40, t.length - q.length) // a shorter name ranks first
  if (t === q) return 1000
  if (t.startsWith(q)) return tight(900)
  const words = t.split(WORD).filter(Boolean)
  if (words.some((w) => w.startsWith(q))) return tight(WORD_START + 40)
  const qWords = q.split(WORD).filter(Boolean)
  if (qWords.length > 1) {
    let at = 0
    const inOrder = qWords.every((qw) => {
      while (at < words.length && !words[at].startsWith(qw)) at++
      return at++ < words.length
    })
    if (inOrder) return tight(700)
  }
  if (t.includes(q)) return tight(550)
  return fuzzy(q.replace(/ /g, ''), t)
}

/** The letters of `q` in order in `t`: 100 to ~350, more for runs and letters that start a word. */
function fuzzy(q: string, t: string): number {
  let score = 100
  let ti = 0
  let prev = -2
  for (const ch of q) {
    const i = t.indexOf(ch, ti)
    if (i < 0) return 0
    if (i === prev + 1) score += 12
    if (i === 0 || WORD.test(t[i - 1])) score += 10
    score -= Math.min(6, i - ti) // gaps cost a little
    prev = i
    ti = i + 1
  }
  return Math.max(1, Math.min(399, score))
}

/** Recent places get a small lift (the latest most), never past a better band. */
export const RECENT_BOOST = 80

/** The places matching `query`, best first: the best of their names (other names count a little less), lifted when
 *  recent, then servers before channels before conversations, then by name. */
export function rankPlaces(places: readonly Place[], query: string, recents: readonly string[] = [], limit = 50): Place[] {
  const recentAt = new Map(recents.map((k, i) => [k, i]))
  const scored: { p: Place; score: number }[] = []
  for (const p of places) {
    let best = 0
    for (let i = 0; i < p.keys.length; i++) {
      const sc = matchScore(query, p.keys[i]) * (i === 0 ? 1 : 0.95)
      if (sc > best) best = sc
    }
    if (best <= 0) continue
    const r = recentAt.get(p.key)
    if (r !== undefined) best += Math.max(10, RECENT_BOOST - r * 8)
    scored.push({ p, score: best })
  }
  scored.sort((a, b) => b.score - a.score || KIND_ORDER[a.p.kind] - KIND_ORDER[b.p.kind] || a.p.name.localeCompare(b.p.name))
  return scored.slice(0, limit).map((x) => x.p)
}

// ---- recents (per device) ----

export const RECENTS_KEY = 'pulse.quickSwitch.recents'
/** How many recents are kept; the switcher shows the first RECENTS_SHOWN that still exist. */
const RECENTS_KEPT = 20
export const RECENTS_SHOWN = 8

export function loadRecents(): string[] {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(RECENTS_KEY) ?? '[]')
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string').slice(0, RECENTS_KEPT) : []
  } catch {
    return []
  }
}

/** `key` visited: first in the list, once. */
export function pushRecent(list: readonly string[], key: string): string[] {
  return [key, ...list.filter((k) => k !== key)].slice(0, RECENTS_KEPT)
}

export function saveRecents(list: readonly string[]): void {
  try {
    localStorage.setItem(RECENTS_KEY, JSON.stringify(list))
  } catch {
    /* recents are a convenience */
  }
}

/** The recents to list with nothing typed: the ones that still exist, newest first, without where you are now. */
export function recentPlaces(places: readonly Place[], recents: readonly string[], here: string | null): Place[] {
  const byKey = new Map(places.map((p) => [p.key, p]))
  return recents.filter((k) => k !== here).flatMap((k) => byKey.get(k) ?? []).slice(0, RECENTS_SHOWN)
}

/** What's typed, split: `>` first means actions only. */
export function parseQuery(raw: string): { actions: boolean; text: string } {
  const t = raw.trimStart()
  return t.startsWith('>') ? { actions: true, text: t.slice(1).trim() } : { actions: false, text: raw.trim() }
}

// ---- results ----

/** A message hit comes only from the slower Messages section (lib/quickMessages.ts, #320). */
export type Row = { type: 'place'; place: Place } | { type: 'action'; action: QuickAction } | { type: 'message'; hit: SearchHit }
/** A titled run of results. The switcher draws whatever sections it's given, in order, as one list: the instant ones
 *  come from here, and a slower source (message search, lib/quickMessages.ts) appends its own section once it
 *  answers, so typing never waits on it. */
export interface ResultSection {
  id: string
  label: string
  rows: Row[]
}

/** Actions listed under the places when a plain query matches them (all of them after `>`). */
export const ACTIONS_UNDER_PLACES = 5

/** What the switcher lists for `raw`: nothing typed, the recents (minus where you are, `here`); `>…`, the actions
 *  only; anything else, the matching places then the matching actions. Empty sections are left out. */
export function buildResults(o: { places: readonly Place[]; actions: readonly QuickAction[]; raw: string; recents: readonly string[]; here: string | null }): ResultSection[] {
  const q = parseQuery(o.raw)
  const place = (p: Place): Row => ({ type: 'place', place: p })
  const action = (a: QuickAction): Row => ({ type: 'action', action: a })
  let sections: ResultSection[]
  if (q.actions) sections = [{ id: 'actions', label: 'Actions', rows: matchActions(o.actions, q.text).map(action) }]
  else if (!q.text) sections = [{ id: 'recent', label: 'Recent', rows: recentPlaces(o.places, o.recents, o.here).map(place) }]
  else {
    const found = rankPlaces(o.places, q.text, o.recents)
    // a voice channel listed above already offers its Join on its row
    const listed = new Set(found.map((p) => `join:${p.id}`))
    sections = [
      { id: 'places', label: 'Jump to', rows: found.map(place) },
      { id: 'actions', label: 'Actions', rows: matchActions(o.actions.filter((a) => !listed.has(a.id)), q.text, ACTIONS_UNDER_PLACES, true).map(action) },
    ]
  }
  return sections.filter((x) => x.rows.length > 0)
}
