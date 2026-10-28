// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { CustomEmoji } from '../lib/protocol/CustomEmoji'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import MessageItem, { INLINE_ERROR_MS } from './MessageItem.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { offsetOf, positionAt, serialize } from '../lib/composerDom'
import { appearance } from '../lib/appearance.svelte'
import { motion } from '../lib/animate.svelte'
import { closeCard, profileUi } from '../lib/profileCard.svelte'

const ce = (id: string, name: string): CustomEmoji => ({
  id, server_id: 'S1', name, hash: id.repeat(64).slice(0, 64), still_hash: null, animated: false, added_by: null, created_at: '2026-10-01T00:00:00Z',
})

const msg = (over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U1', kind: 'normal', content: 'hello',
  reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [],
  reactions: [{ emoji: '👍', count: 1, me: false }], thread_root_id: null, quote: null, qoty_card: null, clip: null, ...over,
})

let cmp: ReturnType<typeof mount> | undefined
const onEdit = vi.fn(async (_c: string) => {})
const onDelete = vi.fn(async () => {})
const onEditClosed = vi.fn()
function show(content = 'hello') {
  const m = msg({ content })
  app.state = { ...emptyState(), messages: { C1: [m] }, customEmoji: { '1': ce('1', 'pog') } }
  cmp = mount(MessageItem, { target: document.body, props: {
    message: app.state.messages['C1'][0], serverId: null, name: 'alex', nameOf: (id: string | null) => id ?? '', replyTo: null,
    grouped: false, mine: true, onReply: () => {}, onEdit, onDelete, meId: 'U1', onEditClosed,
  } })
  flushSync()
}
const alert = () => document.querySelector('[role="alert"]')
const btn = (label: string) => document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!
const box = () => document.querySelector<HTMLElement>('[role="textbox"][aria-label="Edit message"]')
/** The text the edit box holds (what a save would send). */
const value = () => serialize(box()!)
function caretAt(at: number) {
  const p = positionAt(box()!, at)
  const r = document.createRange()
  r.setStart(p.node, p.offset)
  r.collapse(true)
  getSelection()!.removeAllRanges()
  getSelection()!.addRange(r)
  document.dispatchEvent(new Event('selectionchange'))
}
const caret = () => offsetOf(box()!, getSelection()!.anchorNode!, getSelection()!.anchorOffset)
/** Typing that leaves the box holding plain `text`, the caret at its end (as the browser would). */
function type(text: string) {
  const t = box()!
  t.focus()
  t.replaceChildren(document.createTextNode(text))
  caretAt(text.length)
  t.dispatchEvent(new InputEvent('input', { bubbles: true }))
  flushSync()
}
const press = (key: string, init: KeyboardEventInit = {}) => {
  const e = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init })
  box()!.dispatchEvent(e)
  return e
}
async function settle() { for (let i = 0; i < 5; i++) await Promise.resolve(); await tick() }

beforeEach(() => {
  vi.useFakeTimers()
  invoke.mockReset(); invoke.mockResolvedValue(undefined)
  onEdit.mockReset(); onEdit.mockResolvedValue(undefined)
  onDelete.mockReset(); onDelete.mockResolvedValue(undefined)
  onEditClosed.mockReset()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

/** jsdom never reports :focus-visible; say what a browser would for this element. */
function stubFocusVisible(el: HTMLElement, visible: boolean) {
  const real = el.matches.bind(el) as (s: string) => boolean
  vi.spyOn(el, 'matches').mockImplementation(((sel: string) => (sel === ':focus-visible' ? visible : real(sel))) as never)
}

describe('MessageItem errors', () => {
  it('failed_edit_keeps_the_box_and_text_with_the_error', async () => {
    onEdit.mockRejectedValue('edit refused')
    show()
    btn('Edit').click(); flushSync()
    type('changed text')
    press('Enter')
    await settle()
    expect(box()).not.toBeNull()
    expect(value()).toBe('changed text')
    expect(alert()?.textContent).toBe('edit refused')
    // The error stays (no timer) until the next save or Esc.
    vi.advanceTimersByTime(INLINE_ERROR_MS * 3)
    flushSync()
    expect(alert()?.textContent).toBe('edit refused')
    press('Escape'); flushSync()
    expect(box()).toBeNull()
    expect(alert()).toBeNull()
  })

  it('successful_edit_closes_the_box', async () => {
    show()
    btn('Edit').click(); flushSync()
    type('changed text')
    press('Enter')
    await settle()
    expect(onEdit).toHaveBeenCalledWith('changed text')
    expect(box()).toBeNull()
    expect(alert()).toBeNull()
  })

  it('failed_delete_shows_inline_error_that_hides_after_5s', async () => {
    onDelete.mockRejectedValue('cannot delete')
    show()
    btn('Delete').click()
    await settle()
    expect(alert()?.textContent).toBe('cannot delete')
    vi.advanceTimersByTime(INLINE_ERROR_MS - 1); flushSync()
    expect(alert()).not.toBeNull()
    vi.advanceTimersByTime(1); flushSync()
    expect(alert()).toBeNull()
  })

  it('a_newer_error_restarts_the_clock', async () => {
    onDelete.mockRejectedValue('cannot delete')
    show()
    btn('Delete').click(); await settle()
    vi.advanceTimersByTime(INLINE_ERROR_MS - 1000)
    btn('Delete').click(); await settle()
    vi.advanceTimersByTime(INLINE_ERROR_MS - 1000); flushSync()
    expect(alert()).not.toBeNull()
    vi.advanceTimersByTime(1000); flushSync()
    expect(alert()).toBeNull()
  })

  it('unmounting_clears_the_timer', async () => {
    onDelete.mockRejectedValue('cannot delete')
    show()
    btn('Delete').click(); await settle()
    expect(vi.getTimerCount()).toBe(1)
    unmount(cmp!); cmp = undefined
    expect(vi.getTimerCount()).toBe(0)
  })

  it('a_reaction_failing_after_unmount_starts_no_timer', async () => {
    let refuse!: (e: unknown) => void
    invoke.mockImplementation((cmd: string) => (cmd === 'add_reaction' || cmd === 'remove_reaction' ? new Promise((_, rej) => (refuse = rej)) : Promise.resolve(undefined)))
    show()
    document.querySelector<HTMLButtonElement>('.chip:not(.add)')!.click()
    await settle()
    expect(refuse).toBeDefined()
    unmount(cmp!); cmp = undefined
    refuse('reaction refused')
    await settle()
    expect(vi.getTimerCount()).toBe(0)
  })

  it('a_second_save_clears_the_old_error_while_in_flight', async () => {
    onEdit.mockRejectedValueOnce('edit refused')
    show()
    btn('Edit').click(); flushSync()
    type('changed text')
    press('Enter'); await settle()
    expect(alert()?.textContent).toBe('edit refused')
    let finish!: () => void
    onEdit.mockImplementationOnce(() => new Promise<void>((done) => (finish = done)))
    press('Enter'); await settle(); flushSync()
    expect(alert()).toBeNull()
    finish(); await settle(); flushSync()
    expect(box()).toBeNull()
  })

  it('reopening_the_edit_box_shows_no_old_error', async () => {
    onEdit.mockRejectedValueOnce('edit refused')
    show()
    btn('Edit').click(); flushSync()
    type('changed text')
    press('Enter'); await settle()
    expect(alert()?.textContent).toBe('edit refused')
    // the Edit button is hidden while the box is open: reopening goes through Esc
    press('Escape'); flushSync()
    btn('Edit').click(); flushSync()
    expect(alert()).toBeNull()
    expect(value()).toBe('hello')
  })

  it('a_double_enter_while_saving_sends_one_edit', async () => {
    let finish!: () => void
    onEdit.mockImplementation(() => new Promise<void>((done) => (finish = done)))
    show()
    btn('Edit').click(); flushSync()
    type('changed text')
    press('Enter')
    press('Enter')
    await settle()
    expect(onEdit).toHaveBeenCalledTimes(1)
    finish(); await settle(); flushSync()
    expect(box()).toBeNull()
  })

  it('failed_reaction_shows_inline_error_not_the_banner', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'add_reaction' || cmd === 'remove_reaction') throw 'reaction refused'
      return undefined
    })
    show()
    document.querySelector<HTMLButtonElement>('.chip:not(.add)')!.click()
    await settle()
    expect(alert()?.textContent).toBe('reaction refused')
    vi.advanceTimersByTime(INLINE_ERROR_MS); flushSync()
    expect(alert()).toBeNull()
  })
})

describe('MessageItem edit box', () => {
  const edit = async (content: string) => {
    show(content)
    btn('Edit').click(); flushSync()
    await settle()
  }

  it('shows custom emoji as images, keeping the tokens as the text', async () => {
    await edit('hi <:pog:1> there')
    const b = box()!
    expect(b.tagName).not.toBe('TEXTAREA')
    expect(b.getAttribute('contenteditable')).toBe('plaintext-only')
    const img = b.querySelector('img')!
    expect(img.alt).toBe(':pog:')
    expect(b.textContent).not.toContain('<:pog:1>')
    expect(value()).toBe('hi <:pog:1> there')
  })

  it('opens focused with the caret at the end', async () => {
    await edit('hi <:pog:1> there')
    expect(document.activeElement).toBe(box())
    expect(caret()).toBe('hi <:pog:1> there'.length)
  })

  it('enter saves the wire text, emoji tokens included', async () => {
    await edit('hi <:pog:1>')
    box()!.append(document.createTextNode(' more'))
    caretAt(value().length)
    box()!.dispatchEvent(new InputEvent('input', { bubbles: true }))
    flushSync()
    expect(press('Enter').defaultPrevented).toBe(true)
    await settle()
    expect(onEdit).toHaveBeenCalledWith('hi <:pog:1> more')
    expect(box()).toBeNull()
  })

  it('shift_enter_adds_a_line_and_does_not_save', async () => {
    await edit('one')
    caretAt(3)
    press('Enter', { shiftKey: true })
    await settle(); flushSync()
    expect(value()).toBe('one\n')
    expect(onEdit).not.toHaveBeenCalled()
    expect(box()).not.toBeNull()
  })

  it('esc_cancels_without_saving', async () => {
    await edit('hello')
    type('changed')
    // taken here (full view's Esc doesn't also go back to the grid)
    expect(press('Escape').defaultPrevented).toBe(true)
    flushSync()
    expect(box()).toBeNull()
    expect(onEdit).not.toHaveBeenCalled()
  })

  it('an_enter_meant_for_an_ime_does_not_save', async () => {
    await edit('hello')
    type('changed')
    expect(press('Enter', { isComposing: true }).defaultPrevented).toBe(false)
    await settle()
    expect(onEdit).not.toHaveBeenCalled()
    expect(box()).not.toBeNull()
  })

  it('undo and redo work in the edit box', async () => {
    await edit('hello')
    type('changed')
    press('z', { ctrlKey: true }); await settle(); flushSync()
    expect(value()).toBe('hello')
    press('y', { ctrlKey: true }); await settle(); flushSync()
    expect(value()).toBe('changed')
  })

  it('copy carries the tokens and a paste comes in as plain text', async () => {
    await edit('hi <:pog:1>')
    const r = document.createRange()
    r.selectNodeContents(box()!)
    getSelection()!.removeAllRanges()
    getSelection()!.addRange(r)
    const data = new Map<string, string>()
    const clip = (kind: string, from?: Map<string, string>) => {
      const e = new Event(kind, { bubbles: true, cancelable: true }) as ClipboardEvent
      Object.defineProperty(e, 'clipboardData', { value: { setData: (t: string, v: string) => data.set(t, v), getData: (t: string) => (from ?? data).get(t) ?? '' } })
      box()!.dispatchEvent(e)
      return e
    }
    expect(clip('copy').defaultPrevented).toBe(true)
    expect(data.get('text/plain')).toBe('hi <:pog:1>')
    caretAt(value().length)
    expect(clip('paste', new Map([['text/plain', ' <:pog:1>'], ['text/html', '<b>x</b>']])).defaultPrevented).toBe(true)
    await settle(); flushSync()
    expect(value()).toBe('hi <:pog:1> <:pog:1>')
    expect(box()!.querySelectorAll('img')).toHaveLength(2)
    expect(box()!.querySelector('b')).toBeNull()
  })

  it('refuses input past the 4000 limit, never the text already there', async () => {
    await edit('x'.repeat(3998))
    const before = (data: string) => {
      const e = new InputEvent('beforeinput', { bubbles: true, cancelable: true, inputType: 'insertText', data })
      box()!.dispatchEvent(e)
      return e
    }
    expect(before('abc').defaultPrevented).toBe(true)
    expect(before('ab').defaultPrevented).toBe(false)
    expect(value()).toBe('x'.repeat(3998))
  })

  it('a double enter on an emoji edit sends one edit, tokens kept', async () => {
    let finish!: () => void
    onEdit.mockImplementation(() => new Promise<void>((done) => (finish = done)))
    await edit('hi <:pog:1>')
    box()!.prepend(document.createTextNode('oh '))
    caretAt(3)
    box()!.dispatchEvent(new InputEvent('input', { bubbles: true }))
    flushSync()
    press('Enter')
    press('Enter')
    await settle()
    expect(onEdit).toHaveBeenCalledTimes(1)
    expect(onEdit).toHaveBeenCalledWith('oh hi <:pog:1>')
    finish(); await settle(); flushSync()
    expect(box()).toBeNull()
  })

  it('esc while a save is in flight keeps the box until it lands', async () => {
    let fail!: (e: unknown) => void
    onEdit.mockImplementation(() => new Promise<void>((_, rej) => (fail = rej)))
    await edit('hello')
    type('changed')
    press('Enter')
    await settle()
    expect(press('Escape').defaultPrevented).toBe(true)
    flushSync()
    expect(box()).not.toBeNull()
    expect(onEditClosed).not.toHaveBeenCalled()
    fail('edit refused'); await settle(); flushSync()
    expect(alert()?.textContent).toBe('edit refused') // the reason shows, the text is still there
    expect(value()).toBe('changed')
    expect(document.activeElement).toBe(box()) // focus stays to fix it
  })

  it('closing hands focus back to the caller, saved or cancelled', async () => {
    await edit('hello')
    press('Escape'); flushSync()
    expect(onEditClosed).toHaveBeenCalledTimes(1)
    btn('Edit').click(); flushSync(); await settle()
    type('changed')
    press('Enter'); await settle(); flushSync()
    expect(onEditClosed).toHaveBeenCalledTimes(2)
  })

  it('the keys hint describes the box', async () => {
    await edit('hello')
    const hint = document.getElementById(box()!.getAttribute('aria-describedby')!)!
    expect(hint.textContent).toContain('Enter to save')
    expect(hint.textContent).toContain('Shift+Enter')
  })

  it('emoji the app learns about after the box opened show as images', async () => {
    show('hi <:zap:9>')
    btn('Edit').click(); flushSync(); await settle()
    expect(box()!.querySelector('img')).toBeNull()
    app.state = { ...app.state, customEmoji: { ...app.state.customEmoji, '9': ce('9', 'zap') } }
    flushSync()
    expect(box()!.querySelector('img')!.alt).toBe(':zap:')
    expect(value()).toBe('hi <:zap:9>')
  })

  it('a paste past the limit is cut to what fits', async () => {
    await edit('x'.repeat(3998))
    caretAt(3998)
    const e = new Event('paste', { bubbles: true, cancelable: true }) as ClipboardEvent
    Object.defineProperty(e, 'clipboardData', { value: { getData: (t: string) => (t === 'text/plain' ? 'abcdef' : '') } })
    box()!.dispatchEvent(e)
    await settle(); flushSync()
    expect(value()).toBe('x'.repeat(3998) + 'ab')
  })

  it('a save that lands after focus moved on leaves focus where it is', async () => {
    let finish!: () => void
    onEdit.mockImplementation(() => new Promise<void>((done) => (finish = done)))
    await edit('hello')
    type('changed')
    press('Enter')
    await settle()
    const other = document.createElement('input')
    document.body.append(other)
    other.focus() // e.g. another message's edit box, or a search field
    finish(); await settle(); flushSync()
    expect(box()).toBeNull()
    expect(onEditClosed).not.toHaveBeenCalled()
    expect(document.activeElement).toBe(other)
  })

  it('focusing into a message exposes its action buttons', () => {
    show()
    const actions = document.querySelector<HTMLElement>('.actions')!
    expect(actions.classList.contains('open')).toBe(false)
    const b = actions.querySelector<HTMLElement>('button')!
    stubFocusVisible(b, true) // jsdom has no keyboard-focus heuristic
    b.focus(); flushSync()
    expect(actions.classList.contains('open')).toBe(true)
  })

  it('a mouse focus inside the message after keyboard focus closes the bar again', () => {
    show()
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

  it('a mouse-style focus does not open the action bar', () => {
    show()
    const chip = document.querySelector<HTMLElement>('.chip')!
    stubFocusVisible(chip, false)
    chip.focus(); flushSync()
    expect(document.querySelector('.actions')!.classList.contains('open')).toBe(false)
  })
})

describe('MessageItem names and avatars', () => {
  const A = 'a'.repeat(64)
  const S = 'b'.repeat(64)
  const url = (h: string) => `pulse-media://localhost/${h}`
  const u = (id: string, username: string, over: object = {}) => ({ id, username, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] }, ...over })
  function showStyled(content: string, alexOver: object = {}, mentionNames: string[] = []) {
    const m = msg({ content })
    const alex = u('U1', 'alex', alexOver)
    const sam = u('U2', 'sam', { style: { font: 'fredoka', colors: ['#ffc4a3'] } })
    const kim = u('U3', 'kim', { style: { font: null, colors: ['#ffbe7a', '#ff7fae'] } })
    app.state = {
      ...emptyState(), me: alex, messages: { C1: [m] },
      people: { U1: { user: alex, online: true, last_seen_at: null }, U2: { user: sam, online: true, last_seen_at: null }, U3: { user: kim, online: true, last_seen_at: null } },
    }
    cmp = mount(MessageItem, { target: document.body, props: {
      message: app.state.messages['C1'][0], name: 'alex', nameOf: (id: string | null) => id ?? '', replyTo: null, serverId: 'S1',
      grouped: false, mine: false, onReply: () => {}, onEdit, onDelete, meId: 'U9', mentionNames,
    } })
    flushSync()
  }
  beforeEach(() => {
    appearance.showNameStyles = true
    motion.streaming = false
    window.dispatchEvent(new Event('focus'))
    closeCard()
  })
  afterEach(() => closeCard())

  it('the author name uses the style and opens the card', () => {
    showStyled('hello', { style: { font: 'fredoka', colors: ['#ffc4a3'] } })
    const name = document.querySelector<HTMLElement>('.meta .name')!
    expect(name.textContent).toBe('alex')
    expect(name.getAttribute('style')).toContain('Fredoka')
    expect(name.getAttribute('style')).toContain('rgb(255, 196, 163)')
    name.click()
    flushSync()
    expect(profileUi.card).toMatchObject({ userId: 'U1', serverId: 'S1' })
    closeCard()
    document.querySelector<HTMLButtonElement>('.row .avbtn')!.click()
    flushSync()
    expect(profileUi.card).toMatchObject({ userId: 'U1', serverId: 'S1' })
  })

  it('an animated avatar shows its still in chat', () => {
    showStyled('hello', { avatar_hash: A, avatar_still: S })
    const img = document.querySelector<HTMLImageElement>('.row .av img')!
    expect(img.getAttribute('src')).toBe(url(S))
    // it animates while the pointer is anywhere over the message's row, not only the avatar
    document.querySelector('.msg')!.dispatchEvent(new Event('pointerenter'))
    flushSync()
    expect(img.getAttribute('src')).toBe(url(A))
    document.querySelector('.msg')!.dispatchEvent(new Event('pointerleave'))
    flushSync()
    expect(img.getAttribute('src')).toBe(url(S))
  })

  it('mention pills take the mentioned person\'s font and colour', () => {
    showStyled('hi @sam and @kim and @jo', {}, ['sam', 'kim', 'jo'])
    const [sam, kim, jo] = document.querySelectorAll<HTMLElement>('.content .mention')
    expect(sam.style.fontFamily).toContain('Fredoka')
    expect(sam.style.color).toBe('rgb(255, 196, 163)')
    expect(kim.style.backgroundImage).toContain('linear-gradient')
    expect(jo.getAttribute('style')).toBeNull()
    // a profile change restyles the pills live
    app.state = { ...app.state, people: { ...app.state.people, U2: { user: u('U2', 'sam', { style: { font: null, colors: ['#a3d8ff'] } }), online: true, last_seen_at: null } } }
    flushSync()
    expect(document.querySelector<HTMLElement>('.content .mention')!.style.color).toBe('rgb(163, 216, 255)')
    // an unrelated update (someone typing) doesn't restyle the pills
    const pill = document.querySelector<HTMLElement>('.content .mention')!
    pill.setAttribute('style', 'color: red')
    app.state = { ...app.state, typing: { C1: { U3: Date.now() + 5000 } } }
    flushSync()
    expect(pill.getAttribute('style')).toBe('color: red')
    app.state = { ...app.state, people: { ...app.state.people, U2: { user: u('U2', 'sam', { style: { font: null, colors: ['#ffc4a3'] } }), online: true, last_seen_at: null } } }
    flushSync()
    expect(pill.style.color).toBe('rgb(255, 196, 163)')
    // name styles off: plain pills
    appearance.showNameStyles = false
    flushSync()
    expect(document.querySelector<HTMLElement>('.content .mention')!.style.color).toBe('')
  })

  it('clicking a mention pill opens that person\'s card; an unknown name does nothing', () => {
    showStyled('hi @sam and @jo', {}, ['sam', 'jo'])
    const [sam, jo] = document.querySelectorAll<HTMLElement>('.content .mention')
    jo.click()
    flushSync()
    expect(profileUi.card).toBeNull()
    sam.click()
    flushSync()
    expect(profileUi.card).toMatchObject({ userId: 'U2', serverId: 'S1' })
  })
})

describe('mention pill CSS', () => {
  // From the compiled CSS (jsdom applies the cascade but doesn't lay out).
  it('a scaled pill font keeps the line height', async () => {
    const { readFileSync } = await import('node:fs')
    const { resolve } = await import('node:path')
    const { compile } = await import('svelte/compiler')
    const src = readFileSync(resolve(__dirname, 'MessageItem.svelte'), 'utf8')
    const css = compile(src, { filename: 'MessageItem.svelte', css: 'external', cssHash: () => 'svelte-test' }).css!.code
    const style = document.createElement('style')
    style.textContent = css
    document.head.append(style)
    try {
      document.body.innerHTML = '<div class="content svelte-test"><span class="mention" style="--name-scale: 1.2">@kim</span></div>'
      const pill = document.querySelector<HTMLElement>('.mention')!
      const cs = getComputedStyle(pill)
      expect(cs.fontSize).toBe('calc(1em * var(--name-scale, 1))')
      // the same compensation Name uses: the line box stays 1.5 of the message's font
      expect(cs.lineHeight).toBe('calc(1.5em / var(--name-scale, 1))')
      expect(cs.verticalAlign).toBe('baseline')
    } finally {
      style.remove()
    }
  })
})
