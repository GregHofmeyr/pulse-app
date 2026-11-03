// Soundboard prefs and the SoundPlayed listener (the pure rules are in soundboard.ts).
import { loadAudioConfig } from './voiceui'
import { onAppEvent } from './store.svelte'
import { voice } from './voice.svelte'
import { loadSoundboardPrefs, loadSoundMutes, saveSoundboardPrefs, saveSoundMutes, shouldPlay } from './soundboard'
import { soundboardCmd } from './tauri'

export const soundboard = $state({
  volume_pct: loadSoundboardPrefs().volume_pct,
  mutes: loadSoundMutes() as Record<string, true>,
})

export function setSoundboardVolume(pct: number) {
  soundboard.volume_pct = Math.round(Math.min(100, Math.max(0, pct)))
  saveSoundboardPrefs({ volume_pct: soundboard.volume_pct })
}

export function setSoundMute(userId: string, on: boolean) {
  const { [userId]: _off, ...rest } = soundboard.mutes
  soundboard.mutes = on ? { ...rest, [userId]: true } : rest
  saveSoundMutes($state.snapshot(soundboard.mutes))
}

/** Play a sound locally when the server relays it to the call we're in. Returns the unlisten function. */
export function startSoundboardListening(): () => void {
  return onAppEvent((e) => {
    if (e.t !== 'SoundPlayed') return
    const ctx = { myChannel: voice.channelId, volume_pct: soundboard.volume_pct, mutes: soundboard.mutes }
    if (!shouldPlay(e.d, ctx)) return
    // a sound that fails to play is not worth interrupting the call over
    soundboardCmd.play(e.d.media_hash, soundboard.volume_pct, e.d.volume, loadAudioConfig().output ?? null).catch(() => {})
  })
}
