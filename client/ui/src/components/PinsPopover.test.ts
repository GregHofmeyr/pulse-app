// @vitest-environment jsdom
// The pins popover (#178): a chat's pins, newest first, each with Jump and Unpin; live from events.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { PinnedMessage } from '../lib/protocol/PinnedMessage'
import type { QuoteLine } from '../lib/protocol/QuoteLine'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import PinsPopover from './PinsPopover.svelte'
import { app, handleReady, resetState } from '../lib/store.svelte'
import { applyEvent } from '../lib/state'
import { testMe, testReady } from '../lib/testReady'

const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const NOW = Date.parse('2026-11-09T10:00:00.000Z')
const msg = (id: string, content: string, author_id = 'U1'): Message => ({
  id, channel_id: 'C1', author_id, kind: 'normal', content, reply_to_id: null, created_at: '2026-11-09T09:00:00.000Z',
  edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [],
})
const pinned = (m: Message, by: string, expires_at: string | null): PinnedMessage => {
  const pin = { pinned_by: by, pinned_at: '2026-11-09T09:30:00.000Z', expires_at }
  return { message: { ...m, pin }, pin }
}
const list = [
  pinned(msg('M2', 'a link', 'U2'), 'U1', '2026-11-09T13:00:00.000Z'),
  pinned(msg('M1', 'plan for tonight'), 'U2', null),
]

let cmp: ReturnType<typeof mount> | undefined
const onJump = vi.fn()
const onClose = vi.fn()
const settle = async () => { for (let i = 0; i < 5; i++) await Promise.resolve(); flushSync() }
async function show() {
  cmp = mount(PinsPopover, { target: document.body, props: { channelId: 'C1', serverId: 'S1', onJump, onClose } })
  flushSync()
  await settle()
}
const rows = () => [...document.querySelectorAll<HTMLElement>('.pin')]
const button = (row: HTMLElement, label: string) => [...row.querySelectorAll('button')].find((b) => b.textContent?.trim() === label)!

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(NOW)
  resetState()
  handleReady(testReady([{ id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }], {
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }] }],
  }))
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string) => (cmd === 'list_pins' ? list : undefined))
  onJump.mockClear()
  onClose.mockClear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('PinsPopover (#178)', () => {
  it('lists the pins newest first: author, preview, who pinned it and the time left', async () => {
    await show()
    expect(invoke).toHaveBeenCalledWith('list_pins', { channelId: 'C1' })
    expect(document.querySelector('[role=dialog]')?.getAttribute('aria-label')).toBe('Pinned messages')
    const [a, b] = rows()
    expect(a.querySelector('.author')?.textContent).toBe('sam')
    expect(a.querySelector('.said')?.textContent).toBe('a link')
    expect(a.querySelector('.note')?.textContent).toBe('Pinned by alex · 3 hours left')
    expect(b.querySelector('.said')?.textContent).toBe('plan for tonight')
    expect(b.querySelector('.note')?.textContent).toBe('Pinned by sam')
  })

  it('Jump asks to show the message; Unpin unpins it', async () => {
    await show()
    button(rows()[1], 'Jump').click()
    expect(onJump).toHaveBeenCalledWith(expect.objectContaining({ id: 'M1' }))
    button(rows()[0], 'Unpin').click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('unpin_message', { messageId: 'M2' })
  })

  it('a failed unpin says why', async () => {
    await show()
    invoke.mockImplementation(async (c: string) => { if (c === 'unpin_message') throw 'nope' })
    button(rows()[0], 'Unpin').click()
    await settle()
    expect(document.querySelector('[role=alert]')?.textContent).toBe('nope')
  })

  it('follows pins and unpins from anyone, live', async () => {
    await show()
    const p = pinned(msg('M3', 'new one'), 'U2', null)
    app.state = applyEvent(app.state, { t: 'MessagePinned', d: { channel_id: 'C1', pinned: p } }, 0)
    flushSync()
    expect(rows().map((r) => r.querySelector('.said')?.textContent)).toEqual(['new one', 'a link', 'plan for tonight'])
    app.state = applyEvent(app.state, { t: 'MessageUnpinned', d: { channel_id: 'C1', message_id: 'M2' } }, 0)
    flushSync()
    expect(rows().map((r) => r.querySelector('.said')?.textContent)).toEqual(['new one', 'plan for tonight'])
  })

  it('fetches again after a reconnect forgot the list, and stays live', async () => {
    await show()
    expect(rows()).toHaveLength(2)
    invoke.mockClear()
    app.state = { ...app.state, pins: {} } // what Ready does
    flushSync()
    await settle()
    expect(invoke).toHaveBeenCalledWith('list_pins', { channelId: 'C1' })
    expect(rows()).toHaveLength(2)
  })

  it('has a friendly line when nothing is pinned', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_pins' ? [] : undefined))
    await show()
    expect(rows()).toHaveLength(0)
    expect(document.querySelector('.none')?.textContent).toBe('Nothing pinned yet. Pin a message from its menu to keep it here.')
  })
})

describe('PinsPopover in quotes and clips channels (#257)', () => {
  const line = (id: string, text: string | null, speaker: { id?: string; name?: string }, caption: string | null = null): QuoteLine => ({
    id, speaker_id: speaker.id ?? null, speaker_name: speaker.name ?? null, text, media: null, caption, awards: 0, awarders: [], awarded: false,
  })
  const quote = (id: string, lines: QuoteLine[]): Message => ({
    ...msg(id, '', 'U1'), channel_id: 'Q1',
    quote: { lines, year: 2026, said_on: null, reply_count: 0, last_reply_id: null, replies_seen: null, wins: [] },
  })
  const clip = (id: string, caption: string, game_id: string | null = null): Message => ({
    ...msg(id, caption, 'U2'), channel_id: 'K1',
    clip: { thumb_hash: `th-${id}`, duration_ms: 30_000, width: 1920, height: 1080, size: 1, game_id, reply_count: 0, last_reply_id: null, replies_seen: null },
  })
  function showIn(channelId: string, listed: PinnedMessage[]) {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_pins' ? listed : undefined))
    handleReady(testReady([
      { id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 1 },
      { id: 'K1', server_id: 'S1', kind: 'clips', name: 'clips', position: 2 },
    ], {
      members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }] }],
      games: [{ id: 'G1', server_id: 'S1', name: 'Rocket League', created_by: null, created_at: '2026-11-01T00:00:00.000Z' }],
    }))
    cmp = mount(PinsPopover, { target: document.body, props: { channelId, serverId: 'S1', onJump, onClose } })
    flushSync()
    return settle()
  }

  it("a quote reads as its words and who said them; Jump shows it", async () => {
    await showIn('Q1', [
      pinned(quote('Q2', [line('L1', 'a joke', { name: 'kim' }), line('L2', 'not funny', { id: 'U2' })]), 'U2', null),
      pinned(quote('Q3', [{ ...line('L3', null, { name: 'jo' }, 'the cake'), media: { hash: 'p'.repeat(64), still_hash: null, width: 10, height: 10 } }]), 'U1', null),
    ])
    expect(document.querySelector('[role=dialog]')?.getAttribute('aria-label')).toBe('Pinned quotes')
    expect(document.querySelector('h2')?.textContent).toBe('Pinned quotes')
    const [a, b] = rows()
    expect(a.querySelector('.author')?.textContent).toBe('kim, sam')
    expect(a.querySelector('.said')?.textContent).toBe('“a joke” “not funny”')
    expect(a.querySelector('.note')?.textContent).toBe('Pinned by sam')
    expect(b.querySelector('.author')?.textContent).toBe('jo')
    expect(b.querySelector('.said')?.textContent).toBe('[image] the cake')
    button(a, 'Jump').click()
    expect(onJump).toHaveBeenCalledWith(expect.objectContaining({ id: 'Q2' }))
    expect(document.querySelector('.none')).toBeNull()
  })

  it('a clip reads as its thumbnail, its caption (else its game, else Clip) and who posted it; Open opens it', async () => {
    await showIn('K1', [
      pinned(clip('K2', 'that jump'), 'U1', '2026-11-09T13:00:00.000Z'),
      pinned(clip('K3', '', 'G1'), 'U1', null),
      pinned(clip('K4', ''), 'U1', null),
    ])
    expect(document.querySelector('[role=dialog]')?.getAttribute('aria-label')).toBe('Pinned clips')
    const [a, b, c] = rows()
    expect(a.querySelector('img.thumb')?.getAttribute('src')).toContain('th-K2')
    expect(a.querySelector('.said')?.textContent).toBe('that jump')
    expect(a.querySelector('.author')?.textContent).toBe('sam')
    expect(a.querySelector('.note')?.textContent).toBe('Pinned by alex · 3 hours left')
    expect(b.querySelector('.said')?.textContent).toBe('Rocket League')
    expect(c.querySelector('.said')?.textContent).toBe('Clip · 0:30')
    expect(button(a, 'Jump')).toBeUndefined()
    button(a, 'Open').click()
    expect(onJump).toHaveBeenCalledWith(expect.objectContaining({ id: 'K2' }))
    button(b, 'Unpin').click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('unpin_message', { messageId: 'K3' })
  })

  it('says how to pin a quote or a clip when nothing is pinned', async () => {
    await showIn('Q1', [])
    expect(document.querySelector('.none')?.textContent).toBe('Nothing pinned yet. Pin a quote from its right-click menu to keep it here.')
    unmount(cmp!)
    cmp = undefined
    app.state = { ...app.state, pins: {} }
    await showIn('K1', [])
    expect(document.querySelector('.none')?.textContent).toBe('Nothing pinned yet. Pin a clip from its ⋯ menu to keep it here.')
  })
})
