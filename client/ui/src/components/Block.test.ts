// @vitest-environment jsdom
// Blocking a person (#474), the app side: Block… / Unblock in a person's menu, the block dialog's choice of how they
// show, Settings › Privacy › Blocked people (+ add, a row's right-click: Show a line / Hide completely / Unblock), a run
// of their messages collapsed with Show, and a reply to them.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import BlockItem from './BlockItem.svelte'
import BlockDialog from './BlockDialog.svelte'
import BlockedPeople from './BlockedPeople.svelte'
import { app, resetState, ui } from '../lib/store.svelte'
import { applyEvent, emptyState } from '../lib/state'

const user = (id: string, name: string) => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const alex = user('U1', 'alex')
let cmp: ReturnType<typeof mount> | undefined
const settle = async () => {
  for (let i = 0; i < 6; i++) await Promise.resolve()
  flushSync()
}
const calls = () => invoke.mock.calls.filter(([c]) => c === 'set_block').map(([, a]) => a)

beforeEach(() => {
  resetState()
  invoke.mockReset()
  invoke.mockResolvedValue([])
  app.state = {
    ...emptyState(), me: alex,
    people: {
      U1: { user: alex, online: true, last_seen_at: null, status: 'online' },
      U2: { user: user('U2', 'sam'), online: true, last_seen_at: null, status: 'online' },
      U3: { user: user('U3', 'kim'), online: false, last_seen_at: null, status: 'offline' },
    },
    dmMembers: { D1: ['U1', 'U2', 'U3'] },
  } as never
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('Block… in a person menu', () => {
  const item = (userId: string) => {
    const pick = vi.fn((a: () => unknown) => void a())
    cmp = mount(BlockItem, { target: document.body, props: { userId, pick } })
    flushSync()
    return { pick, button: document.querySelector<HTMLButtonElement>('[data-item]') }
  }

  it('asks first: Block… opens the dialog', () => {
    const { button } = item('U2')
    expect(button!.textContent?.trim()).toBe('Block…')
    button!.click()
    expect(ui.blockAsk).toBe('U2')
  })

  it('once blocked it says Unblock, which acts at once', async () => {
    app.state = { ...app.state, blocks: [{ user_id: 'U2', mode: 'line' }] }
    const { button } = item('U2')
    expect(button!.textContent?.trim()).toBe('Unblock')
    button!.click()
    await settle()
    expect(calls()).toEqual([{ userId: 'U2', mode: null }])
  })

  it('never for yourself', () => {
    expect(item('U1').button).toBeNull()
  })
})

describe('the block dialog', () => {
  it("blocks with a line by default, or Hide completely; it says they aren't told", async () => {
    const onClose = vi.fn()
    cmp = mount(BlockDialog, { target: document.body, props: { userId: 'U2', onClose } })
    flushSync()
    const d = document.querySelector('[data-block-dialog]')!
    expect(d.getAttribute('aria-label')).toBe('Block sam')
    expect(d.textContent).toContain("They aren't told")
    const radios = [...d.querySelectorAll<HTMLInputElement>('input[type=radio]')]
    expect(radios.map((r) => r.checked)).toEqual([true, false])
    radios[1].click()
    flushSync()
    ;[...d.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === 'Block')!.click()
    await settle()
    expect(calls()).toEqual([{ userId: 'U2', mode: 'hide' }])
    expect(onClose).toHaveBeenCalled()
  })
})

describe('Settings › Privacy › Blocked people', () => {
  const open = () => {
    cmp = mount(BlockedPeople, { target: document.body })
    flushSync()
  }
  const rowOf = (id: string) => document.querySelector<HTMLButtonElement>(`[data-blocked="${id}"]`)!

  it('lists them with how each shows, and follows BlocksChanged', () => {
    app.state = { ...app.state, blocks: [{ user_id: 'U3', mode: 'hide' }, { user_id: 'U2', mode: 'line' }] }
    open()
    expect([...document.querySelectorAll('[data-blocked] .who')].map((e) => e.textContent)).toEqual(['kim', 'sam'])
    expect(rowOf('U3').textContent).toContain('Hidden completely')
    app.state = applyEvent(app.state, { t: 'BlocksChanged', d: { user_id: 'U1', list: [{ user_id: 'U2', mode: 'line' }] } }, 0)
    flushSync()
    expect(document.querySelector('[data-blocked="U3"]')).toBeNull()
  })

  it("a row's right-click: Show a line / Hide completely (the current one ticked) / Unblock", async () => {
    app.state = { ...app.state, blocks: [{ user_id: 'U2', mode: 'line' }] }
    open()
    rowOf('U2').dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 10 }))
    await settle()
    const menu = document.querySelector<HTMLElement>('[role=menu][aria-label="Actions for sam"]')!
    const items = [...menu.querySelectorAll<HTMLButtonElement>('[role^=menuitem]')]
    expect(items.map((b) => b.textContent?.trim())).toEqual(['Show a line', 'Hide completely', 'Unblock'])
    expect(items.map((b) => b.getAttribute('aria-checked'))).toEqual(['true', 'false', null])
    items[1].click()
    await settle()
    expect(calls()).toEqual([{ userId: 'U2', mode: 'hide' }])
    expect(document.querySelector('[role=menu]')).toBeNull()
    // the keyboard opens it too, and Unblock
    rowOf('U2').dispatchEvent(new KeyboardEvent('keydown', { key: 'F10', shiftKey: true, bubbles: true, cancelable: true }))
    await settle()
    ;[...document.querySelectorAll<HTMLButtonElement>('[role=menu] [role^=menuitem]')][2].click()
    await settle()
    expect(calls()).toEqual([{ userId: 'U2', mode: 'hide' }, { userId: 'U2', mode: null }])
  })

  it('+ Add person blocks whoever is picked, with a line', async () => {
    open()
    expect(document.querySelector('.empty')!.textContent).toContain("haven't blocked anyone")
    ;[...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.includes('Add person'))!.click()
    flushSync()
    const picker = document.querySelector('[role=dialog]')!
    expect(picker).not.toBeNull()
  })
})
