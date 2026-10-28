// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { QuoteLine } from '../lib/protocol/QuoteLine'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
const media = fakeMatchMedia(false)
vi.stubGlobal('matchMedia', media.matchMedia)

import QuoteItem from './QuoteItem.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { motion, setAnimateMode } from '../lib/animate.svelte'

const A = 'a'.repeat(64)
const S = 'b'.repeat(64)
const P = 'c'.repeat(64)
const url = (h: string) => `pulse-media://localhost/${h}`
const alex = { id: 'U1', username: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const sam = { id: 'U2', username: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const kim = { id: 'U3', username: 'kim', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }

const line = (over: Partial<QuoteLine> = {}): QuoteLine => ({
  id: 'L1', speaker_id: 'U2', speaker_name: null, text: 'hello there', media: null, caption: null,
  awards: 0, awarders: [], awarded: false, ...over,
})
const quote = (lines: QuoteLine[], over: Partial<Message> = {}, q: Partial<NonNullable<Message['quote']>> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U1', kind: 'normal', content: '', reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: { lines, year: 2026, said_on: null, reply_count: 0, last_reply_id: null, replies_seen: null, wins: [], ...q }, qoty_card: null, clip: null, ...over,
})
const link = (name_key: string, name: string, user_id: string) => ({ server_id: 'S1', name_key, name, user_id, linked_by: 'U1', linked_at: '2026-10-01T00:00:00Z' })
const names: Record<string, string> = { U1: 'alex', U2: 'sam', U3: 'kim' }
const nameOf = (id: string | null) => names[id ?? ''] ?? 'Pulse'

let cmp: ReturnType<typeof mount> | undefined
const onReply = vi.fn()
const onEdit = vi.fn()
const onDelete = vi.fn(async () => {})
const onOpenReplies = vi.fn()
function show(m: Message, props: Record<string, unknown> = {}, meId = 'U1') {
  app.state = {
    ...emptyState(), me: alex, servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
    members: { S1: [{ user: alex, nickname: null }, { user: sam, nickname: null }, { user: kim, nickname: null }] },
    people: { U1: { user: alex, online: true, last_seen_at: null }, U2: { user: sam, online: true, last_seen_at: null }, U3: { user: kim, online: true, last_seen_at: null } },
    frequentEmoji: ['👍', '🎉', '❤️', '😂'],
    messages: { C1: [m] },
  }
  cmp = mount(QuoteItem, { target: document.body, props: { message: app.state.messages['C1'][0], serverId: 'S1', meId, nameOf, onReply, onEdit, onDelete, onOpenReplies, ...props } })
  flushSync()
}
const settle = () => new Promise((r) => setTimeout(r, 0))
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
const items = () => [...document.querySelectorAll<HTMLElement>('[role=menuitem]')].map((e) => e.textContent?.trim())
const item = (t: string) => [...document.querySelectorAll<HTMLElement>('[role=menuitem]')].find((e) => e.textContent?.trim() === t)!
const openMenu = (n = 0) => {
  document.querySelectorAll<HTMLButtonElement>('button[aria-label="Line actions"]')[n].click()
  flushSync()
}
const see = (el: Element, v = true) => { FakeIntersectionObserver.set(el, v); flushSync() }

beforeEach(() => {
  invoke.mockReset(); invoke.mockResolvedValue(undefined)
  onReply.mockClear(); onEdit.mockClear(); onDelete.mockClear(); onOpenReplies.mockClear()
  setAnimateMode('always'); motion.streaming = false
  window.dispatchEvent(new Event('focus'))
  media.set(false)
  Object.assign(navigator, { clipboard: { writeText: vi.fn(async () => {}) } })
})
afterEach(() => { if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = '' })

/** jsdom never reports :focus-visible; say what a browser would for this element. */
function stubFocusVisible(el: HTMLElement, visible: boolean) {
  const real = el.matches.bind(el) as (s: string) => boolean
  vi.spyOn(el, 'matches').mockImplementation(((sel: string) => (sel === ':focus-visible' ? visible : real(sel))) as never)
}

describe('QuoteItem', () => {
  it('renders lines with speakers and curly marks', () => {
    show(quote([line(), line({ id: 'L2', speaker_id: null, speaker_name: 'Grandpa', text: 'back in my day' })]))
    const texts = [...document.querySelectorAll('.line .text')].map((e) => e.textContent)
    expect(texts).toEqual(['“hello there”', '“back in my day”'])
    expect([...document.querySelectorAll('.line .speaker')].map((e) => e.textContent?.trim())).toEqual(['sam', 'Grandpa'])
    expect(document.body.textContent).toContain('alex')
  })

  it('hides the trophy at 0 and shows the count and awarder avatars', () => {
    show(quote([line(), line({ id: 'L2', awards: 2, awarders: ['U1', 'U3'] })]))
    const trophies = [...document.querySelectorAll('.line')].map((l) => l.querySelector('.trophy')?.textContent?.trim() ?? null)
    expect(trophies).toEqual([null, '🏆 2'])
    expect(document.querySelectorAll('.line')[1].querySelectorAll('.stack .av').length).toBe(2)
  })

  it('an animated image line shows its still until animate allows', () => {
    show(quote([line({ text: null, media: { hash: A, still_hash: S, width: 800, height: 600 }, caption: 'a cat' })]))
    const img = document.querySelector<HTMLImageElement>('.line img.pic')!
    expect(img.getAttribute('src')).toBe(url(S))
    expect(document.querySelector('.caption')?.textContent).toBe('a cat')
    see(img)
    expect(img.getAttribute('src')).toBe(url(A))
    setAnimateMode('never'); flushSync()
    expect(img.getAttribute('src')).toBe(url(S))
  })

  it('a still image line renders plainly', () => {
    show(quote([line({ text: null, media: { hash: P, still_hash: null, width: 10, height: 10 } })]))
    expect(document.querySelector('.line img.pic')!.getAttribute('src')).toBe(url(P))
  })

  it('an image line opens full size', async () => {
    show(quote([line({ text: null, media: { hash: A, still_hash: S, width: 800, height: 600 } })]))
    expect(document.querySelector('[role=dialog]')).toBeNull()
    document.querySelector<HTMLElement>('.line .picbtn')!.click()
    flushSync()
    const big = document.querySelector<HTMLImageElement>('[role=dialog] img')!
    expect(big).toBeTruthy()
    see(big)
    expect(big.getAttribute('src')).toBe(url(A))
    document.querySelector<HTMLElement>('[role=dialog]')!.click()
    flushSync()
    expect(document.querySelector('[role=dialog]')).toBeNull()
  })

  it('award menu item calls give_award and is absent on my own line', async () => {
    show(quote([line(), line({ id: 'L2', speaker_id: 'U1' })]))
    openMenu(0)
    expect(items()).toContain('Give quote award')
    item('Give quote award').click()
    await settle()
    expect(calls('give_award')).toEqual([['give_award', { lineId: 'L1' }]])
    expect(document.querySelector('[role=menu]')).toBeNull()
    openMenu(1)
    expect(items()).not.toContain('Give quote award')
    expect(items()).not.toContain('Take back award')
    expect(items()).toContain('Copy')
  })

  it('take back calls take_award', async () => {
    show(quote([line({ awards: 1, awarders: ['U1'], awarded: true })]))
    openMenu(0)
    expect(items()).not.toContain('Give quote award')
    item('Take back award').click()
    await settle()
    expect(calls('take_award')).toEqual([['take_award', { lineId: 'L1' }]])
  })

  it('copy puts the line text on the clipboard', async () => {
    show(quote([line()]))
    openMenu(0)
    item('Copy').click()
    await settle()
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith('hello there')
  })

  it('the menu opens from the keyboard and right-click', () => {
    show(quote([line()]))
    const l = document.querySelector<HTMLElement>('.line')!
    l.dispatchEvent(new KeyboardEvent('keydown', { key: 'ContextMenu', bubbles: true }))
    flushSync()
    expect(document.querySelector('[role=menu]')).toBeTruthy()
    document.querySelector('[role=menu]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    flushSync()
    expect(document.querySelector('[role=menu]')).toBeNull()
    l.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }))
    flushSync()
    expect(document.querySelector('[role=menu]')).toBeTruthy()
  })

  it('View 2 replies shows new when unseen', () => {
    show(quote([line()], {}, { reply_count: 2, last_reply_id: 'R9', replies_seen: null }))
    const b = document.querySelector<HTMLButtonElement>('.replies')!
    expect(b.textContent).toContain('View 2 replies')
    expect(b.querySelector('.new')?.textContent).toBe('new')
    b.click()
    expect(onOpenReplies).toHaveBeenCalled()
  })

  it('View 1 reply is singular and seen replies carry no marker', () => {
    show(quote([line()], {}, { reply_count: 1, last_reply_id: 'R9', replies_seen: 'R9' }))
    expect(document.querySelector('.replies')!.textContent).toContain('View 1 reply')
    expect(document.querySelector('.replies .new')).toBeNull()
  })

  it('no replies means no link', () => {
    show(quote([line()]))
    expect(document.querySelector('.replies')).toBeNull()
  })

  it('edit and delete only for the poster', async () => {
    show(quote([line()]))
    expect(document.querySelector('button[aria-label="Edit"]')).toBeTruthy()
    document.querySelector<HTMLButtonElement>('button[aria-label="Delete"]')!.click()
    await settle()
    expect(onDelete).toHaveBeenCalled()
    document.querySelector<HTMLButtonElement>('button[aria-label="Edit"]')!.click()
    expect(onEdit).toHaveBeenCalled()
    unmount(cmp!); cmp = undefined; document.body.innerHTML = ''
    show(quote([line()]), {}, 'U3')
    expect(document.querySelector('button[aria-label="Edit"]')).toBeNull()
    expect(document.querySelector('button[aria-label="Delete"]')).toBeNull()
    expect(document.querySelector('button[aria-label="Reply"]')).toBeTruthy()
  })

  it('without onReply there is no Reply', () => {
    show(quote([line()]), { onReply: undefined })
    expect(document.querySelector('button[aria-label="Reply"]')).toBeNull()
  })

  it('compact shows the lines and their speakers only', () => {
    show(quote([line({ awards: 1, awarders: ['U3'] })], { reactions: [{ emoji: '😂', count: 2, me: false }] }, { reply_count: 2 }), { compact: true })
    expect(document.querySelector('.line .text')?.textContent).toBe('“hello there”')
    expect(document.querySelector('.line .speaker')?.textContent?.trim()).toBe('sam')
    expect(document.querySelector('.line .trophy')).toBeTruthy()
    expect(document.querySelector('.meta')).toBeNull() // no poster header
    expect(document.querySelector('.reactions, .reaction-row, button[aria-label="Add reaction"]')).toBeNull()
    expect(document.body.textContent).not.toContain('View 2 replies')
    for (const a of ['Reply', 'Edit', 'Delete']) expect(document.querySelector(`button[aria-label="${a}"]`)).toBeNull()
    // the line menu stays: awards, links and copy are about the line, wherever it shows
    expect(document.querySelector('button[aria-label="Line actions"]')).toBeTruthy()
  })

  it('a failed delete shows on the quote', async () => {
    onDelete.mockRejectedValueOnce(new Error('nope'))
    show(quote([line()]))
    document.querySelector<HTMLButtonElement>('button[aria-label="Delete"]')!.click()
    await settle(); flushSync()
    expect(document.querySelector('[role=alert]')).toBeTruthy()
  })

  it('a plain-name line offers Link to a member…', async () => {
    show(quote([line({ speaker_id: null, speaker_name: 'Grandpa' })]))
    openMenu(0)
    expect(items()).toContain('Link to a member…')
    expect(items()).not.toContain('Unlink')
    item('Link to a member…').click()
    flushSync()
    const input = document.querySelector<HTMLInputElement>('input[aria-label="Find a member"]')!
    input.value = 'sa'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    const opts = [...document.querySelectorAll<HTMLElement>('[role=menuitem]')]
    expect(opts.map((o) => o.textContent?.trim())).toEqual(['sam'])
    opts[0].click()
    await settle()
    expect(calls('link_speaker')).toEqual([['link_speaker', { serverId: 'S1', body: { name: 'Grandpa', user_id: 'U2' } }]])
  })

  it('a linked plain-name line offers Unlink, and a member line neither', async () => {
    show(quote([line({ speaker_id: null, speaker_name: 'Grandpa  X' }), line({ id: 'L2' })]))
    app.state = { ...app.state, speakerLinks: { S1: [link('grandpa x', 'Grandpa X', 'U3')] } }
    flushSync()
    openMenu(0)
    expect(items()).toContain('Unlink')
    expect(items()).not.toContain('Link to a member…')
    item('Unlink').click()
    await settle()
    expect(calls('unlink_speaker')).toEqual([['unlink_speaker', { serverId: 'S1', nameKey: 'grandpa x' }]])
    openMenu(1)
    expect(items()).not.toContain('Unlink')
    expect(items()).not.toContain('Link to a member…')
  })

  it('a line linked to me is my own line', () => {
    show(quote([line({ speaker_id: null, speaker_name: 'Al' })]))
    app.state = { ...app.state, speakerLinks: { S1: [link('al', 'Al', 'U1')] } }
    flushSync()
    openMenu(0)
    expect(items()).not.toContain('Give quote award')
  })

  it('marks each Quote of the Year win', () => {
    show(quote([line()], {}, { wins: [2024, 2025] }))
    expect([...document.querySelectorAll('.win')].map((e) => e.textContent?.trim())).toEqual(['Quote of the Year 2024', 'Quote of the Year 2025'])
  })

  it('suggests the first 3 frequent emoji not already on the quote', () => {
    show(quote([line()], { reactions: [{ emoji: '👍', count: 1, me: false }] }))
    expect([...document.querySelectorAll('.chip.suggest')].map((b) => b.getAttribute('aria-label'))).toEqual(['React with 🎉', 'React with ❤️', 'React with 😂'])
  })

  it('focusing into a quote exposes its action buttons', () => {
    show(quote([line()]))
    const actions = document.querySelector<HTMLElement>('.actions')!
    expect(actions.classList.contains('open')).toBe(false)
    const b = actions.querySelector<HTMLElement>('button')!
    stubFocusVisible(b, true) // jsdom has no keyboard-focus heuristic
    b.focus(); flushSync()
    expect(actions.classList.contains('open')).toBe(true)
  })

  it('a mouse focus inside the quote after keyboard focus closes the bar again', () => {
    show(quote([line()]))
    const actions = document.querySelector<HTMLElement>('.actions')!
    const b = actions.querySelector<HTMLElement>('button')!
    stubFocusVisible(b, true)
    b.focus(); flushSync()
    expect(actions.classList.contains('open')).toBe(true)
    const chip = document.querySelector<HTMLElement>('.chip')!
    stubFocusVisible(chip, false)
    chip.focus(); flushSync()
    expect(actions.classList.contains('open')).toBe(false)
  })

  it('the line label carries the award count', () => {
    show(quote([line({ awards: 2, awarders: ['U1', 'U3'] }), line({ id: 'L2', awards: 1, awarders: ['U1'] }), line({ id: 'L3' })]))
    const labels = [...document.querySelectorAll('.line')].map((l) => l.getAttribute('aria-label'))
    expect(labels[0]).toContain('2 awards')
    expect(labels[1]).toContain('1 award')
    expect(labels[1]).not.toContain('1 awards')
    expect(labels[2]).not.toContain('award')
  })

  it('the line actions button says whether its menu is open', () => {
    show(quote([line()]))
    const b = document.querySelector<HTMLButtonElement>('button[aria-label="Line actions"]')!
    expect(b.getAttribute('aria-expanded')).toBe('false')
    b.click(); flushSync()
    expect(b.getAttribute('aria-expanded')).toBe('true')
  })

  it('an image line without a caption has nothing to copy; with one it copies the caption', async () => {
    show(quote([line({ text: null, media: { hash: P, still_hash: null, width: 10, height: 10 } })]))
    openMenu(0)
    expect(items()).not.toContain('Copy')
    unmount(cmp!); cmp = undefined; document.body.innerHTML = ''
    show(quote([line({ text: null, media: { hash: P, still_hash: null, width: 10, height: 10 }, caption: 'a cat' })]))
    openMenu(0)
    item('Copy').click()
    await settle()
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith('a cat')
  })

  it('the full-size image has alt text and the dialog carries the caption', () => {
    show(quote([line({ text: null, media: { hash: P, still_hash: null, width: 10, height: 10 }, caption: 'a cat' })]))
    document.querySelector<HTMLElement>('.line .picbtn')!.click(); flushSync()
    expect(document.querySelector('[role=dialog] img')!.getAttribute('alt')).toBe('a cat')
    expect(document.querySelector('[role=dialog]')!.getAttribute('aria-label')).toBe('a cat')
  })

  it('suggestions skip emoji you cannot use before taking three', () => {
    show(quote([line()]))
    app.state = { ...app.state, frequentEmoji: ['custom:GONE', '🎉', '❤️', '😂', '👍'] }
    flushSync()
    expect([...document.querySelectorAll('.chip.suggest')].map((b) => b.getAttribute('aria-label'))).toEqual(['React with 🎉', 'React with ❤️', 'React with 😂'])
  })

  it('in the member picker ArrowUp from the first option returns to the input, and an empty box lists members', () => {
    show(quote([line({ speaker_id: null, speaker_name: 'Grandpa' })]))
    openMenu(0)
    item('Link to a member…').click(); flushSync()
    const input = document.querySelector<HTMLInputElement>('input[aria-label="Find a member"]')!
    expect(document.querySelectorAll('[role=menuitem]').length).toBeGreaterThan(0)
    const first = document.querySelector<HTMLElement>('[role=menuitem]')!
    first.focus()
    first.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true }))
    expect(document.activeElement).toBe(input)
  })

  it('a mouse-style focus does not open the action bar', () => {
    show(quote([line()], { reactions: [{ emoji: '👍', count: 1, me: false }] }))
    const chip = document.querySelector<HTMLElement>('.chip')!
    stubFocusVisible(chip, false)
    chip.focus(); flushSync()
    expect(document.querySelector('.actions')!.classList.contains('open')).toBe(false)
  })
})
