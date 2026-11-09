// @vitest-environment jsdom
// A poll in the chat (#157): the card's states (before you vote, after, as its creator, closed, anonymous), voting,
// End poll in the message's menu (its creator's, while open), no Edit, and the ended line's See poll.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { Poll } from '../lib/protocol/Poll'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import MessageItem from './MessageItem.svelte'
import { motion } from '../lib/animate.svelte'
import { app, ui } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { resetPolls } from '../lib/polls.svelte'

const NOW = Date.parse('2026-11-09T10:00:00.000Z')
const view = (over: Partial<Poll> = {}): Poll => ({
  question: 'Games tonight?', options: ['Valheim', 'Apex', 'Rust'], multi: false, anonymous: false,
  ends_at: '2026-11-09T13:00:00.000Z', closed_at: null, closed: false, voters: 0, mine: [], results: null, ...over,
})
const tallies = (votes: number[], voters: string[][] = votes.map(() => [])) => votes.map((v, i) => ({ votes: v, voters: voters[i] }))
const msg = (poll: Poll | null, over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U2', kind: 'normal', content: '📊 Poll: Games tonight?',
  reply_to_id: null, created_at: '2026-11-09T09:00:00.000Z', edited_at: null, deleted: false, mentions: [],
  reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], ...(poll ? { poll } : {}), ...over,
})
const names: Record<string, string> = { U1: 'alex', U2: 'sam', U3: 'kim' }

let cmp: ReturnType<typeof mount> | undefined
function show(m: Message, mine = false) {
  app.state = {
    ...emptyState(),
    channels: { C1: { id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 } },
    messages: { C1: [m] },
  }
  cmp = mount(MessageItem, { target: document.body, props: {
    message: m, serverId: null, name: names[m.author_id ?? ''] ?? '', nameOf: (id: string | null) => (id ? names[id] : 'Someone'), replyTo: null,
    grouped: false, mine, onReply: () => {}, onEdit: async () => {}, onDelete: async () => {}, meId: 'U1',
  } })
  flushSync()
}
const card = () => document.querySelector<HTMLElement>('.poll')!
const rows = () => [...card().querySelectorAll<HTMLElement>('.opt')].map((r) => r.textContent!.replace(/\s+/g, ' ').trim())
const choose = (label: string) => [...card().querySelectorAll<HTMLButtonElement>('button.choose')].find((b) => b.textContent!.trim() === label)!
const menuItems = () => [...document.querySelectorAll<HTMLButtonElement>('[role=menu][aria-label="Message actions"] > [role=menuitem]')].map((b) => b.textContent!.trim())
async function settle() { for (let i = 0; i < 5; i++) await Promise.resolve(); await tick() }

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(NOW)
  resetPolls()
  motion.reducedMotion = false
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  ui.goTo = null
  ui.jump = null
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('the poll card', () => {
  it('before you vote: the question, the options without counts, the set-up and the time left', () => {
    show(msg(view({ voters: 2 })))
    expect(card().querySelector('.q')!.textContent).toBe('Games tonight?')
    expect(card().querySelector('.status')!.textContent).toBe('One choice · 3 hours left')
    expect(rows()).toEqual(['Valheim', 'Apex', 'Rust'])
    expect(card().querySelector('.fill')).toBeNull()
    expect(card().querySelector('.foot')!.textContent!.replace(/\s+/g, ' ').trim()).toBe('2 votes · Vote to see results')
    expect(document.querySelector('.content')).toBeNull() // the card stands in for the text
  })

  it('a click votes; the answer shows your choice, the counts and who voted', async () => {
    show(msg(view({ voters: 1 })))
    invoke.mockResolvedValueOnce(view({ voters: 2, mine: [1], results: tallies([1, 1, 0], [['U3'], ['U1'], []]) }))
    choose('Apex').click()
    await settle()
    flushSync()
    expect(invoke).toHaveBeenCalledWith('vote_poll', { messageId: 'M1', options: [1] })
    // the answer lands on the held message, which the chat passes back down
    const held = app.state.messages.C1[0]
    expect(held.poll!.mine).toEqual([1])
    unmount(cmp!)
    show(held)
    expect(choose('Apex').getAttribute('aria-pressed')).toBe('true')
    // (the K and A are the voters' avatar initials)
    expect(rows()).toEqual(['Valheim K 150%', 'Apex A 150%', 'Rust 00%'])
    expect(card().querySelectorAll('.opt')[0].querySelector('.stack')!.getAttribute('data-tip')).toBe('kim')
    expect(card().querySelector('.fill')!.getAttribute('style')).toContain('scaleX(0.5)')
  })

  it('a click on your single choice takes it back; multiple choice adds to what you have', async () => {
    show(msg(view({ mine: [0], voters: 1, results: tallies([1, 0, 0]) })))
    invoke.mockResolvedValueOnce(view())
    choose('Valheim').click()
    await settle()
    expect(invoke).toHaveBeenLastCalledWith('vote_poll', { messageId: 'M1', options: [] })
    unmount(cmp!)
    show(msg(view({ multi: true, mine: [0], voters: 1, results: tallies([1, 0, 0]) })))
    choose('Rust').click()
    await settle()
    expect(invoke).toHaveBeenLastCalledWith('vote_poll', { messageId: 'M1', options: [0, 2] })
  })

  it('as its creator: the results before voting, and End poll in the menu', async () => {
    show(msg(view({ results: tallies([0, 0, 0]) }), { author_id: 'U1' }), true)
    expect(rows()).toEqual(['Valheim 00%', 'Apex 00%', 'Rust 00%'])
    expect(document.querySelector('button[aria-label="Edit"]')).toBeNull()
    document.querySelector<HTMLButtonElement>('button[aria-label="More"]')!.click()
    flushSync()
    expect(menuItems()).toContain('End poll')
    invoke.mockResolvedValueOnce(view({ closed: true, closed_at: '2026-11-09T10:00:00.000Z', results: tallies([0, 0, 0]) }))
    ;[...document.querySelectorAll<HTMLButtonElement>('[role=menuitem]')].find((b) => b.textContent!.trim() === 'End poll')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('close_poll', { messageId: 'M1' })
  })

  it("someone else's poll has no End poll", () => {
    show(msg(view()))
    document.querySelector<HTMLButtonElement>('button[aria-label="More"]')!.click()
    flushSync()
    expect(menuItems()).not.toContain('End poll')
  })

  it('closed: final results to everyone, the winner emphasised, no voting', () => {
    show(msg(view({ closed: true, voters: 3, results: tallies([2, 1, 0]) })))
    expect(card().querySelector('.status')!.textContent).toBe('One choice · Closed')
    expect(card().querySelector('.foot')!.textContent).toContain('Final results')
    expect([...card().querySelectorAll('.opt.won')].map((r) => r.querySelector('.label')!.textContent)).toEqual(['Valheim'])
    expect(choose('Apex').disabled).toBe(true)
  })

  it('past its end before the sweep, it already reads closed', () => {
    show(msg(view({ ends_at: '2026-11-09T09:30:00.000Z' })), true)
    expect(card().querySelector('.status')!.textContent).toBe('One choice · Closed')
    expect(choose('Apex').disabled).toBe(true)
  })

  it('anonymous: counts, never faces', () => {
    show(msg(view({ anonymous: true, multi: true, mine: [0], voters: 2, results: tallies([2, 1, 0]) })))
    expect(card().querySelector('.status')!.textContent).toBe('Multiple choice · Anonymous · 3 hours left')
    expect(card().querySelector('.stack')).toBeNull()
    expect(card().querySelector('.foot')!.textContent).toContain('2 voters')
  })

  it('bars only grow when motion is allowed', () => {
    motion.reducedMotion = true
    show(msg(view({ mine: [0], voters: 1, results: tallies([1, 0, 0]) })))
    expect(card().classList.contains('animated')).toBe(false)
  })
})

describe('the ended line', () => {
  it('says who won and See poll jumps to the poll', async () => {
    show(msg(null, {
      id: 'L1', author_id: null, kind: 'system', content: "📊 sam's poll ended: Valheim won (2 votes)",
      poll_ended: { poll_message_id: 'M1', author_id: 'U2', winners: ['Valheim'], votes: 2 },
    }))
    const line = document.querySelector<HTMLElement>('[data-message-id="L1"]')!
    expect(line.textContent!.replace(/\s+/g, ' ')).toContain("sam's poll ended: Valheim won (2 votes) · See poll")
    ;[...line.querySelectorAll('button')].find((b) => b.textContent === 'See poll')!.click()
    await settle()
    expect(ui.jump ?? ui.goTo).toBeTruthy()
  })
})
