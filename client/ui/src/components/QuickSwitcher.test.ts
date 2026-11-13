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
import { MESSAGE_DELAY_MS } from '../lib/quickMessages'
import type { SearchHit } from '../lib/protocol/SearchHit'

type Messages = { label: string; serverId: string | null; search: (q: string) => Promise<SearchHit[]>; open: (hit: SearchHit) => void }
const hit = (id: string, text: string, author = 'U2', channel = 'c1') =>
  ({ message: { id, channel_id: channel, author_id: author }, text, highlights: [{ start: 0, end: 5 }], root: null }) as unknown as SearchHit

const user = (id: string, username: string) => ({ id, username, display_name: username, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const ch = (id: string, kind: string, name: string | null, server_id: string | null = null) => ({ id, kind, name, server_id, position: 0 })

const context = (o: Partial<ActionContext> = {}): ActionContext => ({
  call: null, muted: false, deafened: false,
  voiceChannels: [{ id: 'v1', name: 'Lounge', server: 'Game Night' }],
  sounds: [], goLiveOff: null,
  join: vi.fn(), play: vi.fn(), toggleMute: vi.fn(), toggleDeafen: vi.fn(), leave: vi.fn(), goLive: vi.fn(), openSettings: vi.fn(), openInbox: vi.fn(),
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

type Everywhere = { search: (q: string) => Promise<SearchHit[]>; open: (hit: SearchHit) => void }
const open = (recents: string[] = [], here: string | null = null, messages: Messages | null = null, everywhere: Everywhere | null = null) => {
  document.body.innerHTML = '<button id="before">before</button>'
  document.getElementById('before')!.focus()
  cmp = mount(QuickSwitcher, { target: document.body, props: { context: () => ctx, recents, here, messages, everywhere, onGo, onClose } })
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
  it("while a voice row is listed, every place row keeps the Join button's room, so the server names line up", () => {
    open(['c:c2', 'c:v1'])
    expect(names()).toEqual(['setup', 'Lounge'])
    const [text, voice] = options()
    expect(text.querySelector('.end.cols .join-slot')).toBeTruthy()
    expect(voice.querySelector('.end.cols button.join')).toBeTruthy()
    unmount(cmp!)
    cmp = undefined
    open(['c:c2', 'c:c1'])
    expect(document.querySelector('.join-slot')).toBeNull() // no voice row: no empty room on the right
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
  describe('Messages (#320)', () => {
    const wait = (ms: number) => new Promise((r) => setTimeout(r, ms))
    const heads = () => [...document.querySelectorAll('.head')].map((h) => h.textContent)
    const note = () => document.querySelector('.note')?.textContent ?? null
    const messages = (search: Messages['search']): Messages => ({ label: 'Messages in Game Night', serverId: 's1', search: vi.fn(search), open: vi.fn() })

    it('arrive under the instant sections once typing pauses, without moving them; Enter jumps to one', async () => {
      const m = messages(async () => [hit('m1', 'setup is done\nsecond line'), hit('m2', 'setup again')])
      open([], null, m)
      type('set')
      // the instant results are there at once, the messages on their way below them
      expect(names()).toEqual(['setup', 'Open Settings'])
      expect(note()).toBe('Searching messages…')
      expect(m.search).not.toHaveBeenCalled()
      await wait(MESSAGE_DELAY_MS + 30)
      flushSync()
      expect(m.search).toHaveBeenCalledTimes(1)
      expect(m.search).toHaveBeenCalledWith('set')
      expect(heads()).toEqual(['Jump to', 'Actions', 'Messages in Game Night'])
      // the line with the match, the words marked; who and where faintly after it
      expect(names().slice(0, 3)).toEqual(['setup', 'Open Settings', 'setup is done'])
      expect(options()[2].querySelector('mark')!.textContent).toBe('setup')
      expect(options()[2].querySelector('.ctx')!.textContent).toBe('sam · #general')
      expect(note()).toBeNull()
      expect(activeOption()).toBe(options()[0])
      press('ArrowDown')
      press('ArrowDown')
      expect(activeOption()).toBe(options()[2])
      press('Enter')
      await settle()
      expect(onClose).toHaveBeenCalled()
      expect(m.open).toHaveBeenCalledWith(expect.objectContaining({ message: expect.objectContaining({ id: 'm1' }) }))
      expect(onGo).not.toHaveBeenCalled()
    })

    it('typing on drops the older answer; one character and `>` never search', async () => {
      const m = messages(async (q) => [hit(`m-${q}`, `${q} found`)])
      open([], null, m)
      type('s')
      await wait(MESSAGE_DELAY_MS + 30)
      type('>set')
      await wait(MESSAGE_DELAY_MS + 30)
      expect(m.search).not.toHaveBeenCalled()
      expect(note()).toBeNull()
      type('gen')
      await wait(20)
      type('gene')
      await wait(MESSAGE_DELAY_MS + 30)
      flushSync()
      expect(m.search).toHaveBeenCalledTimes(1)
      expect(m.search).toHaveBeenCalledWith('gene')
      expect(names()).toContain('gene found')
    })

    it('nothing anywhere yet: no "Nothing matches" while messages are searched, a quiet note if they fail', async () => {
      const m = messages(() => Promise.reject(new Error('offline')))
      open([], null, m)
      type('zzqx')
      expect(document.querySelector('.empty')).toBeNull()
      expect(note()).toBe('Searching messages…')
      await wait(MESSAGE_DELAY_MS + 30)
      flushSync()
      expect(note()).toBe("Couldn't search messages")
      expect(options()).toHaveLength(0)
    })

    it('no scope (Home with nothing open): no Messages section at all', async () => {
      open()
      type('set')
      expect(note()).toBeNull()
      await wait(MESSAGE_DELAY_MS + 30)
      expect(heads()).toEqual(['Jump to', 'Actions'])
    })
  })
  describe('Messages everywhere (#331)', () => {
    const wait = (ms: number) => new Promise((r) => setTimeout(r, ms))
    const heads = () => [...document.querySelectorAll('.head')].map((h) => h.textContent)
    const note = () => document.querySelector('.note')?.textContent ?? null
    const ctxOf = (name: string) => options().find((o) => o.querySelector('.name')!.textContent === name)?.querySelector('.ctx')?.textContent ?? null
    const here = (search: Messages['search']): Messages => ({ label: 'Messages in Game Night', serverId: 's1', search: vi.fn(search), open: vi.fn() })
    const everywhere = (search: Everywhere['search']): Everywhere => ({ search: vi.fn(search), open: vi.fn() })
    beforeEach(() => {
      app.state = {
        ...app.state,
        servers: [...app.state.servers, { id: 's2', name: 'Book Club', icon_hash: null }] as never,
        channels: {
          ...app.state.channels,
          c9: ch('c9', 'text', 'lobby', 's2'),
          g1: ch('g1', 'group', 'Trip'),
          n1: ch('n1', 'dm', null),
        } as never,
        dmMembers: { ...app.state.dmMembers, g1: ['U1', 'U2', 'U3'], n1: ['U1'] },
        people: { ...app.state.people, U3: { user: user('U3', 'kim'), online: true } } as never,
      }
    })

    it('follow the first section, leaving out its hits; each says where it is; the keys run through both', async () => {
      const m = here(async () => [hit('m1', 'setup here'), hit('m2', 'setup again')])
      const e = everywhere(async () => [
        hit('m2', 'setup again'),
        hit('e1', 'setup in the club', 'U3', 'c9'),
        hit('e2', 'setup in a dm', 'U2', 'd1'),
        hit('e3', 'setup on the trip', 'U2', 'g1'),
        hit('e4', 'setup note', 'U1', 'n1'),
      ])
      open([], null, m, e)
      type('set')
      expect(note()).toBe('Searching messages…')
      await wait(MESSAGE_DELAY_MS + 30)
      flushSync()
      expect(m.search).toHaveBeenCalledWith('set')
      expect(e.search).toHaveBeenCalledWith('set')
      expect(heads()).toEqual(['Jump to', 'Actions', 'Messages in Game Night', 'Messages everywhere'])
      expect(names()).toEqual(['setup', 'Open Settings', 'setup here', 'setup again', 'setup in the club', 'setup in a dm', 'setup on the trip', 'setup note'])
      // the first section's own look; the second also names the server, or the DM or group (once, when it's who wrote it)
      expect(ctxOf('setup here')).toBe('sam · #general')
      expect(ctxOf('setup in the club')).toBe('kim · Book Club · #lobby')
      expect(ctxOf('setup in a dm')).toBe('sam')
      expect(ctxOf('setup on the trip')).toBe('sam · Trip')
      expect(ctxOf('setup note')).toBe('alex · Notes')
      expect(options()[4].querySelector('mark')!.textContent).toBe('setup')
      expect(status()).toBe('8 results')
      expect(note()).toBeNull()
      // the highlight moves from the first section into the second; Enter opens the hit with the second's opener
      for (let i = 0; i < 4; i++) press('ArrowDown')
      expect(activeOption()).toBe(options()[4])
      press('Enter')
      await settle()
      expect(onClose).toHaveBeenCalled()
      expect(e.open).toHaveBeenCalledWith(expect.objectContaining({ message: expect.objectContaining({ id: 'e1' }) }))
      expect(m.open).not.toHaveBeenCalled()
    })

    it('a click on an everywhere hit opens it', async () => {
      const e = everywhere(async () => [hit('e1', 'setup in the club', 'U3', 'c9')])
      open([], null, null, e)
      type('set')
      await wait(MESSAGE_DELAY_MS + 30)
      flushSync()
      options().find((o) => o.textContent!.includes('setup in the club'))!.click()
      await settle()
      expect(e.open).toHaveBeenCalledWith(expect.objectContaining({ message: expect.objectContaining({ id: 'e1' }) }))
    })

    it('nowhere searchable (Home, Notes, a server you only view): just the everywhere section', async () => {
      const e = everywhere(async () => [hit('e1', 'setup in the club', 'U3', 'c9')])
      open([], null, null, e)
      type('set')
      expect(note()).toBe('Searching messages…')
      await wait(MESSAGE_DELAY_MS + 30)
      flushSync()
      expect(heads()).toEqual(['Jump to', 'Actions', 'Messages everywhere'])
    })

    it("waits for the first section, so its rows never push the second's down under the pointer", async () => {
      let answer: (h: SearchHit[]) => void = () => {}
      const m = here(() => new Promise((r) => (answer = r)))
      const e = everywhere(async () => [hit('e1', 'setup in the club', 'U3', 'c9')])
      open([], null, m, e)
      type('set')
      await wait(MESSAGE_DELAY_MS + 30)
      flushSync()
      expect(heads()).toEqual(['Jump to', 'Actions'])
      expect(note()).toBe('Searching messages…')
      answer([hit('m1', 'setup here')])
      await settle()
      flushSync()
      expect(heads()).toEqual(['Jump to', 'Actions', 'Messages in Game Night', 'Messages everywhere'])
      expect(note()).toBeNull()
    })

    it("either failing says so quietly; the other's hits still show", async () => {
      const m = here(() => Promise.reject(new Error('offline')))
      const e = everywhere(async () => [hit('e1', 'setup in the club', 'U3', 'c9')])
      open([], null, m, e)
      type('zzqx')
      await wait(MESSAGE_DELAY_MS + 30)
      flushSync()
      expect(heads()).toEqual(['Messages everywhere'])
      expect(note()).toBe("Couldn't search messages")
      const m2 = here(async () => [hit('m1', 'zzqx here')])
      const e2 = everywhere(() => Promise.reject(new Error('offline')))
      if (cmp) unmount(cmp)
      open([], null, m2, e2)
      type('zzqx')
      await wait(MESSAGE_DELAY_MS + 30)
      flushSync()
      expect(heads()).toEqual(['Messages in Game Night'])
      expect(note()).toBe("Couldn't search messages")
    })

    it('nothing found anywhere: "No results", no empty headings', async () => {
      const m = here(async () => [])
      const e = everywhere(async () => [])
      open([], null, m, e)
      type('zzqx')
      await wait(MESSAGE_DELAY_MS + 30)
      flushSync()
      expect(heads()).toEqual([])
      expect(note()).toBeNull()
      expect(document.querySelector('.empty')!.textContent).toContain('Nothing matches')
    })
  })
  it('Escape closes, and the focus goes back where it was', () => {
    open()
    press('Escape')
    expect(onClose).toHaveBeenCalled()
    expect(document.activeElement).toBe(document.getElementById('before'))
  })
})
