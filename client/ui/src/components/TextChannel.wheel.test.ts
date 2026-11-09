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
import { addQuietSpin, handleWheelEvent, resetWheels, setSpinQuietly } from '../lib/wheels.svelte'
import { QUIET_CARD_MS } from '../lib/wheel'

const spin = (options: string[], extra: Record<string, unknown> = {}) => ({ spinner_id: 'U2', wheel_id: null, name: null, options, winner: 1, angle: 100, ...extra })
const line = (id: string, options: string[]) => ({ id, channel_id: 'C1', author_id: null, kind: 'system', content: '', reply_to_id: null, created_at: '2026-11-09T10:00:00.000Z',
  edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], wheel: spin(options) })

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

  it('/wheel follows "Spin quietly" as the popover last left it (#270)', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'spin_wheel_quietly' ? { id: 'Q1', channel_id: 'C1', spin: spin(['kim', 'jo']) } : []))
    setSpinQuietly(true)
    try {
      const box = show()
      send(box, '/wheel kim, jo')
      await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('spin_wheel_quietly', { channelId: 'C1', request: { options: ['kim', 'jo'] } }))
      expect(invoke).not.toHaveBeenCalledWith('spin_wheel', expect.anything())
      await vi.waitFor(() => expect(document.querySelector('.quiet-card')).not.toBeNull())
    } finally {
      setSpinQuietly(false)
    }
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

// #270: a quiet spin's card shows for everyone in the chat, then goes; Spin again / Remove winner on it are quiet too
describe('quiet spins in the chat', () => {
  const card = () => document.querySelector<HTMLElement>('.quiet-card')
  const cardButton = (text: string) => [...(card()?.querySelectorAll<HTMLButtonElement>('button') ?? [])].find((b) => b.textContent!.trim() === text)
  beforeEach(() => {
    invoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'spin_wheel_quietly') {
        const { request } = args as { request: { options: string[]; wheel_id?: string } }
        return { id: 'Q9', channel_id: 'C1', spin: spin(request.options, request.wheel_id ? { wheel_id: request.wheel_id, name: 'Games' } : {}) }
      }
      return []
    })
  })

  it("someone's quiet spin shows as a card that leaves no line, and goes after a while", async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    try {
      show()
      handleWheelEvent({ t: 'WheelSpunQuietly', d: { spin: { id: 'Q1', channel_id: 'C1', spin: spin(['pizza', 'sushi', 'braai']) } } })
      flushSync()
      expect(card()).not.toBeNull()
      expect(card()!.textContent).toContain('Quiet spin · not saved')
      expect(app.state.messages['C1'] ?? []).toEqual([])
      vi.advanceTimersByTime(QUIET_CARD_MS)
      flushSync()
      expect(card()).toBeNull()
    } finally {
      vi.useRealTimers()
    }
  })

  it('Spin again and Remove winner & spin on a quiet card spin quietly, and it gives way to the new one', async () => {
    show()
    addQuietSpin({ id: 'Q1', channel_id: 'C1', spin: spin(['pizza', 'sushi', 'braai'], { wheel_id: 'W1', name: 'Food' }) }, Date.now() - 2000)
    flushSync()
    // the result shows once it lands; a card that mounts later than a moment after arriving is landed already
    await vi.waitFor(() => expect(cardButton('Remove winner & spin')).toBeTruthy())
    cardButton('Remove winner & spin')!.click()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('spin_wheel_quietly', { channelId: 'C1', request: { wheel_id: 'W1', options: ['pizza', 'braai'] } }))
    await vi.waitFor(() => expect(document.querySelectorAll('.quiet-card')).toHaveLength(1))
  })
})

// #270 (5): a spin you start brings the chat to the latest, as sending does; someone else's leaves your place alone
describe('your spin comes into view', () => {
  let scrolls: unknown[] = []
  /** The history has loaded (opening the chat scrolls to the newest), then you scroll well up. */
  async function scrolledUp() {
    await vi.waitFor(() => expect(app.state.history['C1']?.loaded).toBe(true))
    await new Promise((r) => setTimeout(r, 20))
    const list = document.querySelector<HTMLElement>('.list')!
    Object.defineProperty(list, 'scrollHeight', { configurable: true, get: () => 2000 })
    Object.defineProperty(list, 'clientHeight', { configurable: true, get: () => 500 })
    list.scrollTop = 0
    scrolls = []
    return list
  }
  beforeEach(() => {
    scrolls = []
    Element.prototype.scrollTo = function (this: Element, o?: unknown) {
      if (this.classList.contains('list')) scrolls.push(o)
    } as never
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'spin_wheel') return line('L5', ['kim', 'jo'])
      if (cmd === 'spin_wheel_quietly') return { id: 'Q5', channel_id: 'C1', spin: spin(['kim', 'jo']) }
      return []
    })
  })

  it('/wheel from higher up jumps to the latest', async () => {
    const box = show()
    await scrolledUp()
    scrolls = []
    send(box, '/wheel kim, jo')
    await vi.waitFor(() => expect(scrolls).toContainEqual({ top: 2000 }))
  })

  it('a quiet spin from the popover does too', async () => {
    setSpinQuietly(true)
    try {
      show()
      await scrolledUp()
      document.querySelector<HTMLButtonElement>('button[aria-label="Picker wheel"]')!.click()
      flushSync()
      const field = document.querySelector<HTMLInputElement>('.wheel-popover input[aria-label="Add an option"]')!
      field.value = 'kim, jo'
      field.dispatchEvent(new Event('input'))
      flushSync()
      field.form!.requestSubmit()
      flushSync()
      scrolls = []
      ;[...document.querySelectorAll<HTMLButtonElement>('.wheel-popover button')].find((b) => b.textContent!.trim() === 'Spin')!.click()
      await vi.waitFor(() => expect(scrolls).toContainEqual({ top: 2000 }))
    } finally {
      setSpinQuietly(false)
    }
  })

  it("someone else's spin doesn't move you when you're reading higher up", async () => {
    show()
    await scrolledUp()
    scrolls = []
    handleWheelEvent({ t: 'WheelSpunQuietly', d: { spin: { id: 'Q2', channel_id: 'C1', spin: spin(['a', 'b']) } } })
    app.state = { ...app.state, messages: { ...app.state.messages, C1: [...(app.state.messages['C1'] ?? []), line('L7', ['a', 'b']) as never] } }
    flushSync()
    await new Promise((r) => setTimeout(r, 20))
    expect(scrolls).toEqual([])
  })
})
