// @vitest-environment jsdom
import { flushSync } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import { invoke } from '@tauri-apps/api/core'
import { app, handleNotificationClick, handleReady, maybeAlert, resetState, ui } from './store.svelte'
import { windowFocus } from './windowFocus.svelte'
import { setNotificationsEnabled, setNotificationText } from './notifyPrefs.svelte'
import { NOTIFY_WINDOW_MS } from './notify'
import type { Ready } from './protocol/Ready'
import type { Message } from './protocol/Message'

const user = (id: string, username: string) => ({ id, username, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const ready = (): Ready => ({
  me: user('ME', 'alex'),
  servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
  channels: [
    { id: 'DM1', server_id: null, kind: 'dm', name: null, position: 0 },
    { id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0 },
  ],
  members: [{ server_id: 'S1', members: [{ user: user('ME', 'alex'), nickname: null }, { user: user('U1', 'sam'), nickname: null }] }],
  dm_members: [{ channel_id: 'DM1', user_ids: ['ME', 'U1'] }],
  voice: [],
  people: [{ user: user('ME', 'alex'), online: true, last_seen_at: null }, { user: user('U1', 'sam'), online: true, last_seen_at: null }],
  read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [],
})
let n = 0
const created = (channel: string, content: string, extra: Partial<Message> = {}) => ({
  t: 'MessageCreated' as const,
  d: {
    nonce: null,
    message: {
      id: `M${++n}`, channel_id: channel, author_id: 'U1', kind: 'normal' as const, content, reply_to_id: null, created_at: '2026-10-30T10:00:00Z', edited_at: null,
      deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, ...extra,
    },
  },
})
const notified = () => vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === 'notify_message').map(([, args]) => (args as { notice: unknown }).notice)

beforeEach(() => {
  vi.useFakeTimers()
  resetState()
  handleReady(ready())
  setNotificationsEnabled(true)
  setNotificationText(true)
  windowFocus.focused = false
  ui.openChannelId = null
  ui.goTo = null
  ui.openReplies = null
  flushSync()
  vi.mocked(invoke).mockClear()
})
afterEach(() => {
  resetState()
  vi.useRealTimers()
  windowFocus.focused = true
  flushSync()
})

describe('message notifications', () => {
  it('a DM while the window is unfocused goes to the core', () => {
    maybeAlert(created('DM1', 'hello'))
    expect(notified()).toEqual([{ title: 'sam', body: 'hello', channelId: 'DM1' }])
  })

  it('none while the window is focused', () => {
    windowFocus.focused = true
    flushSync()
    maybeAlert(created('DM1', 'hello'))
    expect(notified()).toEqual([])
  })

  it('none with notifications off', () => {
    setNotificationsEnabled(false)
    maybeAlert(created('DM1', 'hello'))
    expect(notified()).toEqual([])
  })

  it('a burst: one at once, one "and N more" after the window', () => {
    for (let i = 0; i < 5; i++) maybeAlert(created('DM1', `m${i}`))
    expect(notified()).toHaveLength(1)
    vi.advanceTimersByTime(NOTIFY_WINDOW_MS)
    expect(notified()).toEqual([
      { title: 'sam', body: 'm0', channelId: 'DM1' },
      { title: 'sam and 4 more', body: 'm4', channelId: 'DM1' },
    ])
  })

  it('focus during the window cancels the replacement', () => {
    maybeAlert(created('DM1', 'a'))
    maybeAlert(created('DM1', 'b'))
    windowFocus.focused = true
    flushSync()
    vi.advanceTimersByTime(NOTIFY_WINDOW_MS)
    expect(notified()).toHaveLength(1)
  })

  it('signing out cancels the replacement', () => {
    maybeAlert(created('DM1', 'a'))
    maybeAlert(created('DM1', 'b'))
    resetState()
    vi.advanceTimersByTime(NOTIFY_WINDOW_MS)
    expect(notified()).toHaveLength(1)
  })

  it('switching notifications off cancels the replacement', () => {
    maybeAlert(created('DM1', 'a'))
    maybeAlert(created('DM1', 'b'))
    setNotificationsEnabled(false)
    flushSync()
    vi.advanceTimersByTime(NOTIFY_WINDOW_MS)
    expect(notified()).toHaveLength(1)
  })

  it('message text off: no text in the replacement either', () => {
    setNotificationText(false)
    maybeAlert(created('DM1', 'secret one'))
    maybeAlert(created('DM1', 'secret two'))
    vi.advanceTimersByTime(NOTIFY_WINDOW_MS)
    expect(JSON.stringify(notified())).not.toContain('secret')
  })
})

describe('a clicked notification', () => {
  it('opens its conversation', () => {
    handleNotificationClick({ channelId: 'DM1', threadId: null })
    expect(ui.goTo).toBe('DM1')
    expect(ui.openReplies).toBeNull()
  })

  it("a reply opens its quote's replies panel", () => {
    handleNotificationClick({ channelId: 'Q1', threadId: 'R1' })
    expect(ui.goTo).toBe('Q1')
    expect(ui.openReplies).toEqual({ channelId: 'Q1', rootId: 'R1' })
  })

  it('a conversation that no longer exists (or after signing out): nothing to open', () => {
    handleNotificationClick({ channelId: 'GONE', threadId: null })
    expect(ui.goTo).toBeNull()
    resetState()
    handleNotificationClick({ channelId: 'DM1', threadId: null })
    expect(ui.goTo).toBeNull()
    expect(app.state.me).toBeNull()
  })
})
