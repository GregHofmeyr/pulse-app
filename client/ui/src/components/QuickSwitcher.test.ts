// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import QuickSwitcher from './QuickSwitcher.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import type { ActionContext } from '../lib/quickActions'

const user = (id: string, username: string) => ({ id, username, display_name: username, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const ch = (id: string, kind: string, name: string | null, server_id: string | null = null) => ({ id, kind, name, server_id, position: 0 })

const context = (o: Partial<ActionContext> = {}): ActionContext => ({
  call: null, muted: false, deafened: false,
  voiceChannels: [{ id: 'v1', name: 'Lounge', server: 'Game Night' }],
  sounds: [], goLiveOff: null,
  join: vi.fn(), play: vi.fn(), toggleMute: vi.fn(), toggleDeafen: vi.fn(), leave: vi.fn(), goLive: vi.fn(), openSettings: vi.fn(),
  ...o,
})

let cmp: ReturnType<typeof mount> | undefined
let onGo: ReturnType<typeof vi.fn<(p: unknown, join: boolean) => void>>
let onClose: ReturnType<typeof vi.fn<() => void>>
let ctx: ActionContext
beforeEach(() => {
  app.state = {
    ...emptyState(),
    me: user('U1', 'alex') as never,
    servers: [{ id: 's1', name: 'Game Night', icon_hash: null }],
    channels: { c1: ch('c1', 'text', 'general', 's1'), v1: ch('v1', 'voice', 'Lounge', 's1'), c2: ch('c2', 'text', 'setup', 's1'), d1: ch('d1', 'dm', null) } as never,
    dmMembers: { d1: ['U1', 'U2'] },
    people: { U2: { user: user('U2', 'sam'), online: true } } as never,
  }
  onGo = vi.fn<(p: unknown, join: boolean) => void>()
  onClose = vi.fn<() => void>(() => {
    if (cmp) unmount(cmp)
    cmp = undefined
  })
  ctx = context()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

const open = (recents: string[] = [], here: string | null = null) => {
  document.body.innerHTML = '<button id="before">before</button>'
  document.getElementById('before')!.focus()
  cmp = mount(QuickSwitcher, { target: document.body, props: { context: () => ctx, recents, here, onGo, onClose } })
  flushSync()
}
const field = () => document.querySelector<HTMLInputElement>('input[role="combobox"]')!
const type = (text: string) => {
  field().value = text
  field().dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}
const press = (key: string, o: KeyboardEventInit = {}) => {
  const e = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...o })
  field().dispatchEvent(e)
  flushSync()
  return e
}
const settle = () => new Promise((r) => setTimeout(r))
const options = () => [...document.querySelectorAll<HTMLElement>('[role="option"]')]
const names = () => options().map((o) => o.querySelector('.name')!.textContent)
const activeOption = () => document.getElementById(field().getAttribute('aria-activedescendant') ?? '')
const status = () => document.querySelector('[role="status"]')!.textContent

describe('QuickSwitcher', () => {
  it('opens as one floating bar with its field focused: a combobox driving a listbox, no dialog title', () => {
    open()
    const dialog = document.querySelector('[role="dialog"]')!
    expect(dialog.getAttribute('aria-label')).toBe('Quick switcher')
    expect(dialog.querySelector('h2')).toBeNull()
    expect(document.activeElement).toBe(field())
    expect(field().getAttribute('aria-controls')).toBe('qs-list')
    expect(document.getElementById('qs-list')!.getAttribute('role')).toBe('listbox')
  })
  it('nothing typed: the recents (not where you are), under a heading; none yet says how to start', () => {
    open(['c:c1', 'c:v1', 's:s1'], 'c:c1')
    expect(names()).toEqual(['Lounge', 'Game Night'])
    expect(document.querySelector('.head')!.textContent).toBe('Recent')
    unmount(cmp!)
    cmp = undefined
    open()
    expect(options()).toHaveLength(0)
    expect(document.querySelector('.empty')!.textContent).toContain('Start with > for actions')
  })
  it('typing matches places (with their server) and announces the count', () => {
    open()
    type('gene')
    expect(names()).toEqual(['general'])
    expect(options()[0].querySelector('.ctx')!.textContent).toBe('Game Night')
    expect(status()).toBe('1 result')
    type('sam')
    expect(names()).toEqual(['sam'])
    type('zzz')
    expect(options()).toHaveLength(0)
    expect(status()).toBe('No results')
  })
  it('arrows and Tab / Shift+Tab move the highlight, wrapping; the focus stays in the field', () => {
    open(['c:c1', 'c:v1', 's:s1'])
    expect(activeOption()).toBe(options()[0])
    expect(options()[0].getAttribute('aria-selected')).toBe('true')
    press('ArrowDown')
    expect(activeOption()).toBe(options()[1])
    const tab = press('Tab')
    expect(tab.defaultPrevented).toBe(true)
    expect(activeOption()).toBe(options()[2])
    press('ArrowDown')
    expect(activeOption()).toBe(options()[0])
    press('Tab', { shiftKey: true })
    expect(activeOption()).toBe(options()[2])
    press('ArrowUp')
    expect(activeOption()).toBe(options()[1])
    expect(document.activeElement).toBe(field())
  })
  it('the pointer highlights the row under it', () => {
    open(['c:c1', 'c:v1'])
    options()[1].dispatchEvent(new PointerEvent('pointermove', { bubbles: true }))
    flushSync()
    expect(activeOption()).toBe(options()[1])
  })
  it('Enter goes to the highlighted place, closing first; a click does the same', async () => {
    open()
    type('gene')
    press('Enter')
    expect(onClose).toHaveBeenCalled()
    await settle()
    expect(onGo).toHaveBeenCalledWith(expect.objectContaining({ key: 'c:c1' }), false)
    open()
    type('game')
    options()[0].click()
    await settle()
    expect(onGo).toHaveBeenLastCalledWith(expect.objectContaining({ key: 's:s1' }), false)
  })
  it('a voice channel opens its stage; Shift+Enter or its Join button joins too', async () => {
    open()
    type('lounge')
    expect(names()[0]).toBe('Lounge')
    const join = options()[0].querySelector<HTMLButtonElement>('button.join')!
    expect(join.textContent).toBe('Join')
    expect(join.tabIndex).toBe(-1)
    press('Enter', { shiftKey: true })
    await settle()
    expect(onGo).toHaveBeenCalledWith(expect.objectContaining({ key: 'c:v1' }), true)
    open()
    type('lounge')
    options()[0].querySelector<HTMLButtonElement>('button.join')!.click()
    await settle()
    expect(onGo).toHaveBeenLastCalledWith(expect.objectContaining({ key: 'c:v1' }), true)
    expect(onGo).toHaveBeenCalledTimes(2)
  })
  it('matching actions list under the places; `>` lists actions only, and Enter runs one', async () => {
    open()
    type('set')
    expect(names()).toEqual(['setup', 'Open Settings'])
    expect([...document.querySelectorAll('.head')].map((h) => h.textContent)).toEqual(['Jump to', 'Actions'])
    // a voice channel's row has its own Join: no Join action repeats it
    type('lounge')
    expect(names()).toEqual(['Lounge'])
    type('>keyb')
    expect(names()).toEqual(['Settings: Keybinds'])
    press('Enter')
    await settle()
    expect(ctx.openSettings).toHaveBeenCalledWith('keybinds')
    expect(onGo).not.toHaveBeenCalled()
  })
  it('Escape closes, and the focus goes back where it was', () => {
    open()
    press('Escape')
    expect(onClose).toHaveBeenCalled()
    expect(document.activeElement).toBe(document.getElementById('before'))
  })
})
