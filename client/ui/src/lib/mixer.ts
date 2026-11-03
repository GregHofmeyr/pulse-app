// The volume mixer's per-device record and its gain maths (tested in mixer.test.ts). Every notch is a percent on the
// core's `percent_to_gain` curve (voice/mixer.rs): 0 = off, 100 = as recorded, 200 = +12 dB.
import { normalizeSoundPrefs, type SoundName } from './voiceui'

export const MIXER_KEY = 'pulse.mixer'

export type MixerPrefs = {
  v: 1
  calls: { master: number; voices: number; streams: number; soundboard: number }
  callSounds: { master: number; join: number; leave: number; mute: number; unmute: number; deafen: number; undeafen: number }
  streamSounds: { master: number; live: number; live_end: number }
  notifications: { master: number; message: number; badge: number }
  /** No master: one notch. A `<video>` can't play above its own full volume, so Clips stops at 100. */
  media: { clips: number; clipsMuted: boolean }
}
export type SectionId = 'calls' | 'callSounds' | 'streamSounds' | 'notifications' | 'media'

export type Notch = {
  key: string
  label: string
  max: number
  /** What its ▶ plays: one of the UI sounds, or a soundboard sound. */
  test?: 'sound' | 'soundboard'
}
export type MixerSection = { id: SectionId; label: string; master: boolean; notches: Notch[] }

const n = (key: string, label: string, test?: Notch['test']): Notch => ({ key, label, max: 200, ...(test ? { test } : {}) })

/** Sections in display order. A new sound gets its notch here (and in `MixerPrefs`). */
export const MIXER_SECTIONS: readonly MixerSection[] = [
  { id: 'calls', label: 'Calls', master: true, notches: [n('voices', 'Voices'), n('streams', 'Streams'), n('soundboard', 'Soundboard', 'soundboard')] },
  {
    id: 'callSounds', label: 'Call sounds', master: true,
    notches: [n('join', 'Join', 'sound'), n('leave', 'Leave', 'sound'), n('mute', 'Mute', 'sound'), n('unmute', 'Unmute', 'sound'),
      n('deafen', 'Deafen', 'sound'), n('undeafen', 'Undeafen', 'sound')],
  },
  { id: 'streamSounds', label: 'Stream sounds', master: true, notches: [n('live', 'Someone goes live', 'sound'), n('live_end', 'A stream ends', 'sound')] },
  { id: 'notifications', label: 'Notifications', master: true, notches: [n('message', 'Message ding', 'sound'), n('badge', 'Badge unlocked', 'sound')] },
  { id: 'media', label: 'Media', master: false, notches: [{ key: 'clips', label: 'Clips', max: 100 }] },
]

export function defaultMixer(): MixerPrefs {
  return {
    v: 1,
    calls: { master: 100, voices: 100, streams: 100, soundboard: 100 },
    callSounds: { master: 100, join: 100, leave: 100, mute: 100, unmute: 100, deafen: 100, undeafen: 100 },
    streamSounds: { master: 100, live: 100, live_end: 100 },
    notifications: { master: 100, message: 100, badge: 100 },
    media: { clips: 100, clipsMuted: false },
  }
}

/** Percent → linear gain, exactly as the core's `percent_to_gain`: 0 → silence, 1 → −40 dB, 200 → +12 dB. */
export function percentToGain(pct: number): number {
  const p = Math.min(200, Math.max(0, pct))
  if (!(p > 0)) return 0
  const db = p >= 100 ? ((p - 100) / 100) * 12 : ((p - 100) / 100) * 40
  return 10 ** (db / 20)
}

/** Linear gain → the percent that sounds the same (for migrating linear volumes). Anything audible stays ≥ 1. */
export function gainToPercent(gain: number): number {
  if (!(gain > 0)) return 0
  const db = 20 * Math.log10(gain)
  const p = db >= 0 ? 100 + (db / 12) * 100 : 100 + (db / 40) * 100
  return Math.round(Math.min(200, Math.max(1, p)))
}

/** The safe maximum for any final gain: one slider at 200 % (+12 dB). The core caps the same way. */
export const MAX_GAIN = percentToGain(200)

/** Several notches together (section × item × …), capped at `MAX_GAIN`. */
export const mixGain = (...pcts: number[]): number => Math.min(MAX_GAIN, pcts.reduce((g, p) => g * percentToGain(p), 1))

type SoundSection = 'callSounds' | 'streamSounds' | 'notifications'
const SOUND_SECTION: Record<SoundName, SoundSection> = {
  join: 'callSounds', leave: 'callSounds', mute: 'callSounds', unmute: 'callSounds', deafen: 'callSounds', undeafen: 'callSounds',
  live: 'streamSounds', live_end: 'streamSounds', message: 'notifications', badge: 'notifications',
}
export const soundSectionOf = (name: SoundName): SoundSection => SOUND_SECTION[name]

const notch = (p: MixerPrefs, name: SoundName): number => (p[SOUND_SECTION[name]] as Record<string, number>)[name] ?? 100

/** A UI sound's gain: its section master × its own notch. */
export const soundGain = (p: MixerPrefs, name: SoundName): number => mixGain(p[SOUND_SECTION[name]].master, notch(p, name))
/** Just the section master (a sound set's Preview ignores the per-sound notches). */
export const soundMasterGain = (p: MixerPrefs, name: SoundName): number => mixGain(p[SOUND_SECTION[name]].master)

const pct = (v: unknown, fallback: number, max = 200) =>
  typeof v === 'number' && Number.isFinite(v) ? Math.round(Math.min(max, Math.max(0, v))) : fallback
const obj = (v: unknown): Record<string, unknown> => (typeof v === 'object' && v !== null && !Array.isArray(v) ? (v as Record<string, unknown>) : {})

/** A stored v1 record cleaned field by field; null when it isn't one (then it's rebuilt from the old keys). */
export function normalizeMixer(raw: unknown): MixerPrefs | null {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw) || (raw as { v?: unknown }).v !== 1) return null
  const r = raw as Record<string, unknown>
  const d = defaultMixer()
  const clean = <T extends Record<string, number>>(id: Exclude<SectionId, 'media'>, def: T): T => {
    const src = obj(r[id])
    return Object.fromEntries(Object.entries(def).map(([k, v]) => [k, pct(src[k], v)])) as T
  }
  const media = obj(r.media)
  return {
    v: 1,
    calls: clean('calls', d.calls),
    callSounds: clean('callSounds', d.callSounds),
    streamSounds: clean('streamSounds', d.streamSounds),
    notifications: clean('notifications', d.notifications),
    media: { clips: pct(media.clips, 100, 100), clipsMuted: media.clipsMuted === true },
  }
}

/** Today's separate settings, as stored before the mixer. */
export type LegacyPrefs = { sounds: unknown; soundboard: unknown; clipVolume: unknown }

/** The mixer as today's settings had it: an "off" sound → 0, the old (linear) sound volume → the three sound masters
 * at the same loudness, the soundboard volume (already on the curve) as is, the clip volume (linear) by loudness. */
export function migrateMixer(legacy: LegacyPrefs): MixerPrefs {
  const p = defaultMixer()
  if (legacy.sounds !== null && legacy.sounds !== undefined) {
    const s = normalizeSoundPrefs(legacy.sounds)
    const master = gainToPercent(s.volume_pct / 100)
    p.callSounds.master = p.streamSounds.master = p.notifications.master = master
    for (const name of s.off) (p[SOUND_SECTION[name]] as Record<string, number>)[name] = 0
  }
  p.calls.soundboard = pct(obj(legacy.soundboard).volume_pct, 100)
  const clip = obj(legacy.clipVolume)
  if (typeof clip.volume === 'number' && Number.isFinite(clip.volume)) p.media.clips = Math.min(100, gainToPercent(Math.min(1, clip.volume)))
  p.media.clipsMuted = clip.muted === true
  return p
}

function read(key: string): unknown {
  try {
    const raw = localStorage.getItem(key)
    return raw ? JSON.parse(raw) : null
  } catch {
    return null
  }
}

export function saveMixer(p: MixerPrefs) {
  try {
    localStorage.setItem(MIXER_KEY, JSON.stringify(p))
  } catch {
    // storage unavailable: the setting just won't survive a restart
  }
}

/** The stored mixer; the first time (or after a version change) built from today's keys and saved. */
export function loadMixer(): MixerPrefs {
  const stored = normalizeMixer(read(MIXER_KEY))
  if (stored) return stored
  const p = migrateMixer({ sounds: read('pulse.sounds'), soundboard: read('pulse.soundboard'), clipVolume: read('pulse.clipVolume') })
  saveMixer(p)
  return p
}

/** `p` with one section back at its defaults. */
export function resetSection(p: MixerPrefs, id: SectionId): MixerPrefs {
  return { ...p, [id]: defaultMixer()[id] }
}
