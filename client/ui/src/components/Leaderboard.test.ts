// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Leaderboard as Board } from '../lib/protocol/Leaderboard'
import type { Message } from '../lib/protocol/Message'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import Leaderboard from './Leaderboard.svelte'
import { app, handleReady, resetState } from '../lib/store.svelte'
import { testMe, testReady } from '../lib/testReady'

const sam = { id: 'U2', username: 'sam', avatar_hash: null }
const kim = { id: 'U3', username: 'kim', avatar_hash: null }

const quote = (id: string, text: string, reactions = 0): Message => ({
  id, channel_id: 'C1', author_id: 'U1', kind: 'normal', content: `“${text}” — sam`, reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [],
  reactions: reactions ? [{ emoji: '😂', count: reactions, me: false }] : [], thread_root_id: null, qoty_card: null,
  quote: {
    lines: [{ id: `L${id}`, speaker_id: 'U2', speaker_name: null, text, media: null, caption: null, awards: 0, awarders: [], awarded: false }],
    year: 2025, said_on: null, reply_count: 0, last_reply_id: null, replies_seen: null, wins: [],
  },
})
const board: Board = {
  winners: [{ year: 2025, message: quote('Q1', 'the winner'), votes: 3, voters: ['U1', 'U2', 'U3'], speakers: [] }],
  top_year: [quote('Q2', 'funny this year', 4)],
  top_all_time: [quote('Q3', 'funny forever', 9)],
  most_quoted: [{ user_id: 'U2', name: null, count: 7 }, { user_id: null, name: 'a friend', count: 2 }],
  most_awarded: [{ user_id: 'U3', name: null, count: 5 }],
}

let cmp: ReturnType<typeof mount> | undefined
const onOpen = vi.fn()
const onClose = vi.fn()
const settle = () => new Promise((r) => setTimeout(r, 0))
async function show() {
  cmp = mount(Leaderboard, { target: document.body, props: { serverId: 'S1', onOpen, onClose } })
  flushSync()
  await settle()
  flushSync()
}
const section = (title: string) =>
  [...document.querySelectorAll('section')].find((s) => s.querySelector('h3')?.textContent?.trim() === title)

beforeEach(() => {
  resetState()
  handleReady(testReady([{ id: 'C1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0 }], {
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }, { user: kim, nickname: null }] }],
  }))
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string) => (cmd === 'leaderboard' ? board : undefined))
  onOpen.mockClear()
  onClose.mockClear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('Leaderboard', () => {
  it('renders each section', async () => {
    await show()
    expect(invoke).toHaveBeenCalledWith('leaderboard', { serverId: 'S1', year: null })
    expect([...document.querySelectorAll('section h3')].map((h) => h.textContent?.trim())).toEqual([
      'Quote of the Year', 'Most reacted: this year', 'Most reacted: all time', 'Most quoted', 'Most quote awards',
    ])
    expect(section('Most reacted: this year')!.textContent).toContain('funny this year')
    expect(section('Most reacted: this year')!.textContent).toContain('4')
    expect(section('Most reacted: all time')!.textContent).toContain('funny forever')
    const quoted = section('Most quoted')!.textContent!
    expect(quoted).toContain('sam')
    expect(quoted).toContain('7')
    expect(quoted).toContain('a friend')
    expect(section('Most quote awards')!.textContent).toContain('kim')
    expect(app.state.me).toBeTruthy()
  })

  it('winners show who voted', async () => {
    await show()
    const win = section('Quote of the Year')!
    expect(win.textContent).toContain('2025')
    expect(win.textContent).toContain('the winner')
    expect(win.textContent).toContain('3 votes')
    expect(win.querySelector('.stack')?.getAttribute('aria-label')).toBe('alex, sam and kim')
  })

  it('a winner names who said it when the vote ended, not today\'s speaker', async () => {
    const ended: Board = { ...board, winners: [{ ...board.winners[0], speakers: [{ user_id: 'U3', name: null }] }] }
    invoke.mockImplementation(async (cmd: string) => (cmd === 'leaderboard' ? ended : undefined))
    await show()
    expect(section('Quote of the Year')!.querySelector('.by')?.textContent).toBe('kim')
  })

  it('a winner names every speaker of the snapshot, whatever the first line is now', async () => {
    // Line 1 (sam) was removed after the win, so kim's line now comes first.
    const now = quote('Q1', 'two')
    now.quote!.lines[0].speaker_id = 'U3'
    const ended: Board = { ...board, winners: [{ ...board.winners[0], message: now, speakers: [{ user_id: 'U2', name: null }, { user_id: 'U3', name: null }] }] }
    invoke.mockImplementation(async (cmd: string) => (cmd === 'leaderboard' ? ended : undefined))
    await show()
    expect(section('Quote of the Year')!.querySelector('.by')?.textContent).toBe('sam & kim')
  })

  it('a deleted winner keeps its year, speakers and votes, says it was deleted, and has nothing to jump to', async () => {
    const gone: Message = { ...quote('Q1', 'x'), content: '', deleted: true, quote: null }
    const ended: Board = { ...board, winners: [{ ...board.winners[0], message: gone, speakers: [{ user_id: null, name: 'A Friend' }] }] }
    invoke.mockImplementation(async (cmd: string) => (cmd === 'leaderboard' ? ended : undefined))
    await show()
    const win = section('Quote of the Year')!
    expect(win.querySelector('button.entry')).toBeNull()
    expect(win.textContent).toContain('2025')
    expect(win.textContent).toContain('This quote was deleted.')
    expect(win.querySelector('.by')?.textContent).toBe('A Friend')
    expect(win.textContent).toContain('3 votes')
    expect(win.querySelector('.stack')).toBeTruthy()
  })

  it('a winner without a snapshot falls back to the quote\'s speaker', async () => {
    await show()
    expect(section('Quote of the Year')!.querySelector('.by')?.textContent).toBe('sam')
  })

  it('an entry jumps to the quote', async () => {
    await show()
    ;[...document.querySelectorAll<HTMLButtonElement>('button.entry')].find((b) => b.textContent?.includes('funny forever'))!.click()
    expect(onOpen).toHaveBeenCalledWith(board.top_all_time[0])
    ;[...document.querySelectorAll<HTMLButtonElement>('button.entry')].find((b) => b.textContent?.includes('the winner'))!.click()
    expect(onOpen).toHaveBeenLastCalledWith(board.winners[0].message)
  })

  it('says when there is nothing yet, and shows a load error', async () => {
    invoke.mockImplementation(async () => ({ winners: [], top_year: [], top_all_time: [], most_quoted: [], most_awarded: [] }))
    await show()
    expect(document.body.textContent).toContain('No quotes yet')
    unmount(cmp!)
    invoke.mockImplementation(async () => { throw { status: 500, message: 'boom' } })
    await show()
    expect(document.querySelector('[role=alert]')).toBeTruthy()
  })

  it('Escape closes', async () => {
    await show()
    await settle() // the outside-click listener attaches on the next tick
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    expect(onClose).toHaveBeenCalled()
  })
})
