// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Leaderboard as Board } from '../lib/protocol/Leaderboard'
import type { Message } from '../lib/protocol/Message'

const h = vi.hoisted(() => ({ handlers: {} as Record<string, (e: { payload: unknown }) => void> }))
const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, cb: (e: { payload: unknown }) => void) => {
    h.handlers[name] = cb
    return () => {}
  },
}))

import Leaderboard, { resetPeriod } from './Leaderboard.svelte'
import { LOADING_DELAY_MS } from '../lib/loading'
import { REFRESH_DEBOUNCE_MS } from '../lib/leaderboard'
import { localDay } from '../lib/quoteFromMessage'
import { app, handleReady, resetState, startListening } from '../lib/store.svelte'
import { testMe, testReady } from '../lib/testReady'
import { nextTask } from '../lib/testSettle'

const person = (id: string, name: string) => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const sam = person('U2', 'sam')
const kim = person('U3', 'kim')
const jo = person('U4', 'jo')

const quote = (id: string, text: string | string[], reactions = 0, speaker = 'U2'): Message => ({
  id, channel_id: 'C1', author_id: 'U1', kind: 'normal', content: '', reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [],
  reactions: reactions ? [{ emoji: '😂', count: reactions, me: false }] : [], thread_root_id: null, qoty_card: null, clip: null, embeds: [],
  quote: {
    lines: (Array.isArray(text) ? text : [text]).map((t, i) => ({ id: `L${id}${i}`, speaker_id: speaker, speaker_name: null, text: t, media: null, caption: null, awards: 0, awarders: [], awarded: false })),
    year: 2025, said_on: null, reply_count: 0, last_reply_id: null, replies_seen: null, wins: [],
  },
})
const board = (over: Partial<Board> = {}): Board => ({
  winners: [{ year: 2025, message: quote('Q1', 'the winner'), votes: 3, voters: ['U1', 'U2', 'U3'], speakers: [] }],
  top_year: [], top_all_time: [],
  top: [
    { message: quote('Q2', 'funny this month', 6), score: 4 },
    { message: quote('Q3', ['first line', 'second line'], 2, 'U3'), score: 2 },
    { message: quote('Q4', 'my own line', 1, 'U1'), score: 1 },
  ],
  most_quoted: [
    { user_id: 'U2', name: null, count: 7 },
    { user_id: 'U1', name: null, count: 5 },
    { user_id: 'U3', name: null, count: 5 },
    { user_id: null, name: 'a friend', count: 2 },
    { user_id: 'U4', name: null, count: 1 },
  ],
  most_awarded: [{ user_id: 'U3', name: null, count: 5 }],
  ...over,
})

let cmp: ReturnType<typeof mount> | undefined
const onOpen = vi.fn()
const onClose = vi.fn()
async function show() {
  cmp = mount(Leaderboard, { target: document.body, props: { serverId: 'S1', onOpen, onClose } })
  flushSync()
  await nextTask()
  flushSync()
}
const section = (title: string) =>
  [...document.querySelectorAll('section')].find((s) => s.querySelector('h3')?.textContent?.trim() === title)
const tabs = () => [...document.querySelectorAll<HTMLButtonElement>('[role=tab]')]
const boardCalls = () => invoke.mock.calls.filter((c) => c[0] === 'leaderboard').map((c) => c[1])
const event = (t: string, d: unknown) => h.handlers['pulse://event']({ payload: { t, d } })

beforeEach(async () => {
  resetState()
  resetPeriod()
  await startListening()
  handleReady(testReady([{ id: 'C1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0 }], {
    members: [{ server_id: 'S1', members: [testMe, sam, kim, jo].map((user) => ({ user, nickname: null })) }],
  }))
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string) => (cmd === 'leaderboard' ? board() : undefined))
  onOpen.mockClear()
  onClose.mockClear()
})
afterEach(() => {
  vi.useRealTimers()
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('Leaderboard', () => {
  it('asks for the period you are on, counted from your local day, and draws each section', async () => {
    await show()
    expect(boardCalls()).toEqual([{ serverId: 'S1', period: 'month', today: localDay() }])
    expect([...document.querySelectorAll('section h3')].map((x) => x.textContent?.trim())).toEqual([
      'Most reacted', 'Most quoted', 'Quote awards', 'Quote of the Year',
    ])
    expect(section('Most reacted')!.textContent).toContain('funny this month')
    expect(section('Quote awards')!.textContent).toContain('all time')
    expect(section('Quote awards')!.textContent).toContain('kim')
  })

  describe('period tabs', () => {
    it('is a tablist: one selected tab in the tab order, tied to its panel', async () => {
      await show()
      expect(document.querySelector('[role=tablist]')?.getAttribute('aria-label')).toBeTruthy()
      expect(tabs().map((t) => t.textContent)).toEqual(['Week', 'Month', 'Year', 'All time'])
      expect(tabs().map((t) => t.getAttribute('aria-selected'))).toEqual(['false', 'true', 'false', 'false'])
      expect(tabs().map((t) => t.tabIndex)).toEqual([-1, 0, -1, -1])
      const panel = document.querySelector('[role=tabpanel]')!
      expect(panel.getAttribute('aria-labelledby')).toBe(tabs()[1].id)
      expect(tabs()[1].getAttribute('aria-controls')).toBe(panel.id)
      expect(document.activeElement).toBe(tabs()[1])
    })

    it('arrow keys, Home and End move between tabs and pick the period', async () => {
      await show()
      tabs()[1].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }))
      flushSync()
      expect(document.activeElement).toBe(tabs()[2])
      expect(tabs()[2].getAttribute('aria-selected')).toBe('true')
      expect(boardCalls().at(-1)).toMatchObject({ period: 'year' })
      tabs()[2].dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true }))
      flushSync()
      expect(boardCalls().at(-1)).toMatchObject({ period: 'all' })
      tabs()[3].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }))
      flushSync()
      expect(document.activeElement).toBe(tabs()[0])
      expect(boardCalls().at(-1)).toMatchObject({ period: 'week' })
      tabs()[0].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true }))
      flushSync()
      expect(document.activeElement).toBe(tabs()[3])
    })

    it('an empty period says so and offers the next longer one', async () => {
      invoke.mockImplementation(async (cmd: string, a?: unknown) =>
        cmd === 'leaderboard' ? ((a as { period: string }).period === 'week' ? board({ top: [], most_quoted: [] }) : board()) : undefined)
      await show()
      tabs()[0].click()
      await nextTask()
      flushSync()
      expect(section('Most reacted')!.textContent).toContain('No reactions on quotes this week.')
      expect(section('Most quoted')!.textContent).toContain('Nobody has been quoted this week.')
      ;[...section('Most reacted')!.querySelectorAll('button')].find((b) => b.textContent === 'See this month')!.click()
      flushSync()
      expect(tabs()[1].getAttribute('aria-selected')).toBe('true')
    })
  })

  describe('most reacted', () => {
    it('ranks quotes with the score they were ranked by, and the reaction total in its tip', async () => {
      await show()
      const rows = [...section('Most reacted')!.querySelectorAll('li')]
      expect(rows.map((r) => r.querySelector('.rank')?.textContent)).toEqual(['1', '2', '3'])
      expect(rows.map((r) => r.querySelector('.count .n')?.textContent)).toEqual(['4', '2', '1'])
      expect(rows[0].querySelector('.count')?.getAttribute('data-tip')).toBe('4 people reacted (6 reactions)')
      expect(rows[1].querySelector('.count')?.getAttribute('data-tip')).toBe('2 people reacted')
      expect(rows[0].querySelector('.rank')?.classList.contains('g1')).toBe(true)
    })

    it('bars under people rows are each count as a share of the leader', async () => {
      invoke.mockImplementation(async (cmd: string) => (cmd === 'leaderboard'
        ? board({ most_awarded: [{ user_id: 'U3', name: null, count: 8 }, { user_id: 'U2', name: null, count: 4 }, { user_id: null, name: 'a friend', count: 2 }] })
        : undefined))
      await show()
      const rows = [...section('Quote awards')!.querySelectorAll<HTMLElement>('li')]
      expect(rows.map((r) => r.style.getPropertyValue('--share'))).toEqual(['1', '0.5', '0.25'])
      expect(rows.map((r) => r.querySelector('.n')?.textContent)).toEqual(['8', '4', '2'])
      expect(rows[0].textContent).toContain('8 awards')
    })

    it('a quote jumps to the chat, and its button reads the whole quote and its score', async () => {
      await show()
      const jump = section('Most reacted')!.querySelector<HTMLButtonElement>('button.jump')!
      expect(jump.getAttribute('aria-label')).toBe('1. “funny this month” — sam. 4 people reacted. Show in chat')
      jump.click()
      expect(onOpen).toHaveBeenCalledWith(board().top[0].message)
    })

    it('a quote of more lines shows two lines and expands to every line', async () => {
      await show()
      const row = [...section('Most reacted')!.querySelectorAll('li')][1]
      expect(row.textContent).toContain('first line')
      expect(row.textContent).toContain('+1 more')
      expect(row.textContent).not.toContain('second line')
      expect(row.querySelector('.jump')?.getAttribute('data-tip')).toBe('“first line” — kim\n“second line” — kim')
      const expand = row.querySelector<HTMLButtonElement>('button.expand')!
      expect(expand.getAttribute('aria-expanded')).toBe('false')
      expand.click()
      flushSync()
      expect(expand.getAttribute('aria-expanded')).toBe('true')
      expect(row.textContent).toContain('second line')
      // a one-line quote that fits has nothing to expand
      expect([...section('Most reacted')!.querySelectorAll('li')][0].querySelector('button.expand')).toBeNull()
    })

    it('marks a quote you said', async () => {
      await show()
      const rows = [...section('Most reacted')!.querySelectorAll('li')]
      expect(rows.map((r) => r.classList.contains('mine'))).toEqual([false, false, true])
      expect(rows[2].querySelector('.you')?.textContent).toBe('you')
    })
  })

  describe('most quoted', () => {
    it('stands the top three on a podium, in rank order for a screen reader, and lists the rest', async () => {
      await show()
      const places = [...section('Most quoted')!.querySelectorAll('.podium > li')]
      expect(places.map((p) => [...p.classList].find((c) => /^p\d$/.test(c)))).toEqual(['p1', 'p2', 'p3'])
      expect(places.map((p) => p.querySelector('.who-col')?.textContent)).toEqual(['sam', 'alex', 'kim'])
      expect(places[0].textContent).toContain('7 quotes')
      // ties share a rank
      expect(places.map((p) => p.querySelector('.place-n')?.textContent)).toEqual(['1', '2', '2'])
      const rows = [...section('Most quoted')!.querySelectorAll('.rows > li')]
      expect(rows.map((r) => r.querySelector('.who-col')?.textContent)).toEqual(['a friend', 'jo'])
      expect(rows.map((r) => r.querySelector('.rank')?.textContent)).toEqual(['4', '5'])
    })

    it('highlights you', async () => {
      await show()
      const places = [...section('Most quoted')!.querySelectorAll('.podium > li')]
      expect(places.map((p) => p.classList.contains('mine'))).toEqual([false, true, false])
      expect(places[1].querySelector('.you')?.textContent).toBe('you')
    })

    it('a person opens their menu on right-click; a plain name has none', async () => {
      await show()
      const place = section('Most quoted')!.querySelector<HTMLElement>('.podium > li')!
      place.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 10 }))
      flushSync()
      expect(document.querySelector('.person-menu')).toBeTruthy()
      // no buttons on avatars
      expect(document.querySelector('.leaderboard .face button, .leaderboard .face-sm button')).toBeNull()
    })
  })

  describe('Quote of the Year', () => {
    it('winners show the year, the words, the votes and who voted', async () => {
      await show()
      const win = section('Quote of the Year')!
      expect(win.textContent).toContain('2025')
      expect(win.textContent).toContain('the winner')
      expect(win.textContent).toContain('3 votes')
      expect(win.querySelector('.stack')?.getAttribute('aria-label')).toBe('alex, sam and kim')
      win.querySelector<HTMLButtonElement>('button.jump')!.click()
      expect(onOpen).toHaveBeenLastCalledWith(board().winners[0].message)
    })

    it("a winner names who said it when the vote ended, not today's speaker", async () => {
      const b = board()
      b.winners[0].speakers = [{ user_id: 'U2', name: null }, { user_id: 'U3', name: null }]
      invoke.mockImplementation(async (cmd: string) => (cmd === 'leaderboard' ? b : undefined))
      await show()
      expect(section('Quote of the Year')!.querySelector('.by')?.textContent).toBe('sam & kim')
    })

    it('a deleted winner keeps its year, speakers and votes, and has nothing to jump to', async () => {
      const b = board()
      b.winners[0] = { ...b.winners[0], message: { ...quote('Q1', 'x'), content: '', deleted: true, quote: null }, speakers: [{ user_id: null, name: 'A Friend' }] }
      invoke.mockImplementation(async (cmd: string) => (cmd === 'leaderboard' ? b : undefined))
      await show()
      const win = section('Quote of the Year')!
      expect(win.querySelector('button.jump')).toBeNull()
      expect(win.textContent).toContain('This quote was deleted.')
      expect(win.querySelector('.by')?.textContent).toBe('A Friend')
      expect(win.textContent).toContain('3 votes')
    })
  })

  describe('live', () => {
    it('refetches once, after a burst of relevant events goes quiet', async () => {
      await show()
      vi.useFakeTimers()
      const before = boardCalls().length
      event('ReactionAdded', { channel_id: 'C1', message_id: 'Q2', emoji: '😂', user_id: 'U3', count: 2 })
      vi.advanceTimersByTime(REFRESH_DEBOUNCE_MS / 2)
      event('ReactionAdded', { channel_id: 'C1', message_id: 'Q3', emoji: '😂', user_id: 'U3', count: 3 })
      vi.advanceTimersByTime(REFRESH_DEBOUNCE_MS - 1)
      expect(boardCalls().length).toBe(before)
      vi.advanceTimersByTime(1)
      expect(boardCalls().length).toBe(before + 1)
      expect(boardCalls().at(-1)).toMatchObject({ period: 'month' })
    })

    it('ignores events that cannot change it', async () => {
      await show()
      vi.useFakeTimers()
      const before = boardCalls().length
      event('Typing', { channel_id: 'C1', user_ids: ['U2'] })
      event('ReactionAdded', { channel_id: 'C9', message_id: 'X', emoji: '😂', user_id: 'U3', count: 1 })
      vi.advanceTimersByTime(REFRESH_DEBOUNCE_MS * 2)
      expect(boardCalls().length).toBe(before)
    })

    it('keeps showing the board while a refresh is on its way', async () => {
      await show()
      vi.useFakeTimers()
      invoke.mockImplementation(() => new Promise(() => {}))
      event('QuoteAwardChanged', { channel_id: 'C1', message_id: 'Q2', line_id: 'L', user_id: 'U3', given: true, count: 1, awarders: ['U3'] })
      vi.advanceTimersByTime(REFRESH_DEBOUNCE_MS)
      flushSync()
      expect(section('Most reacted')!.textContent).toContain('funny this month')
    })
  })

  it('says when there are no quotes at all, and shows a load error', async () => {
    invoke.mockImplementation(async () => ({ winners: [], top_year: [], top_all_time: [], top: [], most_quoted: [], most_awarded: [] }))
    await show()
    tabs()[3].click()
    await nextTask()
    flushSync()
    expect(document.body.textContent).toContain('No quotes yet.')
    unmount(cmp!)
    invoke.mockImplementation(async () => { throw { status: 500, message: 'boom' } })
    await show()
    expect(document.querySelector('[role=alert]')).toBeTruthy()
  })

  it('says loading only once the board is slow to come', () => {
    vi.useFakeTimers()
    invoke.mockImplementation(() => new Promise(() => {}))
    cmp = mount(Leaderboard, { target: document.body, props: { serverId: 'S1', onOpen, onClose } })
    flushSync()
    expect(document.body.textContent).not.toContain('Loading…')
    vi.advanceTimersByTime(LOADING_DELAY_MS)
    flushSync()
    expect(document.body.textContent).toContain('Loading…')
  })

  it('the most Steam hours over two weeks, when anyone shares some (#383)', async () => {
    invoke.mockImplementation(async (cmd: string) =>
      cmd === 'leaderboard' ? board() : cmd === 'steam_hours' ? { entries: [{ user_id: 'U3', minutes: 1290 }, { user_id: 'U2', minutes: 45 }] } : undefined)
    await show()
    expect(invoke).toHaveBeenCalledWith('steam_hours', { serverId: 'S1' })
    const rows = [...document.querySelectorAll('[data-steam-hours] li')]
    expect(rows.map((r) => [r.querySelector('.rank')?.textContent, r.querySelector('.who-col')?.textContent, r.querySelector('.n')?.textContent])).toEqual([
      ['1', 'kim', '21.5 h'], ['2', 'sam', '45 min'],
    ])
  })

  it('no Steam hours section when nobody has any, or the server has no such board', async () => {
    await show()
    expect(document.querySelector('[data-steam-hours]')).toBeNull()
  })

  it('Escape and the close button close it', async () => {
    await show()
    await nextTask() // the outside-click listener attaches on the next tick
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    expect(onClose).toHaveBeenCalledTimes(1)
    document.querySelector<HTMLButtonElement>('button[aria-label="Close leaderboard"]')!.click()
    expect(onClose).toHaveBeenCalledTimes(2)
    expect(app.state.me).toBeTruthy()
  })
})
