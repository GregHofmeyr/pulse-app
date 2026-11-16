// YouTube watch parties in voice (#384): the pure parts, tested in watchParty.test.ts. The server holds where the
// video was (`position_ms`) at `set_at_ms` by its clock, and whether it's playing; each viewer's player follows that,
// correcting itself when it drifts too far.
import type { LinkEmbed } from './protocol/LinkEmbed'
import type { WatchParty } from './protocol/WatchParty'

/** How far (seconds) a player may drift from the party before it's seeked back. */
export const DRIFT_S = 1.5

/** The furthest a seek goes (48 hours, the server's cap), in seconds. */
export const MAX_SECONDS = 48 * 3600

/** A YouTube video id: exactly 11 of `[A-Za-z0-9_-]` (the server checks the same). */
export const validVideoId = (id: string | null | undefined): id is string => !!id && /^[A-Za-z0-9_-]{11}$/.test(id)

/** A YouTube card's video id, when it has a valid one. */
export function youtubeIdOf(e: LinkEmbed): string | null {
  return e.provider === 'youtube' && validVideoId(e.provider_id) ? e.provider_id : null
}

/** Where the party's video is (seconds) at `serverNowMs`: it runs on while playing. */
export function partySeconds(p: Pick<WatchParty, 'position_ms' | 'playing' | 'set_at_ms'>, serverNowMs: number): number {
  const ran = p.playing ? Math.max(0, serverNowMs - p.set_at_ms) : 0
  return Math.min(MAX_SECONDS, (p.position_ms + ran) / 1000)
}

/** Whether `me` may play, pause, seek and change the video (the host; anyone watching while everyone may). */
export const mayControl = (p: WatchParty, me: string | null | undefined): boolean =>
  !!me && (p.host_id === me || (p.everyone_controls && p.viewers.includes(me)))

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

/** The watch party page (our embed page in party mode: YouTube's player driven by Pulse) for `videoId`, starting at
 *  `startSeconds` (whole seconds; the first sync seeks precisely). */
export function partyPlayerSrc(origin: string, videoId: string, startSeconds = 0): string {
  const base = `${origin.replace(/\/+$/, '')}/embed/youtube/${encodeURIComponent(videoId)}?party=1`
  const start = Math.floor(startSeconds)
  return start > 0 && start <= 7 * 86400 ? `${base}&start=${start}` : base
}

/** "1:05", "1:02:03". */
export function clockText(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds))
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const ss = String(s % 60).padStart(2, '0')
  return h ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`
}
