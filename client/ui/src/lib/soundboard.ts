// Pure soundboard helpers (tested in soundboard.test.ts). Local prefs live in localStorage.
import type { Server } from './protocol/Server'
import type { Sound } from './protocol/Sound'
import type { AppState } from './state'

export type Section = {
  id: 'favourites' | 'frequent' | `server:${string}`
  label: string
  sounds: Sound[]
  server?: Server
}

const MUTES_KEY = 'pulse.soundboard.mutes'
const FREQUENT_SHOWN = 9
const TIDY_COUNT = 5

function read(key: string): unknown {
  try {
    const raw = localStorage.getItem(key)
    return raw ? JSON.parse(raw) : null
  } catch {
    return null
  }
}
function write(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value))
  } catch {
    // storage unavailable: the setting just won't survive a restart
  }
}

/** People whose sounds you never hear (user id -> true). */
export function loadSoundMutes(): Record<string, true> {
  const r = read(MUTES_KEY)
  if (typeof r !== 'object' || r === null || Array.isArray(r)) return {}
  return Object.fromEntries(Object.entries(r).filter(([, v]) => v === true).map(([k]) => [k, true as const]))
}
export const saveSoundMutes = (m: Record<string, true>) => write(MUTES_KEY, m)

/** Whether a relayed SoundPlayed should be heard here (`volume_pct` 0: the soundboard is off). Your own plays in your call do play. */
export function shouldPlay(
  e: { channel_id: string; user_id: string },
  ctx: { myChannel: string | null; volume_pct: number; mutes: Record<string, true> },
): boolean {
  return ctx.myChannel !== null && e.channel_id === ctx.myChannel && ctx.volume_pct > 0 && !ctx.mutes[e.user_id]
}

const byName = (a: Sound, b: Sound) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' })

/** Popover sections: favourites (your order), frequent (top 9), then each server you're in.
 * Favourites and frequent leave out servers you've left (their sounds can't be played). */
export function soundSections(s: AppState, memberOf: Set<string>, servers: Server[]): Section[] {
  const pick = (ids: string[]) => ids.map((id) => s.sounds[id]).filter((x): x is Sound => !!x && memberOf.has(x.server_id))
  const out: Section[] = []
  const fav = pick(s.soundFavourites)
  if (fav.length) out.push({ id: 'favourites', label: 'Favourites', sounds: fav })
  const freq = pick(s.frequentSounds).slice(0, FREQUENT_SHOWN)
  if (freq.length) out.push({ id: 'frequent', label: 'Frequently used', sounds: freq })
  const all = Object.values(s.sounds)
  for (const srv of servers) {
    if (!memberOf.has(srv.id)) continue
    out.push({ id: `server:${srv.id}`, label: srv.name, sounds: all.filter((x) => x.server_id === srv.id).sort(byName), server: srv })
  }
  return out
}

/** Media hashes to fetch on joining a call, most likely played first: `first` (join sounds, #121), favourites,
 * frequently used, then the rest of your servers' sounds. One entry per file; sounds of servers you've left are
 * skipped. */
export function warmOrder(s: AppState, memberOf: Set<string>, first: string[] = []): string[] {
  const out: string[] = []
  const seen = new Set<string>()
  const add = (x: Sound | undefined) => {
    if (!x || !memberOf.has(x.server_id) || seen.has(x.media_hash)) return
    seen.add(x.media_hash)
    out.push(x.media_hash)
  }
  for (const id of first) add(s.sounds[id])
  for (const id of s.soundFavourites) add(s.sounds[id])
  for (const id of s.frequentSounds) add(s.sounds[id])
  for (const x of Object.values(s.sounds).sort(byName)) add(x)
  return out
}

/** Case-insensitive name contains; names that start with the query come first. */
export function searchSounds(sounds: Sound[], q: string): Sound[] {
  const needle = q.trim().toLowerCase()
  if (!needle) return sounds
  const hits = sounds.filter((x) => x.name.toLowerCase().includes(needle))
  const starts = (x: Sound) => x.name.toLowerCase().startsWith(needle)
  return [...hits.filter(starts), ...hits.filter((x) => !starts(x))]
}

/** An error's words: a plain string, an Error, or a coded core error's `message`. */
const messageOf = (e: unknown): string =>
  typeof e === 'string'
    ? e
    : e instanceof Error
      ? e.message
      : typeof e === 'object' && e !== null && typeof (e as { message?: unknown }).message === 'string'
        ? (e as { message: string }).message
        : ''

/** A 429: a play that hit the abuse limit is dropped quietly. By the core's stable code (#123); the words
 * (`ApiError::RateLimited`) are still matched for an error that carries no code. */
export const isRateLimited = (e: unknown): boolean =>
  (typeof e === 'object' && e !== null && (e as { code?: unknown }).code === 'rate_limited') ||
  messageOf(e).startsWith('slow down a little')

/** The core's words for a 404: the sound was deleted meanwhile (its SoundDeleted removes the tile). */
export const isNotFound = (e: unknown): boolean => messageOf(e) === 'not found'

/** [Calls, Soundboard] percents a preview plays at. Previews are for yourself: with the soundboard turned off (either
 * at 0) they still play, at full volume. */
export const previewPercents = (calls: number, soundboard: number): [number, number] =>
  calls > 0 && soundboard > 0 ? [calls, soundboard] : [100, 100]

/** A server's section offers "Tidy up?" once it holds more than this many sounds. */
export const TIDY_THRESHOLD = 24

/** The 5 least-played sounds (oldest first on ties): what to offer when a server is full. */
export function tidyCandidates(sounds: Sound[]): Sound[] {
  return [...sounds]
    .sort((a, b) => a.plays - b.plays || a.created_at.localeCompare(b.created_at))
    .slice(0, TIDY_COUNT)
}
