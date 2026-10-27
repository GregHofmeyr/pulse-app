// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'

// The core: list_messages for the scroll-up page waits until the test releases it.
let releaseOlder: (page: Message[]) => void = () => {}
const invoke = vi.fn(async (cmd: string, args?: { before?: string | null }): Promise<unknown> => {
  if (cmd === 'list_messages' && args?.before) return new Promise((done) => (releaseOlder = done))
  return undefined
})
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import TextChannel from './TextChannel.svelte'
import { app, handleReady, resetState } from '../lib/store.svelte'
import { addHistory, applyReady, emptyState, markHistory, trimHistory } from '../lib/state'

const msg = (n: number): Message => ({
  id: `M${String(n).padStart(4, '0')}`, channel_id: 'C1', author_id: 'U2', kind: 'normal', content: `m${n}`,
  reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null,
})
const page = (from: number, n: number) => Array.from({ length: n }, (_, i) => msg(from + i))

describe('TextChannel: leaving while an older page is loading', () => {
  it('drops the late page (no hole) and does not throw', async () => {
    const me = { id: 'U1', username: 'alex', avatar_hash: null }
    const s = applyReady(emptyState(), {
      me, servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
      channels: [{ id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }],
      members: [{ server_id: 'S1', members: [{ user: me, nickname: null }] }],
      dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [],
    })
    app.state = markHistory(addHistory(s, 'C1', page(100, 200)), 'C1', false) // M0100..M0299
    const c = mount(TextChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    document.querySelector('.list')!.dispatchEvent(new Event('scroll')) // scrollTop 0: load older
    expect(invoke).toHaveBeenCalledWith('list_messages', { channelId: 'C1', before: 'M0100' })

    unmount(c) // switched channel: Shell trims on leave
    app.state = trimHistory(app.state, 'C1')
    const trimmed = app.state.messages['C1']
    releaseOlder(page(90, 10)) // a SHORT late page (M0090..M0099) lands after the trim
    await tick()
    await new Promise((r) => setTimeout(r, 0))

    expect(app.state.messages['C1']).toBe(trimmed)
    expect(app.state.messages['C1'][0].id).toBe('M0200')
    expect(app.state.history['C1'].start).toBe(false) // the trimmed list has not reached the channel start
  })
})

describe('TextChannel: unsent messages', () => {
  beforeEach(() => {
    Element.prototype.scrollTo = () => {} // jsdom has none
    localStorage.clear()
    resetState()
  })
  afterEach(() => (document.body.innerHTML = ''))
  const me = { id: 'U1', username: 'alex', avatar_hash: null }
  const ready = {
    me, servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
    channels: [{ id: 'C1', server_id: 'S1', kind: 'text' as const, name: 'general', position: 0 }],
    members: [{ server_id: 'S1', members: [{ user: me, nickname: null }] }],
    dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [],
  }
  const unsent = (nonce: string, status: 'pending' | 'failed', content = 'hello **world**') =>
    ({ nonce, content, reply_to_id: null, status, created_at: '2026-10-01T00:00:00.000Z' })
  const show = (pendingList: ReturnType<typeof unsent>[]) => {
    app.state = { ...applyReady(emptyState(), ready), pending: { C1: pendingList } }
    app.state = markHistory(addHistory(app.state, 'C1', []), 'C1', true)
    const c = mount(TextChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    return c
  }
  const button = (label: string) => [...document.querySelectorAll('button')].find((b) => b.textContent === label)

  it('unsent_message_looks_like_a_message_faded', () => {
    const c = show([unsent('t1', 'pending')])
    const u = document.querySelector('.unsent') as HTMLElement
    expect(u.querySelector('.meta strong')?.textContent).toBe('alex')
    expect(u.querySelector('.content strong')?.textContent).toBe('world')
    expect(u.style.opacity).toBe('0.6')
    expect(u.querySelector('.actions')).toBeNull()
    expect(document.querySelector('.status')?.textContent).toContain('Sending…')
    unmount(c)
  })

  it('failed_shows_retry_and_discard', () => {
    const c = show([unsent('t2', 'failed')])
    expect(document.querySelector('.status')?.textContent).toContain('Failed to send')
    expect(button('Retry')).toBeTruthy()
    expect(button('Discard')).toBeTruthy()
    unmount(c)
  })

  it('discard_removes_it_and_retry_resends_with_a_new_nonce', () => {
    const c = show([unsent('t3', 'failed', 'one'), unsent('t4', 'failed', 'two')])
    invoke.mockClear()
    const [, discard2] = [...document.querySelectorAll('button')].filter((b) => b.textContent === 'Discard')
    discard2.click()
    flushSync()

    expect(app.state.pending['C1'].map((p) => p.nonce)).toEqual(['t3'])
    button('Retry')!.click()
    flushSync()
    const sent = invoke.mock.calls.find(([cmd]) => cmd === 'send_message')!
    expect(sent[1]).toMatchObject({ channelId: 'C1', content: 'one' })
    expect((sent[1] as { nonce: string }).nonce).not.toBe('t3')
    expect(app.state.pending['C1']).toHaveLength(1)
    expect(app.state.pending['C1'][0].status).toBe('pending')
    unmount(c)
  })

  it('survives_reload', () => {
    localStorage.setItem('pulse.unsent.U1', JSON.stringify({ C1: [unsent('r1', 'pending', 'left behind')], GONE: [unsent('r2', 'pending')] }))
    app.state = emptyState()
    invoke.mockClear()
    handleReady(ready)
    app.state = markHistory(addHistory(app.state, 'C1', []), 'C1', true)
    const c = mount(TextChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    expect(document.querySelector('.unsent')?.textContent).toContain('left behind')
    expect(document.querySelector('.status')?.textContent).toContain('Failed to send')
    expect(app.state.pending['GONE']).toBeUndefined()
    expect(invoke.mock.calls.some(([cmd]) => cmd === 'send_message')).toBe(false)
    unmount(c)
  })
})

describe('TextChannel: a failed delete', () => {
  beforeEach(() => {
    Element.prototype.scrollTo = () => {} // jsdom has none
    localStorage.clear()
    resetState()
  })
  afterEach(() => {
    invoke.mockReset()
    document.body.innerHTML = ''
  })
  const me = { id: 'U1', username: 'alex', avatar_hash: null }

  it('shows_on_the_message_not_as_the_channel_banner', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'delete_message') throw 'cannot delete'
      return undefined
    })
    app.state = applyReady(emptyState(), {
      me, servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
      channels: [{ id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }],
      members: [{ server_id: 'S1', members: [{ user: me, nickname: null }] }],
      dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [],
    })
    app.state = markHistory(addHistory(app.state, 'C1', [{ ...msg(1), author_id: 'U1' }]), 'C1', true)
    const c = mount(TextChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    document.querySelector<HTMLButtonElement>('button[aria-label="Delete"]')!.click()
    for (let i = 0; i < 5; i++) await tick()
    flushSync()
    expect(invoke).toHaveBeenCalledWith('delete_message', { messageId: 'M0001' })
    expect(document.querySelector('.error[role=alert]')).toBeNull()
    expect(document.querySelector('.msg [role=alert]')?.textContent).toBe('cannot delete')
    unmount(c)
  })
})
