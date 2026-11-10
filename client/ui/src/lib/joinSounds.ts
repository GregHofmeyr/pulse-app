// Join sounds (#121), the pure rules (tested in joinSounds.test.ts). Someone's join sound is a sound of a server's
// soundboard, kept on their member entry there (`Member.join_sound`). The server decides whether a join carries it
// (server voice channels only, at most once per 30 s per person) and puts it on `VoiceJoined`; each listener then
// plays it instead of the Join cue, unless the mixer's Join sounds row is at 0 % or they muted that person's.
import type { Event as AppEvent } from './protocol/Event'
import type { JoinSoundPlay } from './protocol/JoinSoundPlay'
import type { AppState } from './state'
import type { Connection } from './voiceui'

const MUTES_KEY = 'pulse.joinSounds.mutes'

/** People whose join sound you hear as the normal Join cue (user id -> true), on this device. */
export function loadJoinSoundMutes(): Record<string, true> {
  try {
    const raw = localStorage.getItem(MUTES_KEY)
    const r: unknown = raw ? JSON.parse(raw) : null
    if (typeof r !== 'object' || r === null || Array.isArray(r)) return {}
    return Object.fromEntries(Object.entries(r).filter(([, v]) => v === true).map(([k]) => [k, true as const]))
  } catch {
    return {}
  }
}
export function saveJoinSoundMutes(m: Record<string, true>) {
  try {
    localStorage.setItem(MUTES_KEY, JSON.stringify(m))
  } catch {
    // storage unavailable: the setting just won't survive a restart
  }
}

/** `userId`'s join sound in `serverId` (a sound id), if they have one and it is still on the soundboard. */
export function joinSoundOf(s: AppState, serverId: string, userId: string): string | null {
  const id = s.members[serverId]?.find((m) => m.user.id === userId)?.join_sound ?? null
  return id && s.sounds[id] ? id : null
}

/** Whether `userId` has a join sound in any server (the person menu offers muting it then). */
export const hasJoinSound = (s: AppState, userId: string): boolean => Object.keys(s.members).some((server) => joinSoundOf(s, server, userId) !== null)

/** What someone else joining your call sounds like: their join sound, or the normal Join cue (none on the event, the
 * Join sounds row at 0 %, or muted for you). Null when it isn't someone else joining the call you're connected to. */
export function peerJoin(
  e: AppEvent,
  ctx: { myId: string | undefined; channelId: string | null; connection: Connection; level: number; mutes: Record<string, true> },
): { kind: 'sound'; play: JoinSoundPlay } | { kind: 'cue' } | null {
  if (e.t !== 'VoiceJoined') return null
  if (!ctx.channelId || ctx.connection !== 'connected' || e.d.channel_id !== ctx.channelId || e.d.user_id === ctx.myId) return null
  const play = e.d.join_sound
  return play && ctx.level > 0 && !ctx.mutes[e.d.user_id] ? { kind: 'sound', play } : { kind: 'cue' }
}

/** Your own Join cue waits for your join's event when that join may carry your join sound: a server voice channel
 * where you have one, with the Join sounds row above 0 %. */
export function holdOwnCue(s: AppState, channelId: string, myId: string | undefined, level: number): boolean {
  const server = s.channels[channelId]?.server_id
  return !!server && !!myId && level > 0 && joinSoundOf(s, server, myId) !== null
}

export type JoinAction = { kind: 'sound'; play: JoinSoundPlay } | { kind: 'cue' }

/** How long your Join cue waits for your join's event before playing the normal cue anyway. */
export const OWN_JOIN_WAIT_MS = 3000

/** Your own join sounds once: when your connection comes up (`connected`) and when the server announces your join
 * (`announced`), in either order. A held cue plays the join sound the event carries, or the normal cue if it has none
 * or never comes within `waitMs`. */
export function ownJoin(act: (a: JoinAction, channel: string) => void, waitMs = OWN_JOIN_WAIT_MS) {
  let pending: { channel: string; timer: ReturnType<typeof setTimeout> } | null = null
  let early: { channel: string; play: JoinSoundPlay | null; at: number } | null = null
  const of = (play: JoinSoundPlay | null): JoinAction => (play ? { kind: 'sound', play } : { kind: 'cue' })
  return {
    /** Your connection to `channel` came up. False: not held, play the normal cue now. */
    connected(channel: string, hold: boolean, now = Date.now()): boolean {
      if (pending) clearTimeout(pending.timer)
      pending = null
      if (!hold) return false
      const e = early
      early = null
      if (e && e.channel === channel && now - e.at < waitMs) {
        act(of(e.play), channel)
        return true
      }
      const timer = setTimeout(() => {
        pending = null
        act({ kind: 'cue' }, channel)
      }, waitMs)
      pending = { channel, timer }
      return true
    },
    /** The server announced your join to `channel`, with or without your join sound. */
    announced(channel: string, play: JoinSoundPlay | null, now = Date.now()) {
      if (pending && pending.channel === channel) {
        clearTimeout(pending.timer)
        pending = null
        act(of(play), channel)
        return
      }
      early = { channel, play, at: now }
    },
    /** You moved to `channel` (null: left): whatever waits for another call is dropped, so a quick switch never plays
     * the call you just left. */
    moved(channel: string | null) {
      if (pending && pending.channel !== channel) {
        clearTimeout(pending.timer)
        pending = null
      }
      if (early && early.channel !== channel) early = null
    },
    /** Stop: nothing waits any more. */
    reset() {
      if (pending) clearTimeout(pending.timer)
      pending = null
      early = null
    },
  }
}
