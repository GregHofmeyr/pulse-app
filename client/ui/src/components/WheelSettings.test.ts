// @vitest-environment jsdom
// Settings › Wheels (#158): a server's wheels in Server settings, your Just me wheels in your own Settings; New wheel,
// Edit (rename, options) and Delete, which asks first. The lists follow WheelSaved / WheelDeleted.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'
import type { Wheel } from '../lib/protocol/Wheel'

const games: Wheel = { id: 'W1', scope: 'server', server_id: 'S1', owner_id: null, name: 'Games', options: ['Valheim', 'Apex'], updated_at: '' }
const tank: Wheel = { id: 'W2', scope: 'personal', server_id: null, owner_id: 'U1', name: 'Tank', options: [], updated_at: '' }
const invoke = vi.fn(async (cmd: string, args?: { id?: string; request?: Record<string, unknown> }): Promise<unknown> => {
  if (cmd === 'list_server_wheels') return [games]
  if (cmd === 'list_my_wheels') return [tank]
  if (cmd === 'update_wheel') return { ...[games, tank].find((w) => w.id === args!.id), ...args!.request }
  if (cmd === 'create_wheel') return { id: 'W9', scope: 'server', server_id: 'S1', owner_id: null, name: args!.request!.name, options: [], updated_at: '' }
  return undefined
})
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import WheelSettings from './WheelSettings.svelte'
import ServerSettings from './ServerSettings.svelte'
import Settings from './Settings.svelte'
import { resetState } from '../lib/store.svelte'
import { handleWheelEvent, resetWheels } from '../lib/wheels.svelte'

const server = { id: 'S1', name: 'Main', icon_hash: null }
let cmp: ReturnType<typeof mount> | undefined
async function show(props: { server?: typeof server | null } = { server }) {
  cmp = mount(WheelSettings, { target: document.body, props })
  await vi.waitFor(() => expect(document.querySelectorAll('.wheel-list li').length).toBeGreaterThan(0))
  flushSync()
}
const names = () => [...document.querySelectorAll('.wheel-list .name')].map((n) => n.textContent)
const button = (text: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent!.trim() === text)!

beforeEach(() => {
  resetState()
  resetWheels()
  invoke.mockClear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('WheelSettings', () => {
  it("lists the server's wheels with their option counts and says who may change them", async () => {
    await show()
    expect(invoke).toHaveBeenCalledWith('list_server_wheels', { serverId: 'S1' })
    expect(document.querySelector('h2')!.textContent).toBe('Wheels')
    expect(names()).toEqual(['Games'])
    expect(document.querySelector('.wheel-list small')!.textContent).toBe('2 options')
    expect(document.querySelector('.hint')!.textContent).toContain('Anyone in the server')
  })

  it('Just me: your own wheels', async () => {
    await show({ server: null })
    expect(invoke).toHaveBeenCalledWith('list_my_wheels', undefined)
    expect(names()).toEqual(['Tank'])
    expect(document.querySelector('.hint')!.textContent).toContain('Only you see these')
  })

  it('+ New wheel names one and opens it for options', async () => {
    await show()
    button('New wheel').click()
    flushSync()
    const field = document.querySelector<HTMLInputElement>('input[aria-label="New wheel\'s name"]')!
    field.value = 'Dinner'
    field.dispatchEvent(new Event('input'))
    flushSync()
    field.form!.requestSubmit()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('create_wheel', { request: { server_id: 'S1', name: 'Dinner', options: [] } }))
    await vi.waitFor(() => expect(names()).toEqual(['Dinner', 'Games']))
    expect(document.querySelector('.edit input[aria-label="Add an option"]')).not.toBeNull()
  })

  it('Edit renames and changes options, saving each change', async () => {
    await show()
    document.querySelector<HTMLButtonElement>('button[aria-label="Edit Games"]')!.click()
    flushSync()
    const add = document.querySelector<HTMLInputElement>('.edit input[aria-label="Add an option"]')!
    add.value = 'Among Us, Lethal Company'
    add.dispatchEvent(new Event('input'))
    flushSync()
    add.form!.requestSubmit()
    flushSync()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('update_wheel', { id: 'W1', request: { options: ['Valheim', 'Apex', 'Among Us', 'Lethal Company'] } }))
    const name = document.querySelector<HTMLInputElement>('.edit input[aria-label="Name of Games"]')!
    name.value = 'Games tonight'
    name.dispatchEvent(new Event('input', { bubbles: true }))
    name.dispatchEvent(new FocusEvent('blur'))
    flushSync()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('update_wheel', { id: 'W1', request: { name: 'Games tonight' } }))
  })

  it('Delete asks first', async () => {
    await show()
    document.querySelector<HTMLButtonElement>('button[aria-label="Delete Games"]')!.click()
    flushSync()
    expect(document.querySelector('.q')!.textContent).toBe('Delete Games?')
    expect(invoke).not.toHaveBeenCalledWith('delete_wheel', expect.anything())
    button('Delete').click()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('delete_wheel', { id: 'W1' }))
    await vi.waitFor(() => expect(names()).toEqual([]))
  })

  it("follows other people's changes", async () => {
    await show()
    handleWheelEvent({ t: 'WheelSaved', d: { wheel: { ...games, id: 'W5', name: 'Apex legend' } } })
    flushSync()
    expect(names()).toEqual(['Apex legend', 'Games'])
    handleWheelEvent({ t: 'WheelDeleted', d: { wheel_id: 'W1', server_id: 'S1', owner_id: null } })
    flushSync()
    expect(names()).toEqual(['Apex legend'])
  })
})

describe('where the Wheels pages are', () => {
  it('Server settings has a Wheels section after Soundboard', () => {
    cmp = mount(ServerSettings, { target: document.body, props: { server, onClose: () => {} } })
    flushSync()
    const nav = [...document.querySelectorAll('nav button')].map((b) => b.textContent)
    expect(nav.slice(0, 3)).toEqual(['Emoji', 'Soundboard', 'Wheels'])
  })

  it('your Settings has Wheels after Chat', () => {
    cmp = mount(Settings, { target: document.body, props: { onClose: () => {} } })
    flushSync()
    const nav = [...document.querySelectorAll('nav button')].map((b) => b.textContent)
    expect(nav[nav.indexOf('Chat') + 1]).toBe('Wheels')
  })
})

// #264: in either dialog, Escape with a wheel's edit or a new wheel open closes just that, asking first (inline) when
// something typed would be lost; with neither open it closes the dialog as before.
describe('Escape in Settings › Wheels', () => {
  let closes = 0
  async function inDialog(which: 'server' | 'mine') {
    closes = 0
    cmp = which === 'server'
      ? mount(ServerSettings, { target: document.body, props: { server, onClose: () => closes++ } })
      : mount(Settings, { target: document.body, props: { section: 'wheels', onClose: () => closes++ } })
    flushSync()
    if (which === 'server') { button('Wheels').click(); flushSync() }
    await vi.waitFor(() => expect(document.querySelectorAll('.wheel-list li').length).toBeGreaterThan(0))
    flushSync()
  }
  const esc = (el: Element = document.activeElement ?? document.body) => {
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
  }
  const typeIn = (el: HTMLInputElement, text: string) => {
    el.focus()
    el.value = text
    el.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
  }
  const question = () => document.querySelector('.wheel-list .q, .new .q')?.textContent ?? null
  const wheelName = (which: 'server' | 'mine') => (which === 'server' ? 'Games' : 'Tank')
  const openEdit = (which: 'server' | 'mine') => {
    document.querySelector<HTMLButtonElement>(`button[aria-label="Edit ${wheelName(which)}"]`)!.click()
    flushSync()
  }
  const editOpen = () => document.querySelector('.wheel-list .edit') !== null

  for (const which of ['server', 'mine'] as const) {
    describe(which === 'server' ? 'Server settings' : 'your Settings', () => {
      it('with nothing open, Escape closes the dialog as before', async () => {
        await inDialog(which)
        esc()
        expect(closes).toBe(1)
      })

      it('an edit with nothing unsaved: Escape closes just the edit, then a second closes the dialog', async () => {
        await inDialog(which)
        openEdit(which)
        expect(editOpen()).toBe(true)
        esc()
        expect(editOpen()).toBe(false)
        expect(question()).toBeNull()
        expect(closes).toBe(0)
        // focus is back on the wheel's Edit button
        expect(document.activeElement?.getAttribute('aria-label')).toBe(`Edit ${wheelName(which)}`)
        esc()
        expect(closes).toBe(1)
      })

      it('a renamed but unsaved name asks first; Keep editing goes back to the field, keeping the typing', async () => {
        await inDialog(which)
        openEdit(which)
        const name = document.querySelector<HTMLInputElement>(`.edit input[aria-label="Name of ${wheelName(which)}"]`)!
        typeIn(name, 'Renamed')
        esc(name)
        expect(question()).toBe('Discard changes?')
        expect(document.activeElement?.textContent?.trim()).toBe('Keep editing')
        expect(editOpen()).toBe(true)
        expect(closes).toBe(0)
        expect(invoke).not.toHaveBeenCalledWith('update_wheel', expect.objectContaining({ request: { name: 'Renamed' } }))
        button('Keep editing').click()
        flushSync()
        expect(question()).toBeNull()
        expect(document.activeElement).toBe(name)
        expect(name.value).toBe('Renamed')
      })

      it('a second Escape while asking means Keep editing; Discard drops the typing and closes the edit', async () => {
        await inDialog(which)
        openEdit(which)
        const name = document.querySelector<HTMLInputElement>(`.edit input[aria-label="Name of ${wheelName(which)}"]`)!
        typeIn(name, 'Renamed')
        esc(name)
        esc()
        expect(question()).toBeNull()
        expect(editOpen()).toBe(true)
        expect(closes).toBe(0)
        esc()
        button('Discard').click()
        flushSync()
        expect(editOpen()).toBe(false)
        expect(closes).toBe(0)
        expect(invoke).not.toHaveBeenCalledWith('update_wheel', expect.objectContaining({ request: { name: 'Renamed' } }))
        openEdit(which)
        expect(document.querySelector<HTMLInputElement>(`.edit input[aria-label="Name of ${wheelName(which)}"]`)!.value).toBe(wheelName(which))
      })

      it('an option typed but not added counts as unsaved', async () => {
        await inDialog(which)
        openEdit(which)
        typeIn(document.querySelector<HTMLInputElement>('.edit input[aria-label="Add an option"]')!, 'Lethal Company')
        esc()
        expect(question()).toBe('Discard changes?')
        button('Discard').click()
        flushSync()
        expect(editOpen()).toBe(false)
      })

      it('a new wheel: empty closes at once; with a name it asks, and Discard drops it', async () => {
        await inDialog(which)
        button('New wheel').click()
        flushSync()
        esc()
        expect(document.querySelector('.new')).toBeNull()
        expect(closes).toBe(0)
        expect(document.activeElement?.textContent?.trim()).toBe('New wheel')
        button('New wheel').click()
        flushSync()
        typeIn(document.querySelector<HTMLInputElement>('input[aria-label="New wheel\'s name"]')!, 'Dinner')
        esc()
        expect(question()).toBe('Discard changes?')
        expect(closes).toBe(0)
        esc() // Keep editing
        expect(question()).toBeNull()
        expect(document.querySelector<HTMLInputElement>('input[aria-label="New wheel\'s name"]')!.value).toBe('Dinner')
        esc()
        button('Discard').click()
        flushSync()
        expect(document.querySelector('.new')).toBeNull()
        button('New wheel').click()
        flushSync()
        expect(document.querySelector<HTMLInputElement>('input[aria-label="New wheel\'s name"]')!.value).toBe('')
        expect(invoke).not.toHaveBeenCalledWith('create_wheel', expect.anything())
      })

      it('Escape from the dialog itself (focus outside the pane) still closes the open edit first', async () => {
        await inDialog(which)
        openEdit(which)
        document.querySelector<HTMLElement>('[role=dialog]')!.focus()
        esc()
        expect(editOpen()).toBe(false)
        expect(closes).toBe(0)
      })
    })
  }
})
