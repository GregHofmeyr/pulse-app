// YouTube watch parties in voice (#384): the pure parts, tested in watchParty.test.ts. The server holds where the
// video was (`position_ms`) at `set_at_ms` by its clock, and whether it's playing; each viewer's player follows that,
// correcting itself when it drifts too far.
import type { LinkEmbed } from './protocol/LinkEmbed'
import type { WatchParty } from './protocol/WatchParty'
import type { WatchPartyItem } from './protocol/WatchPartyItem'
import type { WatchSource } from './protocol/WatchSource'

/** How far (seconds) a player may drift from the party before it's seeked back. */
export const DRIFT_S = 1.5

/** The furthest a seek goes (48 hours, the server's cap), in seconds. */
export const MAX_SECONDS = 48 * 3600

/** A YouTube video id: exactly 11 of `[A-Za-z0-9_-]` (the server checks the same). */
export const validVideoId = (id: string | null | undefined): id is string => !!id && /^[A-Za-z0-9_-]{11}$/.test(id)

/** What a party can play (#459): a source and its id there. */
export type PartyMedia = { source: WatchSource; id: string }

/** Whether `id` is well formed for `source` (the server checks the same: `WatchSource::valid_id`). */
export function validMedia(source: WatchSource, id: string | null | undefined): id is string {
  if (!id) return false
  switch (source) {
    case 'youtube': return validVideoId(id)
    case 'twitch_video': return /^[0-9]{1,15}$/.test(id)
    case 'twitch_channel': return /^[a-z0-9_]{1,25}$/.test(id)
    case 'twitch_clip': return /^[A-Za-z0-9_-]{1,100}$/.test(id)
  }
}

const media = (source: WatchSource, id: string | null | undefined): PartyMedia | null => (validMedia(source, id) ? { source, id } : null)

/** What a card plays in a party: a YouTube video, or a Twitch VOD (`video:<id>`), channel (`channel:<login>`) or clip
 *  (a bare slug), as the server's link previews name them. */
export function partyMediaOf(e: LinkEmbed): PartyMedia | null {
  const id = e.provider_id
  if (!id) return null
  if (e.provider === 'youtube') return media('youtube', id)
  if (e.provider !== 'twitch') return null
  if (id.startsWith('video:')) return media('twitch_video', id.slice(6))
  if (id.startsWith('channel:')) return media('twitch_channel', id.slice(8))
  return media('twitch_clip', id)
}

/** A party video's id as its link preview and the embed page name it (`/embed/{provider}/{id}`). */
export function embedOf(m: PartyMedia): { provider: 'youtube' | 'twitch'; id: string } {
  switch (m.source) {
    case 'youtube': return { provider: 'youtube', id: m.id }
    case 'twitch_video': return { provider: 'twitch', id: `video:${m.id}` }
    case 'twitch_channel': return { provider: 'twitch', id: `channel:${m.id}` }
    case 'twitch_clip': return { provider: 'twitch', id: m.id }
  }
}

/** A source's badge: YouTube, Twitch (a VOD), Live (a channel) or Clip. */
export const sourceLabel = (s: WatchSource | undefined): string =>
  s === 'twitch_video' ? 'Twitch' : s === 'twitch_channel' ? 'Live' : s === 'twitch_clip' ? 'Clip' : 'YouTube'

/** What a party video's title reads as when it has none. */
export const untitled = (s: WatchSource | undefined): string =>
  s === 'twitch_video' ? 'A Twitch video' : s === 'twitch_channel' ? 'A Twitch channel' : s === 'twitch_clip' ? 'A Twitch clip' : 'A YouTube video'

/** Twitch clips are at most a minute long and say nothing back (no control API): the party moves on from one this
 *  long after it started. */
export const CLIP_S = 60

/** Where the party's video is (seconds) at `serverNowMs`: it runs on while playing. */
export function partySeconds(p: Pick<WatchParty, 'position_ms' | 'playing' | 'set_at_ms'>, serverNowMs: number): number {
  const ran = p.playing ? Math.max(0, serverNowMs - p.set_at_ms) : 0
  return Math.min(MAX_SECONDS, (p.position_ms + ran) / 1000)
}

/** Whether `me` may play, pause, seek and change the video (the host; anyone watching while everyone may). */
export const mayControl = (p: WatchParty, me: string | null | undefined): boolean =>
  !!me && (p.host_id === me || (p.everyone_controls && p.viewers.includes(me)))

/** Whether `me` may skip the current video (#464): whoever may control the party, and whoever put it on. */
export const maySkip = (p: WatchParty, me: string | null | undefined): boolean => !!me && (mayControl(p, me) || p.added_by === me)

/** Whether `me` may move or remove a queued video (#464): whoever may control the party, and whoever added it. */
export const mayEdit = (p: WatchParty, item: WatchPartyItem, me: string | null | undefined): boolean =>
  !!me && (mayControl(p, me) || item.added_by === me)

/** How long before the end the "Up next" card shows (seconds). */
export const UP_NEXT_S = 15

/** Seconds until the video ends while that's [`UP_NEXT_S`] or less, with something queued and the party playing (the
 *  "Up next" card's countdown); null otherwise, or while the player hasn't said how long the video is. */
export function upNextIn(p: WatchParty, at: number, duration: number | null): number | null {
  if (p.source === 'twitch_clip') duration = CLIP_S
  if (p.source === 'twitch_channel') duration = null
  if (!p.queue.length || !p.playing || !duration || duration <= 0) return null
  const left = duration - at
  return left <= UP_NEXT_S ? Math.max(0, Math.ceil(left)) : null
}

/** Twitch's own pages, which look like `twitch.tv/<channel>` but aren't a channel (as the server's list). */
const TWITCH_RESERVED = new Set(['directory', 'videos', 'search', 'settings', 'subscriptions', 'inventory', 'wallet', 'drops', 'downloads', 'jobs', 'p', 'turbo', 'prime', 'store', 'login', 'signup', 'friends', 'messages', 'moderator', 'popout', 'embed', 'team', 'broadcast', 'following'])

/** What a pasted or dropped link plays in a party (#459): a YouTube video, or a Twitch VOD, channel or clip. Only the
 *  providers' own hosts: a lookalike is no link. */
export function mediaFromLink(text: string): PartyMedia | null {
  const yt = youtubeIdFromLink(text)
  if (yt) return { source: 'youtube', id: yt }
  let u: URL
  try {
    u = new URL(text.trim())
  } catch {
    return null
  }
  if (u.protocol !== 'https:' && u.protocol !== 'http:') return null
  const host = u.hostname.toLowerCase()
  const segs = u.pathname.split('/').filter(Boolean)
  if (host === 'clips.twitch.tv') return segs.length === 1 && segs[0] !== 'embed' ? media('twitch_clip', segs[0]) : null
  if (!['twitch.tv', 'www.twitch.tv', 'm.twitch.tv'].includes(host)) return null
  if (segs.length === 3 && segs[1] === 'clip') return media('twitch_clip', segs[2])
  if (segs.length === 2 && segs[0] === 'videos') return media('twitch_video', segs[1])
  if (segs.length === 1 && !TWITCH_RESERVED.has(segs[0].toLowerCase())) return media('twitch_channel', segs[0].toLowerCase())
  return null
}

/** The YouTube video a pasted or dropped link points at (watch, youtu.be, Shorts, embed and live links), or null.
 *  The host must be YouTube's own: a lookalike is no link. */
export function youtubeIdFromLink(text: string): string | null {
  let u: URL
  try {
    u = new URL(text.trim())
  } catch {
    return null
  }
  if (u.protocol !== 'https:' && u.protocol !== 'http:') return null
  const host = u.hostname.toLowerCase()
  const segs = u.pathname.split('/').filter(Boolean)
  let id: string | null = null
  if (host === 'youtu.be') id = segs.length === 1 ? segs[0] : null
  else if (['youtube.com', 'www.youtube.com', 'm.youtube.com', 'music.youtube.com', 'youtube-nocookie.com', 'www.youtube-nocookie.com'].includes(host)) {
    if (segs.length === 1 && segs[0] === 'watch') id = u.searchParams.get('v')
    else if (segs.length === 2 && ['shorts', 'embed', 'live', 'v'].includes(segs[0])) id = segs[1]
  }
  return validVideoId(id) ? id : null
}

/** The first link in dropped or pasted text (a uri-list's first line that isn't a comment, or the text itself). */
export function linkIn(text: string): string {
  return (
    text
      .split(/\r?\n/)
      .map((l) => l.trim())
      .find((l) => l && !l.startsWith('#')) ?? ''
  )
}

/** Whether `me` has the party open. */
export const watching = (p: WatchParty | null | undefined, me: string | null | undefined): boolean => !!p && !!me && p.viewers.includes(me)

/** YouTube's player states, as its iframe API numbers them. */
export const YT = { unstarted: -1, ended: 0, playing: 1, paused: 2, buffering: 3, cued: 5 } as const

/** What the player last told us. */
export type PlayerInfo = { state: number | null; time: number | null; duration: number | null }

/** One command to the party page's player. */
export type PlayerCommand = { func: 'play' } | { func: 'pause' } | { func: 'seek'; to: number }

/** What the local player needs to match the party at `serverNowMs`: play or pause, and a seek when it's more than
 *  [`DRIFT_S`] off. Nothing while the player hasn't said where it is, nor while it's buffering (its time stands
 *  still then, and seeking would only restart the wait). An ended video stays ended while the party plays past it. */
export function correction(p: WatchParty, info: PlayerInfo, serverNowMs: number): PlayerCommand[] {
  // a clip has no control API; a live channel plays and pauses but never seeks (it's always at the live edge)
  if (p.source === 'twitch_clip') return []
  if (p.source === 'twitch_channel') {
    if (info.state === null) return []
    if (p.playing && info.state !== YT.playing && info.state !== YT.buffering) return [{ func: 'play' }]
    if (!p.playing && (info.state === YT.playing || info.state === YT.buffering)) return [{ func: 'pause' }]
    return []
  }
  if (info.state === null || info.time === null) return []
  const out: PlayerCommand[] = []
  const target = partySeconds(p, serverNowMs)
  const ended = info.state === YT.ended
  const past = info.duration !== null && info.duration > 0 && target >= info.duration - 0.5
  if (p.playing && info.state !== YT.playing && info.state !== YT.buffering && !(ended && past)) out.push({ func: 'play' })
  if (!p.playing && (info.state === YT.playing || info.state === YT.buffering)) out.push({ func: 'pause' })
  if (info.state === YT.buffering || (ended && past)) return out
  if (Math.abs(info.time - target) > DRIFT_S && !past) out.push({ func: 'seek', to: Math.max(0, Math.min(target, MAX_SECONDS)) })
  return out
}

/** The watch party page (our embed page in party mode: the provider's player driven by Pulse) for a video, starting
 *  at `startSeconds` (whole seconds; the first sync seeks precisely). A Twitch channel is live (no start); a Twitch
 *  clip has no control API, so it's the clip's ordinary page (#459). A bare id is a YouTube video. */
export function partyPlayerSrc(origin: string, video: PartyMedia | string, startSeconds = 0): string {
  const m: PartyMedia = typeof video === 'string' ? { source: 'youtube', id: video } : video
  const e = embedOf(m)
  const page = `${origin.replace(/\/+$/, '')}/embed/${e.provider}/${encodeURIComponent(e.id)}`
  if (m.source === 'twitch_clip') return page
  const base = `${page}?party=1`
  const start = Math.floor(startSeconds)
  return m.source !== 'twitch_channel' && start > 0 && start <= 7 * 86400 ? `${base}&start=${start}` : base
}

/** "1:05", "1:02:03". */
export function clockText(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds))
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const ss = String(s % 60).padStart(2, '0')
  return h ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`
}
