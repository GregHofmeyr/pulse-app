// Soundboard prefs and the SoundPlayed listener (the pure rules are in soundboard.ts). The volume is the mixer's
// Soundboard notch: the popover's slider and Settings → Volume mixer move the same value.
//
// "Who played it": while someone's sound plays here, their id is in `soundboard.playing`, and the voice list and
// call tiles light their ring for it (unioned at render time). It is never mixed into `voice.speaking`, which is the
// real voice activity that speaking-based features read.
import { untrack } from 'svelte'
import { loadAudioConfig } from './voiceui'
import { app, onAppEvent } from './store.svelte'
import { voice } from './voice.svelte'
import { memberServers } from './reactions'
import { loadSoundMutes, saveSoundMutes, shouldPlay, SOUND_COOLDOWN_MS, warmOrder } from './soundboard'
import { soundboardCmd } from './tauri'
import { mixer, setNotch } from './mixer.svelte'

const muted = $state({ people: loadSoundMutes() as Record<string, true> })
const rings = $state({ users: new Set<string>() })
/** When each lit ring goes out (ms since epoch) and its timer. */
const ringEnds = new Map<string, { until: number; timer: ReturnType<typeof setTimeout> }>()
/** How long to light a ring when neither the event nor the store knows the sound's length. */
const FALLBACK_RING_MS = 2000

export const soundboard = {
  /** The Soundboard notch, 0-200 (0 = off). */
  get volume_pct(): number {
    return mixer.calls.soundboard
  },
  set volume_pct(pct: number) {
    setSoundboardVolume(pct)
  },
  get mutes(): Record<string, true> {
    return muted.people
  },
  set mutes(m: Record<string, true>) {
    muted.people = m
  },
  /** People whose sound is playing here right now (their ring lights). */
  get playing(): Set<string> {
    return rings.users
  },
}

/** Light `userId`'s ring for `ms`. A new play of theirs cuts off the one before (#214), so it restarts the ring for
 * its own length, shorter or not. */
export function lightRing(userId: string, ms: number) {
  const until = Date.now() + ms
  const prev = ringEnds.get(userId)
  if (prev) clearTimeout(prev.timer)
  const timer = setTimeout(() => {
    ringEnds.delete(userId)
    const next = new Set(rings.users)
    next.delete(userId)
    rings.users = next
  }, ms)
  ringEnds.set(userId, { until, timer })
  if (!rings.users.has(userId)) rings.users = new Set([...rings.users, userId])
}

function clearRings() {
  for (const { timer } of ringEnds.values()) clearTimeout(timer)
  ringEnds.clear()
  if (rings.users.size) rings.users = new Set()
}

let lastCall: string | null = null
/** The call you're in changed: rings belong to one call; joining one (from none) fetches your servers' sounds into the
 * disk cache, so a first play doesn't wait on the network. */
export function callChanged(channelId: string | null) {
  const joined = lastCall === null && channelId !== null
  if (channelId !== lastCall) clearRings()
  lastCall = channelId
  if (joined) {
    const hashes = warmOrder(app.state, memberServers(app.state))
    if (hashes.length) soundboardCmd.warm(hashes).catch(() => {})
  }
}

// Your own cooldown after a play (#214), shared by every soundboard on screen: true while it holds.
const cool = $state({ on: false })
let coolTimer: ReturnType<typeof setTimeout> | undefined

/** Whether your plays are cooling down (the tiles show it). */
export const soundCooling = (): boolean => cool.on

/** Start a play: false (and nothing changes) while the last one's cooldown holds, else true and the cooldown starts. */
export function beginSoundPlay(): boolean {
  if (cool.on) return false
  cool.on = true
  coolTimer = setTimeout(() => (cool.on = false), SOUND_COOLDOWN_MS)
  return true
}

/** Tests: no cooldown. */
export function resetSoundCooldown() {
  clearTimeout(coolTimer)
  cool.on = false
}

export function setSoundboardVolume(pct: number) {
  setNotch('calls', 'soundboard', pct)
}

export function setSoundMute(userId: string, on: boolean) {
  const { [userId]: _off, ...rest } = muted.people
  muted.people = on ? { ...rest, [userId]: true } : rest
  saveSoundMutes($state.snapshot(muted.people))
}

/** Play a sound locally when the server relays it to the call we're in, and light the player's ring while it plays.
 * Also warms sounds on joining a call, and new ones added during it. Returns the unlisten function. */
export function startSoundboardListening(): () => void {
  const stopWatching = $effect.root(() => {
    $effect(() => {
      const id = voice.channelId
      untrack(() => callChanged(id))
    })
  })
  const stopEvents = onAppEvent((e) => {
    if (e.t === 'SoundCreated') {
      if (voice.channelId) soundboardCmd.warm([e.d.sound.media_hash]).catch(() => {})
      return
    }
    if (e.t !== 'SoundPlayed') return
    const { master, soundboard: level } = mixer.calls
    const ctx = { myChannel: voice.channelId, volume_pct: master > 0 ? level : 0, mutes: muted.people }
    if (!shouldPlay(e.d, ctx)) return
    const ms = e.d.duration_ms || app.state.sounds[e.d.sound_id]?.duration_ms || FALLBACK_RING_MS
    soundboardCmd
      // the player: the core cuts off their previous sound (#214); others' carry on
      .play(e.d.media_hash, master, level, e.d.volume, loadAudioConfig().output ?? null, e.d.sound_id, e.d.user_id)
      // lit once it's actually queued, for as long as it plays
      .then(() => {
        if (voice.channelId === e.d.channel_id) lightRing(e.d.user_id, ms)
      })
      // a sound that fails to play is not worth interrupting the call over
      .catch(() => {})
  })
  return () => {
    stopEvents()
    stopWatching()
    clearRings()
  }
}
