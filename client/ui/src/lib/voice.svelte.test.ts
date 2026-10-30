// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'

const h = vi.hoisted(() => ({ handlers: {} as Record<string, (e: { payload: unknown }) => void>, calls: [] as string[] }))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string) => {
    h.calls.push(cmd)
    return undefined
  }),
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, cb: (e: { payload: unknown }) => void) => {
    h.handlers[name] = cb
    return () => {}
  },
}))

import { app, startListening } from './store.svelte'
import { emptyState } from './state'
import { startVoiceListening, voice } from './voice.svelte'

const me = { id: 'ME', username: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const removed = (channel_id: string, user_id = 'ME') => h.handlers['pulse://event']({ payload: { t: 'ChannelRemoved', d: { channel_id, user_id } } })

describe('removed from a conversation', () => {
  let off: (() => void) | undefined
  beforeEach(async () => {
    h.calls.length = 0
    app.state = { ...emptyState(), me }
    Object.assign(voice, { channelId: null, joining: null, leaving: null })
    await startListening()
    off?.()
    off = await startVoiceListening()
  })

  it('leaves the call in it, through the normal leave', () => {
    voice.channelId = 'G1'
    removed('G1')
    expect(h.calls).toEqual(['leave_voice'])
    expect(voice.leaving).toBe('G1')
  })

  it('calls off a join to it', () => {
    voice.joining = 'G1'
    removed('G1')
    expect(h.calls).toEqual(['leave_voice'])
  })

  it('leaves a call elsewhere alone', () => {
    voice.channelId = 'G2'
    removed('G1')
    expect(h.calls).toEqual([])
  })

  it('leaves alone a call it is already leaving for another join', () => {
    voice.channelId = 'G1'
    voice.leaving = 'G1'
    voice.joining = 'C9'
    removed('G1')
    expect(h.calls).toEqual([])
  })
})

describe('the mic event', () => {
  const mic = (live: boolean, error: string | null) => h.handlers['voice://event']({ payload: { kind: 'mic', live, error } })
  beforeEach(async () => {
    Object.assign(voice, { channelId: 'V1', error: '', stage: { stage: 'starting_audio', attempt: null } })
    await startVoiceListening()
  })

  it('a mic that failed to publish shows its error, and you are still in the room', () => {
    mic(false, 'Microphone unavailable')
    expect(voice.error).toBe('Microphone unavailable')
    expect(voice.stage.stage).toBe('connected')
  })

  it('a mic that went live leaves the error alone', () => {
    voice.error = 'earlier trouble'
    mic(true, null)
    expect(voice.error).toBe('earlier trouble')
  })
})
