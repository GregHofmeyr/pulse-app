// @vitest-environment jsdom
// Quick peek (#619, spec §4.4): resting on a card or row ~600 ms (or Space on it) shows its title and first lines of
// notes with Open and Pin to side; leaving both, or Escape, closes it.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Build } from '../lib/protocol/Build'
import type { Message } from '../lib/protocol/Message'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args), convertFileSrc: (s: string) => s }))

import BuildCard from './BuildCard.svelte'
import BuildRow from './BuildRow.svelte'
import BuildPeek from './BuildPeek.svelte'
import { PEEK_OPEN_DELAY } from '../lib/hoverIntent'
import { forgetPinnedBuild, pinnedBuild } from '../lib/pinnedBuild.svelte'
import { handleReady, resetState, ui } from '../lib/store.svelte'
import { testReady } from '../lib/testReady'

const build = (id: string, b: Partial<Build> = {}): Message => ({
  id, channel_id: 'B1', author_id: 'U2', kind: 'normal', content: '', reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null, embeds: [], clip: null,
  build: {
    title: `Build ${id}`, title_auto: false, game_id: 'G1', game_guessed: false, link: null, notes: 'one\ntwo\nthree\nfour\nfive\nsix', quick_drop: false,
    updated_at: null, archived_at: null, archived_by: null, reply_count: 0, last_reply_id: null, replies_seen: null, ...b,
  },
})

let cmp: ReturnType<typeof mount> | undefined
const peek = () => document.querySelector<HTMLElement>('.build-peek')
const hover = (sel: string, type: 'mouseenter' | 'mouseleave') => {
  document.querySelector(sel)!.dispatchEvent(new MouseEvent(type))
  flushSync()
}
const wait = (ms: number) => {
  vi.advanceTimersByTime(ms)
  flushSync()
}
const button = (text: string) => [...peek()!.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === text)!

beforeEach(() => {
  vi.useFakeTimers()
  try { localStorage.clear() } catch { /* none */ }
  invoke.mockReset()
  invoke.mockImplementation(async () => undefined)
  forgetPinnedBuild()
  resetState()
  handleReady(testReady([{ id: 'B1', server_id: 'S1', kind: 'builds', name: 'raven-builds', position: 0 }], {
    games: [{ id: 'G1', server_id: 'S1', name: 'The First Descendant', created_by: 'U1', created_at: '2026-10-01T00:00:00Z' }],
  }))
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

const card = (onOpen = vi.fn()) => {
  cmp = mount(BuildCard, { target: document.body, props: { message: build('M001'), serverId: 'S1', nameOf: () => 'sam', onOpen } })
  flushSync()
  return onOpen
}

describe('BuildPeek', () => {
  it('hovering 600 ms opens the peek, less does not', () => {
    card()
    hover('.build-card', 'mouseenter')
    wait(PEEK_OPEN_DELAY - 50)
    expect(peek()).toBeNull()
    hover('.build-card', 'mouseleave')
    wait(1000)
    expect(peek(), 'left before it opened').toBeNull()
    hover('.build-card', 'mouseenter')
    wait(PEEK_OPEN_DELAY)
    expect(PEEK_OPEN_DELAY).toBe(600)
    expect(peek()).not.toBeNull()
    expect(peek()!.querySelector('.caption')?.textContent).toBe('The First Descendant')
    expect(peek()!.querySelector('.title')?.textContent).toBe('Build M001')
    expect(peek()!.querySelector('.notes')?.textContent).toBe('one\ntwo\nthree\nfour')
  })

  it('moving from the card onto the peek keeps it; leaving both closes it', () => {
    card()
    hover('.build-card', 'mouseenter')
    wait(PEEK_OPEN_DELAY)
    hover('.build-card', 'mouseleave')
    hover('.build-peek', 'mouseenter')
    wait(1000)
    expect(peek()).not.toBeNull()
    hover('.build-peek', 'mouseleave')
    wait(1000)
    expect(peek()).toBeNull()
  })

  it('a row peeks too, and Space on a focused card opens it at once', () => {
    cmp = mount(BuildRow, { target: document.body, props: { message: build('M002'), serverId: 'S1', nameOf: () => 'sam', onOpen: () => {} } })
    flushSync()
    hover('.build-row', 'mouseenter')
    wait(PEEK_OPEN_DELAY)
    expect(peek()?.querySelector('.title')?.textContent).toBe('Build M002')
    unmount(cmp)
    cmp = undefined
    document.body.innerHTML = ''
    const onOpen = card()
    const open = document.querySelector<HTMLButtonElement>('.build-card button.open')!
    open.focus()
    const down = new KeyboardEvent('keydown', { key: ' ', bubbles: true, cancelable: true })
    open.dispatchEvent(down)
    flushSync()
    expect(down.defaultPrevented, 'Space peeks instead of pressing the button').toBe(true)
    expect(peek()).not.toBeNull()
    expect(onOpen).not.toHaveBeenCalled()
  })

  it('Open goes to the page', () => {
    const onOpen = card()
    hover('.build-card', 'mouseenter')
    wait(PEEK_OPEN_DELAY)
    button('Open').click()
    flushSync()
    expect(onOpen).toHaveBeenCalledWith(expect.objectContaining({ id: 'M001' }))
    expect(peek()).toBeNull()
  })

  it('Open without a handler opens the build where it lives', () => {
    cmp = mount(BuildPeek, { target: document.body, props: { message: build('M003'), serverId: 'S1', anchor: document.body, onClose: () => {} } })
    flushSync()
    button('Open').click()
    expect(ui.goTo).toBe('B1')
    expect(ui.openBuild).toEqual({ channelId: 'B1', messageId: 'M003' })
  })

  it('Pin to side pins it', () => {
    card()
    hover('.build-card', 'mouseenter')
    wait(PEEK_OPEN_DELAY)
    button('Pin to side').click()
    flushSync()
    expect(pinnedBuild.key).toEqual({ channelId: 'B1', messageId: 'M001' })
    expect(peek()).toBeNull()
  })

  it('Escape closes it', () => {
    card()
    hover('.build-card', 'mouseenter')
    wait(PEEK_OPEN_DELAY)
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(peek()).toBeNull()
  })

  // #636 review: the keyboard way in and out
  it('Space moves the focus into the peek, Escape brings it back to the card; the card says it is expanded', () => {
    card()
    const open = document.querySelector<HTMLButtonElement>('.build-card button.open')!
    expect(open.getAttribute('aria-expanded')).toBe('false')
    open.focus()
    open.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true, cancelable: true }))
    flushSync()
    expect(peek()!.contains(document.activeElement)).toBe(true)
    expect(open.getAttribute('aria-expanded')).toBe('true')
    expect(open.getAttribute('aria-controls')).toBe(peek()!.id)
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(peek()).toBeNull()
    expect(document.activeElement).toBe(open)
    expect(open.getAttribute('aria-expanded')).toBe('false')
  })

  it('a held Space (repeats) never toggles it', () => {
    card()
    const open = document.querySelector<HTMLButtonElement>('.build-card button.open')!
    open.focus()
    open.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true, cancelable: true }))
    flushSync()
    const again = new KeyboardEvent('keydown', { key: ' ', repeat: true, bubbles: true, cancelable: true })
    open.dispatchEvent(again)
    flushSync()
    expect(peek()).not.toBeNull()
    expect(again.defaultPrevented, 'still not a button press').toBe(true)
  })

  it('a scroll or a resize closes it; a scroll inside it does not', () => {
    card()
    hover('.build-card', 'mouseenter')
    wait(PEEK_OPEN_DELAY)
    peek()!.dispatchEvent(new Event('scroll'))
    flushSync()
    expect(peek()).not.toBeNull()
    document.dispatchEvent(new Event('scroll'))
    flushSync()
    expect(peek()).toBeNull()
    hover('.build-card', 'mouseenter')
    wait(PEEK_OPEN_DELAY)
    window.dispatchEvent(new Event('resize'))
    flushSync()
    expect(peek()).toBeNull()
  })
})
