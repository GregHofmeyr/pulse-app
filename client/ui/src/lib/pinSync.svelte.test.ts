// @vitest-environment jsdom
// Pins of loaded messages after a reconnect (#258): chats with loaded messages are marked, one pins fetch sets them.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import { notePinEvent, pinSync, refreshPins } from './pinSync.svelte'
import { app, handleReady, resetState } from './store.svelte'
import { resetServerClock, serverNow } from './serverClock'
import { addHistory, applyEvent } from './state'
import { testReady } from './testReady'
import type { Message } from './protocol/Message'
import type { Pin } from './protocol/Pin'
import type { PinnedMessage } from './protocol/PinnedMessage'

const TEXT = { id: 'C1', server_id: 'S1', kind: 'text' as const, name: 'general', position: 0 }
const QUOTES = { id: 'Q1', server_id: 'S1', kind: 'quotes' as const, name: 'quotes', position: 1 }
const msg = (id: string, channel = 'C1', pin?: Pin): Message => ({
  id, channel_id: channel, author_id: 'U2', kind: 'normal', content: id, reply_to_id: null, created_at: '2026-11-09T09:00:00.000Z',
  edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [],
  ...(pin ? { pin } : {}),
})
const pin = (by = 'U2'): Pin => ({ pinned_by: by, pinned_at: '2026-11-09T10:00:00.000Z', expires_at: null })
const pinOf = (id: string) => app.state.messages['C1'].find((m) => m.id === id)?.pin ?? null
async function settle() { for (let i = 0; i < 10; i++) await Promise.resolve() }

beforeEach(() => {
  resetState()
  invoke.mockReset(); invoke.mockResolvedValue(undefined)
  handleReady(testReady([TEXT, QUOTES]))
  // an older page with M1 (pinned) and M2, and a quotes channel's messages
  app.state = addHistory(app.state, 'C1', [msg('M1', 'C1', pin()), msg('M2')])
  app.state = addHistory(app.state, 'Q1', [msg('Q', 'Q1')])
})
afterEach(() => resetState())

describe('pins after a reconnect (#258)', () => {
  it('marks every chat with loaded messages that takes pins', () => {
    expect(pinSync.stale).toEqual({})
    handleReady(testReady([TEXT, QUOTES]))
    expect(pinSync.stale).toEqual({ C1: true })
  })

  it("one fetch of the chat's pins sets its loaded messages, older pages included", async () => {
    handleReady(testReady([TEXT, QUOTES]))
    // while away: M1 was unpinned and M2 pinned
    invoke.mockImplementation(async (c: string) => (c === 'list_pins' ? [{ message: msg('M2'), pin: pin('U1') }] satisfies PinnedMessage[] : undefined))
    await refreshPins('C1')
    expect(invoke).toHaveBeenCalledWith('list_pins', { channelId: 'C1' })
    expect(invoke).toHaveBeenCalledTimes(1)
    expect(pinOf('M1')).toBeNull()
    expect(pinOf('M2')).toEqual(pin('U1'))
    expect(pinSync.stale).toEqual({})
  })

  it('a pin event while the list loads wins over the list', async () => {
    let answer: (l: PinnedMessage[]) => void = () => {}
    invoke.mockImplementation((c: string) => (c === 'list_pins' ? new Promise((r) => (answer = r)) : Promise.resolve(undefined)))
    const done = refreshPins('C1')
    const e = { t: 'MessagePinned' as const, d: { channel_id: 'C1', pinned: { message: msg('M2'), pin: pin('U1') } } }
    app.state = applyEvent(app.state, e, 0)
    notePinEvent(e)
    answer([{ message: msg('M1'), pin: pin() }])
    await done
    expect(pinOf('M2')).toEqual(pin('U1'))
    expect(pinOf('M1')).toEqual(pin())
  })

  it('fetches again when a reconnect lands while it loads', async () => {
    let calls = 0
    invoke.mockImplementation(async (c: string) => {
      if (c !== 'list_pins') return undefined
      calls++
      if (calls === 1) handleReady(testReady([TEXT, QUOTES]))
      return []
    })
    await refreshPins('C1')
    await settle()
    expect(calls).toBe(2)
    expect(pinSync.stale).toEqual({})
  })

  it("each Ready sets the server's clock that times pins", () => {
    const T = Date.parse('2026-11-09T10:00:00.000Z')
    vi.useFakeTimers()
    vi.setSystemTime(T + 5 * 60_000)
    try {
      handleReady(testReady([TEXT, QUOTES], { server_now: new Date(T).toISOString() }))
      expect(serverNow()).toBe(T)
    } finally {
      vi.useRealTimers()
      resetServerClock(null)
    }
  })

  it('a failed fetch leaves the pins as they were', async () => {
    invoke.mockImplementation(async (c: string) => { if (c === 'list_pins') throw "can't reach the server" })
    await refreshPins('C1')
    expect(pinOf('M1')).toEqual(pin())
  })
})
