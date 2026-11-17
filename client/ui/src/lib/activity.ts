// "Listening to…" (#275): how someone's music activity reads, and where its link goes. Pure. The server sends it
// only while they share it and look online to you (Invisible, appear-offline-to and offline show none), so whatever
// is in the state may be shown. Other players (#322): a source label and a generic icon, never Spotify's links.
import { isMediaHash, mediaUrl } from './emoji'
import type { AppState } from './state'
import type { Listening } from './protocol/Listening'
import type { MusicPlayer } from './protocol/MusicPlayer'

/** The link-out, in Spotify's approved wording (#196): straight to the song when its id is known (MPRIS gives it),
 *  else to a Spotify search for it (Windows gives no id), which opens Spotify rather than a song. */
export type ListenLink = { href: string; label: 'Listen on Spotify' | 'Open Spotify' }

const ID = /^(track|episode):([A-Za-z0-9]{22})$/

/** Which player it's from: no `player` is Spotify (older servers and clients only knew Spotify). */
export const playerOf = (l: Pick<Listening, 'player'>): MusicPlayer => l.player ?? 'spotify'

export const isSpotify = (l: Pick<Listening, 'player'>): boolean => playerOf(l) === 'spotify'

const NAMES: Record<MusicPlayer, string> = {
  spotify: 'Spotify',
  youtube_music: 'YouTube Music',
  apple_music: 'Apple Music',
  tidal: 'Tidal',
  deezer: 'Deezer',
  foobar2000: 'foobar2000',
  browser: 'a browser',
  other: 'another player',
}

/** The player's name, as a source label ("YouTube Music", "a browser"). */
export const playerName = (p: MusicPlayer): string => NAMES[p] ?? NAMES.other

/** The profile card's heading: "Listening to Spotify" (as before), "Listening on Tidal", "Listening in a browser". */
export function sourceLine(l: Pick<Listening, 'player'>): string {
  const p = playerOf(l)
  if (p === 'spotify') return 'Listening to Spotify'
  if (p === 'browser') return 'Listening in a browser'
  return `Listening on ${playerName(p)}`
}

/** Spotify's link-out; none for another player (#322: no Spotify links for what Spotify didn't play). */
export function listenLink(l: Listening): ListenLink | null {
  if (!isSpotify(l)) return null
  const m = l.spotify_id ? ID.exec(l.spotify_id) : null
  if (m) return { href: `https://open.spotify.com/${m[1]}/${m[2]}`, label: 'Listen on Spotify' }
  const q = [l.title, l.artist].filter(Boolean).join(' ')
  return { href: `https://open.spotify.com/search/${encodeURIComponent(q)}`, label: 'Open Spotify' }
}

/** "Song · Artist" (or just the song). */
export const songLine = (l: Pick<Listening, 'title' | 'artist'>): string => (l.artist ? `${l.title} · ${l.artist}` : l.title)

/** "Listening to Song · Artist". */
export const activityLine = (l: Listening): string => `Listening to ${songLine(l)}`

/** The line with its source, for a tooltip: "Listening to Song · Artist on Tidal" (Spotify's: as the line). */
export function activityTip(l: Listening): string {
  const p = playerOf(l)
  if (p === 'spotify') return activityLine(l)
  return `${activityLine(l)} ${p === 'browser' ? 'in' : 'on'} ${playerName(p)}`
}

/** The album art's URL, when the server found one (only ever Spotify's). */
export const artUrl = (l: Listening): string | null => (l.art && isMediaHash(l.art.hash) ? mediaUrl(l.art.hash) : null)

/** What `userId` is listening to, as the server lets you see it. */
export const listeningOf = (s: AppState, userId: string): Listening | null => s.people[userId]?.listening ?? null
