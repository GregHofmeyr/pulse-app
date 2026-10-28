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
import { app, handleReady, resetState, ui } from '../lib/store.svelte'
import { testQoty } from '../lib/testReady'
import { addHistory, applyEvent, applyReady, emptyState, markHistory, trimHistory } from '../lib/state'

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
      dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [],
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
    dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [],
  }
  const unsent = (nonce: string, status: 'pending' | 'failed', content = 'hello **world**') =>
    ({ nonce, content, reply_to_id: null, thread_root_id: null, status, created_at: '2026-10-01T00:00:00.000Z' })
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

  it('retrying a failed message keeps the reply target you just picked (as replies do, M24)', () => {
    const c = show([unsent('t5', 'failed', 'again')])
    app.state = markHistory(addHistory(app.state, 'C1', [msg(7)]), 'C1', true)
    flushSync()
    invoke.mockClear()
    document.querySelector<HTMLButtonElement>('.msg:not(.unsent) button[aria-label="Reply"]')!.click()
    flushSync()
    button('Retry')!.click()
    flushSync()
    expect(invoke.mock.calls.find(([cmd]) => cmd === 'send_message')![1]).toMatchObject({ content: 'again', replyToId: null })
    expect(document.querySelector('.replying')).toBeTruthy() // still replying to M0007
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
      dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [],
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

describe('TextChannel: the edit box', () => {
  beforeEach(() => {
    Element.prototype.scrollTo = () => {} // jsdom has none
    resetState()
  })
  afterEach(() => (document.body.innerHTML = ''))

  it('closing it puts focus back in the message box', async () => {
    const me = { id: 'U1', username: 'alex', avatar_hash: null }
    const s = applyReady(emptyState(), {
      me, servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
      channels: [{ id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }],
      members: [{ server_id: 'S1', members: [{ user: me, nickname: null }] }],
      dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [],
    })
    app.state = markHistory(addHistory(s, 'C1', [{ ...msg(1), author_id: 'U1' }]), 'C1', true)
    const c = mount(TextChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    document.querySelector<HTMLButtonElement>('button[aria-label="Edit"]')!.click()
    flushSync()
    await tick()
    const edit = document.querySelector<HTMLElement>('[aria-label="Edit message"]')!
    expect(document.activeElement).toBe(edit)
    edit.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(document.querySelector('[aria-label="Edit message"]')).toBeNull()
    expect(document.activeElement).toBe(document.querySelector('[aria-label="Message #general"]'))
    unmount(c)
  })
})

describe('TextChannel: quotes channel', () => {
  beforeEach(() => {
    Element.prototype.scrollTo = () => {}
    localStorage.clear()
    resetState()
    invoke.mockReset()
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_replies' ? [] : undefined))
  })
  const mounted: ReturnType<typeof mount>[] = []
  afterEach(() => {
    while (mounted.length) unmount(mounted.pop()!)
    document.body.innerHTML = ''
  })
  const me = { id: 'U1', username: 'alex', avatar_hash: null }
  const ready = (kind: 'quotes' | 'text') => ({
    me, servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
    channels: [{ id: 'C1', server_id: 'S1', kind, name: 'quotes', position: 0 }],
    members: [{ server_id: 'S1', members: [{ user: me, nickname: null }] }],
    dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [],
  })
  const quoteMsg: Message = {
    ...msg(1), author_id: 'U1', content: '“hi” — alex',
    quote: { lines: [{ id: 'L1', speaker_id: 'U1', speaker_name: null, text: 'hi', media: null, caption: null, awards: 0, awarders: [], awarded: false }], year: 2026, said_on: null, reply_count: 0, last_reply_id: null, replies_seen: null, wins: [] },
  }
  const system: Message = { ...msg(2), author_id: null, kind: 'system', content: 'Nobody voted this year.' }
  const show = (kind: 'quotes' | 'text', list: Message[]) => {
    app.state = markHistory(addHistory(applyReady(emptyState(), ready(kind)), 'C1', list), 'C1', true)
    const c = mount(TextChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    mounted.push(c)
    flushSync()
    return c
  }

  it('renders quotes as quote items, system lines as before, and has the quotes header', () => {
    show('quotes', [quoteMsg, system])
    expect(document.querySelectorAll('.quote').length).toBe(1)
    expect(document.querySelector('.line .text')?.textContent).toBe('“hi”')
    expect(document.body.textContent).toContain('Nobody voted this year.')
    expect(document.querySelector('button[aria-label="Leaderboard"]')).toBeTruthy()
    expect([...document.querySelectorAll('button')].some((b) => b.textContent === 'Start Quote of the Year')).toBe(true)
  })

  it('renders a Quote of the Year result as its card, and drops it when the card is removed', () => {
    const card: Message = {
      ...msg(2), author_id: null, kind: 'system', content: 'Quote of the Year 2026: “hi” — alex',
      qoty_card: { event_id: 'E1', year: 2026, message_id: quoteMsg.id, votes: 1, voters: ['U1'], reason: 'time', speakers: [] },
    }
    show('quotes', [quoteMsg, card])
    const c = document.querySelector('.qoty-card')!
    expect(c.textContent).toContain('hi')
    expect(c.textContent).toContain('Quote of the Year 2026')
    expect(c.textContent).toContain('ended by time')
    // a reopen hard-deletes the card
    app.state = applyEvent(app.state, { t: 'MessageDeleted', d: { channel_id: 'C1', message_id: card.id, thread_root_id: null, author_id: null, mentions: [] } }, Date.now())
    flushSync()
    expect(document.querySelector('.qoty-card')).toBeNull()
    expect(document.body.textContent).not.toContain('Quote of the Year 2026')
  })

  it('a fetched winner gives way to the live quote once the channel loads it', async () => {
    const card: Message = {
      ...msg(2), author_id: null, kind: 'system', content: 'Quote of the Year 2026: “hi” — alex',
      qoty_card: { event_id: 'E1', year: 2026, message_id: quoteMsg.id, votes: 1, voters: ['U1'], reason: 'time', speakers: [] },
    }
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_messages_around' ? [quoteMsg] : undefined))
    show('quotes', [card])
    for (let i = 0; i < 3; i++) await tick()
    await new Promise((r) => setTimeout(r, 0))
    flushSync()
    expect(document.querySelector('.qoty-card .q p')?.textContent).toBe('“hi”')
    // the channel loads the quote, since edited: the card follows the live one, not its snapshot
    const live: Message = { ...quoteMsg, quote: { ...quoteMsg.quote!, lines: [{ ...quoteMsg.quote!.lines[0], text: 'hi there' }] } }
    app.state = { ...app.state, messages: { ...app.state.messages, C1: [live, card] } }
    flushSync()
    expect(document.querySelector('.qoty-card .q p')?.textContent).toBe('“hi there”')
  })

  it('a fetched winner switches to the deleted placeholder as soon as its quote is deleted', async () => {
    const card: Message = {
      ...msg(2), author_id: null, kind: 'system', content: 'Quote of the Year 2026: “hi” — alex',
      qoty_card: { event_id: 'E1', year: 2026, message_id: quoteMsg.id, votes: 1, voters: ['U1'], reason: 'time', speakers: [{ user_id: 'U1', name: null }] },
    }
    const settled = async () => {
      for (let i = 0; i < 3; i++) await tick()
      await new Promise((r) => setTimeout(r, 0))
      flushSync()
    }
    for (const event of ['deleted', 'updated'] as const) {
      let fetches = 0
      // The first fetch finds the quote; any re-fetch never answers (the placeholder can't wait on it).
      invoke.mockImplementation((cmd: string) => (cmd === 'list_messages_around' ? (fetches++ ? new Promise(() => {}) : Promise.resolve([quoteMsg])) : Promise.resolve(undefined)))
      show('quotes', [card])
      await settled()
      expect(document.querySelector('.qoty-card .q p')?.textContent, event).toBe('“hi”')
      app.state = event === 'deleted'
        ? applyEvent(app.state, { t: 'MessageDeleted', d: { channel_id: 'C1', message_id: quoteMsg.id, thread_root_id: null, author_id: 'U1', mentions: [] } }, Date.now())
        : applyEvent(app.state, { t: 'MessageUpdated', d: { message: { ...card, content: 'Quote of the Year 2026: a deleted quote' } } }, Date.now())
      flushSync()
      expect(document.querySelector('.qoty-card .q')?.textContent, event).toBe('This quote was deleted.')
      expect(document.querySelector('.qoty-card .who')?.textContent, event).toContain('Quote of the Year 2026')
      for (const c of mounted.splice(0)) unmount(c)
      document.body.innerHTML = ''
    }
  })

  it('offers Start only while no vote is live', () => {
    show('quotes', [])
    const start = () => [...document.querySelectorAll('button')].find((b) => b.textContent === 'Start Quote of the Year')
    expect(start()).toBeTruthy()
    app.state = applyEvent(app.state, { t: 'QotyStarted', d: { event: testQoty() } }, Date.now())
    flushSync()
    expect(start()).toBeUndefined()
  })

  it('header buttons: the leaderboard opens its popover, Start calls back', () => {
    app.state = markHistory(addHistory(applyReady(emptyState(), ready('quotes')), 'C1', []), 'C1', true)
    const onStartQoty = vi.fn()
    mounted.push(mount(TextChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1', onStartQoty } }))
    flushSync()
    document.querySelector<HTMLButtonElement>('button[aria-label="Leaderboard"]')!.click()
    flushSync()
    expect(document.querySelector('.leaderboard')).toBeTruthy()
    expect(invoke).toHaveBeenCalledWith('leaderboard', { serverId: 'S1', year: null })
    ;[...document.querySelectorAll('button')].find((b) => b.textContent === 'Start Quote of the Year')!.click()
    expect(onStartQoty).toHaveBeenCalled()
  })

  it('a leaderboard entry closes the popover and jumps to the quote in its channel', async () => {
    const other: Message = { ...quoteMsg, id: 'Q9', channel_id: 'C2' }
    const board = { winners: [], top_year: [], top_all_time: [other], most_quoted: [], most_awarded: [] }
    invoke.mockImplementation(async (cmd: string) => (cmd === 'leaderboard' ? board : undefined))
    const r = ready('quotes')
    app.state = markHistory(addHistory(applyReady(emptyState(), { ...r, channels: [...r.channels, { id: 'C2', server_id: 'S1', kind: 'quotes', name: 'more', position: 1 }] }), 'C1', []), 'C1', true)
    mounted.push(mount(TextChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } }))
    flushSync()
    document.querySelector<HTMLButtonElement>('button[aria-label="Leaderboard"]')!.click()
    flushSync()
    await new Promise((r) => setTimeout(r, 0))
    flushSync()
    document.querySelector<HTMLButtonElement>('.leaderboard button.entry')!.click()
    flushSync()
    expect(document.querySelector('.leaderboard')).toBeNull()
    expect(ui.goTo).toBe('C2')
    expect(invoke).toHaveBeenCalledWith('list_messages_around', { channelId: 'C2', messageId: 'Q9' })
  })

  it('Reply and View N replies open the replies panel, one at a time; Escape closes it', async () => {
    const q2: Message = { ...quoteMsg, id: 'M0002', quote: { ...quoteMsg.quote!, reply_count: 1, last_reply_id: 'R1' } }
    show('quotes', [quoteMsg, q2])
    expect(document.querySelector('.replies-panel')).toBeNull()
    document.querySelector<HTMLButtonElement>('.list .quote button[aria-label="Reply"]')!.click()
    flushSync()
    expect(document.querySelectorAll('.replies-panel')).toHaveLength(1)
    expect(document.querySelector('.replies-panel .quote')?.getAttribute('data-message-id')).toBe('M0001')
    expect(invoke).toHaveBeenCalledWith('list_replies', { messageId: 'M0001', before: null })
    ;[...document.querySelectorAll<HTMLButtonElement>('.list button.replies')].find((b) => b.textContent?.includes('View 1 reply'))!.click()
    flushSync()
    expect(document.querySelectorAll('.replies-panel')).toHaveLength(1)
    expect(document.querySelector('.replies-panel .quote')?.getAttribute('data-message-id')).toBe('M0002')
    await new Promise((r) => setTimeout(r, 0))
    document.querySelector('.replies-panel [role="textbox"]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(document.querySelector('.replies-panel')).toBeNull()
  })

  const unsentReply = (nonce: string, root: string, status: 'pending' | 'failed' = 'failed') =>
    ({ nonce, content: `reply ${nonce}`, reply_to_id: null, thread_root_id: root, status, created_at: '2026-10-01T00:00:00.000Z' })
  const settled = async () => {
    for (let i = 0; i < 3; i++) await tick()
    await new Promise((r) => setTimeout(r, 0))
    flushSync()
  }

  it('I5: a quote with an unsent reply says so while its panel is closed, and the marker opens it', async () => {
    show('quotes', [quoteMsg])
    app.state = { ...app.state, pending: { C1: [unsentReply('a', 'M0001')] } }
    flushSync()
    const marker = () => document.querySelector<HTMLButtonElement>('.list .quote button.unsent-reply')
    expect(marker()?.textContent?.trim()).toBe('Unsent reply')
    expect(document.querySelector('.list .unsent')).toBeNull() // not in the channel list itself
    marker()!.click()
    flushSync()
    await settled()
    expect(document.querySelector('.replies-panel .quote')?.getAttribute('data-message-id')).toBe('M0001')
    expect(document.querySelector('.replies-panel .unsent')).toBeTruthy() // Retry / Discard live here
    expect(marker()).toBeNull() // the panel shows it now
    app.state = { ...app.state, pending: { C1: [unsentReply('a', 'M0001'), unsentReply('b', 'M0001')] } }
    document.querySelector<HTMLButtonElement>('.replies-panel button[aria-label="Close replies"]')!.click()
    flushSync()
    expect(marker()?.textContent?.trim()).toBe('2 unsent replies')
  })

  it('M16: the replies panel closes when its quote is deleted', async () => {
    show('quotes', [quoteMsg])
    document.querySelector<HTMLButtonElement>('.list .quote button[aria-label="Reply"]')!.click()
    flushSync()
    await settled()
    expect(document.querySelector('.replies-panel')).toBeTruthy()
    app.state = applyEvent(app.state, { t: 'MessageDeleted', d: { channel_id: 'C1', message_id: 'M0001', thread_root_id: null, author_id: 'U1', mentions: [] } }, Date.now())
    flushSync()
    await settled()
    expect(document.querySelector('.replies-panel')).toBeNull()
  })

  it("M16: the pinned quote stays current after it leaves the channel's list", async () => {
    show('quotes', [quoteMsg])
    document.querySelector<HTMLButtonElement>('.list .quote button[aria-label="Reply"]')!.click()
    flushSync()
    await settled()
    // "Jump to present" replaced the list: the quote isn't loaded there any more
    app.state = { ...app.state, messages: { ...app.state.messages, C1: [msg(500)] } }
    flushSync()
    const edited: Message = { ...quoteMsg, edited_at: '2026-10-02T00:00:00.000Z', quote: { ...quoteMsg.quote!, lines: [{ ...quoteMsg.quote!.lines[0], text: 'hi there' }] } }
    app.state = applyEvent(app.state, { t: 'MessageUpdated', d: { message: edited } }, Date.now())
    flushSync()
    expect(document.querySelector('.replies-panel .quote .line .text')?.textContent).toBe('“hi there”')
    app.state = applyEvent(app.state, { t: 'MessageDeleted', d: { channel_id: 'C1', message_id: 'M0001', thread_root_id: null, author_id: 'U1', mentions: [] } }, Date.now())
    flushSync()
    await settled()
    expect(document.querySelector('.replies-panel')).toBeNull()
  })

  it('M20: closing replies with Escape or ✕ puts focus back on what opened them', async () => {
    const q: Message = { ...quoteMsg, quote: { ...quoteMsg.quote!, reply_count: 1, last_reply_id: 'R1' } }
    show('quotes', [q])
    for (const how of ['escape', 'close'] as const) {
      const opener = [...document.querySelectorAll<HTMLButtonElement>('.list button.replies')].find((b) => b.textContent?.includes('View 1 reply'))!
      opener.focus()
      opener.click()
      flushSync()
      await settled()
      expect(document.activeElement?.closest('.replies-panel'), how).toBeTruthy() // the reply box took it
      if (how === 'escape') document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
      else document.querySelector<HTMLButtonElement>('.replies-panel button[aria-label="Close replies"]')!.click()
      flushSync()
      await settled()
      expect(document.querySelector('.replies-panel'), how).toBeNull()
      expect(document.activeElement, how).toBe(opener)
    }
  })

  it('has no message composer, and a text channel has no quote header', () => {
    show('quotes', [])
    expect(document.querySelector('.composer, [contenteditable]')).toBeNull()
    while (mounted.length) unmount(mounted.pop()!)
    document.body.innerHTML = ''
    show('text', [msg(3)])
    expect(document.querySelector('button[aria-label="Leaderboard"]')).toBeNull()
    expect(document.querySelector('[contenteditable]')).toBeTruthy()
  })

  it('opening an edit keeps the new-quote draft', () => {
    show('quotes', [quoteMsg])
    const box = () => document.querySelector<HTMLTextAreaElement>('.qc:not([hidden] .qc) textarea.qtext')
    const draft = document.querySelector<HTMLTextAreaElement>('textarea.qtext')!
    draft.value = 'half written'; draft.dispatchEvent(new Event('input', { bubbles: true })); flushSync()
    document.querySelector<HTMLButtonElement>('button[aria-label="Edit"]')!.click(); flushSync()
    expect(box()!.value).toBe('hi')
    expect(document.querySelectorAll('textarea.qtext')).toHaveLength(2)
    ;[...document.querySelectorAll<HTMLButtonElement>('.foot button')].find((b) => b.textContent === 'Cancel')!.click(); flushSync()
    expect(document.querySelectorAll('textarea.qtext')).toHaveLength(1)
    expect(document.querySelector<HTMLTextAreaElement>('textarea.qtext')!.value).toBe('half written')
  })
})
