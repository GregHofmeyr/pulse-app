// @vitest-environment jsdom
// The picker wheel from the message box (#158): `/wheel a, b, c` spins a quick wheel straight away, `/wheel` on its
// own (or with one option) opens the wheel popover, and the composer's wheel button opens it too.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

const invoke = vi.fn(async (cmd: string, _args?: unknown): Promise<unknown> => {
  if (cmd === 'list_messages') return []
  if (cmd === 'list_server_wheels' || cmd === 'list_my_wheels') return []
  return undefined
})
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import TextChannel from './TextChannel.svelte'
import { forgetQuickWheels } from './WheelPopover.svelte'
import { app, resetState } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { testReady } from '../lib/testReady'
import { resetWheels } from '../lib/wheels.svelte'

let cmp: ReturnType<typeof mount> | undefined
function show() {
  app.state = applyReady(emptyState(), testReady([{ id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }]))
  cmp = mount(TextChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
  flushSync()
  return document.querySelector<HTMLElement>('[role="textbox"]')!
}
function send(box: HTMLElement, text: string) {
  box.replaceChildren(document.createTextNode(text))
  box.focus()
  const r = document.createRange()
  r.setStart(box.firstChild!, text.length)
  getSelection()!.removeAllRanges()
  getSelection()!.addRange(r)
  box.dispatchEvent(new InputEvent('input', { bubbles: true }))
  flushSync()
  box.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }))
  flushSync()
}
const popover = () => document.querySelector('.wheel-popover')

beforeEach(() => {
  Element.prototype.scrollTo = () => {} // jsdom has none
  resetState()
  resetWheels()
  forgetQuickWheels()
  invoke.mockClear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('/wheel in the message box', () => {
  it('with options spins a quick wheel at once, and sends no message', async () => {
    const box = show()
    send(box, '/wheel pizza, sushi, braai')
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('spin_wheel', { channelId: 'C1', request: { options: ['pizza', 'sushi', 'braai'] } }))
    expect(invoke.mock.calls.some(([c]) => c === 'send_message' || c === 'queue_message')).toBe(false)
    expect(app.state.pending['C1'] ?? []).toEqual([])
    expect(popover()).toBeNull()
  })

  it('on its own opens the wheel popover', async () => {
    const box = show()
    send(box, '/wheel')
    flushSync()
    expect(popover()).not.toBeNull()
    expect(invoke).not.toHaveBeenCalledWith('spin_wheel', expect.anything())
  })

  it('with one option opens the popover with it on the quick wheel', async () => {
    const box = show()
    send(box, '/wheel pizza')
    flushSync()
    await vi.waitFor(() => expect([...document.querySelectorAll('.wheel-popover .options .text')].map((t) => t.textContent)).toEqual(['pizza']))
  })

  it('a refused spin says why above the box', async () => {
    invoke.mockImplementationOnce(async () => []) // the history fetch
    const box = show()
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'spin_wheel') throw 'one spin every few seconds; try again in a moment'
      return []
    })
    send(box, '/wheel a, b')
    await vi.waitFor(() => expect(document.querySelector('.chan > .error')?.textContent).toContain('one spin every few seconds'))
  })

  // #262: the message box suggests the chat's commands; /whe + Tab gives "/wheel ", then the options spin it
  it('/whe and Tab completes to /wheel, with the usage tip while options are typed, and Enter spins', async () => {
    const box = show()
    box.replaceChildren(document.createTextNode('/whe'))
    box.focus()
    const r = document.createRange()
    r.setStart(box.firstChild!, 4)
    getSelection()!.removeAllRanges()
    getSelection()!.addRange(r)
    box.dispatchEvent(new InputEvent('input', { bubbles: true }))
    flushSync()
    expect(document.querySelector('[role=listbox][aria-label="Commands"]')?.textContent).toContain('/wheel')
    box.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true }))
    flushSync()
    await vi.waitFor(() => expect(box.textContent).toBe('/wheel '))
    expect(document.querySelector('.command-tip')?.textContent).toContain('Enter to spin')
    send(box, '/wheel kim, jo')
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('spin_wheel', { channelId: 'C1', request: { options: ['kim', 'jo'] } }))
  })

  it('an unknown /word is sent as an ordinary message', async () => {
    const box = show()
    send(box, '/shrug hi')
    await vi.waitFor(() => expect(app.state.pending['C1']?.map((p) => p.content)).toEqual(['/shrug hi']))
  })

  it('the wheel button by the emoji button opens and closes the popover', () => {
    show()
    const btn = document.querySelector<HTMLButtonElement>('button[aria-label="Picker wheel"]')!
    expect(btn.getAttribute('data-tip')).toBe('Picker wheel')
    btn.click()
    flushSync()
    expect(popover()).not.toBeNull()
    expect(btn.getAttribute('aria-expanded')).toBe('true')
    btn.click()
    flushSync()
    expect(popover()).toBeNull()
  })
})
