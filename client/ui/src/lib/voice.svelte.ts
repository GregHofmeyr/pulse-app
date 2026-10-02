// Voice state from the Rust VoiceManager (voice://event).
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { loadAudioConfig, loadVolumes, playSound, saveVolumes, transitionSounds, type AudioConfig, type Flags } from './voiceui'

type Connection = 'connecting' | 'connected' | 'reconnecting' | 'disconnected'
type VoiceEvent =
  | { kind: 'state'; channel_id: string | null; connection: Connection; controls: Flags }
  | { kind: 'speaking'; user_ids: string[] }
  | { kind: 'quality'; user_id: string; quality: string }
  | { kind: 'levels'; mic: number; speaker: number }
  | { kind: 'device_stalled' }
  | { kind: 'device_recovered' }

export const voice = $state({
  channelId: null as string | null,
  connection: 'disconnected' as Connection,
  controls: { muted: false, deafened: false } as Flags,
  speaking: new Set<string>(),
  quality: {} as Record<string, string>,
  levels: { mic: 0, speaker: 0 },
  stalled: false,
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
        if (!e.channel_id) voice.speaking = new Set()
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
        break
      case 'device_stalled':
        voice.stalled = true
        break
      case 'device_recovered':
        voice.stalled = false
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
}
