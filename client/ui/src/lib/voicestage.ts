// Join progress as the UI shows it, folded from voice://event (tested in voicestage.test.ts).
import type { StageName, VoiceEvent } from './voiceui'

export type Stage = StageName | 'disconnected'
export type StageState = { stage: Stage; attempt: number | null }

export function reduceStage(cur: StageState, e: VoiceEvent): StageState {
  switch (e.kind) {
    case 'stage':
      return { stage: e.stage, attempt: e.attempt }
    case 'state':
      if (e.connection === 'disconnected') return { stage: 'disconnected', attempt: null }
      if (e.connection === 'reconnecting') {
        // LiveKit's own reconnect carries no attempt; a rejoin's `stage` event arrived first and keeps its number
        return { stage: 'reconnecting', attempt: cur.stage === 'reconnecting' ? cur.attempt : null }
      }
      if (e.connection === 'connected' && cur.stage === 'reconnecting') return { stage: 'connected', attempt: null }
      return cur
    case 'mic':
      // microphone trouble is not a failed join: you are in the room
      return e.error && cur.stage === 'starting_audio' ? { stage: 'connected', attempt: null } : cur
    default:
      return cur
  }
}

export function stageLabel({ stage, attempt }: StageState): string {
  switch (stage) {
    case 'requesting_access': return 'Requesting access'
    case 'connecting': return 'Connecting to voice'
    case 'starting_audio': return 'Starting audio'
    case 'connected': return 'Connected'
    case 'reconnecting': return attempt ? `Reconnecting… (attempt ${attempt})` : 'Reconnecting…'
    case 'disconnected': return 'Disconnected'
  }
}

export const isJoining = (stage: Stage) => stage === 'requesting_access' || stage === 'connecting'
