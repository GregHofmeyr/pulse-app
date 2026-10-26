// Voice state from the Rust VoiceManager (voice://event).
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { streamApi } from './stream/stream.svelte'
import { loadAudioConfig, loadVolumes, playSound, saveVolumes, transitionSounds, type AudioConfig, type Flags, type NsLevel } from './voiceui'

type Connection = 'connecting' | 'connected' | 'reconnecting' | 'disconnected'
type VoiceEvent =
  | { kind: 'state'; channel_id: string | null; connection: Connection; controls: Flags }
  | { kind: 'speaking'; user_ids: string[] }
  | { kind: 'quality'; user_id: string; quality: string }
  | { kind: 'levels'; mic: number; speaker: number; gate_open: boolean }
  | { kind: 'noise_suppression'; active: NsLevel; note: string | null }
  | { kind: 'device_stalled' }
  | { kind: 'device_recovered' }
  | { kind: 'mic'; live: boolean; error: string | null }

export const voice = $state({
  channelId: null as string | null,
  connection: 'disconnected' as Connection,
  controls: { muted: false, deafened: false } as Flags,
  speaking: new Set<string>(),
  quality: {} as Record<string, string>,
  levels: { mic: 0, speaker: 0 },
  /** Whether the voice gate is letting the mic through. */
  gateOpen: false,
  /** Suppression actually running (Strong can be loading or have fallen back) and why. */
  ns: { active: null as NsLevel | null, note: null as string | null },
  stalled: false,
  /** Our mic is in the room (shows a moment after connected). */
  micLive: false,
  error: '',
  volumes: loadVolumes() as Record<string, number>,
})

export async function startVoiceListening() {
  return listen<VoiceEvent>('voice://event', ({ payload: e }) => {
    switch (e.kind) {
      case 'state': {
        const prev = { channelId: voice.channelId, connection: voice.connection, controls: voice.controls }
        for (const snd of transitionSounds(prev, { channelId: e.channel_id, connection: e.connection, controls: e.controls })) playSound(snd)
        voice.channelId = e.channel_id
        voice.connection = e.connection
        voice.controls = e.controls
        // a stream never outlives your voice channel: leaving or switching ends yours and what you watch
        if (prev.channelId !== e.channel_id) void streamApi.onVoiceChanged(e.channel_id)
        if (!e.channel_id) voice.speaking = new Set()
        if (!e.channel_id) voice.micLive = false
        break
      }
      case 'speaking':
        voice.speaking = new Set(e.user_ids)
        break
      case 'quality':
        voice.quality = { ...voice.quality, [e.user_id]: e.quality }
        break
      case 'levels':
        voice.levels = { mic: e.mic, speaker: e.speaker }
        voice.gateOpen = e.gate_open
        break
      case 'noise_suppression':
        voice.ns = { active: e.active, note: e.note }
        break
      case 'device_stalled':
        voice.stalled = true
        break
      case 'device_recovered':
        voice.stalled = false
        break
      case 'mic':
        voice.micLive = e.live
        if (e.error) voice.error = e.error
        break
    }
  })
}

const errText = (e: unknown) => (typeof e === 'string' ? e : 'Voice error')

export const voiceApi = {
  async join(channelId: string) {
    voice.error = ''
    voice.stalled = false
    try {
      await invoke('join_voice', { channelId, config: loadAudioConfig() })
      // re-apply remembered per-user volumes
      for (const [userId, percent] of Object.entries(voice.volumes)) await invoke('set_peer_volume', { userId, percent })
    } catch (e) {
      voice.error = errText(e)
    }
  },
  /** Hovering a voice channel: the core fetches its join token ahead of the click. Resolves once
   *  the core has taken the request (it fetches in the background and keeps the token). */
  prefetch: (channelId: string): Promise<void> => invoke('prefetch_voice', { channelId }),
  leave: () => invoke('leave_voice'),
  toggleMute: () => invoke('toggle_mute'),
  toggleDeafen: () => invoke('toggle_deafen'),
  async setVolume(userId: string, percent: number) {
    voice.volumes = { ...voice.volumes, [userId]: percent }
    saveVolumes(voice.volumes)
    await invoke('set_peer_volume', { userId, percent })
  },
  setAudioConfig: (config: AudioConfig) => invoke('set_audio_config', { config }),
  listDevices: () => invoke<{ inputs: { name: string; is_default: boolean }[]; outputs: { name: string; is_default: boolean }[] }>('list_audio_devices'),
  startMicTest: (config: AudioConfig) => invoke('start_mic_test', { config }),
  stopMicTest: () => invoke('stop_mic_test'),
  /** Levels are only sent while something shows them: call with true when a meter appears, false when it goes. */
  watchMeters: (on: boolean) => invoke('watch_voice_meters', { on }),
}
