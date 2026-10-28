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
import { testMe, testQoty, testReady } from '../lib/testReady'
import { reactiveProps } from '../lib/testProps.svelte'

const sam = { id: 'U2', username: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const kim = { id: 'U3', username: 'kim', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const HOUR = 3_600_000

const quote = (id: string, text: string, speaker = 'U2', reactions = 0): Message => ({
  id, channel_id: 'C1', author_id: 'U1', kind: 'normal', content: `“${text}”`, reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [],
  reactions: reactions ? [{ emoji: '😂', count: reactions, me: false }] : [], thread_root_id: null, qoty_card: null, clip: null,
  quote: {
    lines: [{ id: `L${id}`, speaker_id: speaker, speaker_name: null, text, media: null, caption: null, awards: 0, awarders: [], awarded: false }],
    year: 2026, said_on: null, reply_count: 0, last_reply_id: null, replies_seen: null, wins: [],
  },
})
const opt = (m: Message, voters: string[] = [], added_by: string | null = null): QotyOption => ({ message: m, added_by, votes: voters.length, voters })
const event = (over: Partial<QotyEvent> = {}): QotyEvent => testQoty({
  started_by: 'U2', ends_at: new Date(Date.now() + 2 * HOUR + 5 * 60_000 + 30_000).toISOString(),
  options: [opt(quote('Q1', 'first one', 'U2', 4)), opt(quote('Q2', 'second one', 'U3', 2))], members: 3, ...over,
})
const card = (over: Partial<Message> = {}, voters = ['U2', 'U3']): Message => ({
  id: 'K1', channel_id: 'C1', author_id: null, kind: 'system', content: 'Quote of the Year 2026: “first one” — sam', reply_to_id: null,
  created_at: new Date(Date.now() - 5 * 60_000).toISOString(), edited_at: null, deleted: false, mentions: [], reactions: [],
  thread_root_id: null, quote: null, clip: null,
  qoty_card: { event_id: 'E1', year: 2026, message_id: 'Q1', votes: voters.length, voters, reason: 'early', speakers: [] },
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
    await settle()
    expect(calls('vote_qoty')).toEqual([{ eventId: 'E1', messageId: 'Q1' }])
    // the server's answer: my vote is on Q1
    set(event({ options: [opt(quote('Q1', 'first one', 'U2', 4), ['U1']), opt(quote('Q2', 'second one', 'U3', 2))], voters: 1 }))
    expect(option('Q1').classList.contains('mine')).toBe(true)
    expect(button('Your vote', option('Q1'))!.getAttribute('aria-pressed')).toBe('true')
    button('Vote', option('Q2'))!.click()
    await settle()
    expect(calls('vote_qoty')).toEqual([{ eventId: 'E1', messageId: 'Q1' }, { eventId: 'E1', messageId: 'Q2' }])
    button('Your vote', option('Q1'))!.click()
    expect(calls('unvote_qoty')).toEqual([{ eventId: 'E1' }])
  })

  it('each option shows who voted', async () => {
    await openVoting(event({
      options: [opt(quote('Q1', 'first one'), ['U2', 'U3']), opt(quote('Q2', 'second one'), [], 'U3')], voters: 2,
    }))
    expect(option('Q1').querySelector('[role=img]')?.getAttribute('aria-label')).toBe('sam and kim')
    // the number is read out as "2 votes"
    expect(option('Q1').querySelector('.count')?.getAttribute('aria-hidden')).toBe('true')
    expect(text(option('Q1').querySelector('.tally'))).toContain('2 votes')
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
    const won = quote('Q8', 'won before')
    won.quote!.wins = [2025]
    invoke.mockImplementation(async (cmd: string) => (cmd === 'search_quotes' ? [quote('Q1', 'first one'), quote('Q7', 'from december'), won] : undefined))
    await openVoting()
    button('Add a quote')!.click()
    flushSync()
    const input = document.querySelector<HTMLInputElement>('input[aria-label="Search quotes"]')!
    input.value = 'dec'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await new Promise((r) => setTimeout(r, 300))
    await settle()
    expect(calls('search_quotes')).toEqual([{ serverId: 'S1', q: 'dec' }])
    // already an option, or won an earlier year: not offered
    const results = [...document.querySelectorAll<HTMLButtonElement>('.results button')]
    expect(results.map(text)).toEqual(['“from december” sam'])
    results[0].click()
    expect(calls('add_qoty_option')).toEqual([{ eventId: 'E1', messageId: 'Q7' }])
  })

  it('animates a vote', async () => {
    await openVoting()
    button('Vote', option('Q1'))!.click()
    await settle()
    expect(option('Q1').classList.contains('popped')).toBe(true)
  })

  it('a vote in flight holds further clicks', async () => {
    let answer: () => void = () => {}
    invoke.mockImplementation((cmd: string) => (cmd === 'vote_qoty' ? new Promise<undefined>((r) => (answer = () => r(undefined))) : Promise.resolve(undefined)))
    await openVoting()
    button('Vote', option('Q1'))!.click()
    flushSync()
    expect(button('Vote', option('Q2'))!.disabled).toBe(true)
    button('Vote', option('Q1'))!.click()
    button('Vote', option('Q2'))!.click()
    expect(calls('vote_qoty')).toEqual([{ eventId: 'E1', messageId: 'Q1' }])
    answer()
    await settle()
    expect(button('Vote', option('Q2'))!.disabled).toBe(false)
  })

  it('a refused vote shows why and does not celebrate', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'vote_qoty') throw { status: 409, message: 'That vote has ended.' }
    })
    await openVoting()
    button('Vote', option('Q1'))!.click()
    await settle()
    expect(option('Q1').classList.contains('popped')).toBe(false)
    expect(document.querySelector('.qoty-voting [role=alert]')).toBeTruthy()
  })
})

describe('Quote of the Year voting window lifecycle', () => {
  const dialog = () => document.querySelector('[role=dialog][aria-label="Quote of the Year 2026"]')
  const ended = () => event({ ended_at: new Date().toISOString(), end_reason: 'early' })

  it('a window open when the vote ends does not pop back open when it is reopened', async () => {
    set(event())
    show(QotyBanner, { serverId: 'S1' })
    button('Vote', banner()!)!.click()
    flushSync()
    expect(dialog()).toBeTruthy()
    app.state = applyEvent(app.state, { t: 'QotyEnded', d: { event: ended() } }, Date.now())
    flushSync()
    expect(dialog()).toBeNull()
    // the same event, reopened
    set(event())
    await settle()
    expect(banner()).toBeTruthy()
    expect(dialog()).toBeNull()
  })

  it('the strip\'s window does not pop back open when the vote is reopened', async () => {
    set(event())
    show(QotyStrip, { serverId: 'S1' })
    button('Open voting')!.click()
    flushSync()
    expect(dialog()).toBeTruthy()
    app.state = applyEvent(app.state, { t: 'QotyEnded', d: { event: ended() } }, Date.now())
    flushSync()
    set(event())
    await settle()
    expect(document.querySelector('.qoty-strip')).toBeTruthy()
    expect(dialog()).toBeNull()
  })

  it('switching to another server with a live vote does not open its window', async () => {
    set(event())
    set(event({ id: 'E2', server_id: 'S2' }))
    const props = reactiveProps({ serverId: 'S1' })
    mounted.push(mount(QotyBanner, { target: document.body, props }))
    flushSync()
    button('Vote', banner()!)!.click()
    flushSync()
    expect(dialog()).toBeTruthy()
    props.serverId = 'S2'
    flushSync()
    await settle()
    expect(banner()).toBeTruthy()
    expect(dialog()).toBeNull()
  })
})

describe('Quote of the Year voting window details', () => {
  async function openVoting(e = event()) {
    set(e)
    show(QotyBanner, { serverId: 'S1' })
    button('Vote', banner()!)!.click()
    flushSync()
    await settle()
  }
  const items = () => [...document.querySelectorAll<HTMLElement>('[role=menuitem]')].map((x) => x.textContent?.trim())
  const lineMenu = () => {
    option('Q1').querySelector<HTMLButtonElement>('button[aria-label="Line actions"]')!.click()
    flushSync()
  }

  it('options follow reactions, edits and awards of their quotes', async () => {
    await openVoting()
    expect(text(option('Q1'))).toContain('4 reactions')
    app.state = applyEvent(app.state, { t: 'ReactionAdded', d: { channel_id: 'C1', message_id: 'Q1', emoji: '😂', user_id: 'U3', count: 5 } }, Date.now())
    flushSync()
    expect(text(option('Q1'))).toContain('5 reactions')
    app.state = applyEvent(app.state, { t: 'MessageUpdated', d: { message: quote('Q1', 'first one, fixed', 'U2') } }, Date.now())
    flushSync()
    expect(text(option('Q1'))).toContain('first one, fixed')
    // an edit's reactions aren't trusted: the score stays
    expect(text(option('Q1'))).toContain('5 reactions')
    lineMenu()
    expect(items()).toContain('Give quote award')
    document.querySelector('[role=menu]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(document.querySelector('[role=menu]')).toBeNull()
    expect(dialogOpen()).toBe(true)
    app.state = applyEvent(app.state, { t: 'QuoteAwardChanged', d: { channel_id: 'C1', message_id: 'Q1', line_id: 'LQ1', user_id: 'U1', given: true, count: 1, awarders: ['U1'] } }, Date.now())
    flushSync()
    lineMenu()
    expect(items()).toContain('Take back award')
    expect(items()).not.toContain('Give quote award')
  })

  it('a line I awarded before the window opened offers to take it back', async () => {
    const q = quote('Q1', 'first one', 'U2')
    q.quote!.lines[0] = { ...q.quote!.lines[0], awards: 1, awarders: ['U1'], awarded: false } // as the server sends it
    await openVoting(event({ options: [opt(q), opt(quote('Q2', 'second one', 'U3'))] }))
    lineMenu()
    expect(items()).toContain('Take back award')
  })

  it('a second click right after a vote lands takes it back', async () => {
    invoke.mockImplementation(async (cmd: string) =>
      cmd === 'vote_qoty' ? event({ options: [opt(quote('Q1', 'first one', 'U2', 4), ['U1']), opt(quote('Q2', 'second one', 'U3', 2))], voters: 1 }) : undefined)
    await openVoting()
    button('Vote', option('Q1'))!.click()
    await settle()
    // the gateway's QotyUpdated hasn't arrived yet; the answer already says it's mine
    expect(option('Q1').classList.contains('mine')).toBe(true)
    button('Your vote', option('Q1'))!.click()
    await settle()
    expect(calls('vote_qoty')).toEqual([{ eventId: 'E1', messageId: 'Q1' }])
    expect(calls('unvote_qoty')).toEqual([{ eventId: 'E1' }])
  })

  it('vote buttons name their quote and the vote count is announced', async () => {
    await openVoting(event({ options: [opt(quote('Q1', 'first one', 'U2'), ['U1']), opt(quote('Q2', 'second one', 'U3'))], voters: 1 }))
    expect(option('Q1').querySelector('button.vote')!.getAttribute('aria-label')).toBe('Your vote: “first one” — sam')
    expect(option('Q2').querySelector('button.vote')!.getAttribute('aria-label')).toBe('Vote for “second one” — kim')
    const live = document.querySelector('.qoty-voting [aria-live=polite]')!
    expect(text(live)).toBe('1 of 3 have voted')
    // the banner's ticking line is not a live region
    expect(banner()!.querySelector('[aria-live]')).toBeNull()
  })

  it('the end question takes focus and is announced; Cancel and Escape return to "End voting…"', async () => {
    await openVoting()
    button('End voting…')!.click()
    flushSync()
    await settle()
    const cancel = button('Cancel', document.querySelector('.confirm')!)!
    expect(document.activeElement).toBe(cancel)
    const q = document.getElementById(cancel.getAttribute('aria-describedby') ?? '')
    expect(text(q)).toBe("End voting now? Everyone's votes are final.")
    cancel.click()
    flushSync()
    await settle()
    expect(document.activeElement).toBe(button('End voting…'))
    button('End voting…')!.click()
    flushSync()
    await settle()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    await settle()
    expect(document.querySelector('.confirm')).toBeNull()
    expect(dialogOpen()).toBe(true)
    expect(document.activeElement).toBe(button('End voting…'))
  })

  it('after adding a quote, focus goes back to "Add a quote"', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'search_quotes' ? [quote('Q7', 'from december')] : undefined))
    await openVoting()
    await search('dec')
    document.querySelector<HTMLButtonElement>('.results button')!.click()
    await settle()
    expect(document.querySelector('input[aria-label="Search quotes"]')).toBeNull()
    expect(document.activeElement).toBe(button('Add a quote'))
  })

  it('a full vote shows the server\'s answer when adding', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'search_quotes') return [quote('Q7', 'from december')]
      if (cmd === 'add_qoty_option') throw 'This vote has enough options.'
    })
    await openVoting()
    await search('dec')
    document.querySelector<HTMLButtonElement>('.results button')!.click()
    await settle()
    expect(text(document.querySelector('.qoty-voting [role=alert]'))).toBe('This vote has enough options.')
    // still adding: the search stays
    expect(document.querySelector('input[aria-label="Search quotes"]')).toBeTruthy()
  })

  const dialogOpen = () => !!document.querySelector('[role=dialog][aria-label="Quote of the Year 2026"]')
  async function search(q: string) {
    button('Add a quote')!.click()
    flushSync()
    const input = document.querySelector<HTMLInputElement>('input[aria-label="Search quotes"]')!
    input.value = q
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await new Promise((r) => setTimeout(r, 300))
    await settle()
  }
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
    // a tie names every leader
    set(event({ options: [opt(quote('Q1', 'first one', 'U2'), ['U1']), opt(quote('Q2', 'second one', 'U3'), ['U2']), opt(quote('Q3', 'third', 'U1'))], voters: 2 }))
    expect(text(document.querySelector('.qoty-strip'))).toContain('Quote of the Year · sam & kim lead with 1 · 2h 5m left')
    // each person once across the tied quotes: a "sam & kim" quote tied with a "sam" one
    const both = quote('Q4', 'together', 'U2')
    both.quote!.lines.push({ ...both.quote!.lines[0], id: 'LQ4b', speaker_id: 'U3', text: 'yes' })
    set(event({ options: [opt(both, ['U1']), opt(quote('Q5', 'alone', 'U2'), ['U3'])], voters: 2 }))
    expect(text(document.querySelector('.qoty-strip'))).toContain('Quote of the Year · sam & kim lead with 1 · 2h 5m left')
    // the verb follows the names shown: two tied quotes by sam are "sam leads"
    set(event({ options: [opt(quote('Q6', 'one', 'U2'), ['U1']), opt(quote('Q7', 'two', 'U2'), ['U3'])], voters: 2 }))
    expect(text(document.querySelector('.qoty-strip'))).toContain('Quote of the Year · sam leads with 1 · 2h 5m left')
    // ...and one quote said by sam and kim is "sam & kim lead"
    set(event({ options: [opt(both, ['U1', 'U3'])], voters: 2 }))
    expect(text(document.querySelector('.qoty-strip'))).toContain('Quote of the Year · sam & kim lead with 2 · 2h 5m left')
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

  it('a quote that is not loaded is fetched and shown with its lines and speakers', async () => {
    const q = quote('Q1', 'first one', 'U2')
    q.quote!.lines.push({ ...q.quote!.lines[0], id: 'LQ1b', speaker_id: 'U3', text: 'and another' })
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_messages_around' ? [quote('Q2', 'newer'), q, quote('Q0', 'older')] : undefined))
    showCard(card({}, ['U2']), null)
    await settle()
    expect(calls('list_messages_around')).toEqual([{ channelId: 'C1', messageId: 'Q1' }])
    const c = document.querySelector('.qoty-card')!
    expect([...c.querySelectorAll('.q p')].map(text)).toEqual(['“first one”', '“and another”'])
    expect(text(c.querySelector('.who'))).toBe('sam & kim · Quote of the Year 2026')
    expect(text(c)).toContain('1 vote · ended early')
    expect(text(c)).not.toContain('newer')
  })

  it('the card names the speakers as they were when the vote ended, not today\'s', () => {
    // The winning line was later changed to sam; the vote ended with kim saying it.
    const ended = card()
    ended.qoty_card!.speakers = [{ user_id: 'U3', name: null }]
    showCard(ended)
    expect(text(document.querySelector('.qoty-card .who'))).toBe('kim · Quote of the Year 2026')
  })

  it('a plain-name speaker in the snapshot shows that name', () => {
    const ended = card()
    ended.qoty_card!.speakers = [{ user_id: null, name: 'A Friend' }]
    showCard(ended)
    expect(text(document.querySelector('.qoty-card .who'))).toBe('A Friend · Quote of the Year 2026')
  })

  it('a quote that cannot be fetched falls back to the text the server wrote', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_messages_around') throw { code: 'network', message: 'offline' }
    })
    showCard(card({}, ['U2']), null)
    await settle()
    expect(text(document.querySelector('.qoty-card'))).toContain('“first one” — sam')
  })

  it('a winner that was deleted says so, with the speakers at the end, not its words', async () => {
    const snap = (m: Message) => {
      m.qoty_card!.speakers = [{ user_id: 'U2', name: null }]
      return m
    }
    const deleted = (where: string) => {
      const c = document.querySelectorAll('.qoty-card')
      const last = c[c.length - 1]
      expect(text(last.querySelector('.q')), where).toBe('This quote was deleted.')
      expect(text(last.querySelector('.who')), where).toBe('sam · Quote of the Year 2026')
      expect(text(last), where).not.toContain('first one')
    }
    // Loaded in the channel, deleted.
    showCard(snap(card()), { ...quote('Q1', 'first one'), deleted: true, content: '', quote: null })
    deleted('loaded')
    // Fetched: the window has it, deleted.
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_messages_around' ? [{ ...quote('Q1', 'x'), deleted: true, content: '', quote: null }] : undefined))
    showCard(snap(card()), null)
    await settle()
    deleted('fetched')
    // Fetched: the server no longer shows it.
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_messages_around' ? null : undefined))
    showCard(snap(card()), null)
    await settle()
    deleted('gone')
  })

  it('the no-vote line is plain', () => {
    showCard(card({ content: "Nobody voted in Quote of the Year 2026, so there's no winner.", qoty_card: { event_id: 'E1', year: 2026, message_id: null, votes: 0, voters: [], reason: 'time', speakers: [] } }), null)
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
    await settle()
    expect(calls('vote_qoty')).toEqual([{ eventId: 'E1', messageId: 'Q1' }])
    expect(option('Q1').classList.contains('popped')).toBe(false)
    expect(document.querySelector('.qoty-voting.animated')).toBeNull()
  })
})
