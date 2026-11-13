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
const alex = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const kim = { id: 'U3', username: 'kim', display_name: 'kim', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }

const line = (over: Partial<QuoteLine> = {}): QuoteLine => ({
  id: 'L1', speaker_id: 'U2', speaker_name: null, text: 'hello there', media: null, caption: null,
  awards: 0, awarders: [], awarded: false, ...over,
})
const quote = (lines: QuoteLine[], over: Partial<Message> = {}, q: Partial<NonNullable<Message['quote']>> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U1', kind: 'normal', content: '', reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: { lines, year: 2026, said_on: null, reply_count: 0, last_reply_id: null, replies_seen: null, wins: [], ...q }, qoty_card: null, clip: null, embeds: [], ...over,
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
    people: { U1: { user: alex, online: true, last_seen_at: null, status: 'online' }, U2: { user: sam, online: true, last_seen_at: null, status: 'online' }, U3: { user: kim, online: true, last_seen_at: null, status: 'online' } },
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
/** A line's menu is its right-click menu (#244: no ⋯ button on the line any more). */
const openMenu = (n = 0) => {
  document.querySelectorAll<HTMLElement>('.line')[n].dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }))
  flushSync()
}
const awardBtn = (n = 0) => document.querySelectorAll<HTMLElement>('.line')[n].querySelector<HTMLButtonElement>('button.award')
const copyBtn = () => document.querySelector<HTMLButtonElement>('.actions button[aria-label="Copy quote"]')
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
    expect(items()).toContain('Copy quote')
  })

  describe('the award button (#244)', () => {
    it("sits on each line someone else said, says what it does, and one click gives the award", async () => {
      show(quote([line(), line({ id: 'L2', speaker_id: 'U1' })]))
      const b = awardBtn(0)!
      expect(b.getAttribute('aria-label')).toBe('Give quote award')
      expect(b.dataset.tip).toBe('Give quote award')
      expect(b.getAttribute('aria-pressed')).toBe('false')
      expect(b.classList.contains('icon-btn')).toBe(true)
      expect(b.querySelector('svg')?.getAttribute('width')).toBe('16')
      // not on my own line
      expect(awardBtn(1)).toBeNull()
      b.click()
      await settle()
      expect(calls('give_award')).toEqual([['give_award', { lineId: 'L1' }]])
      expect(document.querySelector('[role=menu]')).toBeNull()
    })

    it('once given it shows pressed and takes the award back', async () => {
      show(quote([line({ awards: 1, awarders: ['U1'], awarded: true })]))
      const b = awardBtn(0)!
      expect(b.getAttribute('aria-label')).toBe('Take back award')
      expect(b.dataset.tip).toBe('Take back award')
      expect(b.getAttribute('aria-pressed')).toBe('true')
      b.click()
      await settle()
      expect(calls('take_award')).toEqual([['take_award', { lineId: 'L1' }]])
    })

    it('no line carries a ⋯ menu button any more', () => {
      show(quote([line(), line({ id: 'L2', speaker_name: 'Jo', speaker_id: null })]))
      expect(document.querySelector('button[aria-label="Line actions"]')).toBeNull()
    })
  })

  describe('Copy quote (#244)', () => {
    const two = () => quote([line(), line({ id: 'L2', speaker_id: 'U3', text: 'general kenobi' })])
    const whole = '“hello there” — sam\n“general kenobi” — kim'

    it('the hover bar copies the whole quote, every line with who said it', async () => {
      show(two())
      const b = copyBtn()!
      expect(b.dataset.tip).toBe('Copy quote')
      expect(b.querySelector('svg')?.getAttribute('width')).toBe('16')
      b.click()
      await settle(); flushSync()
      expect(navigator.clipboard.writeText).toHaveBeenCalledWith(whole)
      // the tooltip says it worked
      expect(b.dataset.tip).toBe('Copied')
    })

    it("a line's right-click menu copies the whole quote too", async () => {
      show(two())
      openMenu(1)
      item('Copy quote').click()
      await settle()
      expect(navigator.clipboard.writeText).toHaveBeenCalledWith(whole)
    })

    it('anyone can copy, not only the poster', () => {
      show(two(), {}, 'U2')
      expect(copyBtn()).toBeTruthy()
    })
  })

  it('take back calls take_award', async () => {
    show(quote([line({ awards: 1, awarders: ['U1'], awarded: true })]))
    openMenu(0)
    expect(items()).not.toContain('Give quote award')
    item('Take back award').click()
    await settle()
    expect(calls('take_award')).toEqual([['take_award', { lineId: 'L1' }]])
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
    // asked first: a quote takes its replies with it
    expect(onDelete).not.toHaveBeenCalled()
    expect(document.querySelector('[role="alertdialog"]')?.textContent).toContain('Delete this quote? Its replies go too.')
    document.querySelector<HTMLButtonElement>('[role="alertdialog"] button.go')!.click()
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
    // awarding stays (it is about the line, wherever it shows), and so does the right-click menu with Copy quote
    expect(awardBtn(0)).toBeTruthy()
    expect(copyBtn()).toBeNull()
    openMenu(0)
    expect(items()).toContain('Copy quote')
  })

  it('a failed delete shows on the quote', async () => {
    onDelete.mockRejectedValueOnce(new Error('nope'))
    show(quote([line()]))
    // Shift+click: no question
    document.querySelector<HTMLButtonElement>('button[aria-label="Delete"]')!.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, shiftKey: true }))
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

  it('a line with nothing to do (my own words, said by me, in a quote with no words) has no award and no menu', () => {
    show(quote([line({ speaker_id: 'U1', text: null, media: { hash: P, still_hash: null, width: 10, height: 10 } }), line({ id: 'L2', speaker_id: 'U1', text: '', caption: null })]))
    const lines = document.querySelectorAll<HTMLElement>('.line')
    expect(awardBtn(0)).toBeNull()
    expect(awardBtn(1)).toBeNull()
    // my own picture still has Save image (#206)
    openMenu(0)
    expect(items()).toEqual(['Save image', 'Pin quote'])
    document.querySelector('[role=menu]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    flushSync()
    // the whole quote can still be pinned from it (#257); where the quote can't be pinned (a QotY option), no menu
    lines[1].dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }))
    flushSync()
    expect(items()).toEqual(['Pin quote'])
    document.querySelector('[role=menu]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    flushSync()
    unmount(cmp!); cmp = undefined; document.body.innerHTML = ''
    show(quote([line({ speaker_id: 'U1', text: '', caption: null })]), { compact: true })
    const only = document.querySelector<HTMLElement>('.line')!
    only.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }))
    only.dispatchEvent(new KeyboardEvent('keydown', { key: 'ContextMenu', bubbles: true }))
    flushSync()
    expect(document.querySelector('[role=menu]')).toBeNull()
  })

  describe('Save image (#206)', () => {
    const pic = (over: Partial<QuoteLine> = {}) => line({ text: null, media: { hash: A, still_hash: S, width: 800, height: 600 }, caption: 'a cat', ...over })
    const saves = () => calls('save_image').map((c) => c[1])
    const overlay = () => document.querySelector<HTMLButtonElement>('.line button[aria-label="Save image"]')

    it("in an image line's menu (and right-click) it saves the original file, named after the caption", async () => {
      show(quote([pic({ speaker_id: 'U1' })]))
      openMenu(0)
      expect(items()).toContain('Save image')
      item('Save image').click()
      await settle()
      // the full file, never its still frame
      expect(saves()).toEqual([{ hash: A, caption: 'a cat' }])
      expect(document.querySelector('[role=menu]')).toBeNull()
      document.querySelector<HTMLElement>('.line')!.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }))
      flushSync()
      expect(items()).toContain('Save image')
    })

    it('a text line has no Save image', () => {
      show(quote([line()]))
      openMenu(0)
      expect(items()).not.toContain('Save image')
      expect(overlay()).toBeNull()
    })

    it("the clips' download icon sits on the image, with a tooltip, and saves it", async () => {
      show(quote([pic({ caption: null })]))
      const b = overlay()!
      expect(b.classList.contains('clip-dl')).toBe(true)
      expect(b.dataset.tip).toBe('Save image')
      b.click()
      await settle()
      expect(saves()).toEqual([{ hash: A, caption: '' }])
      // it isn't the open-full-size button's click
      flushSync()
      expect(document.querySelector('[role=dialog]')).toBeNull()
    })

    it('the full-size view has it too', async () => {
      show(quote([pic()]))
      document.querySelector<HTMLElement>('.line .picbtn')!.click(); flushSync()
      const b = document.querySelector<HTMLButtonElement>('[role=dialog] button[aria-label="Save image"]')!
      b.click()
      await settle()
      expect(saves()).toEqual([{ hash: A, caption: 'a cat' }])
      // saving doesn't close the view
      flushSync()
      expect(document.querySelector('[role=dialog]')).toBeTruthy()
    })

    it('cancelling the dialog does nothing; a failure shows on the quote; a second click while saving is ignored', async () => {
      let answer: () => Promise<unknown> = async () => false
      invoke.mockImplementation(async (cmd: string) => (cmd === 'save_image' ? answer() : undefined))
      show(quote([pic()]))
      overlay()!.click()
      await settle(); flushSync()
      expect(document.querySelector('[role=alert]')).toBeNull()
      answer = async () => Promise.reject("couldn't save the file: denied")
      overlay()!.click()
      await settle(); flushSync()
      expect(document.querySelector('[role=alert]')?.textContent).toBe("couldn't save the file: denied")
      let finish: (v: boolean) => void = () => {}
      answer = () => new Promise((r) => (finish = r))
      const before = saves().length
      overlay()!.click()
      overlay()!.click()
      await settle()
      expect(saves().length).toBe(before + 1)
      finish(true)
    })
  })

  it('a quote of only uncaptioned images has nothing to copy; a caption copies with its speaker', async () => {
    show(quote([line({ text: null, media: { hash: P, still_hash: null, width: 10, height: 10 } })]))
    openMenu(0)
    expect(items()).not.toContain('Copy quote')
    expect(copyBtn()).toBeNull()
    unmount(cmp!); cmp = undefined; document.body.innerHTML = ''
    show(quote([line({ text: null, media: { hash: P, still_hash: null, width: 10, height: 10 }, caption: 'a cat' })]))
    openMenu(0)
    item('Copy quote').click()
    await settle()
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith('[image] a cat — sam')
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

describe('QuoteItem names', () => {
  it('a member-linked speaker is styled, a plain name isn\'t', () => {
    show(quote([line(), line({ id: 'L2', speaker_id: null, speaker_name: 'Grandpa', text: 'back in my day' })]))
    app.state = {
      ...app.state,
      people: { ...app.state.people, U2: { user: { ...sam, style: { font: 'fredoka', colors: ['#ffc4a3'] } }, online: true, last_seen_at: null, status: 'online' } },
    }
    flushSync()
    const [first, second] = document.querySelectorAll<HTMLElement>('.line .speaker')
    const styled = first.querySelector<HTMLElement>('.name')!
    expect(styled.textContent).toBe('sam')
    expect(styled.getAttribute('style')).toContain('Fredoka')
    expect(second.querySelector('.name')).toBeNull()
    expect(second.textContent).toBe('Grandpa')
    // the poster's name is a Name too
    expect(document.querySelector('.meta .name')!.textContent).toBe('alex')
  })
})

describe('the person menu on a quote (#135): its poster and its member speakers', () => {
  const menu = () => document.querySelector<HTMLElement>('[role=menu]')
  const ids = () => [...document.querySelectorAll<HTMLElement>('[role=menu] [data-item]')].map((i) => i.dataset.item)
  const rightClick = (el: Element) => {
    const e = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 30, clientY: 40 })
    el.dispatchEvent(e)
    flushSync()
    return e
  }
  const key = (el: Element, k: string, init: KeyboardEventInit = {}) => {
    el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init }))
    flushSync()
  }

  it("right-click the poster's name or avatar: their person menu, in this server", () => {
    show(quote([line()], { author_id: 'U2' }))
    expect(rightClick(document.querySelector('.meta .name')!).defaultPrevented).toBe(true)
    expect(menu()!.getAttribute('aria-label')).toBe('Actions for sam')
    expect(ids()).toEqual(['profile', 'message', 'nick', 'private-nick', 'live-notify', 'appear-offline'])
    key(document.activeElement!, 'Escape')
    rightClick(document.querySelector('.avbtn')!)
    expect(menu()!.getAttribute('aria-label')).toBe('Actions for sam')
  })

  it("Shift+F10 on the poster's name opens it; Esc gives focus back", () => {
    show(quote([line()], { author_id: 'U2' }))
    const name = document.querySelector<HTMLElement>('.meta .name')!
    name.focus()
    key(name, 'F10', { shiftKey: true })
    expect(menu()).not.toBeNull()
    key(document.activeElement!, 'Escape')
    expect(menu()).toBeNull()
    expect(document.activeElement).toBe(name)
  })

  it("a member speaker's name: their person menu, not the line's; the rest of the line keeps the line menu", () => {
    show(quote([line({ speaker_id: 'U3' })]))
    const name = document.querySelector<HTMLElement>('.line .speaker .name')!
    rightClick(name)
    expect(menu()!.getAttribute('aria-label')).toBe('Actions for kim')
    expect(items()).not.toContain('Copy quote')
    key(document.activeElement!, 'Escape')
    name.focus()
    key(name, 'ContextMenu')
    expect(menu()!.getAttribute('aria-label')).toBe('Actions for kim')
    key(document.activeElement!, 'Escape')
    openMenu(0)
    expect(items()).toContain('Copy quote')
  })
})

describe('pins on a quote (#257)', () => {
  const NOW = Date.parse('2026-11-09T10:00:00.000Z')
  const pinned = (expires_at: string | null) => ({ pin: { pinned_by: 'U2', pinned_at: '2026-11-09T08:00:00.000Z', expires_at } })
  const flyout = () => document.querySelector<HTMLElement>('[role=menu][aria-label="Pin quote for"]')
  const lengths = () => [...(flyout()?.querySelectorAll<HTMLButtonElement>('[role=menuitem]') ?? [])]
  beforeEach(async () => {
    vi.useFakeTimers({ toFake: ['Date'] })
    vi.setSystemTime(NOW)
    ;(await import('../lib/serverClock')).resetServerClock(null)
  })
  afterEach(() => vi.useRealTimers())

  it("the line's menu pins the whole quote, for a length picked from the flyout", async () => {
    show(quote([line()]))
    openMenu(0)
    expect(items()).toContain('Pin quote')
    expect(items()).not.toContain('Unpin quote')
    item('Pin quote').dispatchEvent(new Event('pointerenter'))
    flushSync()
    expect(lengths().map((b) => b.textContent?.trim())).toEqual(['1 hour', '1 day', '1 week', 'Forever'])
    lengths()[1].click()
    await settle()
    expect(calls('pin_message')).toEqual([['pin_message', { messageId: 'M1', duration: 'day' }]])
    expect(document.querySelector('[role=menu]')).toBeNull()
  })

  it('a pinned quote offers Unpin quote, and shows the marker by its time', async () => {
    show(quote([line()], pinned('2026-11-11T13:00:00.000Z')))
    const mark = document.querySelector<HTMLElement>('.meta .pin-mark')
    expect(mark?.dataset.tip).toBe('Pinned by sam · 2 days left')
    expect(mark?.querySelector('svg')?.getAttribute('width')).toBe('14')
    openMenu(0)
    expect(items()).not.toContain('Pin quote')
    item('Unpin quote').click()
    await settle()
    expect(calls('unpin_message')).toEqual([['unpin_message', { messageId: 'M1' }]])
  })

  it('a Forever pin reads without a time; one run out shows nowhere', () => {
    show(quote([line()], pinned(null)))
    expect(document.querySelector<HTMLElement>('.pin-mark')?.dataset.tip).toBe('Pinned by sam')
    unmount(cmp!); cmp = undefined; document.body.innerHTML = ''
    show(quote([line()], pinned('2026-11-09T09:00:00.000Z')))
    expect(document.querySelector('.pin-mark')).toBeNull()
    openMenu(0)
    expect(items()).toContain('Pin quote')
  })

  it("a refused pin says why and fetches the chat's pins to set it right", async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'pin_message') throw 'This chat already has 50 pins. Unpin something first.'
      return cmd === 'list_pins' ? [] : undefined
    })
    show(quote([line()]))
    openMenu(0)
    item('Pin quote').dispatchEvent(new Event('pointerenter'))
    flushSync()
    lengths()[0].click()
    await settle()
    flushSync()
    expect(document.querySelector('[role=alert]')?.textContent).toBe('This chat already has 50 pins. Unpin something first.')
    expect(calls('list_pins')).toEqual([['list_pins', { channelId: 'C1' }]])
  })

  it("a QotY option (compact) has no pin item and no marker", () => {
    show(quote([line()], pinned(null)), { compact: true })
    expect(document.querySelector('.pin-mark')).toBeNull()
    openMenu(0)
    expect(items()).not.toContain('Pin quote')
    expect(items()).not.toContain('Unpin quote')
  })
})
