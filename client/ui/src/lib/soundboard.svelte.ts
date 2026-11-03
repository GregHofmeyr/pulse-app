// Soundboard prefs and the SoundPlayed listener (the pure rules are in soundboard.ts). The volume is the mixer's
// Soundboard notch: the popover's slider and Settings → Volume mixer move the same value.
import { loadAudioConfig } from './voiceui'
import { onAppEvent } from './store.svelte'
import { voice } from './voice.svelte'
import { loadSoundMutes, saveSoundMutes, shouldPlay } from './soundboard'
import { soundboardCmd } from './tauri'
import { mixer, setNotch } from './mixer.svelte'

const muted = $state({ people: loadSoundMutes() as Record<string, true> })

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
}

export function setSoundboardVolume(pct: number) {
  setNotch('calls', 'soundboard', pct)
}

export function setSoundMute(userId: string, on: boolean) {
  const { [userId]: _off, ...rest } = muted.people
  muted.people = on ? { ...rest, [userId]: true } : rest
  saveSoundMutes($state.snapshot(muted.people))
}

/** Play a sound locally when the server relays it to the call we're in. Returns the unlisten function. */
export function startSoundboardListening(): () => void {
  return onAppEvent((e) => {
    if (e.t !== 'SoundPlayed') return
    const { master, soundboard: level } = mixer.calls
    const ctx = { myChannel: voice.channelId, volume_pct: master > 0 ? level : 0, mutes: muted.people }
    if (!shouldPlay(e.d, ctx)) return
    // a sound that fails to play is not worth interrupting the call over
    soundboardCmd.play(e.d.media_hash, master, level, e.d.volume, loadAudioConfig().output ?? null).catch(() => {})
  })
}
