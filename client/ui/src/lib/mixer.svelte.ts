// The volume mixer's live state, shared by Settings, the soundboard popover and the clip players (the pure rules are in
// mixer.ts). Every change is saved at once; the Calls levels also go to the core, which applies Voices and Streams.
import { invoke } from '@tauri-apps/api/core'
import { MIXER_SECTIONS, loadMixer, resetSection, saveMixer, type MixerPrefs, type SectionId } from './mixer'

export const mixer = $state<MixerPrefs>(loadMixer())

const save = () => saveMixer($state.snapshot(mixer) as MixerPrefs)

/** The Calls master, Voices and Streams to the core (it keeps them with or without a call). */
export function pushCallMix(): Promise<void> {
  const { master, voices, streams } = mixer.calls
  try {
    return Promise.resolve(invoke<void>('set_call_mix', { callsPercent: master, voicesPercent: voices, streamsPercent: streams }))
  } catch (e) {
    return Promise.reject(e)
  }
}
/** Fire and forget: the core applies the levels to a live call whenever they land, and keeps them for the next. */
export const pushCallMixQuietly = () => void pushCallMix().catch(() => {})

/** One notch (`master` or an item of `section`), clamped to its range. Unknown notches are ignored. */
export function setNotch(section: SectionId, key: string, pct: number) {
  const s = MIXER_SECTIONS.find((x) => x.id === section)
  const max = key === 'master' && s?.master ? 200 : s?.notches.find((x) => x.key === key)?.max
  if (max === undefined || !Number.isFinite(pct)) return
  ;(mixer[section] as Record<string, number | boolean>)[key] = Math.round(Math.min(max, Math.max(0, pct)))
  save()
  if (section === 'calls' && key !== 'soundboard') pushCallMixQuietly()
}

export function setClipsMuted(muted: boolean) {
  mixer.media.clipsMuted = muted
  save()
}

/** "Reset to defaults" for one section. */
export function resetMixerSection(id: SectionId) {
  const next = resetSection($state.snapshot(mixer) as MixerPrefs, id)
  ;(mixer as Record<SectionId, unknown>)[id] = next[id]
  save()
  if (id === 'calls') pushCallMixQuietly()
}

/** Re-reads the saved record (startup and tests). */
export function reloadMixer() {
  Object.assign(mixer, loadMixer())
}
