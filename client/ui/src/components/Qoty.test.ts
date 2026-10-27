// @vitest-environment jsdom
import { flushSync, mount, unmount, type Component } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { QotyEvent } from '../lib/protocol/QotyEvent'
import type { QotyOption } from '../lib/protocol/QotyOption'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import QotyBanner from './QotyBanner.svelte'
import QotyCard from './QotyCard.svelte'
import QotyStart from './QotyStart.svelte'
import QotyStrip from './QotyStrip.svelte'
import { motion } from '../lib/animate.svelte'
import { defaultQotyYear } from '../lib/quotes'
import { applyEvent } from '../lib/state'
import { app, clock, handleReady, resetState } from '../lib/store.svelte'
import { testMe, testReady } from '../lib/testReady'

const sam = { id: 'U2', username: 'sam', avatar_hash: null }
const kim = { id: 'U3', username: 'kim', avatar_hash: null }
const HOUR = 3_600_000

const quote = (id: string, text: string, speaker = 'U2', reactions = 0): Message => ({
  id, channel_id: 'C1', author_id: 'U1', kind: 'normal', content: `“${text}”`, reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [],
  reactions: reactions ? [{ emoji: '😂', count: reactions, me: false }] : [], thread_root_id: null, qoty_card: null,
  quote: {
    lines: [{ id: `L${id}`, speaker_id: speaker, speaker_name: null, text, media: null, caption: null, awards: 0, awarders: [], awarded: false }],
    year: 2026, said_on: null, reply_count: 0, last_reply_id: null, replies_seen: null, wins: [],
  },
})
const opt = (m: Message, voters: string[] = [], added_by: string | null = null): QotyOption => ({ message: m, added_by, votes: voters.length, voters })
const event = (over: Partial<QotyEvent> = {}): QotyEvent => ({
  id: 'E1', server_id: 'S1', year: 2026, started_by: 'U2', started_at: '2026-10-27T10:00:00Z',
  ends_at: new Date(Date.now() + 2 * HOUR + 5 * 60_000 + 30_000).toISOString(), ended_at: null, ended_by: null, end_reason: null,
  reopen_until: null,
  options: [opt(quote('Q1', 'first one', 'U2', 4)), opt(quote('Q2', 'second one', 'U3', 2))],
  voters: 0, members: 3, ...over,
})
const card = (over: Partial<Message> = {}, voters = ['U2', 'U3']): Message => ({
  id: 'K1', channel_id: 'C1', author_id: null, kind: 'system', content: 'Quote of the Year 2026: “first one” — sam', reply_to_id: null,
  created_at: new Date(Date.now() - 5 * 60_000).toISOString(), edited_at: null, deleted: false, mentions: [], reactions: [],
  thread_root_id: null, quote: null,
  qoty_card: { event_id: 'E1', year: 2026, message_id: 'Q1', votes: voters.length, voters, reason: 'early' },
  ...over,
})

const set = (e: QotyEvent) => {
  app.state = applyEvent(app.state, { t: 'QotyUpdated', d: { event: e } }, Date.now())
  flushSync()
}

let mounted: ReturnType<typeof mount>[] = []
function show<P extends Record<string, any>>(c: Component<P>, props: P) {
  mounted.push(mount(c, { target: document.body, props }))
  flushSync()
}
const settle = async () => {
  await new Promise((r) => setTimeout(r, 0))
  flushSync()
}
const text = (el: Element | null) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim()
const button = (label: string, root: ParentNode = document) =>
  [...root.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b) === label || b.getAttribute('aria-label') === label)
const radio = (label: string) => [...document.querySelectorAll<HTMLElement>('[role=radio]')].find((b) => text(b) === label)
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd).map((c) => c[1])
const option = (id: string) => document.querySelector<HTMLElement>(`[data-option="${id}"]`)!
const banner = () => document.querySelector('.qoty-banner')

beforeEach(() => {
  resetState()
  handleReady(testReady([
    { id: 'C1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0 },
    { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 1 },
  ], {
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }, { user: kim, nickname: null }] }],
  }))
  clock.nowIso = new Date().toISOString()
  motion.reducedMotion = false
  invoke.mockReset()
  invoke.mockImplementation(async () => undefined)
})
afterEach(() => {
  mounted.forEach((c) => unmount(c))
  mounted = []
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('Quote of the Year banner', () => {
  it('banner appears and disappears with the event', () => {
    show(QotyBanner, { serverId: 'S1' })
    expect(banner()).toBeNull()
    app.state = applyEvent(app.state, { t: 'QotyStarted', d: { event: event() } }, Date.now())
    flushSync()
    expect(text(banner())).toContain('Quote of the Year 2026')
    expect(text(banner())).toContain('LIVE')
    expect(button('Vote', banner()!)).toBeTruthy()
    app.state = applyEvent(app.state, { t: 'QotyEnded', d: { event: event({ ended_at: new Date().toISOString(), end_reason: 'early' }) } }, Date.now())
    flushSync()
    expect(banner()).toBeNull()
  })

  it('banner shows N of M and time left', () => {
    set(event({ voters: 2, members: 3 }))
    show(QotyBanner, { serverId: 'S1' })
    expect(text(banner())).toContain('2 of 3 have voted')
    expect(text(banner())).toContain('2h 5m left')
    set(event({ ends_at: null }))
    expect(text(banner())).toContain('until ended')
    // a vote whose time has run out is not live, even before the server says it ended
    set(event({ ends_at: new Date(Date.now() - 60_000).toISOString() }))
    expect(banner()).toBeNull()
  })

  it('draws its pixel spill with crisp edges, the same for the same event', () => {
    set(event())
    show(QotyBanner, { serverId: 'S1' })
    const svg = banner()!.querySelector('svg.pixel-spill')!
    expect(svg.getAttribute('shape-rendering')).toBe('crispEdges')
    const first = svg.innerHTML
    set(event({ voters: 1 }))
    expect(banner()!.querySelector('svg.pixel-spill')!.innerHTML).toBe(first)
  })
})

describe('Quote of the Year voting', () => {
  async function openVoting(e = event()) {
    set(e)
    show(QotyBanner, { serverId: 'S1' })
    button('Vote', banner()!)!.click()
    flushSync()
    await settle()
    expect(document.querySelector('[role=dialog][aria-label="Quote of the Year 2026"]')).toBeTruthy()
  }

  it('voting calls vote, move and unvote', async () => {
    await openVoting()
    expect(text(option('Q1'))).toContain('first one')
    expect(text(option('Q1'))).toContain('sam')
    expect(text(option('Q1'))).toContain('4 reactions')
    button('Vote', option('Q1'))!.click()
    expect(calls('vote_qoty')).toEqual([{ eventId: 'E1', messageId: 'Q1' }])
    // the server's answer: my vote is on Q1
    set(event({ options: [opt(quote('Q1', 'first one', 'U2', 4), ['U1']), opt(quote('Q2', 'second one', 'U3', 2))], voters: 1 }))
    expect(option('Q1').classList.contains('mine')).toBe(true)
    expect(button('Your vote', option('Q1'))!.getAttribute('aria-pressed')).toBe('true')
    button('Vote', option('Q2'))!.click()
    expect(calls('vote_qoty')).toEqual([{ eventId: 'E1', messageId: 'Q1' }, { eventId: 'E1', messageId: 'Q2' }])
    button('Your vote', option('Q1'))!.click()
    expect(calls('unvote_qoty')).toEqual([{ eventId: 'E1' }])
  })

  it('each option shows who voted', async () => {
    await openVoting(event({
      options: [opt(quote('Q1', 'first one'), ['U2', 'U3']), opt(quote('Q2', 'second one'), [], 'U3')], voters: 2,
    }))
    expect(option('Q1').querySelector('[role=img]')?.getAttribute('aria-label')).toBe('sam and kim')
    expect(text(option('Q1'))).toContain('2')
    expect(option('Q2').querySelector('[role=img]')).toBeNull()
    expect(text(option('Q2'))).toContain('added by kim')
    expect(text(option('Q1'))).not.toContain('added by')
  })

  it('end asks first then calls end_qoty', async () => {
    await openVoting()
    button('End voting…')!.click()
    flushSync()
    expect(text(document.querySelector('.confirm'))).toContain("End voting now? Everyone's votes are final.")
    expect(calls('end_qoty')).toEqual([])
    button('Cancel', document.querySelector('.confirm')!)!.click()
    flushSync()
    expect(document.querySelector('.confirm')).toBeNull()
    button('End voting…')!.click()
    flushSync()
    button('End voting', document.querySelector('.confirm')!)!.click()
    expect(calls('end_qoty')).toEqual([{ eventId: 'E1' }])
  })

  it('adds a quote found by search', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'search_quotes' ? [quote('Q1', 'first one'), quote('Q7', 'from december')] : undefined))
    await openVoting()
    button('Add a quote')!.click()
    flushSync()
    const input = document.querySelector<HTMLInputElement>('input[aria-label="Search quotes"]')!
    input.value = 'dec'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await new Promise((r) => setTimeout(r, 300))
    await settle()
    expect(calls('search_quotes')).toEqual([{ serverId: 'S1', q: 'dec' }])
    // already an option: not offered again
    const results = [...document.querySelectorAll<HTMLButtonElement>('.results button')]
    expect(results.map(text)).toEqual(['“from december” sam'])
    results[0].click()
    expect(calls('add_qoty_option')).toEqual([{ eventId: 'E1', messageId: 'Q7' }])
  })

  it('animates a vote', async () => {
    await openVoting()
    button('Vote', option('Q1'))!.click()
    flushSync()
    expect(option('Q1').classList.contains('popped')).toBe(true)
  })
})

describe('Quote of the Year start', () => {
  it('start offers 3 hours, a day, custom 5 min-14 days and until ended', async () => {
    show(QotyStart, { serverId: 'S1', onClose: () => {} })
    expect([...document.querySelectorAll('[role=radiogroup][aria-label="Length"] [role=radio]')].map(text))
      .toEqual(['3 hours', '1 day', 'Custom', 'Until ended'])
    const start = () => button('Start')!
    // a start in flight ignores more presses: let each one finish
    const press = async () => {
      start().click()
      await settle()
    }
    await press()
    expect(calls('start_qoty').at(-1)).toEqual({ serverId: 'S1', body: { year: defaultQotyYear(new Date()), minutes: 180 } })
    radio('1 day')!.click()
    flushSync()
    await press()
    expect(calls('start_qoty').at(-1)).toMatchObject({ body: { minutes: 1440 } })
    radio('Until ended')!.click()
    flushSync()
    await press()
    expect(calls('start_qoty').at(-1)).toMatchObject({ body: { minutes: null } })

    radio('Custom')!.click()
    flushSync()
    const amount = document.querySelector<HTMLInputElement>('input[aria-label="Custom length"]')!
    const type = (v: string) => {
      amount.value = v
      amount.dispatchEvent(new Event('input', { bubbles: true }))
      flushSync()
    }
    type('4')
    expect(start().disabled).toBe(true)
    type('5')
    expect(start().disabled).toBe(false)
    await press()
    expect(calls('start_qoty').at(-1)).toMatchObject({ body: { minutes: 5 } })
    radio('hours')!.click()
    flushSync()
    type('336') // 14 days
    expect(start().disabled).toBe(false)
    await press()
    expect(calls('start_qoty').at(-1)).toMatchObject({ body: { minutes: 20160 } })
    type('337')
    expect(start().disabled).toBe(true)
  })

  it('start defaults to last year in January', () => {
    vi.useFakeTimers({ toFake: ['Date'] })
    vi.setSystemTime(new Date(2027, 0, 12, 12))
    show(QotyStart, { serverId: 'S1', onClose: () => {} })
    expect(document.querySelector<HTMLInputElement>('input[aria-label="Year"]')!.value).toBe('2026')
    unmount(mounted.pop()!)
    vi.setSystemTime(new Date(2027, 1, 1, 12))
    show(QotyStart, { serverId: 'S1', onClose: () => {} })
    expect(document.querySelector<HTMLInputElement>('input[aria-label="Year"]')!.value).toBe('2027')
  })

  it('shows why the server refused, and closes once started', async () => {
    const onClose = vi.fn()
    invoke.mockImplementation(async () => {
      throw { status: 409, message: 'There is already a vote for 2026.' }
    })
    show(QotyStart, { serverId: 'S1', onClose })
    button('Start')!.click()
    await settle()
    expect(document.querySelector('[role=alert]')).toBeTruthy()
    expect(onClose).not.toHaveBeenCalled()
    invoke.mockImplementation(async () => event())
    button('Start')!.click()
    await settle()
    expect(onClose).toHaveBeenCalled()
  })
})

describe('Quote of the Year voice strip', () => {
  it('the strip shows the leader in voice', () => {
    show(QotyStrip, { serverId: 'S1' })
    expect(document.querySelector('.qoty-strip')).toBeNull()
    set(event({ options: [opt(quote('Q1', 'first one', 'U2'), ['U1', 'U3']), opt(quote('Q2', 'second one', 'U3'), ['U2'])], voters: 3 }))
    expect(text(document.querySelector('.qoty-strip'))).toContain('Quote of the Year · sam leads with 2 · 2h 5m left')
    expect(button('Open voting')).toBeTruthy()
    set(event({ ends_at: null }))
    expect(text(document.querySelector('.qoty-strip'))).toContain('Quote of the Year · no votes yet · until ended')
    button('Open voting')!.click()
    flushSync()
    expect(document.querySelector('[role=dialog][aria-label="Quote of the Year 2026"]')).toBeTruthy()
  })
})

describe('Quote of the Year winner card', () => {
  const showCard = (m: Message, q: Message | null = quote('Q1', 'first one')) => show(QotyCard, { message: m, serverId: 'S1', quote: q })

  it('the card shows who voted', () => {
    showCard(card())
    const c = document.querySelector('.qoty-card')!
    expect(text(c)).toContain('“')
    expect(text(c)).toContain('first one')
    expect(text(c)).toContain('sam')
    expect(text(c)).toContain('Quote of the Year 2026')
    expect(text(c)).toContain('2 votes · ended early')
    expect(c.querySelector('[role=img]')?.getAttribute('aria-label')).toBe('sam and kim')
  })

  it('without the quote loaded it shows the text the server wrote', () => {
    showCard(card({}, ['U2']), null)
    const c = document.querySelector('.qoty-card')!
    expect(text(c)).toContain('“first one” — sam')
    expect(text(c)).toContain('1 vote · ended early')
  })

  it('the no-vote line is plain', () => {
    showCard(card({ content: "Nobody voted in Quote of the Year 2026, so there's no winner.", qoty_card: { event_id: 'E1', year: 2026, message_id: null, votes: 0, voters: [], reason: 'time' } }), null)
    expect(document.querySelector('.qoty-card')).toBeNull()
    expect(text(document.body)).toContain("Nobody voted in Quote of the Year 2026, so there's no winner.")
  })

  it('a removed card shows nothing', () => {
    showCard(card({ deleted: true, content: '' }))
    expect(text(document.body)).toBe('')
  })

  it('card offers reopen only within the window', async () => {
    const ended = (reopen: number | null) => event({ ended_at: new Date().toISOString(), end_reason: 'early', reopen_until: reopen === null ? null : new Date(Date.now() + reopen).toISOString() })
    set(ended(HOUR))
    showCard(card())
    expect(button('Reopen voting')).toBeTruthy()
    set(ended(-60_000))
    expect(button('Reopen voting')).toBeUndefined()
    set(ended(null))
    expect(button('Reopen voting')).toBeUndefined()
    // another event (a later year) doesn't make this card reopenable
    set({ ...ended(HOUR), id: 'E2' })
    expect(button('Reopen voting')).toBeUndefined()

    set(ended(HOUR))
    button('Reopen voting')!.click()
    flushSync()
    expect(document.querySelector('input[aria-label="Year"]')).toBeNull()
    expect([...document.querySelectorAll('[role=radiogroup][aria-label="Length"] [role=radio]')].map(text))
      .toEqual(['3 hours', '1 day', 'Custom', 'Until ended'])
    button('Reopen')!.click()
    await settle()
    expect(calls('reopen_qoty')).toEqual([{ eventId: 'E1', minutes: 180 }])
    expect(document.querySelector('[role=dialog]')).toBeNull() // done: the dialog closes
    button('Reopen voting')!.click()
    flushSync()
    radio('Until ended')!.click()
    flushSync()
    button('Reopen')!.click()
    expect(calls('reopen_qoty').at(-1)).toEqual({ eventId: 'E1', minutes: null })
  })
})

describe('Quote of the Year motion', () => {
  it('no animation classes under reduced motion', async () => {
    // a fresh winner card animates in; one from earlier doesn't
    mounted.push(mount(QotyCard, { target: document.body, props: { message: card({ created_at: new Date().toISOString() }), serverId: 'S1', quote: quote('Q1', 'first one') } }))
    flushSync()
    expect(document.querySelector('.qoty-card.reveal')).toBeTruthy()
    unmount(mounted.pop()!)
    mounted.push(mount(QotyCard, { target: document.body, props: { message: card({ created_at: '2026-01-01T00:00:00Z' }), serverId: 'S1', quote: quote('Q1', 'first one') } }))
    flushSync()
    expect(document.querySelector('.qoty-card.reveal')).toBeNull()
    unmount(mounted.pop()!)

    motion.reducedMotion = true
    mounted.push(mount(QotyCard, { target: document.body, props: { message: card({ created_at: new Date().toISOString() }), serverId: 'S1', quote: quote('Q1', 'first one') } }))
    flushSync()
    expect(document.querySelector('.qoty-card')).toBeTruthy()
    expect(document.querySelector('.qoty-card.reveal')).toBeNull()

    set(event())
    show(QotyBanner, { serverId: 'S1' })
    button('Vote', banner()!)!.click()
    flushSync()
    button('Vote', option('Q1'))!.click()
    flushSync()
    expect(option('Q1').classList.contains('popped')).toBe(false)
    expect(document.querySelector('.qoty-voting.animated')).toBeNull()
  })
})
