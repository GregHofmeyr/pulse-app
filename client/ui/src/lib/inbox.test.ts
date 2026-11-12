// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest'
import { appendPage, applyToList, badgeText, highlightParts, loadTab, markRead, oneLine, saveTab, whereLabel } from './inbox'
import { emptyState, type AppState } from './state'
import type { Channel } from './protocol/Channel'
import type { Event } from './protocol/Event'
import type { InboxItem } from './protocol/InboxItem'
import { testItem as item } from './testInbox'

const ch = (id: string, over: Partial<Channel>): Channel => ({ id, server_id: null, kind: 'text', name: null, position: 0, category_id: null, icon_hash: null, icon_still: null, bitrate: null, ...over }) as Channel

function state(): AppState {
  const s = emptyState()
  return {
    ...s,
    servers: [{ id: 'S1', name: 'Game Night' } as AppState['servers'][number]],
    channels: {
      C1: ch('C1', { server_id: 'S1', name: 'general' }),
      V1: ch('V1', { server_id: 'S1', kind: 'voice', name: 'Lounge' }),
      D1: ch('D1', { kind: 'dm' }),
      G1: ch('G1', { kind: 'group', name: 'Squad' }),
    },
  }
}

describe('inbox (#306)', () => {
  afterEach(() => localStorage.clear())

  it('says where an item is: server › channel, a voice chat, a group, a DM, a reply, a gone chat', () => {
    const s = state()
    expect(whereLabel(s, item('M1'))).toBe('Game Night › #general')
    expect(whereLabel(s, item('M1', {}, { channel_id: 'V1' }))).toBe('Game Night › Lounge')
    expect(whereLabel(s, item('M1', {}, { channel_id: 'G1' }))).toBe('Squad')
    expect(whereLabel(s, item('M1', {}, { channel_id: 'D1' }))).toBe('Direct message')
    expect(whereLabel(s, item('M1', {}, { thread_root_id: 'Q1' }))).toBe('Game Night › #general › replies')
    expect(whereLabel(s, item('M1', {}, { channel_id: 'GONE' }))).toBe('a deleted chat')
  })

  it('picks out your mention, shows names not usernames, and leaves unknown @words alone', () => {
    const lookup = (n: string) => ({ alex: 'ALEX', sam: 'SAM' })[n.toLowerCase()] ?? null
    const label = (id: string) => ({ ALEX: 'Alex A', SAM: 'Sam' })[id] ?? id
    expect(highlightParts('hey @alex and @sam, @nobody', lookup, label, 'ALEX')).toEqual([
      { text: 'hey ', me: false },
      { text: '@Alex A', me: true },
      { text: ' and @Sam, @nobody', me: false },
    ])
    expect(highlightParts('no mentions', lookup, label, 'ALEX')).toEqual([{ text: 'no mentions', me: false }])
    expect(oneLine('  two\n\nlines  here ')).toBe('two lines here')
  })

  it('remembers the tab per device, All by default', () => {
    expect(loadTab()).toBe('all')
    saveTab('mentions')
    expect(loadTab()).toBe('mentions')
    saveTab('all')
    expect(loadTab()).toBe('all')
  })

  it('a new item goes on top, once; the Mentions tab takes only mentions', () => {
    const list = [item('M1')]
    const added = (i: InboxItem): Event => ({ t: 'InboxItemAdded', d: { user_id: 'ALEX', item: i, unread: 2 } })
    expect(applyToList(list, added(item('M2')), 'all').map((i) => i.message.id)).toEqual(['M2', 'M1'])
    expect(applyToList(list, added(item('M1')), 'all')).toBe(list)
    expect(applyToList(list, added(item('M3', { kind: 'reply' })), 'mentions')).toBe(list)
    expect(applyToList(list, added(item('M3', { kind: 'reply' })), 'all')).toHaveLength(2)
  })

  it('read on another device, mark all read, and deletes', () => {
    const list = [item('M3'), item('M2'), item('M1')]
    const up = (d: Partial<Extract<Event, { t: 'InboxUpdated' }>['d']>): Event => ({ t: 'InboxUpdated', d: { user_id: 'ALEX', unread: 0, read: [], read_upto: null, removed: [], ...d } })
    expect(applyToList(list, up({ read: ['M2'] }), 'all').map((i) => i.read)).toEqual([false, true, false])
    expect(applyToList(list, up({ read_upto: 'M2' }), 'all').map((i) => i.read)).toEqual([false, true, true])
    expect(applyToList(list, up({ removed: ['M3'] }), 'all').map((i) => i.message.id)).toEqual(['M2', 'M1'])
    expect(applyToList(list, up({}), 'all')).toBe(list)
  })

  it('a read point reads its channel’s items up to it, a seen point its thread’s', () => {
    const list = [item('M3', {}, { thread_root_id: 'Q1' }), item('M2'), item('M1', {}, { channel_id: 'C2' })]
    const read: Event = { t: 'ReadStateUpdated', d: { user_id: 'ALEX', channel_id: 'C1', last_read_message_id: 'M9', mentions: 0, unread: 0 } }
    expect(applyToList(list, read, 'all').map((i) => i.read)).toEqual([false, true, false])
    const seen: Event = { t: 'ThreadSeen', d: { user_id: 'ALEX', channel_id: 'C1', root_id: 'Q1', last_seen_id: 'M3', mentions_cleared: 1, mentions: 0 } }
    expect(applyToList(list, seen, 'all').map((i) => i.read)).toEqual([true, false, false])
    const older: Event = { t: 'ThreadSeen', d: { user_id: 'ALEX', channel_id: 'C1', root_id: 'Q1', last_seen_id: 'M2', mentions_cleared: 0, mentions: 0 } }
    expect(applyToList(list, older, 'all')).toBe(list)
  })

  it('pages append without repeats; reading keeps the array when nothing changes; the badge caps at 99+', () => {
    const list = [item('M3'), item('M2')]
    expect(appendPage(list, [item('M2'), item('M1')]).map((i) => i.message.id)).toEqual(['M3', 'M2', 'M1'])
    expect(markRead(list, () => false)).toBe(list)
    expect(badgeText(7)).toBe('7')
    expect(badgeText(100)).toBe('99+')
  })
})
