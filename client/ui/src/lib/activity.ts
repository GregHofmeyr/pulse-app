// "Listening to…" (#275): how someone's Spotify activity reads, and where its link goes. Pure. The server sends it
// only while they share it and look online to you (Invisible, appear-offline-to and offline show none), so whatever
// is in the state may be shown.
import { isMediaHash, mediaUrl } from './emoji'
import type { AppState } from './state'
import type { Listening } from './protocol/Listening'

/** The link-out, in Spotify's approved wording (#196): straight to the song when its id is known (MPRIS gives it),
 *  else to a Spotify search for it (Windows gives no id), which opens Spotify rather than a song. */
export type ListenLink = { href: string; label: 'Listen on Spotify' | 'Open Spotify' }

const ID = /^(track|episode):([A-Za-z0-9]{22})$/

export function listenLink(l: Listening): ListenLink {
  const m = l.spotify_id ? ID.exec(l.spotify_id) : null
  if (m) return { href: `https://open.spotify.com/${m[1]}/${m[2]}`, label: 'Listen on Spotify' }
  const q = [l.title, l.artist].filter(Boolean).join(' ')
  return { href: `https://open.spotify.com/search/${encodeURIComponent(q)}`, label: 'Open Spotify' }
}

/** "Song · Artist" (or just the song). */
export const songLine = (l: Pick<Listening, 'title' | 'artist'>): string => (l.artist ? `${l.title} · ${l.artist}` : l.title)

/** "Listening to Song · Artist". */
export const activityLine = (l: Listening): string => `Listening to ${songLine(l)}`

/** The album art's URL, when the server found one. */
export const artUrl = (l: Listening): string | null => (l.art && isMediaHash(l.art.hash) ? mediaUrl(l.art.hash) : null)

/** What `userId` is listening to, as the server lets you see it. */
export const listeningOf = (s: AppState, userId: string): Listening | null => s.people[userId]?.listening ?? null
