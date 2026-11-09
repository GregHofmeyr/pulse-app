import { invoke } from '@tauri-apps/api/core'
import { loadMixer, ringtoneGain, soundGain, soundMasterGain } from './mixer'
import type { Event as AppEvent } from './protocol/Event'
// Pure helpers for the voice UI (tested in voiceui.test.ts).

/** Your own mute / deafen, and (from the core) the server's of you (#140), which the core adds. */
export type Flags = { muted: boolean; deafened: boolean; server_muted?: boolean; server_deafened?: boolean }
export type SoundName =
  | 'join' | 'leave' | 'mute' | 'unmute' | 'deafen' | 'undeafen' | 'muted_talk' | 'message' | 'live' | 'live_end' | 'badge'

/** The events, in display order. */
export const SOUND_NAMES: readonly SoundName[] = [
  'join', 'leave', 'mute', 'unmute', 'deafen', 'undeafen', 'muted_talk', 'message', 'live', 'live_end', 'badge',
]
export const SOUND_LABELS: Record<SoundName, string> = {
  join: 'Join a call',
  leave: 'Leave a call',
  mute: 'Mute',
  unmute: 'Unmute',
  deafen: 'Deafen',
  undeafen: 'Undeafen',
  muted_talk: 'Talking while muted',
  message: 'New message',
  live: 'Someone goes live',
  live_end: 'A stream ends',
  badge: 'Badge unlocked',
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

/** The incoming-call ringtones (#174), one wav each in `src-tauri/ringtone` (Classic is `ring.wav`). Not part of a
 * sound set: the one you pick rings whatever the set. */
export type RingtoneId = 'classic' | 'marimba' | 'chiptune' | 'phone'
export const RINGTONES: readonly { id: RingtoneId; label: string; blurb: string }[] = [
  { id: 'classic', label: 'Classic', blurb: 'Two soft rising trills, the original ring.' },
  { id: 'marimba', label: 'Marimba', blurb: 'A gentle wooden mallet figure, up and back down.' },
  { id: 'chiptune', label: 'Chiptune', blurb: 'A quick retro arpeggio, like an old handheld.' },
  { id: 'phone', label: 'Classic phone', blurb: 'The two-tone ring-ring of an old desk phone.' },
]

/** `volume_pct` and `off` are from before the volume mixer: read only to migrate them (lib/mixer.ts). `ringtone` is
 * the one incoming calls ring with (#174), on this device. */
export type SoundPrefs = { set: SoundSetId; volume_pct: number; off: SoundName[]; ringtone: RingtoneId }
export const defaultSoundPrefs: SoundPrefs = { set: 'classic', volume_pct: 100, off: [], ringtone: 'classic' }

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
  const ringtone = RINGTONES.find((t) => t.id === r.ringtone)?.id ?? defaultSoundPrefs.ringtone
  return { set, volume_pct, off, ringtone }
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
  /** You've been talking into a muted mic (reported for The Doos badge). */
  | { kind: 'muted_talk' }
  /** You're talking into your own muted mic: play the "you're muted" double ding (#252). The core paces it (every
   * 12 s while you keep talking, again at once after a re-mute) and never sends it while you're deafened. */
  | { kind: 'muted_cue' }

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

/** Someone else joining or leaving the call you're in (#219): the same join and leave chirps (and mixer notches) as
 * your own. Yours come from the connection (`transitionSounds`), so your own events are left out here; mid-join or
 * reconnecting you hear nothing, as the roster settles. */
export function peerCallSound(e: AppEvent, me: { myId: string | undefined; channelId: string | null; connection: Connection }): SoundName | null {
  if (e.t !== 'VoiceJoined' && e.t !== 'VoiceLeft') return null
  if (!me.channelId || me.connection !== 'connected' || e.d.channel_id !== me.channelId || e.d.user_id === me.myId) return null
  return e.t === 'VoiceJoined' ? 'join' : 'leave'
}

function invokeSound(name: SoundName, set: SoundSetId, gain: number) {
  if (!(gain > 0)) return
  // on the output chosen in settings, like voice (null = system default); the core caps and soft-limits the gain
  void invoke('play_sound', { name, set, volume: gain, output: loadAudioConfig().output }).catch(() => {})
}

/** Played by the Rust core (never an <audio> element: WebKitGTK + missing GStreamer sink aborts the renderer), at its
 * volume mixer level (section × sound; 0 % plays nothing). */
export function playSound(name: SoundName) {
  invokeSound(name, loadSoundPrefs().set, soundGain(loadMixer(), name))
}

/** One sound at a given linear gain (the mixer's ▶ test buttons). */
export function playSoundAt(name: SoundName, gain: number) {
  invokeSound(name, loadSoundPrefs().set, gain)
}

function invokeRinging(ringtone: RingtoneId, gain: number) {
  if (!(gain > 0)) return
  const { set } = loadSoundPrefs()
  void invoke('play_sound', { name: 'ringing', set, volume: gain, output: loadAudioConfig().output, ringtone }).catch(() => {})
}

/** One ring of the chosen ringtone at a given linear gain (the mixer's Ringing ▶, #50). The core plays the looping
 * ringtone itself while a call rings (`start_ringtone`); this is a single cycle of it. */
export function playRingingAt(gain: number) {
  invokeRinging(loadSoundPrefs().ringtone, gain)
}

/** The ringtone picker's ▶ (#174): one ring of `id` at Call sounds × Ringing, without choosing it. */
export function previewRingtone(id: RingtoneId) {
  invokeRinging(id, ringtoneGain(loadMixer()))
}

/** What "Preview" plays: a few events, in a row. */
export const PREVIEW: readonly SoundName[] = ['join', 'mute', 'unmute', 'live', 'leave']
export const PREVIEW_GAP_MS = 450
let previewTimers: ReturnType<typeof setTimeout>[] = []

export function stopPreview() {
  previewTimers.forEach(clearTimeout)
  previewTimers = []
}

/** Plays PREVIEW in `set`, each at its section's master (the per-sound notches don't apply). Replaces a running preview. */
export function previewSet(set: SoundSetId) {
  stopPreview()
  const mix = loadMixer()
  PREVIEW.forEach((name, i) => {
    const play = () => invokeSound(name, set, soundMasterGain(mix, name))
    if (i === 0) play()
    else previewTimers.push(setTimeout(play, i * PREVIEW_GAP_MS))
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

/** Echo cancellation (#76): Auto turns it off when the output is headphones or a headset (the core decides, per device). */
export type EchoMode = 'auto' | 'on' | 'off'
export type AudioConfig = {
  input: string | null
  output: string | null
  input_gain_pct: number
  sensitivity: number
  echo_mode: EchoMode
  noise_suppression: NsLevel
  auto_sensitivity: boolean
  auto_gain: boolean
}
export const defaultAudioConfig: AudioConfig = {
  input: null, output: null, input_gain_pct: 100, sensitivity: 0.02, echo_mode: 'auto',
  noise_suppression: 'strong', auto_sensitivity: true, auto_gain: false,
}
/** Settings saved before levels existed carried `noise_suppress: boolean`; before #76, `echo_cancel: boolean` (off by
 *  hand stays Off, the old default on becomes Auto). */
export function migrateAudioConfig(raw: Record<string, unknown>): AudioConfig {
  const { noise_suppress, echo_cancel, ...rest } = raw
  const c = { ...defaultAudioConfig, ...rest } as AudioConfig
  if (raw.noise_suppression === undefined && noise_suppress === false) c.noise_suppression = 'off'
  if (raw.echo_mode === undefined && echo_cancel === false) c.echo_mode = 'off'
  return c
}
export const loadAudioConfig = () => migrateAudioConfig(load<Record<string, unknown>>('pulse.audio', {}))
export const saveAudioConfig = (c: AudioConfig) => save('pulse.audio', c)

export const loadVolumes = () => load<Record<string, number>>('pulse.volumes', {})
export const saveVolumes = (v: Record<string, number>) => save('pulse.volumes', v)

/** People muted just for you ("Mute for me"): local only, remembered like volumes. */
export const loadLocalMutes = () => load<Record<string, true>>('pulse.localmutes', {})
export const saveLocalMutes = (m: Record<string, true>) => save('pulse.localmutes', m)
