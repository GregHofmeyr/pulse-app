import { invoke } from '@tauri-apps/api/core'
// Pure helpers for the voice UI (tested in voiceui.test.ts).

export type Flags = { muted: boolean; deafened: boolean }
export type SoundName = 'join' | 'leave' | 'mute' | 'unmute' | 'deafen' | 'undeafen' | 'message'

export const clampVolume = (pct: number) => Math.round(Math.min(200, Math.max(0, pct)))

export function volumeLabel(pct: number): string {
  const v = clampVolume(pct)
  return v === 0 ? 'Muted' : `${v}%`
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

/** Played by the Rust core (never an <audio> element: WebKitGTK + missing GStreamer sink aborts the renderer). */
export function playSound(name: SoundName) {
  void invoke('play_sound', { name }).catch(() => {})
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

export type NsLevel = 'off' | 'standard' | 'strong'
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
