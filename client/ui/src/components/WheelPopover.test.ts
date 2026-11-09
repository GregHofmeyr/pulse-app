// @vitest-environment jsdom
// The wheel popover (#158): Server / Just me, the saved list, a quick wheel's editing (add, paste, remove, shuffle,
// clear), saved wheels saving as they change, Save as wheel, and Spin.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'
import type { Wheel } from '../lib/protocol/Wheel'

const serverWheels: Wheel[] = [
  { id: 'W1', scope: 'server', server_id: 'S1', owner_id: null, name: 'Games tonight', options: ['Valheim', 'Apex', 'Among Us'], updated_at: '' },
]
const myWheels: Wheel[] = [
  { id: 'W2', scope: 'personal', server_id: null, owner_id: 'U1', name: 'Tank', options: ['Reinhardt', 'Sigma'], updated_at: '' },
]
const spunLine = { id: 'L1', channel_id: 'C1', author_id: null, kind: 'system', content: '', reply_to_id: null, created_at: '2026-11-09T10:00:00.000Z',
  edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [],
  wheel: { spinner_id: 'U1', wheel_id: null, name: null, options: ['a', 'b'], winner: 0, angle: 300 } }
const invoke = vi.fn(async (cmd: string, args?: { id?: string; request?: Record<string, unknown> }): Promise<unknown> => {
  if (cmd === 'list_server_wheels') return serverWheels
  if (cmd === 'list_my_wheels') return myWheels
  if (cmd === 'update_wheel') return { ...[...serverWheels, ...myWheels].find((w) => w.id === args!.id), ...args!.request }
  if (cmd === 'create_wheel') return { id: 'W9', scope: args!.request!.server_id ? 'server' : 'personal', server_id: args!.request!.server_id ?? null, owner_id: args!.request!.server_id ? null : 'U1', name: args!.request!.name, options: args!.request!.options, updated_at: '' }
  if (cmd === 'spin_wheel') return spunLine
  return undefined
})
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import WheelPopover, { forgetQuickWheels } from './WheelPopover.svelte'
import { app, resetState } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { testReady } from '../lib/testReady'
import { resetWheels, setSpinQuietly, wheelPrefs } from '../lib/wheels.svelte'

let cmp: ReturnType<typeof mount> | undefined
let closed = 0
async function open(serverId: string | null = 'S1', prefill: string[] | null = null) {
  const anchor = document.createElement('button')
  document.body.append(anchor)
  cmp = mount(WheelPopover, { target: document.body, props: { anchor, channelId: 'C1', serverId, prefill, onClose: () => closed++ } })
  await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('list_my_wheels', undefined))
  flushSync()
  await Promise.resolve()
  flushSync()
}
const pop = () => document.querySelector<HTMLElement>('.wheel-popover')!
const rows = () => [...pop().querySelectorAll<HTMLButtonElement>('.wheel-row')].map((r) => [...r.children].map((c) => c.textContent!.trim()).filter(Boolean).join(' ') || r.textContent!.trim())
const options = () => [...pop().querySelectorAll('.options li .text')].map((t) => t.textContent)
const labels = () => [...pop().querySelectorAll('.wheel svg text')].map((t) => t.textContent)
const button = (text: string) => [...pop().querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent!.trim() === text)!
function type(text: string) {
  const field = pop().querySelector<HTMLInputElement>('input[aria-label="Add an option"]')!
  field.value = text
  field.dispatchEvent(new Event('input'))
  flushSync()
  field.form!.requestSubmit()
  flushSync()
}

beforeEach(() => {
  resetState()
  resetWheels()
  forgetQuickWheels()
  invoke.mockClear()
  closed = 0
  app.state = applyReady(emptyState(), testReady([{ id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }]))
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('WheelPopover', () => {
  it('opens on a quick wheel, with the server wheels listed and Just me a switch away', async () => {
    await open()
    expect(pop().getAttribute('aria-label')).toBe('Picker wheel')
    expect(invoke).toHaveBeenCalledWith('list_server_wheels', { serverId: 'S1' })
    expect(rows()).toEqual(['Quick wheel 0', 'Games tonight 3'])
    expect(pop().querySelector('.what')!.textContent).toContain('One-off, not saved')
    expect(button('Add at least 2 options').disabled).toBe(true)
    pop().querySelectorAll<HTMLButtonElement>('[role="radio"]')[1].click()
    flushSync()
    expect(rows()).toEqual(['Quick wheel 0', 'Tank 2'])
  })

  it('in a DM: Just me wheels only, no switch', async () => {
    await open(null)
    expect(pop().querySelector('[role="radiogroup"]')).toBeNull()
    expect(invoke).not.toHaveBeenCalledWith('list_server_wheels', expect.anything())
    expect(rows()).toEqual(['Quick wheel 0', 'Tank 2'])
  })

  it('a quick wheel: add, paste several, skip repeats, remove, shuffle, clear all', async () => {
    await open()
    type('pizza')
    type('sushi, braai, Pizza')
    expect(options()).toEqual(['pizza', 'sushi', 'braai'])
    expect(labels()).toEqual(['pizza', 'sushi', 'braai'])
    pop().querySelector<HTMLButtonElement>('button[aria-label="Remove sushi"]')!.click()
    flushSync()
    expect(options()).toEqual(['pizza', 'braai'])
    type('tacos, ramen')
    button('Shuffle').click()
    flushSync()
    expect([...options()].sort()).toEqual(['braai', 'pizza', 'ramen', 'tacos'])
    button('Clear all').click()
    flushSync()
    expect(options()).toEqual([])
    expect(pop().querySelector('.editor .empty')!.textContent).toContain('No options yet')
    expect(invoke).not.toHaveBeenCalledWith('update_wheel', expect.anything())
  })

  it('/wheel with one option starts the quick wheel with it', async () => {
    await open('S1', ['pizza'])
    expect(options()).toEqual(['pizza'])
  })

  it('Spin sends the quick wheel to the chat and closes', async () => {
    await open()
    type('a, b')
    button('Spin').click()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('spin_wheel', { channelId: 'C1', request: { options: ['a', 'b'] } }))
    await vi.waitFor(() => expect(closed).toBe(1))
    expect(app.state.messages['C1']?.[0].id).toBe('L1')
  })

  it('a saved wheel spins by id and saves each change', async () => {
    await open()
    ;[...pop().querySelectorAll<HTMLButtonElement>('.wheel-row')][1].click()
    flushSync()
    expect(pop().querySelector<HTMLInputElement>('input[aria-label="Wheel name"]')!.value).toBe('Games tonight')
    expect(pop().querySelector('.what')!.textContent).toContain('Server wheel')
    type('Lethal Company')
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('update_wheel', { id: 'W1', request: { options: ['Valheim', 'Apex', 'Among Us', 'Lethal Company'] } }))
    button('Spin').click()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('spin_wheel', { channelId: 'C1', request: { wheel_id: 'W1' } }))
  })

  it('a refusal stays in the popover', async () => {
    await open()
    type('a, b')
    invoke.mockRejectedValueOnce('one spin every few seconds; try again in a moment')
    button('Spin').click()
    await vi.waitFor(() => expect(pop().querySelector('[role="alert"]')?.textContent).toContain('one spin every few seconds'))
    expect(closed).toBe(0)
  })

  it('Save as wheel: Server or Just me, a name, then it is the selected saved wheel', async () => {
    await open()
    type('a, b, c')
    pop().querySelector<HTMLButtonElement>('.stage .btn-secondary')!.click()
    flushSync()
    const form = pop().querySelector<HTMLFormElement>('form[aria-label="Save as wheel"]')!
    const name = form.querySelector<HTMLInputElement>('input')!
    name.value = 'Letters'
    name.dispatchEvent(new Event('input'))
    flushSync()
    form.requestSubmit()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('create_wheel', { request: { server_id: 'S1', name: 'Letters', options: ['a', 'b', 'c'] } }))
    await vi.waitFor(() => expect(pop().querySelector('.what .name')!.textContent).toBe('Letters'))
    expect(rows()).toContain('Letters 3')
  })

  it('+ New wheel makes one in the shown scope and selects it', async () => {
    await open()
    pop().querySelector<HTMLButtonElement>('.saved-head button[aria-label="New wheel"]')!.click()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('create_wheel', { request: { server_id: 'S1', name: 'New wheel', options: [] } }))
    await vi.waitFor(() => expect(pop().querySelector<HTMLInputElement>('input[aria-label="Wheel name"]')?.value).toBe('New wheel'))
  })
})

// #270: Just me wheels spin privately, in the popover; other spins can be quiet (remembered per device).
describe('WheelPopover spins (#270)', () => {
  let frames: ((t: number) => void)[] = []
  const runFrames = (t: number) => {
    const now = frames
    frames = []
    for (const f of now) f(t)
    flushSync()
  }
  beforeEach(() => {
    frames = []
    vi.stubGlobal('requestAnimationFrame', (f: (t: number) => void) => (frames.push(f), frames.length))
    vi.stubGlobal('cancelAnimationFrame', () => {})
    setSpinQuietly(false)
  })
  const quietBox = () => pop().querySelector<HTMLInputElement>('input[type="checkbox"][aria-label="Spin quietly"], label.quietly input')
  const justMe = () => {
    pop().querySelectorAll<HTMLButtonElement>('[role="radio"]')[1].click()
    flushSync()
    ;[...pop().querySelectorAll<HTMLButtonElement>('.wheel-row')][1].click()
    flushSync()
  }

  it('a Just me wheel spins here, for you alone: nothing is sent, the popover stays and shows the winner', async () => {
    await open()
    justMe()
    expect(pop().querySelector('.what')!.textContent).toContain('only you see')
    expect(quietBox()).toBeNull()
    vi.spyOn(Math, 'random').mockReturnValue(0.75) // the second of two slices
    button('Spin').click()
    flushSync()
    runFrames(0)
    runFrames(100_000)
    vi.mocked(Math.random).mockRestore()
    expect(invoke.mock.calls.some(([c]) => String(c).startsWith('spin_'))).toBe(false)
    expect(closed).toBe(0)
    expect(pop().querySelector('.result')!.textContent).toContain('Sigma')
    expect(app.state.messages['C1'] ?? []).toEqual([])
    // Remove winner & spin takes it off this spin's wheel only (nothing saved), then spins the rest
    expect(button('Spin again')).toBeTruthy()
    expect(pop().querySelector('.result button')).toBeNull() // two options: nothing to remove
  })

  it('Remove winner & spin on a private spin leaves the saved wheel alone', async () => {
    myWheels[0] = { ...myWheels[0], options: ['Reinhardt', 'Sigma', 'Winston'] }
    try {
      await open()
      justMe()
      vi.spyOn(Math, 'random').mockReturnValue(0.5) // Sigma
      button('Spin').click()
      runFrames(0)
      runFrames(100_000)
      expect(pop().querySelector('.result')!.textContent).toContain('Sigma')
      button('Remove winner & spin').click()
      flushSync()
      runFrames(0)
      runFrames(100_000)
      vi.mocked(Math.random).mockRestore()
      expect(labels()).toEqual(['Reinhardt', 'Winston'])
      expect(options()).toEqual(['Reinhardt', 'Sigma', 'Winston']) // the saved wheel keeps it
      expect(invoke).not.toHaveBeenCalledWith('update_wheel', expect.anything())
    } finally {
      myWheels[0] = { ...myWheels[0], options: ['Reinhardt', 'Sigma'] }
    }
  })

  it('"Spin quietly" sends a quiet spin, closes, and is remembered', async () => {
    await open()
    type('a, b')
    const box = quietBox()!
    expect(box.checked).toBe(false)
    box.click()
    flushSync()
    expect(wheelPrefs.quiet).toBe(true)
    invoke.mockImplementationOnce(async () => ({ id: 'Q1', channel_id: 'C1', spin: spunLine.wheel }))
    button('Spin').click()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('spin_wheel_quietly', { channelId: 'C1', request: { options: ['a', 'b'] } }))
    await vi.waitFor(() => expect(closed).toBe(1))
    expect(app.state.messages['C1'] ?? []).toEqual([])
    unmount(cmp!)
    cmp = undefined
    document.body.innerHTML = ''
    await open()
    expect(quietBox()!.checked).toBe(true)
  })
})

// #264: a light net for closing mid-edit. The quick wheel's options outlive the popover; Escape in a field first
// leaves it (or empties the add field), and only a second Escape closes the popover.
describe('WheelPopover Escape', () => {
  const esc = (el: Element) => {
    const ev = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
    el.dispatchEvent(ev)
    flushSync()
    return ev
  }
  const addField = () => pop().querySelector<HTMLInputElement>('input[aria-label="Add an option"]')!

  it('the quick wheel survives closing and opening again', async () => {
    await open()
    type('Valheim, Apex')
    unmount(cmp!)
    cmp = undefined
    document.body.innerHTML = ''
    await open()
    expect(options()).toEqual(['Valheim', 'Apex'])
  })

  it('Escape in the add field with text empties it, then leaves the field, then closes', async () => {
    await open()
    const f = addField()
    f.focus()
    f.value = 'Ape'
    f.dispatchEvent(new Event('input'))
    flushSync()
    esc(f)
    expect(addField().value).toBe('')
    expect(document.activeElement).toBe(f)
    expect(closed).toBe(0)
    esc(f)
    expect(document.activeElement).toBe(pop()) // out of the field, still in the popover
    expect(closed).toBe(0)
    esc(document.activeElement!)
    expect(closed).toBe(1)
  })

  it('Escape in a saved wheel\'s name field leaves it (keeping the name), then a second closes', async () => {
    await open()
    ;[...pop().querySelectorAll<HTMLButtonElement>('.wheel-row')][1].click()
    flushSync()
    const name = pop().querySelector<HTMLInputElement>('input[aria-label="Wheel name"]')!
    name.focus()
    name.value = 'Games later'
    name.dispatchEvent(new Event('input'))
    flushSync()
    esc(name)
    expect(document.activeElement).toBe(pop())
    expect(closed).toBe(0)
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('update_wheel', { id: 'W1', request: { name: 'Games later' } }))
    esc(document.activeElement!)
    expect(closed).toBe(1)
  })

  it('Escape on a button closes straight away', async () => {
    await open()
    const b = pop().querySelector<HTMLButtonElement>('.wheel-row')!
    b.focus()
    esc(b)
    expect(closed).toBe(1)
  })
})
