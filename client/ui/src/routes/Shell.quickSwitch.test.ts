// @vitest-environment jsdom
// The quick switcher in the shell (#309): which keys open it, where it takes you, its actions, and the recents.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a), convertFileSrc: (s: string) => s }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ onFocusChanged: async () => () => {}, isFocused: async () => true, minimize() {}, toggleMaximize() {}, close() {}, startDragging() {} }),
}))
vi.mock('../components/GoLiveDialog.svelte', () => ({ default: () => {} }))
vi.mock('../lib/stream/room', () => ({ StreamRoom: { connect: async () => ({}), pick: async () => null }, STREAM_UNAVAILABLE: 'unavailable' }))

import Shell from './Shell.svelte'
import { app } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { testMe, testReady } from '../lib/testReady'
import { RECENTS_KEY } from '../lib/quickSwitch'
import { nextTask } from '../lib/testSettle'

let cmp: ReturnType<typeof mount> | undefined
beforeEach(() => {
  localStorage.clear()
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string) => (cmd === 'list_messages' ? [] : undefined))
  Element.prototype.scrollTo ??= () => {}
  app.state = applyReady(emptyState(), testReady([
    { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
    { id: 'T2', server_id: 'S1', kind: 'text', name: 'memes', position: 1 },
    { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 },
  ]))
  cmp = mount(Shell, { target: document.body, props: { user: testMe, onLogout: () => {} } })
  flushSync()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

const switcher = () => document.querySelector('[role="dialog"][aria-label="Quick switcher"]')
const field = () => switcher()!.querySelector<HTMLInputElement>('input[role="combobox"]')!
const keydown = (target: EventTarget, key: string, o: KeyboardEventInit = {}) => {
  const e = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...o })
  target.dispatchEvent(e)
  flushSync()
  return e
}
const type = (text: string) => {
  field().value = text
  field().dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}
/** Types, then runs the search debounce on fake time (instead of sleeping through it), back on real timers after. */
const typeThenSearch = async (text: string) => {
  vi.useFakeTimers()
  try {
    type(text)
    await vi.advanceTimersByTimeAsync(260)
  } finally {
    vi.useRealTimers()
  }
}
const tab = (name: string) => [...document.querySelectorAll<HTMLButtonElement>('nav .tab')].find((b) => b.textContent?.includes(name))!

describe('the quick switcher in the shell', () => {
  it('Ctrl+K opens it and Ctrl+K again closes it; the key is taken (not the webview\'s)', () => {
    const e = keydown(document.body, 'k', { ctrlKey: true })
    expect(e.defaultPrevented).toBe(true)
    expect(switcher()).not.toBeNull()
    expect(document.activeElement).toBe(field())
    keydown(field(), 'k', { ctrlKey: true })
    expect(switcher()).toBeNull()
  })
  it('`/` opens it with nothing focused, and is not typed into it', () => {
    const e = keydown(document.body, '/')
    expect(e.defaultPrevented).toBe(true)
    expect(switcher()).not.toBeNull()
    expect(field().value).toBe('')
  })
  it('`/` in the message box stays the slash commands\' (#262); Ctrl+K still opens from there', async () => {
    tab('Main').click()
    flushSync()
    await nextTask()
    const box = document.querySelector<HTMLElement>('[contenteditable][role="textbox"]')!
    expect(box).not.toBeNull()
    box.focus()
    const e = keydown(box, '/')
    expect(e.defaultPrevented).toBe(false)
    expect(switcher()).toBeNull()
    keydown(box, 'k', { ctrlKey: true })
    expect(switcher()).not.toBeNull()
    // Esc: closed, and the focus is back in the box
    keydown(field(), 'Escape')
    expect(switcher()).toBeNull()
    expect(document.activeElement).toBe(box)
  })
  it('not over another dialog', () => {
    document.querySelector<HTMLButtonElement>('button[aria-label="Settings"]')!.click()
    flushSync()
    expect(document.querySelector('[aria-modal="true"]')).not.toBeNull()
    keydown(document.body, 'k', { ctrlKey: true })
    expect(switcher()).toBeNull()
  })
  it('Enter on a channel goes there (its server too), and the visit becomes a recent', async () => {
    keydown(document.body, 'k', { ctrlKey: true })
    type('mem')
    keydown(field(), 'Enter')
    await nextTask()
    flushSync()
    expect(switcher()).toBeNull()
    expect(tab('Main').classList.contains('active')).toBe(true)
    expect(document.querySelector('.panel-head')!.textContent).toContain('Main')
    expect(JSON.parse(localStorage.getItem(RECENTS_KEY)!)[0]).toBe('c:T2')
    // next time, with nothing typed: the recents, without where you are
    keydown(document.body, 'k', { ctrlKey: true })
    expect([...switcher()!.querySelectorAll('[role="option"] .name')].map((n) => n.textContent)).not.toContain('memes')
  })
  it('a voice channel opens its stage; Shift+Enter joins it', async () => {
    keydown(document.body, 'k', { ctrlKey: true })
    type('lounge')
    keydown(field(), 'Enter', { shiftKey: true })
    await nextTask()
    flushSync()
    expect(invoke).toHaveBeenCalledWith('join_voice', expect.objectContaining({ channelId: 'V1' }))
  })
  it('> actions: Settings pages open Settings at that page', async () => {
    keydown(document.body, 'k', { ctrlKey: true })
    type('>keybinds')
    keydown(field(), 'Enter')
    await nextTask()
    flushSync()
    const settings = document.querySelector('[aria-modal="true"]')
    expect(settings).not.toBeNull()
    expect(settings!.querySelector('[aria-current="page"], .active, [aria-selected="true"]')?.textContent).toContain('Keybinds')
  })
  describe('messages everywhere (#331)', () => {
    const searches = () => invoke.mock.calls.filter((c) => c[0] === 'search_messages').map((c) => c[1])
    const hit = (id: string, channel: string, text: string) => ({ message: { id, channel_id: channel, author_id: testMe.id }, text, highlights: [], root: null })
    const heads = () => [...switcher()!.querySelectorAll('.head')].map((h) => h.textContent)

    it('from Home: one search, everywhere, leaving nothing out', async () => {
      invoke.mockImplementation(async (cmd: string) =>
        cmd === 'search_messages' ? { hits: [hit('m1', 'T1', 'pizza tonight')], next_cursor: null, total: 1 } : cmd === 'list_messages' ? [] : undefined)
      keydown(document.body, 'k', { ctrlKey: true })
      await typeThenSearch('pizza')
      flushSync()
      expect(searches()).toEqual([{ serverId: null, channelId: null, everywhere: true, query: expect.objectContaining({ q: 'pizza' }) }])
      expect(heads()).toEqual(['Messages everywhere'])
      expect(switcher()!.querySelector('.ctx')!.textContent).toContain('Main · #general')
    })
    it('in a server you are in: that server first, then everywhere but it', async () => {
      tab('Main').click()
      flushSync()
      await nextTask()
      invoke.mockImplementation(async (cmd: string, a: { everywhere?: boolean }) =>
        cmd === 'search_messages'
          ? { hits: a.everywhere ? [] : [hit('m1', 'T1', 'pizza tonight')], next_cursor: null, total: 1 }
          : cmd === 'list_messages' ? [] : undefined)
      keydown(document.body, 'k', { ctrlKey: true })
      await typeThenSearch('pizza')
      flushSync()
      expect(searches()).toEqual([
        { serverId: 'S1', channelId: null, query: expect.objectContaining({ q: 'pizza' }) },
        { serverId: 'S1', channelId: null, everywhere: true, query: expect.objectContaining({ q: 'pizza' }) },
      ])
      expect(heads()).toEqual(['Messages in Main'])
    })
  })
})

// #619: a build found from Ctrl+K opens its page in its channel
describe('builds from the switcher (#619)', () => {
  it('builds: searches builds, and Enter opens the build page', async () => {
    const { ui } = await import('../lib/store.svelte')
    app.state = applyReady(emptyState(), testReady([
      { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
      { id: 'B1', server_id: 'S1', kind: 'builds', name: 'raven-builds', position: 1 },
    ]))
    const found = {
      id: 'M001', channel_id: 'B1', author_id: 'U2', kind: 'normal', content: '', reply_to_id: null, created_at: '2026-10-01T10:00:00.000Z', edited_at: null,
      deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, embeds: [], clip: null,
      build: { title: 'Raven bunny', title_auto: false, game_id: null, game_guessed: false, link: null, notes: null, quick_drop: false, updated_at: null, archived_at: null, archived_by: null, reply_count: 0, last_reply_id: null, replies_seen: null },
    }
    invoke.mockImplementation(async (cmd: string) => (cmd === 'search_builds' ? [found] : cmd === 'list_messages' || cmd === 'list_builds' ? [] : undefined))
    keydown(document.body, 'k', { ctrlKey: true })
    await typeThenSearch('builds: raven')
    flushSync()
    expect(invoke).toHaveBeenCalledWith('search_builds', { q: 'raven', game: null })
    keydown(field(), 'Enter')
    await nextTask()
    flushSync()
    expect(switcher()).toBeNull()
    // its channel shows, and asks for the build to open its page
    expect(document.querySelector('[data-testid="builds-channel"]')).not.toBeNull()
    expect(invoke).toHaveBeenCalledWith('get_build', { messageId: 'M001' })
    expect(ui.openBuild).toBeNull()
  })
})
