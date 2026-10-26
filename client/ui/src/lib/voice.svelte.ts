// Voice state from the Rust VoiceManager (voice://event).
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { streamApi } from './stream/stream.svelte'
import { app } from './store.svelte'
import { loadAudioConfig, loadVolumes, playSound, saveVolumes, transitionSounds, type AudioConfig, type Connection, type Flags, type LinkReport, type NsLevel, type VoiceEvent } from './voiceui'
import { isJoining, reduceStage, type StageState } from './voicestage'

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
  /** Join progress, folded from the core's stage and state events. */
  stage: { stage: 'disconnected', attempt: null } as StageState,
  /** The channel being joined: set on click, cleared once audio starts or the join fails. */
  joining: null as string | null,
  /** The channel just left, until the server's list drops you. */
  leaving: null as string | null,
  joinFailed: null as { channelId: string; error: string } | null,
  /** Latest link report (ping, jitter, loss...). */
  link: null as LinkReport | null,
  volumes: loadVolumes() as Record<string, number>,
})

// Forget `leaving` once the server's list no longer has you in that channel. The one settle path:
// it follows app state wherever the update lands, whether or not any component is mounted.
$effect.root(() => {
  $effect(() => {
    const ch = voice.leaving
    if (ch && !(app.state.voice[ch] ?? []).some((m) => m.user_id === app.state.me?.id)) voice.leaving = null
  })
})

/** The newest join; an older one's result is stale. */
let joinSeq = 0

export async function startVoiceListening() {
  return listen<VoiceEvent>('voice://event', ({ payload: e }) => {
    voice.stage = reduceStage(voice.stage, e)
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
      case 'link':
        voice.link = e
        break
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
    // the optimistic join ends once audio is starting in the channel being joined: not on the audio stage
    // alone, since a cancelled join's disconnect can land after it and leave channelId unset for a moment
    if (voice.joining && voice.channelId === voice.joining && voice.stage.stage !== 'disconnected' && !isJoining(voice.stage.stage)) voice.joining = null
  })
}

const errText = (e: unknown) => (typeof e === 'string' ? e : 'Voice error')

export const voiceApi = {
  async join(channelId: string) {
    voice.error = ''
    voice.stalled = false
    // show the join at once, before the core has answered
    voice.joinFailed = null
    voice.leaving = voice.channelId ?? voice.leaving
    voice.joining = channelId
    const seq = ++joinSeq
    voice.stage = { stage: 'requesting_access', attempt: null }
    try {
      await invoke('join_voice', { channelId, config: loadAudioConfig() })
    } catch (e) {
      // a newer join cancelled this one: not a failure
      if (e === 'cancelled' || seq !== joinSeq) return
      voice.error = errText(e)
      voice.joinFailed = { channelId, error: errText(e) }
      voice.joining = null
      voice.stage = { stage: 'disconnected', attempt: null }
      return
    }
    try {
      // re-apply remembered per-user volumes
      for (const [userId, percent] of Object.entries(voice.volumes)) await invoke('set_peer_volume', { userId, percent })
    } catch (e) {
      voice.error = errText(e)
    }
  },
  retry: async () => {
    if (voice.joinFailed) await voiceApi.join(voice.joinFailed.channelId)
  },
  /** Hovering a voice channel: the core fetches its join token ahead of the click. */
  prefetch: (channelId: string) => void invoke('prefetch_voice', { channelId }).catch(() => {}),
  leave() {
    // your row goes at once, not when the server's list catches up
    voice.leaving = voice.channelId ?? voice.joining ?? voice.leaving
    voice.joining = null
    ++joinSeq
    return invoke('leave_voice')
  },
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
  /** Link stats tick every second only while the connection panel is open and the window focused. */
  watchLink: (on: boolean) => invoke('watch_voice_link', { on }),
  /** Levels are only sent while something shows them: call with true when a meter appears, false when it goes. */
  watchMeters: (on: boolean) => invoke('watch_voice_meters', { on }),
}
