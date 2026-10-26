import { describe, expect, it } from 'vitest'
import { isJoining, reduceStage, stageLabel, type StageState } from './voicestage'
import type { VoiceEvent } from './voiceui'

const flags = { muted: false, deafened: false }
const stage = (s: Extract<VoiceEvent, { kind: 'stage' }>['stage'], attempt: number | null = null): VoiceEvent => ({ kind: 'stage', stage: s, attempt })
const state = (connection: 'connecting' | 'connected' | 'reconnecting' | 'disconnected'): VoiceEvent => ({ kind: 'state', channel_id: 'V1', connection, controls: flags })
const run = (start: StageState, events: VoiceEvent[]) => events.reduce(reduceStage, start)
const idle: StageState = { stage: 'disconnected', attempt: null }

describe('reduceStage', () => {
  it('stages_follow_the_core_in_order', () => {
    let cur = idle
    const seen: string[] = []
    for (const s of ['requesting_access', 'connecting', 'starting_audio', 'connected'] as const) {
      cur = reduceStage(cur, stage(s))
      seen.push(cur.stage)
    }
    expect(seen).toEqual(['requesting_access', 'connecting', 'starting_audio', 'connected'])
    expect(reduceStage(cur, state('disconnected'))).toEqual(idle)
  })

  it('livekit_reconnect_then_resume_is_connected', () => {
    const cur = run({ stage: 'connected', attempt: null }, [state('reconnecting')])
    expect(cur).toEqual({ stage: 'reconnecting', attempt: null })
    expect(reduceStage(cur, state('connected'))).toEqual({ stage: 'connected', attempt: null })
  })

  it('rejoin_attempt_is_kept_through_state_reconnecting', () => {
    const cur = run({ stage: 'connected', attempt: null }, [stage('reconnecting', 2), state('reconnecting')])
    expect(cur).toEqual({ stage: 'reconnecting', attempt: 2 })
  })

  it('mic_error_while_starting_audio_counts_as_connected', () => {
    const cur: StageState = { stage: 'starting_audio', attempt: null }
    expect(reduceStage(cur, { kind: 'mic', live: false, error: 'no mic' }).stage).toBe('connected')
    expect(reduceStage(cur, { kind: 'mic', live: true, error: null }).stage).toBe('starting_audio')
    expect(reduceStage({ stage: 'connecting', attempt: null }, { kind: 'mic', live: false, error: 'x' }).stage).toBe('connecting')
  })

  it('state_connected_leaves_other_stages_alone', () => {
    expect(reduceStage({ stage: 'starting_audio', attempt: null }, state('connected')).stage).toBe('starting_audio')
  })

  it('labels_match_the_spec_copy', () => {
    const l = (s: StageState['stage'], attempt: number | null = null) => stageLabel({ stage: s, attempt })
    expect(l('requesting_access')).toBe('Requesting access')
    expect(l('connecting')).toBe('Connecting to voice')
    expect(l('starting_audio')).toBe('Starting audio')
    expect(l('connected')).toBe('Connected')
    expect(l('reconnecting')).toBe('Reconnecting…')
    expect(l('reconnecting', 2)).toBe('Reconnecting… (attempt 2)')
    expect(l('disconnected')).toBe('Disconnected')
  })

  it('only the first two stages are joining', () => {
    expect(['requesting_access', 'connecting', 'starting_audio', 'connected'].map((s) => isJoining(s as StageState['stage']))).toEqual([true, true, false, false])
  })
})
