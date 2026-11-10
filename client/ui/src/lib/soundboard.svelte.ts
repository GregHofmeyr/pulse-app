// Soundboard prefs and the SoundPlayed listener (the pure rules are in soundboard.ts). The volume is the mixer's
// Soundboard notch: the popover's slider and Settings → Volume mixer move the same value.
//
// "Who played it": while someone's sound plays here their ring lights (soundRings.svelte.ts).
import { untrack } from 'svelte'
import { loadAudioConfig } from './voiceui'
import { app, onAppEvent } from './store.svelte'
import { voice } from './voice.svelte'
import { memberServers } from './reactions'
import { loadSoundMutes, saveSoundMutes, shouldPlay, warmOrder } from './soundboard'
import { soundboardCmd } from './tauri'
import { mixer, setNotch } from './mixer.svelte'
import { clearRings, lightRing, rings } from './soundRings.svelte'

export { lightRing }

const muted = $state({ people: loadSoundMutes() as Record<string, true> })
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

let lastCall: string | null = null
/** The call you're in changed: rings belong to one call; joining one (from none) fetches your servers' sounds into the
 * disk cache, so a first play (or someone's join sound) doesn't wait on the network. */
export function callChanged(channelId: string | null) {
  const joined = lastCall === null && channelId !== null
  if (channelId !== lastCall) clearRings()
  lastCall = channelId
  if (joined) {
    // the join sounds (#121) of the people in this call's server first: the next join is likeliest to need one
    const server = app.state.channels[channelId]?.server_id
    const joinSounds = server ? (app.state.members[server] ?? []).flatMap((m) => (m.join_sound ? [m.join_sound] : [])) : []
    const hashes = warmOrder(app.state, memberServers(app.state), joinSounds)
    if (hashes.length) soundboardCmd.warm(hashes).catch(() => {})
  }
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
