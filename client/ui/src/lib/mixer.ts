// The volume mixer's per-device record and its gain maths (tested in mixer.test.ts). Every notch is a percent on the
// core's `percent_to_gain` curve (voice/mixer.rs): 0 = off, 100 = as recorded, 200 = +12 dB.
import type { SoundName } from './voiceui'

export const MIXER_KEY = 'pulse.mixer'

export type MixerPrefs = {
  v: 1
  calls: { master: number; voices: number; streams: number; soundboard: number; join_sounds: number }
  callSounds: {
    master: number; join: number; leave: number; mute: number; unmute: number; deafen: number; undeafen: number; muted_talk: number; ringing: number
  }
  streamSounds: { master: number; live: number; live_end: number }
  notifications: { master: number; message: number; badge: number; wheel_tick: number; wheel_result: number }
  /** No master: one notch. A `<video>` can't play above its own full volume, so Clips stops at 100. */
  media: { clips: number; clipsMuted: boolean }
}
export type SectionId = 'calls' | 'callSounds' | 'streamSounds' | 'notifications' | 'media'

export type Notch = {
  key: string
  label: string
  max: number
  /** What its ▶ plays: one of the UI sounds, or a soundboard sound. */
  test?: 'sound' | 'soundboard' | 'ring'
}
export type MixerSection = { id: SectionId; label: string; master: boolean; notches: Notch[] }

const n = (key: string, label: string, test?: Notch['test']): Notch => ({ key, label, max: 200, ...(test ? { test } : {}) })

/** Sections in display order. A new sound gets its notch here (and in `MixerPrefs`). */
export const MIXER_SECTIONS: readonly MixerSection[] = [
  { id: 'calls', label: 'Calls', master: true, notches: [n('voices', 'Voices'), n('streams', 'Streams'), n('soundboard', 'Soundboard', 'soundboard'),
    // people's join sounds (#121): played like soundboard sounds; 0 = everyone's join is the normal Join cue instead
    n('join_sounds', 'Join sounds', 'soundboard')] },
  {
    id: 'callSounds', label: 'Call sounds', master: true,
    notches: [n('join', 'Join', 'sound'), n('leave', 'Leave', 'sound'), n('mute', 'Mute', 'sound'), n('unmute', 'Unmute', 'sound'),
      n('deafen', 'Deafen', 'sound'), n('undeafen', 'Undeafen', 'sound'), n('muted_talk', 'Talking while muted', 'sound'),
      n('ringing', 'Ringing', 'ring')],
  },
  { id: 'streamSounds', label: 'Stream sounds', master: true, notches: [n('live', 'Someone goes live', 'sound'), n('live_end', 'A stream ends', 'sound')] },
  { id: 'notifications', label: 'Notifications', master: true, notches: [n('message', 'Message ding', 'sound'), n('badge', 'Badge unlocked', 'sound'), n('wheel_tick', 'Picker wheel tick', 'sound'),
    n('wheel_result', 'Picker wheel result', 'sound')] },
  { id: 'media', label: 'Media', master: false, notches: [{ key: 'clips', label: 'Clips', max: 100 }] },
]

export function defaultMixer(): MixerPrefs {
  return {
    v: 1,
    calls: { master: 100, voices: 100, streams: 100, soundboard: 100, join_sounds: 100 },
    callSounds: { master: 100, join: 100, leave: 100, mute: 100, unmute: 100, deafen: 100, undeafen: 100, muted_talk: 100, ringing: 100 },
    streamSounds: { master: 100, live: 100, live_end: 100 },
    notifications: { master: 100, message: 100, badge: 100, wheel_tick: 100, wheel_result: 100 },
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

/** The safe maximum for any final gain: one slider at 200 % (+12 dB). The core caps the same way. */
export const MAX_GAIN = percentToGain(200)

/** Several notches together (section × item × …), capped at `MAX_GAIN`. */
export const mixGain = (...pcts: number[]): number => Math.min(MAX_GAIN, pcts.reduce((g, p) => g * percentToGain(p), 1))

type SoundSection = 'callSounds' | 'streamSounds' | 'notifications'
const SOUND_SECTION: Record<SoundName, SoundSection> = {
  join: 'callSounds', leave: 'callSounds', mute: 'callSounds', unmute: 'callSounds', deafen: 'callSounds', undeafen: 'callSounds',
  muted_talk: 'callSounds',
  live: 'streamSounds', live_end: 'streamSounds', message: 'notifications', badge: 'notifications', wheel_tick: 'notifications',
  wheel_result: 'notifications',
}
export const soundSectionOf = (name: SoundName): SoundSection => SOUND_SECTION[name]

const notch = (p: MixerPrefs, name: SoundName): number => (p[SOUND_SECTION[name]] as Record<string, number>)[name] ?? 100

/** A UI sound's gain: its section master × its own notch. */
export const soundGain = (p: MixerPrefs, name: SoundName): number => mixGain(p[SOUND_SECTION[name]].master, notch(p, name))
/** The incoming-call ringtone's gain (#50): Call sounds master × Ringing. Not part of a sound set: one ringtone. */
export const ringtoneGain = (p: MixerPrefs): number => mixGain(p.callSounds.master, p.callSounds.ringing)
/** Just the section master (a sound set's Preview ignores the per-sound notches). */
export const soundMasterGain = (p: MixerPrefs, name: SoundName): number => mixGain(p[SOUND_SECTION[name]].master)

const pct = (v: unknown, fallback: number, max = 200) =>
  typeof v === 'number' && Number.isFinite(v) ? Math.round(Math.min(max, Math.max(0, v))) : fallback
const obj = (v: unknown): Record<string, unknown> => (typeof v === 'object' && v !== null && !Array.isArray(v) ? (v as Record<string, unknown>) : {})

/** A stored v1 record cleaned field by field; null when it isn't one (then the defaults are used). */
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

/** The stored mixer; the defaults when there's none, it's unreadable or it's a newer version's (after a downgrade,
 * left in place until a change is saved). Pre-mixer keys (`pulse.soundboard`, `pulse.clipVolume`, the old fields of
 * `pulse.sounds`, before 0.2.17) are ignored. */
export function loadMixer(): MixerPrefs {
  return normalizeMixer(read(MIXER_KEY)) ?? defaultMixer()
}

/** `p` with one section back at its defaults. */
export function resetSection(p: MixerPrefs, id: SectionId): MixerPrefs {
  return { ...p, [id]: defaultMixer()[id] }
}
