// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
import type { Message } from '../lib/protocol/Message'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => [] }))

// Installed before anything is wired: the observer and the watchers are created once.
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
const media = fakeMatchMedia(false)
vi.stubGlobal('matchMedia', media.matchMedia)

import MessageItem from './MessageItem.svelte'
import ReactionRow from './ReactionRow.svelte'
import EmojiPicker from './EmojiPicker.svelte'
import Composer from './Composer.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { motion, setAnimateMode } from '../lib/animate.svelte'
import { windowFocus } from '../lib/windowFocus.svelte'

const A = 'a'.repeat(64) // animated
const S = 'b'.repeat(64) // its still first frame
const P = 'c'.repeat(64) // a still emoji
const url = (h: string) => `pulse-media://localhost/${h}`
const ce = (id: string, name: string, hash: string, still_hash: string | null): CustomEmoji => ({
  id, server_id: 'S1', name, hash, still_hash, animated: still_hash !== null, added_by: null, created_at: '2026-10-01T00:00:00Z',
})
const spin = ce('E1', 'spin', A, S)
const pog = ce('E2', 'pog', P, null)
const msg = (over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U2', kind: 'normal', content: 'look <a:spin:E1> <:pog:E2>',
  reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], ...over,
})

function load(m: Message) {
  const me = { id: 'U1', username: 'alex', avatar_hash: null }
  app.state = {
    ...emptyState(), me,
    servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
    members: { S1: [{ user: me, nickname: null }] },
    customEmoji: { E1: spin, E2: pog },
    messages: { C1: [m] },
  }
}
let cmp: ReturnType<typeof mount> | undefined
function showMessage(m = msg()) {
  load(m)
  cmp = mount(MessageItem, { target: document.body, props: {
    message: app.state.messages['C1'][0], name: 'sam', nameOf: (id: string | null) => id ?? '', replyTo: null, grouped: false,
    mine: false, onReply: () => {}, onEdit: async () => {}, onDelete: async () => {}, meId: 'U1',
  } })
  flushSync()
}
const see = (el: Element, v = true) => {
  FakeIntersectionObserver.set(el, v)
  flushSync()
}
const fire = (el: EventTarget, type: string) => {
  el.dispatchEvent(new Event(type))
  flushSync()
}
const src = (el: Element | null) => el!.getAttribute('src')

beforeEach(() => {
  setAnimateMode('always')
  motion.streaming = false
  window.dispatchEvent(new Event('focus'))
  media.set(false)
  flushSync()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('animated emoji in a message', () => {
  const imgs = () => [...document.querySelectorAll<HTMLImageElement>('.content img.emoji')]

  it('plays only while on-screen and the window is focused', () => {
    showMessage()
    const [anim, still] = imgs()
    expect(src(anim)).toBe(url(S)) // not yet reported on-screen
    see(anim)
    expect(src(anim)).toBe(url(A))
    expect(src(still)).toBe(url(P)) // still emoji untouched
    fire(window, 'blur')
    expect(windowFocus.focused).toBe(false)
    expect(src(anim)).toBe(url(S))
    fire(window, 'focus')
    expect(src(anim)).toBe(url(A))
    see(anim, false)
    expect(src(anim)).toBe(url(S))
  })

  it('going_live_freezes_and_stopping_resumes', () => {
    showMessage()
    const [anim] = imgs()
    see(anim)
    expect(src(anim)).toBe(url(A))
    motion.streaming = true
    flushSync()
    expect(src(anim)).toBe(url(S))
    motion.streaming = false
    flushSync()
    expect(src(anim)).toBe(url(A))
  })

  it('follows the setting and reduced motion', () => {
    showMessage()
    const [anim] = imgs()
    see(anim)
    setAnimateMode('never')
    flushSync()
    expect(src(anim)).toBe(url(S))
    setAnimateMode('hover')
    flushSync()
    expect(src(anim)).toBe(url(S))
    fire(anim, 'pointerenter')
    expect(src(anim)).toBe(url(A))
    fire(anim, 'pointerleave')
    expect(src(anim)).toBe(url(S))
    setAnimateMode('always')
    flushSync()
    expect(src(anim)).toBe(url(A))
    media.set(true)
    flushSync()
    expect(src(anim)).toBe(url(S))
  })

  it('a remount (edit) wires the new images and lets go of the old', () => {
    showMessage()
    see(imgs()[0])
    expect(FakeIntersectionObserver.watching()).toBe(1)
    app.state = { ...app.state, messages: { C1: [msg({ content: '<a:spin:E1> <a:spin:E1>', edited_at: '2026-10-02T00:00:00.000Z' })] } }
    unmount(cmp!)
    cmp = mount(MessageItem, { target: document.body, props: {
      message: app.state.messages['C1'][0], name: 'sam', nameOf: (id: string | null) => id ?? '', replyTo: null, grouped: false,
      mine: false, onReply: () => {}, onEdit: async () => {}, onDelete: async () => {}, meId: 'U1',
    } })
    flushSync()
    expect(FakeIntersectionObserver.watching()).toBe(2)
    for (const i of imgs()) see(i)
    expect(imgs().map(src)).toEqual([url(A), url(A)])
  })

  it('the emoji state changing re-renders and re-wires', () => {
    showMessage(msg({ id: 'M2', content: '<:pog:E2>' })) // a new id: the render cache keys on it
    expect(FakeIntersectionObserver.watching()).toBe(0)
    app.state = { ...app.state, customEmoji: { ...app.state.customEmoji, E2: { ...spin, id: 'E2', name: 'pog' } } }
    flushSync()
    const [img] = imgs()
    expect(src(img)).toBe(url(S))
    see(img)
    expect(src(img)).toBe(url(A))
  })
})

describe('animated emoji in reactions', () => {
  function showRow() {
    load(msg({ reactions: [{ emoji: 'custom:E1', count: 1, me: false }, { emoji: 'custom:E2', count: 1, me: false }] }))
    cmp = mount(ReactionRow, { target: document.body, props: { message: app.state.messages['C1'][0], meId: 'U1', nameOf: (id: string | null) => id ?? '' } })
    flushSync()
    return [...document.querySelectorAll<HTMLImageElement>('.chip img')]
  }

  it('plays under Always without any hover', () => {
    const [anim, still] = showRow()
    see(anim)
    expect(src(anim)).toBe(url(A))
    expect(src(still)).toBe(url(P))
    fire(window, 'blur')
    expect(src(anim)).toBe(url(S))
  })

  it('under On hover, plays while the pointer is over the chip', () => {
    setAnimateMode('hover')
    const [anim] = showRow()
    see(anim)
    expect(src(anim)).toBe(url(S))
    fire(anim.closest('.chip')!, 'pointerenter')
    expect(src(anim)).toBe(url(A))
    fire(anim.closest('.chip')!, 'pointerleave')
    expect(src(anim)).toBe(url(S))
  })

  it('never plays under Never, hover or not', () => {
    setAnimateMode('never')
    const [anim] = showRow()
    see(anim)
    fire(anim.closest('.chip')!, 'pointerenter')
    expect(src(anim)).toBe(url(S))
  })
})

describe('animated emoji in the picker and autocomplete', () => {
  it('picker cells follow the rule, hover counted over the cell', () => {
    load(msg())
    cmp = mount(EmojiPicker, { target: document.body, props: { onPick: () => {}, onClose: () => {}, servers: app.state.servers, memberOf: new Set(['S1']) } })
    flushSync()
    const cell = [...document.querySelectorAll<HTMLButtonElement>('.cell')].find((c) => c.dataset.key === 'custom:E1')!
    const img = cell.querySelector('img')!
    see(img)
    expect(src(img)).toBe(url(A))
    setAnimateMode('hover')
    flushSync()
    expect(src(img)).toBe(url(S))
    fire(cell, 'pointerenter')
    expect(src(img)).toBe(url(A))
    fire(window, 'blur')
    expect(src(img)).toBe(url(S))
  })

  it('autocomplete rows follow the rule', () => {
    load(msg())
    cmp = mount(Composer, { target: document.body, props: { placeholder: 'Message', draftKey: `K${Math.random()}`, replyingTo: null, onCancelReply: () => {}, onSend: () => {}, onTyping: () => {} } })
    flushSync()
    const box = document.querySelector<HTMLElement>('[role="textbox"]')!
    box.replaceChildren(document.createTextNode(':spi'))
    const r = document.createRange()
    r.setStart(box.firstChild!, 4)
    getSelection()!.removeAllRanges()
    getSelection()!.addRange(r)
    box.dispatchEvent(new InputEvent('input', { bubbles: true }))
    flushSync()
    const img = document.querySelector<HTMLImageElement>('[role=option] img')!
    expect(src(img)).toBe(url(S))
    see(img)
    expect(src(img)).toBe(url(A))
    setAnimateMode('hover')
    flushSync()
    expect(src(img)).toBe(url(S))
    fire(img.closest('[role=option]')!, 'pointerenter')
    expect(src(img)).toBe(url(A))
  })
})
