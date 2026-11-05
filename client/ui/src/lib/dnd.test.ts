// @vitest-environment jsdom
/// <reference types="node" />
// Do Not Disturb silences what grabs attention (#45): the message sound, desktop notifications, mention pings and the
// badge-unlocked sound. Unread badges and mention counts still count. (Stream toasts: stream/streamFlow.test.ts.)
import { readdirSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { flushSync } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import { invoke } from '@tauri-apps/api/core'
import { app, handleReady, maybeAlert, resetState, ui } from './store.svelte'
import { windowFocus } from './windowFocus.svelte'
import { setNotificationsEnabled, setNotificationText } from './notifyPrefs.svelte'
import { messageSound, shouldAlert, type AlertInput } from './conversations'
import { notifyMessage } from './notify'
import { applyEvent } from './state'
import { badgeStore, handleBadgeEvent } from './badges.svelte'
import { testReady } from './testReady'
import type { Message } from './protocol/Message'
import type { OwnStatus } from './protocol/OwnStatus'

const user = (id: string, username: string) => ({ id, username, display_name: username, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const msg = (channel: string, over: Partial<Message> = {}): Message => ({
  id: `M${Math.random()}`, channel_id: channel, author_id: 'U2', kind: 'normal', content: 'hello', reply_to_id: null, created_at: '2026-11-05T10:00:00Z', edited_at: null,
  deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], ...over,
})
const DM = { id: 'DM1', server_id: null, kind: 'dm' as const, name: null, position: 0 }
const TEXT = { id: 'T1', server_id: 'S1', kind: 'text' as const, name: 'general', position: 0 }
const input = (m: Message, over: Partial<AlertInput> = {}): AlertInput => ({
  message: m, me: 'U1', channel: m.channel_id === 'DM1' ? DM : TEXT, open: false, openThread: null, focused: false, muted: false, ...over,
})

describe('shouldAlert under DND', () => {
  it('a DM and a mention alert, and with dnd neither does', () => {
    const dm = input(msg('DM1'))
    const mention = input(msg('T1', { mentions: ['U1'] }))
    expect(shouldAlert(dm)).toBe(true)
    expect(shouldAlert(mention)).toBe(true)
    expect(shouldAlert({ ...dm, dnd: true })).toBe(false)
    expect(shouldAlert({ ...mention, dnd: true })).toBe(false)
    expect(shouldAlert({ ...dm, dnd: false })).toBe(true)
  })
  it('no message sound, no desktop notification', () => {
    const v = { ...input(msg('DM1')), dnd: true }
    expect(messageSound({ ...v, lastPlayedAt: 0, now: 10_000 })).toBe(false)
    const s = applyEvent({ ...app.state }, { t: 'ChannelCreated', d: { channel: DM } }, 0)
    expect(notifyMessage(s, v, { enabled: true, showText: true })).toBeNull()
    expect(notifyMessage(s, { ...v, dnd: false }, { enabled: true, showText: true })).not.toBeNull()
  })
})

describe('maybeAlert under DND', () => {
  const ready = (status: OwnStatus) => testReady([DM, TEXT], {
    me: user('U1', 'alex'),
    members: [{ server_id: 'S1', members: [{ user: user('U1', 'alex'), nickname: null }, { user: user('U2', 'sam'), nickname: null }] }],
    dm_members: [{ channel_id: 'DM1', user_ids: ['U1', 'U2'] }],
    people: [{ user: user('U1', 'alex'), online: true, last_seen_at: null, status: 'online' }, { user: user('U2', 'sam'), online: true, last_seen_at: null, status: 'online' }],
    read_states: [{ channel_id: 'DM1', last_read_message_id: null, unread: 0, mentions: 0 }, { channel_id: 'T1', last_read_message_id: null, unread: 0, mentions: 0 }],
    status,
  })
  const created = (m: Message) => ({ t: 'MessageCreated' as const, d: { message: m, nonce: null } })
  const cmds = (name: string) => vi.mocked(invoke).mock.calls.filter(([c]) => c === name)
  const sounds = (name: string) => cmds('play_sound').filter(([, a]) => (a as { name: string }).name === name)
  /** What the store does with a live event: alert, then apply. */
  const receive = (m: Message) => {
    const e = created(m)
    maybeAlert(e)
    app.state = applyEvent(app.state, e, Date.now())
  }

  // a day apart per test: the message sound's gap is kept across tests (module state)
  let day = 0
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date(Date.UTC(2026, 10, 5 + day++, 10)))
    resetState()
    setNotificationsEnabled(true)
    setNotificationText(true)
    windowFocus.focused = false
    ui.openChannelId = null
    flushSync()
  })
  afterEach(() => {
    resetState()
    vi.useRealTimers()
    windowFocus.focused = true
    flushSync()
  })

  it('DND: a DM and a mention make no sound and no notification, but unread and mentions still count', () => {
    handleReady(ready({ status: 'dnd', dnd_until: null }))
    vi.mocked(invoke).mockClear()
    receive(msg('DM1'))
    vi.advanceTimersByTime(5000)
    receive(msg('T1', { mentions: ['U1'] }))
    vi.advanceTimersByTime(5000)
    expect(sounds('message')).toEqual([])
    expect(cmds('notify_message')).toEqual([])
    expect(app.state.reads.DM1.unread).toBe(1)
    expect(app.state.reads.T1.mentions).toBe(1)
  })

  it('Online: the same DM sounds and notifies', () => {
    handleReady(ready({ status: 'online', dnd_until: null }))
    vi.mocked(invoke).mockClear()
    receive(msg('DM1'))
    vi.advanceTimersByTime(5000)
    expect(sounds('message')).toHaveLength(1)
    expect(cmds('notify_message')).toHaveLength(1)
  })

  it('a DND whose timer ran out alerts again, on time', () => {
    const end = Date.now() + 3_600_000
    handleReady(ready({ status: 'dnd', dnd_until: new Date(end).toISOString() }))
    vi.mocked(invoke).mockClear()
    vi.setSystemTime(end - 1000)
    receive(msg('DM1'))
    expect(sounds('message')).toEqual([])
    vi.setSystemTime(end)
    receive(msg('DM1'))
    expect(sounds('message')).toHaveLength(1)
  })
})

describe('badges under DND', () => {
  const earned = () => handleBadgeEvent({ t: 'BadgeEarned', d: { user_id: 'U1', badge: { key: 'the_yapper', earned_at: '2026-11-03T00:00:00Z', first_earner: false, tier: 1, rarity: 'common', first_tiers: [], count: 0, hidden: false, seen: true }, name: 'The Yapper', icon: '💬' } })
  const badgeSounds = () => vi.mocked(invoke).mock.calls.filter(([c, a]) => c === 'play_sound' && (a as { name: string }).name === 'badge')
  beforeEach(() => {
    resetState()
    badgeStore.reset()
    vi.mocked(invoke).mockClear()
  })

  it('DND: the toast still shows, quietly', () => {
    app.state = { ...app.state, me: user('U1', 'alex'), myStatus: { status: 'dnd', dnd_until: null } }
    earned()
    expect(badgeStore.toasts).toHaveLength(1)
    expect(badgeSounds()).toEqual([])
  })
  it('Online: toast and sound', () => {
    app.state = { ...app.state, me: user('U1', 'alex') }
    earned()
    expect(badgeStore.toasts).toHaveLength(1)
    expect(badgeSounds()).toHaveLength(1)
  })
})

describe('what DND touches', () => {
  // The sounds of a call you're in (join / leave, mute / deafen, soundboard, live_end) and unread counts never ask:
  // only the alerts above do. A new caller of dndActive is a new thing silenced, so it's a choice made here.
  it('only the message alerts, the live toast, the badge sound and call ringing check it', () => {
    const src = resolve(__dirname, '..')
    const callers = (readdirSync(src, { recursive: true }) as string[])
      .filter((f) => /\.(ts|svelte)$/.test(f) && !/\.test\.ts$/.test(f) && f !== 'lib/status.ts')
      .filter((f) => /\bdndActive\(/.test(readFileSync(resolve(src, f), 'utf8')))
      .sort()
    // lib/calls.ts: ringing for incoming calls (#50: no popover, ringtone or notification under DND; calls.svelte.test.ts)
    expect(callers).toEqual(['lib/badges.svelte.ts', 'lib/calls.ts', 'lib/store.svelte.ts', 'lib/stream/stream.svelte.ts'])
  })
})
