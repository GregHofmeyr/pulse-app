import { invoke } from '@tauri-apps/api/core'
// Pure helpers for the voice UI (tested in voiceui.test.ts).

export type Flags = { muted: boolean; deafened: boolean }
export type SoundName =
  | 'join' | 'leave' | 'mute' | 'unmute' | 'deafen' | 'undeafen' | 'message' | 'live' | 'live_end'

/** The events, in display order. */
export const SOUND_NAMES: readonly SoundName[] = [
  'join', 'leave', 'mute', 'unmute', 'deafen', 'undeafen', 'message', 'live', 'live_end',
]
export const SOUND_LABELS: Record<SoundName, string> = {
  join: 'Join a call',
  leave: 'Leave a call',
  mute: 'Mute',
  unmute: 'Unmute',
  deafen: 'Deafen',
  undeafen: 'Undeafen',
  message: 'New message',
  live: 'Someone goes live',
  live_end: 'A stream ends',
}

export type SoundSetId =
  | 'classic' | 'twostep-bright' | 'twostep-warm' | 'twostep-deep'
  | 'minimal-soft' | 'minimal-wood' | 'minimal-glass' | 'goofy'
export const SOUND_SETS: readonly { id: SoundSetId; label: string; blurb: string }[] = [
  { id: 'classic', label: 'Classic', blurb: 'Soft sine chirps, the original sounds.' },
  { id: 'twostep-bright', label: 'Two-step · Bright', blurb: 'Two quick high notes, rising for on and falling for off.' },
  { id: 'twostep-warm', label: 'Two-step · Warm', blurb: 'The same two-note shape, mid register with a rounder tone.' },
  { id: 'twostep-deep', label: 'Two-step · Deep', blurb: 'The same two-note shape, low and full.' },
  { id: 'minimal-soft', label: 'Minimal · Soft', blurb: 'One short, soft note per event.' },
  { id: 'minimal-wood', label: 'Minimal · Wood', blurb: 'One short plucked, woody note.' },
  { id: 'minimal-glass', label: 'Minimal · Glass', blurb: 'One short, bell-like note.' },
  { id: 'goofy', label: 'Goofy', blurb: "Playful bleeps and zaps from Kenney's free packs." },
]

export type SoundPrefs = { set: SoundSetId; volume_pct: number; off: SoundName[] }
export const defaultSoundPrefs: SoundPrefs = { set: 'classic', volume_pct: 100, off: [] }

/** Clean whatever is in storage: anything unexpected falls back to the default for that field. */
export function normalizeSoundPrefs(raw: unknown): SoundPrefs {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return { ...defaultSoundPrefs, off: [] }
  const r = raw as Record<string, unknown>
  const set = SOUND_SETS.find((s) => s.id === r.set)?.id ?? defaultSoundPrefs.set
  const volume_pct =
    typeof r.volume_pct === 'number' && Number.isFinite(r.volume_pct)
      ? Math.round(Math.min(100, Math.max(0, r.volume_pct)))
      : defaultSoundPrefs.volume_pct
  const off = Array.isArray(r.off) ? SOUND_NAMES.filter((n) => (r.off as unknown[]).includes(n)) : []
  return { set, volume_pct, off }
}
export function loadSoundPrefs(): SoundPrefs {
  try {
    const raw = localStorage.getItem('pulse.sounds')
    return normalizeSoundPrefs(raw ? JSON.parse(raw) : null)
  } catch {
    return normalizeSoundPrefs(null)
  }
}
export const saveSoundPrefs = (p: SoundPrefs) => save('pulse.sounds', p)

export type Connection = 'connecting' | 'connected' | 'reconnecting' | 'disconnected'
export type NsLevel = 'off' | 'standard' | 'strong'

/** One report from the core's link sampler (`LinkReport` in voice/linkstats.rs). Hand-written: the
 * client's events are not part of the generated protocol bindings. */
export type LinkReport = {
  rtt_ms: number | null
  jitter_ms: number | null
  loss_pct: number | null
  transport: 'udp' | 'tcp' | 'turn' | null
  server: string | null
  version: string
  /** [ms ago, rtt ms], oldest first. */
  history: [number, number][]
}

/** The core's join progress; the UI folds these into a `StageState` (voicestage.ts). */
export type StageName = 'requesting_access' | 'connecting' | 'starting_audio' | 'connected' | 'reconnecting'

export type VoiceEvent =
  | { kind: 'state'; channel_id: string | null; connection: Connection; controls: Flags }
  | { kind: 'stage'; stage: StageName; attempt: number | null }
  | ({ kind: 'link' } & LinkReport)
  | { kind: 'speaking'; user_ids: string[] }
  | { kind: 'quality'; user_id: string; quality: string }
  | { kind: 'levels'; mic: number; speaker: number; gate_open: boolean }
  | { kind: 'noise_suppression'; active: NsLevel; note: string | null }
  | { kind: 'device_stalled' }
  | { kind: 'device_recovered' }
  | { kind: 'mic_silent'; silent: boolean }
  | { kind: 'mic'; live: boolean; error: string | null }

export const clampVolume = (pct: number) => Math.round(Math.min(200, Math.max(0, pct)))

export function volumeLabel(pct: number): string {
  const v = clampVolume(pct)
  // 0%, not "Muted": that reads as the person's own mute (Mute for me has its own mark)
  return `${v}%`
}

/** Which sound a mute/deafen transition plays (deafen wins over mute). */
export function soundFor(prev: Flags, next: Flags): SoundName | null {
  if (prev.deafened !== next.deafened) return next.deafened ? 'deafen' : 'undeafen'
  if (prev.muted !== next.muted) return next.muted ? 'mute' : 'unmute'
  return null
}

type VoiceSnap = { channelId: string | null; connection: string; controls: Flags }

/** Sounds for a voice state change. "Joined" = became connected to a channel we weren't already connected to. */
export function transitionSounds(prev: VoiceSnap, next: VoiceSnap): SoundName[] {
  const out: SoundName[] = []
  const wasIn = prev.connection === 'connected' || prev.connection === 'reconnecting'
  if (next.connection === 'connected' && next.channelId && (!wasIn || prev.channelId !== next.channelId)) out.push('join')
  if (prev.connection === 'connected' && !next.channelId) out.push('leave')
  const c = soundFor(prev.controls, next.controls)
  if (c) out.push(c)
  return out
}

function invokeSound(name: SoundName, set: SoundSetId, volume_pct: number) {
  // on the output chosen in settings, like voice (null = system default)
  void invoke('play_sound', { name, set, volume: volume_pct / 100, output: loadAudioConfig().output }).catch(() => {})
}

/** Played by the Rust core (never an <audio> element: WebKitGTK + missing GStreamer sink aborts the renderer). */
export function playSound(name: SoundName) {
  const { set, volume_pct, off } = loadSoundPrefs()
  if (volume_pct === 0 || off.includes(name)) return
  invokeSound(name, set, volume_pct)
}

/** What "Preview" plays: a few events, in a row. */
export const PREVIEW: readonly SoundName[] = ['join', 'mute', 'unmute', 'live', 'leave']
export const PREVIEW_GAP_MS = 450
let previewTimers: ReturnType<typeof setTimeout>[] = []

export function stopPreview() {
  previewTimers.forEach(clearTimeout)
  previewTimers = []
}

/** Plays PREVIEW in `set` at the current volume, ignoring the per-event switches. Replaces a running preview. */
export function previewSet(set: SoundSetId) {
  stopPreview()
  const { volume_pct } = loadSoundPrefs()
  if (volume_pct === 0) return
  PREVIEW.forEach((name, i) => {
    if (i === 0) invokeSound(name, set, volume_pct)
    else previewTimers.push(setTimeout(() => invokeSound(name, set, volume_pct), i * PREVIEW_GAP_MS))
  })
}

// Per-machine preferences (not shared state): safe in localStorage, guarded for private modes.
function load<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key)
    return raw ? { ...fallback, ...JSON.parse(raw) } : fallback
  } catch {
    return fallback
  }
}
function save(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value))
  } catch {
    /* ignore */
  }
}

export type AudioConfig = {
  input: string | null
  output: string | null
  input_gain_pct: number
  sensitivity: number
  echo_cancel: boolean
  noise_suppression: NsLevel
  auto_sensitivity: boolean
  auto_gain: boolean
}
export const defaultAudioConfig: AudioConfig = {
  input: null, output: null, input_gain_pct: 100, sensitivity: 0.02, echo_cancel: true,
  noise_suppression: 'strong', auto_sensitivity: true, auto_gain: false,
}
/** Settings saved before levels existed carried `noise_suppress: boolean`. */
export function migrateAudioConfig(raw: Record<string, unknown>): AudioConfig {
  const { noise_suppress, ...rest } = raw
  const c = { ...defaultAudioConfig, ...rest } as AudioConfig
  if (raw.noise_suppression === undefined && noise_suppress === false) c.noise_suppression = 'off'
  return c
}
export const loadAudioConfig = () => migrateAudioConfig(load<Record<string, unknown>>('pulse.audio', {}))
export const saveAudioConfig = (c: AudioConfig) => save('pulse.audio', c)

export const loadVolumes = () => load<Record<string, number>>('pulse.volumes', {})
export const saveVolumes = (v: Record<string, number>) => save('pulse.volumes', v)

/** People muted just for you ("Mute for me"): local only, remembered like volumes. */
export const loadLocalMutes = () => load<Record<string, true>>('pulse.localmutes', {})
export const saveLocalMutes = (m: Record<string, true>) => save('pulse.localmutes', m)
