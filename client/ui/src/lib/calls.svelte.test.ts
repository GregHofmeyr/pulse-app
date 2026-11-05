// @vitest-environment jsdom
// The ring store (#50): popover vs band, the ringtone, Join / Decline, DND and mute gating, the OS notification.
import { flushSync } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import { invoke } from '@tauri-apps/api/core'
import { app, handleReady, resetState, ui } from './store.svelte'
import { calls, callUser, declineCall, handleCallEvent, joinCall, popoverRings, resetCalls, ringIn, startCall } from './calls.svelte'
import { voice } from './voice.svelte'
import { windowFocus } from './windowFocus.svelte'
import { setNotificationsEnabled } from './notifyPrefs.svelte'
import { applyEvent } from './state'
import { testReady } from './testReady'
import type { CallRing } from './protocol/CallRing'
import type { Message } from './protocol/Message'
import type { OwnStatus } from './protocol/OwnStatus'
import type { Ready } from './protocol/Ready'

const user = (id: string, username: string) => ({ id, username, display_name: username, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const DM = { id: 'DM1', server_id: null, kind: 'dm' as const, name: null, position: 0 }
const GROUP = { id: 'G1', server_id: null, kind: 'group' as const, name: 'Friends', position: 0 }
const NOW = Date.UTC(2026, 10, 5, 10)
const ready = (over: Partial<Ready> = {}) =>
  testReady([DM, GROUP], {
    me: user('U1', 'alex'),
    dm_members: [{ channel_id: 'DM1', user_ids: ['U1', 'U2'] }, { channel_id: 'G1', user_ids: ['U1', 'U2', 'U3'] }],
    people: [
      { user: user('U1', 'alex'), online: true, last_seen_at: null, status: 'online' },
      { user: user('U2', 'sam'), online: true, last_seen_at: null, status: 'online' },
      { user: user('U3', 'jo'), online: true, last_seen_at: null, status: 'online' },
    ],
    read_states: [{ channel_id: 'DM1', last_read_message_id: null, unread: 0, mentions: 0 }],
    ...over,
  })
const ring = (channel = 'DM1', id = 'C1'): CallRing => ({ call_id: id, channel_id: channel, caller_id: 'U2', expires_at: new Date(NOW + 30_000).toISOString() })
const ringing = (r: CallRing) => {
  handleCallEvent({ t: 'CallRinging', d: { ring: r } })
  flushSync()
}
const cmds = (name: string) => vi.mocked(invoke).mock.calls.filter(([c]) => c === name)

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(NOW)
  resetState()
  resetCalls()
  flushSync()
  handleReady(ready())
  ui.openChannelId = null
  ui.goTo = null
  windowFocus.focused = true
  setNotificationsEnabled(true)
  voice.channelId = null
  voice.joining = null
  voice.leaving = null
  flushSync()
  vi.mocked(invoke).mockClear()
})
afterEach(() => {
  resetCalls()
  flushSync()
  vi.useRealTimers()
})

describe('where a ring shows', () => {
  it('a popover outside the conversation, the band (no popover) inside it', () => {
    ringing(ring())
    expect(popoverRings()).toHaveLength(1)
    expect(ringIn('DM1')).not.toBeNull()
    ui.openChannelId = 'DM1'
    flushSync()
    expect(popoverRings()).toEqual([])
    expect(ringIn('DM1')?.caller_id).toBe('U2')
    // another conversation open: the popover is back
    ui.openChannelId = 'G1'
    expect(popoverRings()).toHaveLength(1)
  })
  it('a ring from Ready (a second device connecting mid-ring) shows too', () => {
    handleReady(ready({ ringing: [ring('G1')] }))
    flushSync()
    expect(ringIn('G1')).not.toBeNull()
  })
})

describe('the ringtone', () => {
  it('plays while ringing, at the mixer level, and stops on Decline, which tells the server', async () => {
    localStorage.setItem('pulse.mixer', JSON.stringify({ v: 1, callSounds: { master: 100, ringing: 50 } }))
    ringing(ring())
    expect(cmds('start_ringtone')).toHaveLength(1)
    expect((cmds('start_ringtone')[0][1] as { volume: number }).volume).toBeLessThan(1)
    await declineCall('DM1')
    flushSync()
    expect(cmds('decline_call')).toEqual([['decline_call', { channelId: 'DM1' }]])
    expect(cmds('stop_ringtone')).toHaveLength(1)
    expect(calls.rings).toEqual([])
    localStorage.removeItem('pulse.mixer')
  })
  it('rings the ringtone picked in Settings (#174), Classic when none is', () => {
    ringing(ring())
    expect(cmds('start_ringtone')[0][1]).toMatchObject({ ringtone: 'classic' })
    resetCalls()
    flushSync()
    vi.mocked(invoke).mockClear()
    localStorage.setItem('pulse.sounds', JSON.stringify({ set: 'goofy', ringtone: 'chiptune' }))
    ringing(ring())
    expect(cmds('start_ringtone')[0][1]).toMatchObject({ ringtone: 'chiptune' })
    localStorage.removeItem('pulse.sounds')
  })
  it('stops when the ring ends on another device, and only for my own ring end', () => {
    ringing(ring())
    handleCallEvent({ t: 'CallRingEnded', d: { channel_id: 'DM1', call_id: 'C1', user_id: 'U3', reason: 'answered' } })
    flushSync()
    expect(cmds('stop_ringtone')).toEqual([])
    handleCallEvent({ t: 'CallRingEnded', d: { channel_id: 'DM1', call_id: 'C1', user_id: 'U1', reason: 'answered' } })
    flushSync()
    expect(cmds('stop_ringtone')).toHaveLength(1)
    expect(ringIn('DM1')).toBeNull()
  })
  it('stops when the call ends, and on a timeout', () => {
    ringing(ring('DM1', 'C1'))
    ringing(ring('G1', 'C2'))
    expect(cmds('start_ringtone')).toHaveLength(1)
    handleCallEvent({ t: 'CallEnded', d: { channel_id: 'DM1', call_id: 'C1' } })
    flushSync()
    expect(cmds('stop_ringtone'), 'G1 still rings').toEqual([])
    handleCallEvent({ t: 'CallRingEnded', d: { channel_id: 'G1', call_id: 'C2', user_id: 'U1', reason: 'timed_out' } })
    flushSync()
    expect(cmds('stop_ringtone')).toHaveLength(1)
  })
  it('a ring stops by itself at its expiry (a missed ring end)', () => {
    ringing(ring())
    vi.advanceTimersByTime(29_000)
    flushSync()
    expect(ringIn('DM1')).not.toBeNull()
    vi.advanceTimersByTime(1_500)
    flushSync()
    expect(ringIn('DM1')).toBeNull()
    expect(cmds('stop_ringtone')).toHaveLength(1)
  })
  it('the caller is never rung', () => {
    handleCallEvent({ t: 'CallRinging', d: { ring: { ...ring(), caller_id: 'U1' } } })
    flushSync()
    expect(calls.rings).toEqual([])
    expect(cmds('start_ringtone')).toEqual([])
  })
})

describe('Join', () => {
  it('goes to the conversation, leaves other voice, joins the call and stops the ringtone', async () => {
    voice.channelId = 'V1'
    ringing(ring())
    const joined = joinCall('DM1')
    // the call you were in is being left, straight away (one voice connection per client)
    expect(voice.leaving).toBe('V1')
    expect(voice.joining).toBe('DM1')
    await joined
    flushSync()
    expect(ui.goTo).toBe('DM1')
    expect(cmds('join_voice')).toHaveLength(1)
    expect((cmds('join_voice')[0][1] as { channelId: string }).channelId).toBe('DM1')
    expect(cmds('stop_ringtone')).toHaveLength(1)
    expect(cmds('decline_call')).toEqual([])
  })
  it('the phone button starts the call, then connects', async () => {
    await startCall('G1')
    expect(cmds('start_call')).toEqual([['start_call', { channelId: 'G1' }]])
    expect((cmds('join_voice')[0][1] as { channelId: string }).channelId).toBe('G1')
  })
  it('calling someone opens your DM with them first', async () => {
    vi.mocked(invoke).mockImplementation(async (c) => (c === 'create_dm' ? DM : undefined) as never)
    await callUser('U2')
    expect(cmds('create_dm')).toHaveLength(1)
    expect(ui.goTo).toBe('DM1')
    expect(cmds('start_call')).toEqual([['start_call', { channelId: 'DM1' }]])
    vi.mocked(invoke).mockImplementation(async () => undefined)
  })
})

describe('Do Not Disturb and muted conversations', () => {
  const line: Message = {
    id: 'L1', channel_id: 'DM1', author_id: null, kind: 'system', content: 'sam started a call', reply_to_id: null,
    created_at: new Date(NOW).toISOString(), edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
    quote: null, qoty_card: null, clip: null, embeds: [], call: { call_id: 'C1', started_by: 'U2', ended_at: null, joined: [] },
  }
  const dnd: OwnStatus = { status: 'dnd', dnd_until: null }

  it('DND: no popover, no ringtone, no notification; the band and the line still show', () => {
    handleReady(ready({ status: dnd }))
    windowFocus.focused = false
    flushSync()
    vi.mocked(invoke).mockClear()
    ringing(ring())
    app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: line, nonce: null } }, Date.now())
    flushSync()
    expect(popoverRings()).toEqual([])
    expect(cmds('start_ringtone')).toEqual([])
    expect(cmds('notify_message')).toEqual([])
    expect(ringIn('DM1')).not.toBeNull()
    expect(app.state.reads.DM1.unread).toBe(1)
  })
  it('a muted conversation rings silently; another one still rings', () => {
    handleReady(ready({ mutes: [{ target_kind: 'channel', target_id: 'DM1', until: null }] }))
    windowFocus.focused = false
    flushSync()
    vi.mocked(invoke).mockClear()
    ringing(ring())
    expect(popoverRings()).toEqual([])
    expect(cmds('start_ringtone')).toEqual([])
    expect(cmds('notify_message')).toEqual([])
    ringing(ring('G1', 'C2'))
    expect(popoverRings().map((r) => r.channel_id)).toEqual(['G1'])
    expect(cmds('start_ringtone')).toHaveLength(1)
    expect(cmds('notify_message')).toHaveLength(1)
  })
})

describe('the OS notification', () => {
  it('unfocused: "sam is calling you", and a click opens the conversation', () => {
    windowFocus.focused = false
    ringing(ring())
    expect(cmds('notify_message')).toEqual([['notify_message', { notice: { title: 'sam is calling you', body: 'Incoming call', channelId: 'DM1' } }]])
    ringing(ring('G1', 'C2'))
    expect(cmds('notify_message')[1][1]).toEqual({ notice: { title: 'sam is calling you', body: 'Call in Friends', channelId: 'G1' } })
  })
  it('focused, or with notifications off: none', () => {
    ringing(ring())
    windowFocus.focused = false
    setNotificationsEnabled(false)
    ringing(ring('G1', 'C2'))
    expect(cmds('notify_message')).toEqual([])
  })
})
