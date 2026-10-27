// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))

import Composer from './Composer.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
import { offsetOf, positionAt, serialize } from '../lib/composerDom'

const ce = (id: string, server_id: string, name: string): CustomEmoji => ({
  id, server_id, name, hash: id.repeat(64).slice(0, 64), still_hash: null, animated: false, added_by: null, created_at: '2026-10-01T00:00:00Z',
})

let cmp: ReturnType<typeof mount> | undefined
const onSend = vi.fn()
type Box = HTMLElement
function show(draftKey = `K${Math.random()}`) {
  const me = { id: 'U1', username: 'alex', avatar_hash: null }
  app.state = {
    ...emptyState(), me,
    servers: [{ id: 'S1', name: 'Main', icon_hash: null }, { id: 'S2', name: 'Other', icon_hash: null }],
    members: { S1: [{ user: me, nickname: null }], S2: [] },
    customEmoji: { '1': ce('1', 'S1', 'pog'), '2': ce('2', 'S2', 'pogchamp') },
  }
  cmp = mount(Composer, {
    target: document.body,
    props: { placeholder: 'Message', draftKey, replyingTo: null, onCancelReply: () => {}, onSend, onTyping: () => {} },
  })
  flushSync()
  return document.querySelector<HTMLElement>('[role="textbox"]')!
}
/** The message text the box holds (what would be sent). */
const value = (box: Box) => serialize(box)
/** Put the caret at text offset `at` (the browser's selection, as a click or arrow key leaves it). */
function caretAt(box: Box, at: number) {
  const p = positionAt(box, at)
  const r = document.createRange()
  r.setStart(p.node, p.offset)
  r.collapse(true)
  getSelection()!.removeAllRanges()
  getSelection()!.addRange(r)
  document.dispatchEvent(new Event('selectionchange'))
}
const caret = (box: Box) => {
  const sel = getSelection()!
  return offsetOf(box, sel.anchorNode!, sel.anchorOffset)
}
/** Typing that leaves the box holding plain `text` with the caret at its end (as the browser would). */
function type(box: Box, text: string) {
  box.replaceChildren(document.createTextNode(text))
  caretAt(box, text.length)
  box.dispatchEvent(new InputEvent('input', { bubbles: true }))
  flushSync()
}
const press = (box: Box, key: string, init: KeyboardEventInit = {}) => {
  box.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init }))
  box.dispatchEvent(new KeyboardEvent('keyup', { key, bubbles: true, ...init }))
  flushSync()
}
const options = () => [...document.querySelectorAll('[aria-label="Insert emoji"] [role=option]')].map((o) => o.textContent!.trim())

beforeEach(() => { onSend.mockClear(); localStorage.clear() })
afterEach(() => { if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = '' })

describe('Composer emoji', () => {
  it('lists your servers\' custom emoji (with server name) and standard ones, at most 8', () => {
    const box = show()
    type(box, 'gg :po')
    const o = options()
    expect(o[0].replace(/\s+/g, ' ')).toBe(':pog: Main')
    expect(o.some((x) => x.includes('pogchamp'))).toBe(false) // not a member of S2
    type(box, 'gg :sm')
    expect(options()).toHaveLength(8)
  })

  it('arrows + Enter insert the pick; Esc dismisses; no popup after a URL colon', async () => {
    const box = show()
    type(box, ':po')
    press(box, 'ArrowDown')
    expect(document.querySelector('[aria-label="Insert emoji"] [aria-selected=true]')!.textContent).not.toContain('pog')
    press(box, 'ArrowUp')
    press(box, 'Enter')
    await tick()
    expect(value(box)).toBe('<:pog:1> ')
    expect(onSend).not.toHaveBeenCalled()
    type(box, 'x :thumbsup')
    press(box, 'Escape')
    expect(options()).toHaveLength(0)
    type(box, 'see http://xy')
    expect(options()).toHaveLength(0)
  })

  it('the emoji button opens the picker and inserts at the caret', async () => {
    const box = show()
    type(box, 'ab')
    caretAt(box, 1)
    ;(document.querySelector('[aria-label="Emoji"]') as HTMLButtonElement).click()
    flushSync()
    const cell = document.querySelector('.picker button.cell[aria-label=":grinning:"]') as HTMLButtonElement
    cell.click()
    await tick()
    expect(value(box)).toBe('a😀 b')
    expect(document.querySelector('.picker')).toBeNull()
  })

  it('shift_click_inserts_and_keeps_the_picker_open', async () => {
    const box = show()
    type(box, 'ab')
    caretAt(box, 1)
    ;(document.querySelector('[aria-label="Emoji"]') as HTMLButtonElement).click()
    flushSync()
    const shiftClick = (name: string) => {
      document.querySelector(`.picker button.cell[aria-label=":${name}:"]`)!
        .dispatchEvent(new MouseEvent('click', { bubbles: true, shiftKey: true }))
    }
    shiftClick('grinning')
    await tick()
    shiftClick('joy')
    await tick()
    expect(value(box)).toBe('a😀 😂 b')
    expect(document.querySelector('.picker')).not.toBeNull()
    expect(document.activeElement).not.toBe(box)
    // the caret moved past what was added, ready for the next pick
    ;(document.querySelector('[aria-label="Emoji"]') as HTMLButtonElement).click() // close the picker
    flushSync()
    box.focus() // back to typing
    expect(caret(box)).toBe('a😀 😂 '.length)
  })

  it('plain_click_inserts_and_closes', async () => {
    const box = show()
    type(box, 'ab')
    caretAt(box, 1)
    ;(document.querySelector('[aria-label="Emoji"]') as HTMLButtonElement).click()
    flushSync()
    ;(document.querySelector('.picker button.cell[aria-label=":joy:"]') as HTMLButtonElement).click()
    await tick()
    expect(value(box)).toBe('a😂 b')
    expect(document.querySelector('.picker')).toBeNull()
  })

  it('autocomplete uses your saved skin tone where the emoji has one', async () => {
    localStorage.setItem('pulse.skinTone', '3')
    const box = show()
    type(box, ':thumbsup')
    expect(document.querySelector('[aria-label="Insert emoji"] img')!.getAttribute('src')).toBe('/emoji/1f44d-1f3fd.svg')
    press(box, 'Enter')
    await tick()
    expect(value(box)).toBe('👍🏽 ')
    type(box, ':grinning')
    press(box, 'Enter')
    await tick()
    expect(value(box)).toBe('😀 ') // no tones: unchanged
  })

  it('a custom emoji shows as its image in the box, and is sent as its token', async () => {
    const box = show()
    type(box, 'gg :po')
    press(box, 'Enter')
    await tick()
    const img = box.querySelector('img')!
    expect(img.alt).toBe(':pog:')
    expect(box.textContent).not.toContain('<:pog:1>') // not shown as text
    expect(value(box)).toBe('gg <:pog:1> ')
    press(box, 'Enter')
    expect(onSend).toHaveBeenCalledWith('gg <:pog:1>')
    expect(value(box)).toBe('')
    expect(box.querySelector('img')).toBeNull()
  })

  it('custom emoji picked in the picker (and shift-clicked several) show as images', async () => {
    const box = show()
    type(box, 'ab')
    caretAt(box, 1)
    ;(document.querySelector('[aria-label="Emoji"]') as HTMLButtonElement).click()
    flushSync()
    const cell = () => document.querySelector('.picker button.cell[aria-label=":pog:"]')!
    cell().dispatchEvent(new MouseEvent('click', { bubbles: true, shiftKey: true }))
    await tick()
    cell().dispatchEvent(new MouseEvent('click', { bubbles: true, shiftKey: true }))
    await tick()
    expect(value(box)).toBe('a<:pog:1> <:pog:1> b')
    expect(box.querySelectorAll('img')).toHaveLength(2)
  })

  it('shift_enter_adds_a_line_and_enter_sends_them_all', async () => {
    const box = show()
    type(box, 'one')
    press(box, 'Enter', { shiftKey: true })
    await tick()
    expect(value(box)).toBe('one\n')
    expect(onSend).not.toHaveBeenCalled()
    box.lastChild!.before(document.createTextNode('two')) // typed on the new line
    box.dispatchEvent(new InputEvent('input', { bubbles: true }))
    flushSync()
    press(box, 'Enter')
    expect(onSend).toHaveBeenCalledWith('one\ntwo')
  })

  it('pasting keeps only the text, at the caret', async () => {
    const box = show()
    type(box, 'ab')
    caretAt(box, 1)
    const e = new Event('paste', { bubbles: true, cancelable: true }) as ClipboardEvent
    Object.defineProperty(e, 'clipboardData', { value: { getData: (t: string) => (t === 'text/plain' ? 'X<:pog:1>' : '<b>X</b>') } })
    box.dispatchEvent(e)
    await tick()
    expect(e.defaultPrevented).toBe(true)
    expect(value(box)).toBe('aX<:pog:1>b') // a pasted token is an emoji too
    expect(box.querySelector('b')).toBeNull()
    expect(box.querySelector('img')).not.toBeNull()
  })

  it('a draft comes back with its emoji as images', async () => {
    const key = `K${Math.random()}`
    let box = show(key)
    type(box, 'hi :po')
    press(box, 'Enter')
    await tick()
    unmount(cmp!)
    cmp = undefined
    box = show(key)
    expect(value(box)).toBe('hi <:pog:1> ')
    expect(box.querySelector('img')).not.toBeNull()
  })

  it('it is a labelled multi-line text box with a placeholder', () => {
    const box = show()
    expect(box.getAttribute('contenteditable')).toBe('true')
    expect(box.getAttribute('aria-multiline')).toBe('true')
    expect(box.getAttribute('aria-label')).toBe('Message')
    expect(box.dataset.placeholder).toBe('Message')
    expect(box.classList.contains('empty')).toBe(true)
    type(box, 'x')
    expect(box.classList.contains('empty')).toBe(false)
  })
})
