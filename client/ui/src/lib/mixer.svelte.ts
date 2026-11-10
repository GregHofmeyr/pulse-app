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
let inFlight = false
let again = false

/** Fire and forget: the core applies the levels to a live call whenever they land, and keeps them for the next. One
 * push at a time (the core runs commands concurrently, so a drag's pushes could land out of order); whatever changed
 * meanwhile goes in one more push with the latest levels. */
export function pushCallMixQuietly() {
  if (inFlight) {
    again = true
    return
  }
  inFlight = true
  again = false
  void pushCallMix()
    .catch(() => {})
    .finally(() => {
      inFlight = false
      if (again) pushCallMixQuietly()
    })
}

/** One notch (`master` or an item of `section`), clamped to its range. Unknown notches are ignored. */
export function setNotch(section: SectionId, key: string, pct: number) {
  const s = MIXER_SECTIONS.find((x) => x.id === section)
  const max = key === 'master' && s?.master ? 200 : s?.notches.find((x) => x.key === key)?.max
  if (max === undefined || !Number.isFinite(pct)) return
  ;(mixer[section] as Record<string, number | boolean>)[key] = Math.round(Math.min(max, Math.max(0, pct)))
  save()
  // the core applies Voices and Streams; the soundboard and join sounds go with each play
  if (section === 'calls' && key !== 'soundboard' && key !== 'join_sounds') pushCallMixQuietly()
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
