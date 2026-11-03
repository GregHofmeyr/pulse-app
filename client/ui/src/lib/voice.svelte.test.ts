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

const me = { id: 'ME', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
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

  it('leaves the call of a voice channel that was deleted, and only that one', () => {
    const deleted = (channel_id: string) => h.handlers['pulse://event']({ payload: { t: 'ChannelDeleted', d: { channel_id, server_id: 'S1' } } })
    voice.channelId = 'V1'
    deleted('V2')
    expect(h.calls).toEqual([])
    deleted('V1')
    expect(h.calls).toEqual(['leave_voice'])
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

describe('talking into a muted mic', () => {
  beforeEach(async () => {
    h.calls.length = 0
    await startVoiceListening()
  })

  it('is reported for The Doos', () => {
    h.handlers['voice://event']({ payload: { kind: 'muted_talk' } })
    expect(h.calls).toEqual(['report_muted_talk'])
  })
})

describe('joining', () => {
  beforeEach(() => {
    h.calls.length = 0
    Object.assign(voice, { channelId: null, joining: null, leaving: null })
  })
  it('hands the core the volume mixer’s Calls levels before it joins', async () => {
    const { voiceApi } = await import('./voice.svelte')
    await voiceApi.join('V1')
    expect(h.calls).toContain('set_call_mix')
    expect(h.calls.indexOf('set_call_mix')).toBeLessThan(h.calls.indexOf('join_voice'))
  })
})

describe('server mute and disconnect (#140)', () => {
  const lounge = { id: 'V1', server_id: 'S1', kind: 'voice' as const, name: 'Lounge', position: 0 }
  const event = (payload: unknown) => h.handlers['pulse://event']({ payload })
  beforeEach(async () => {
    h.calls.length = 0
    app.state = { ...emptyState(), me, channels: { V1: lounge } }
    Object.assign(voice, { channelId: null, joining: null, leaving: null, disconnectedBy: null, controls: { muted: false, deafened: false } })
    await startListening()
    await startVoiceListening()
  })

  it('tells the core when you are server muted in your call’s server, and when it is undone', async () => {
    const { flushSync } = await import('svelte')
    voice.channelId = 'V1'
    flushSync()
    expect(h.calls).not.toContain('set_server_voice')
    event({ t: 'ServerVoiceChanged', d: { state: { server_id: 'S1', user_id: 'ME', muted_by: 'U2', deafened_by: null } } })
    flushSync()
    expect(h.calls).toContain('set_server_voice')
    // the core answers with its state
    voice.controls = { muted: false, deafened: false, server_muted: true, server_deafened: false }
    h.calls.length = 0
    flushSync()
    expect(h.calls).toEqual([])
    event({ t: 'ServerVoiceChanged', d: { state: { server_id: 'S1', user_id: 'ME', muted_by: null, deafened_by: null } } })
    flushSync()
    expect(h.calls).toEqual(['set_server_voice'])
  })

  it('remembers who disconnected you until you join again', async () => {
    const { voiceApi } = await import('./voice.svelte')
    voice.channelId = 'V1'
    event({ t: 'VoiceDisconnected', d: { channel_id: 'V1', user_id: 'ME', by: 'U2' } })
    expect(voice.disconnectedBy).toEqual({ by: 'U2', channelId: 'V1' })
    await voiceApi.join('V1')
    expect(voice.disconnectedBy).toBeNull()
  })
})
