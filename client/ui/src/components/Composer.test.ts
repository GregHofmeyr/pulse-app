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
  const me = { id: 'U1', username: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
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
  box.focus() // a click in the box focuses it as it places the caret
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
    await tick()
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
    // plain text only: no bold/italic shortcuts, no HTML pastes or drops
    expect(box.getAttribute('contenteditable')).toBe('plaintext-only')
    expect(box.getAttribute('aria-placeholder')).toBe('Message')
    expect(box.getAttribute('dir')).toBe('auto')
    expect(box.getAttribute('aria-multiline')).toBe('true')
    expect(box.getAttribute('aria-label')).toBe('Message')
    expect(box.dataset.placeholder).toBe('Message')
    expect(box.classList.contains('empty')).toBe(true)
    type(box, 'x')
    expect(box.classList.contains('empty')).toBe(false)
  })

  const beforeInput = (box: Box, inputType: string, data: string | null = null) => {
    const e = new InputEvent('beforeinput', { bubbles: true, cancelable: true, inputType, data })
    box.dispatchEvent(e)
    flushSync()
    return e
  }
  /** Put the emoji draft in place: 'hi <:pog:1> there', the caret after "there". */
  async function emojiDraft() {
    const box = show()
    type(box, 'hi :po')
    press(box, 'Enter')
    await tick()
    box.lastChild!.textContent += 'there'
    box.dispatchEvent(new InputEvent('input', { bubbles: true }))
    caretAt(box, value(box).length)
    flushSync()
    return box
  }

  it('an unrelated app update leaves the box (and its caret) alone', async () => {
    // a draft with an emoji, back after switching channel
    const key = `K${Math.random()}`
    let box = show(key)
    type(box, 'hi :po')
    press(box, 'Enter')
    await tick()
    unmount(cmp!)
    box = show(key)
    caretAt(box, 3)
    const img = box.querySelector('img')
    const at = caret(box)
    app.state = { ...app.state } // any gateway event
    flushSync()
    expect(box.querySelector('img')).toBe(img) // not rebuilt
    expect(caret(box)).toBe(at)
  })

  it('at the 4000 limit new input is refused, never the text already there', async () => {
    const box = show()
    const full = 'x'.repeat(3990) + ' <:pog:1>'.slice(0, 5) // 3995
    type(box, full)
    caretAt(box, 10)
    expect(beforeInput(box, 'insertText', 'abcdef').defaultPrevented).toBe(true) // 4006 > 4000
    expect(beforeInput(box, 'insertText', 'abcde').defaultPrevented).toBe(false) // fits exactly
    // a pick that doesn't fit whole isn't inserted at all
    type(box, 'x'.repeat(3996) + ' :po')
    press(box, 'Enter')
    await tick()
    expect(value(box)).toBe('x'.repeat(3996) + ' :po')
    // if something gets past (an IME commit), the text goes back to what it was, not cut short
    type(box, 'y'.repeat(4000))
    box.replaceChildren(document.createTextNode('Z' + 'y'.repeat(4000)))
    box.dispatchEvent(new InputEvent('input', { bubbles: true }))
    flushSync()
    expect(value(box)).toBe('y'.repeat(4000))
  })

  it('copy and cut carry the tokens, so pasting back gives the emoji', async () => {
    const box = await emojiDraft()
    const r = document.createRange()
    r.selectNodeContents(box)
    getSelection()!.removeAllRanges()
    getSelection()!.addRange(r)
    const data = new Map<string, string>()
    const clip = (type: string) => {
      const e = new Event(type, { bubbles: true, cancelable: true }) as ClipboardEvent
      Object.defineProperty(e, 'clipboardData', { value: { setData: (t: string, v: string) => data.set(t, v), getData: (t: string) => data.get(t) ?? '' } })
      box.dispatchEvent(e)
      return e
    }
    expect(clip('copy').defaultPrevented).toBe(true)
    expect(data.get('text/plain')).toBe('hi <:pog:1> there')
    clip('cut')
    await tick()
    expect(value(box)).toBe('')
    clip('paste')
    await tick()
    expect(value(box)).toBe('hi <:pog:1> there')
    expect(box.querySelector('img')).not.toBeNull()
  })

  it('formatting shortcuts do nothing (nothing they do would be sent)', () => {
    const box = show()
    expect(beforeInput(box, 'formatBold').defaultPrevented).toBe(true)
    expect(beforeInput(box, 'formatItalic').defaultPrevented).toBe(true)
  })

  it('undo and redo step through the box\'s own history', async () => {
    const box = show()
    type(box, 'gg :po')
    press(box, 'Enter')
    await tick()
    expect(value(box)).toBe('gg <:pog:1> ')
    expect(beforeInput(box, 'historyUndo').defaultPrevented).toBe(true)
    await tick()
    expect(value(box)).toBe('gg :po')
    beforeInput(box, 'historyRedo')
    await tick()
    expect(value(box)).toBe('gg <:pog:1> ')
  })

  it('ctrl_z_and_ctrl_shift_z_or_ctrl_y_undo_and_redo_even_with_nothing_in_the_browsers_history', async () => {
    const box = show()
    type(box, 'gg :po')
    press(box, 'Enter')
    await tick()
    press(box, 'z', { ctrlKey: true })
    await tick()
    expect(value(box)).toBe('gg :po')
    press(box, 'z', { ctrlKey: true, shiftKey: true })
    await tick()
    expect(value(box)).toBe('gg <:pog:1> ')
    press(box, 'z', { ctrlKey: true })
    await tick()
    press(box, 'y', { ctrlKey: true })
    await tick()
    expect(value(box)).toBe('gg <:pog:1> ')
  })

  it('keys meant for an IME candidate list are left to the IME', () => {
    const box = show()
    type(box, ':po')
    const e = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true, isComposing: true })
    box.dispatchEvent(e)
    flushSync()
    expect(e.defaultPrevented).toBe(false)
    expect(value(box)).toBe(':po')
    expect(onSend).not.toHaveBeenCalled()
  })

  it('focus coming back puts the caret where it was, not at the start', async () => {
    const box = show()
    type(box, 'hello')
    caretAt(box, 3)
    box.focus()
    ;(document.querySelector('[aria-label="Emoji"]') as HTMLButtonElement).focus()
    getSelection()!.removeAllRanges() // focus elsewhere takes the selection
    box.focus()
    await tick()
    expect(caret(box)).toBe(3)
  })

  it('a re-render while focus is elsewhere does not move where the next pick goes', async () => {
    const box = show()
    type(box, 'ab')
    caretAt(box, 1)
    box.focus()
    ;(document.querySelector('[aria-label="Emoji"]') as HTMLButtonElement).click()
    flushSync()
    ;(document.querySelector<HTMLElement>('.picker input') ?? document.body).focus()
    const cell = () => document.querySelector('.picker button.cell[aria-label=":pog:"]')!
    cell().dispatchEvent(new MouseEvent('click', { bubbles: true, shiftKey: true }))
    await tick()
    // the re-render collapsed a selection still in the box to its start, and the browser said so
    // (focus stays in the picker)
    const r = document.createRange()
    r.setStart(box, 0)
    getSelection()!.removeAllRanges()
    getSelection()!.addRange(r)
    document.dispatchEvent(new Event('selectionchange'))
    cell().dispatchEvent(new MouseEvent('click', { bubbles: true, shiftKey: true }))
    await tick()
    expect(value(box)).toBe('a<:pog:1> <:pog:1> b')
  })

  it('a text ending in a newline keeps its last line', () => {
    const box = show()
    type(box, 'a\n') // the browser left no line break after it
    expect(box.lastChild!.nodeName).toBe('BR')
    expect(value(box)).toBe('a\n')
  })

  it('a drop puts in its text, like a paste', async () => {
    const box = show()
    type(box, 'ab')
    caretAt(box, 2)
    const e = new Event('drop', { bubbles: true, cancelable: true }) as DragEvent
    Object.defineProperty(e, 'dataTransfer', { value: { getData: (t: string) => (t === 'text/plain' ? ' <:pog:1>' : '<img src=x>') } })
    box.dispatchEvent(e)
    await tick()
    expect(e.defaultPrevented).toBe(true)
    expect(value(box)).toBe('ab <:pog:1>')
  })

  it('sending then leaving the channel at once is fine', async () => {
    const box = show()
    type(box, 'hi')
    press(box, 'Enter')
    unmount(cmp!)
    cmp = undefined
    await tick()
    expect(box.isConnected).toBe(false)
  })

  it('typing right after an undo is a step of its own (and drops the redo)', async () => {
    const box = show()
    type(box, 'gg :po')
    press(box, 'Enter')
    await tick()
    press(box, 'z', { ctrlKey: true })
    await tick()
    expect(value(box)).toBe('gg :po')
    type(box, 'gg :pox') // within a second of the undo
    press(box, 'y', { ctrlKey: true }) // nothing to redo now
    await tick()
    expect(value(box)).toBe('gg :pox')
    press(box, 'z', { ctrlKey: true })
    await tick()
    expect(value(box)).toBe('gg :po')
  })

  it('a drop goes where it was dropped, and a drag inside the box moves the text', async () => {
    const box = show()
    type(box, 'abc def')
    const at = (n: number) => {
      const p = positionAt(box, n)
      return { offsetNode: p.node, offset: p.offset }
    }
    const pointAt = vi.fn()
    Object.assign(document, { caretPositionFromPoint: pointAt })
    const drop = (text: string) => {
      const e = new Event('drop', { bubbles: true, cancelable: true }) as DragEvent
      Object.defineProperty(e, 'dataTransfer', { value: { getData: (t: string) => (t === 'text/plain' ? text : '') } })
      box.dispatchEvent(e)
    }
    try {
      pointAt.mockReturnValue(at(3))
      drop('X')
      await tick()
      expect(value(box)).toBe('abcX def')
      // drag "abc" from the start to the end
      const r = document.createRange()
      const p0 = positionAt(box, 0), p3 = positionAt(box, 3)
      r.setStart(p0.node, p0.offset)
      r.setEnd(p3.node, p3.offset)
      getSelection()!.removeAllRanges()
      getSelection()!.addRange(r)
      const own = new Map<string, string>()
      const start = new Event('dragstart', { bubbles: true }) as DragEvent
      Object.defineProperty(start, 'dataTransfer', { value: { setData: (t: string, v: string) => own.set(t, v) } })
      box.dispatchEvent(start)
      pointAt.mockReturnValue(at(8))
      const e = new Event('drop', { bubbles: true, cancelable: true }) as DragEvent
      Object.defineProperty(e, 'dataTransfer', { value: { getData: (t: string) => own.get(t) ?? '' } })
      box.dispatchEvent(e)
      await tick()
      expect(value(box)).toBe('X defabc')
    } finally {
      delete (document as { caretPositionFromPoint?: unknown }).caretPositionFromPoint
    }
  })

  it('a press that does not end in focus does not count as a click later', async () => {
    const box = show()
    type(box, 'hello')
    caretAt(box, 4)
    ;(document.querySelector('[aria-label="Emoji"]') as HTMLButtonElement).focus()
    box.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    box.dispatchEvent(new PointerEvent('pointerup', { bubbles: true })) // dragged away: no focus
    getSelection()!.removeAllRanges()
    box.focus() // later, by Tab
    await tick()
    expect(caret(box)).toBe(4)
  })

  it('a draft whose emoji arrive after it shows them once they do', async () => {
    const key = `K${Math.random()}`
    let box = show(key)
    type(box, 'hi :po')
    press(box, 'Enter')
    await tick()
    unmount(cmp!)
    const { '1': pog, ...rest } = app.state.customEmoji
    app.state = { ...app.state, customEmoji: rest }
    cmp = mount(Composer, { target: document.body, props: { placeholder: 'Message', draftKey: key, replyingTo: null, onCancelReply: () => {}, onSend, onTyping: () => {} } })
    flushSync()
    box = document.querySelector<HTMLElement>('[role="textbox"]')!
    expect(box.querySelector('img')).toBeNull() // not known yet: its text
    app.state = { ...app.state, customEmoji: { ...rest, '1': pog } }
    flushSync()
    expect(box.querySelector('img')).not.toBeNull()
    expect(value(box)).toBe('hi <:pog:1> ')
  })

  it('dragging emoji within the box moves them as emoji, and a drag carries the tokens', async () => {
    const box = show()
    type(box, 'ab :po')
    press(box, 'Enter')
    await tick()
    expect(value(box)).toBe('ab <:pog:1> ')
    const pointAt = vi.fn()
    Object.assign(document, { caretPositionFromPoint: pointAt })
    try {
      const r = document.createRange()
      const p0 = positionAt(box, 3), p1 = positionAt(box, 12)
      r.setStart(p0.node, p0.offset)
      r.setEnd(p1.node, p1.offset)
      getSelection()!.removeAllRanges()
      getSelection()!.addRange(r)
      const data = new Map<string, string>()
      const dt = { setData: (t: string, v: string) => data.set(t, v), getData: (t: string) => (t === 'text/plain' ? ':pog: ' : data.get(t) ?? '') }
      const start = new Event('dragstart', { bubbles: true }) as DragEvent
      Object.defineProperty(start, 'dataTransfer', { value: dt })
      box.dispatchEvent(start)
      expect(data.get('text/plain')).toBe('<:pog:1> ')
      const p = positionAt(box, 0)
      pointAt.mockReturnValue({ offsetNode: p.node, offset: p.offset })
      const drop = new Event('drop', { bubbles: true, cancelable: true }) as DragEvent
      Object.defineProperty(drop, 'dataTransfer', { value: dt })
      box.dispatchEvent(drop)
      await tick()
      expect(value(box)).toBe('<:pog:1> ab ')
    } finally {
      delete (document as { caretPositionFromPoint?: unknown }).caretPositionFromPoint
    }
  })

  it('a drag that never ended here is not taken for a move by a later drop from outside', async () => {
    const box = show()
    type(box, 'abc def')
    const r = document.createRange()
    const p0 = positionAt(box, 0), p3 = positionAt(box, 3)
    r.setStart(p0.node, p0.offset)
    r.setEnd(p3.node, p3.offset)
    getSelection()!.removeAllRanges()
    getSelection()!.addRange(r)
    box.dispatchEvent(new Event('dragstart', { bubbles: true })) // its dragend got lost
    caretAt(box, 7)
    const drop = new Event('drop', { bubbles: true, cancelable: true }) as DragEvent
    Object.defineProperty(drop, 'dataTransfer', { value: { getData: (t: string) => (t === 'text/plain' ? 'XYZ' : '') } })
    box.dispatchEvent(drop)
    await tick()
    expect(value(box)).toBe('abc defXYZ') // nothing cut
  })

  it('a touch tap keeps its own caret (focus comes after the finger lifts)', async () => {
    const box = show()
    type(box, 'hello')
    caretAt(box, 4)
    ;(document.querySelector('[aria-label="Emoji"]') as HTMLButtonElement).focus()
    box.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'touch' }))
    box.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, pointerType: 'touch' }))
    caretAt(box, 2) // the tap put it here, as focus arrived
    await tick()
    expect(caret(box)).toBe(2)
  })

  it('an emoji that can never be shown does not make every update rebuild the box', async () => {
    const box = show()
    app.state = { ...app.state, customEmoji: { ...app.state.customEmoji, '7': { ...ce('7', 'S1', 'bad'), hash: 'not-a-hash' } } }
    type(box, 'x <:bad:7> y')
    const node = box.firstChild
    app.state = { ...app.state, customEmoji: { ...app.state.customEmoji } } // the emoji map changes
    flushSync()
    app.state = { ...app.state } // and any other event
    flushSync()
    expect(box.firstChild).toBe(node) // left alone
  })

  it('after an undo the popups follow where the caret went', async () => {
    const box = show()
    type(box, ':pog')
    press(box, 'Escape') // dismissed
    caretAt(box, 0)
    // typed at the start, as its own undo step
    vi.spyOn(Date, 'now').mockReturnValue(Date.now() + 5000)
    try {
      box.replaceChildren(document.createTextNode('xyz :pog'))
      // the browser has moved the selection by the input event; its selectionchange comes later
      const p = positionAt(box, 4)
      getSelection()!.collapse(p.node, p.offset)
      box.dispatchEvent(new InputEvent('input', { bubbles: true }))
      flushSync()
    } finally {
      vi.restoreAllMocks()
    }
    expect(options()).toHaveLength(0) // no query before the caret
    press(box, 'z', { ctrlKey: true })
    await tick()
    expect(value(box)).toBe(':pog')
    expect(options()).toHaveLength(0) // the caret went to 0, not left at 4 over ":pog"
    press(box, 'Enter')
    expect(onSend).toHaveBeenCalledWith(':pog')
  })

  it('a paste mid-text opens the emoji popup where it ends', async () => {
    const box = show()
    type(box, 'a b')
    caretAt(box, 1)
    const e = new Event('paste', { bubbles: true, cancelable: true }) as ClipboardEvent
    Object.defineProperty(e, 'clipboardData', { value: { getData: (t: string) => (t === 'text/plain' ? ' :po' : '') } })
    box.dispatchEvent(e)
    flushSync()
    expect(value(box)).toBe('a :po b')
    expect(options()[0]).toContain(':pog:')
    press(box, 'Enter')
    await tick()
    expect(value(box)).toBe('a <:pog:1>  b')
  })

  it('a drag from another box is not taken for a move here', async () => {
    const box = show()
    type(box, 'abc def')
    const r = document.createRange()
    const p0 = positionAt(box, 0), p3 = positionAt(box, 3)
    r.setStart(p0.node, p0.offset)
    r.setEnd(p3.node, p3.offset)
    getSelection()!.removeAllRanges()
    getSelection()!.addRange(r)
    const own = new Map<string, string>()
    const start = new Event('dragstart', { bubbles: true }) as DragEvent
    Object.defineProperty(start, 'dataTransfer', { value: { setData: (t: string, v: string) => own.set(t, v) } })
    box.dispatchEvent(start) // its dragend got lost
    caretAt(box, 7)
    // another box's drag: same marker type, its own id
    const other = new Map([['text/plain', 'XYZ'], ...[...own.keys()].filter((k) => k !== 'text/plain').map((k) => [k, 'other'] as [string, string])])
    const drop = new Event('drop', { bubbles: true, cancelable: true }) as DragEvent
    Object.defineProperty(drop, 'dataTransfer', { value: { getData: (t: string) => other.get(t) ?? '' } })
    box.dispatchEvent(drop)
    await tick()
    expect(value(box)).toBe('abc defXYZ') // nothing cut
  })
})

describe('Composer mentions', () => {
  it('autocomplete rows show styled names', () => {
    const sam = { id: 'U2', username: 'sam', avatar_hash: null, avatar_still: null, style: { font: 'fredoka', colors: ['#ffc4a3'] } }
    app.state = { ...emptyState(), members: { S1: [{ user: sam, nickname: null }] } }
    cmp = mount(Composer, {
      target: document.body,
      props: { placeholder: 'Message', draftKey: `K${Math.random()}`, replyingTo: null, onCancelReply: () => {}, onSend, onTyping: () => {}, mentionables: [{ id: 'U2', name: 'sam' }], serverId: 'S1' },
    })
    flushSync()
    const box = document.querySelector<HTMLElement>('[role="textbox"]')!
    box.focus()
    type(box, '@s')
    const opt = document.querySelector<HTMLElement>('[role=listbox][aria-label="Mention someone"] [role=option]')!
    expect(opt.textContent).toBe('@sam')
    const name = opt.querySelector<HTMLElement>('.name')!
    expect(name.tagName).toBe('SPAN')
    expect(name.getAttribute('style')).toContain('Fredoka')
    expect(name.getAttribute('style')).toContain('rgb(255, 196, 163)')
  })
})
