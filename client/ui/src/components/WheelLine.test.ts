// @vitest-environment jsdom
// A picker wheel spin in the chat (#158): live it spins to the server's angle and lands on its winner, then folds into
// the line; seen later it is the line at once; Spin again, Remove winner & spin and Save as wheel.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import WheelLine from './WheelLine.svelte'
import { app, resetState } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { testReady } from '../lib/testReady'
import { motion } from '../lib/animate.svelte'
import { windowFocus } from '../lib/windowFocus.svelte'
import { noteSpin, resetWheels } from '../lib/wheels.svelte'
import { HOLD_MS, QUIET_SPIN_MS, SPIN_MS, winnerAt } from '../lib/wheel'
import type { Message } from '../lib/protocol/Message'
import type { WheelSpin } from '../lib/protocol/WheelSpin'

const OPTIONS = ['pizza', 'sushi', 'braai', 'tacos']
const line = (over: Partial<WheelSpin> = {}): Message => ({
  id: 'L1', channel_id: 'C1', author_id: null, kind: 'system', content: '🎡 alex spun a quick wheel → braai', reply_to_id: null,
  created_at: '2026-11-09T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null, clip: null, embeds: [],
  // as the server draws it: at 135° the pointer reads slice floor((360 - 135) / 90) = 2, the winner
  wheel: { spinner_id: 'U1', wheel_id: null, name: null, options: OPTIONS, winner: 2, angle: 135, ...over },
})

let cmp: ReturnType<typeof mount> | undefined
const show = (m: Message) => {
  cmp = mount(WheelLine, { target: document.body, props: { message: m, serverId: 'S1', time: '10:00' } })
  flushSync()
}
const rotation = () => Number(/rotate\(([-\d.e]+)\)/.exec(document.querySelector('.wheel svg > g')!.getAttribute('transform')!)![1])
const ticks = () => invoke.mock.calls.filter(([c, a]) => c === 'play_sound' && (a as { name: string }).name === 'wheel_tick').length

// requestAnimationFrame driven by hand: each step runs the frame callbacks at the given time
let frames: ((t: number) => void)[] = []
const runFrame = (t: number) => {
  const now = frames
  frames = []
  for (const f of now) f(t)
  flushSync()
}

beforeEach(() => {
  resetState()
  resetWheels()
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  app.state = applyReady(emptyState(), testReady([{ id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }]))
  motion.reducedMotion = false
  motion.streaming = false
  windowFocus.focused = true
  frames = []
  vi.stubGlobal('requestAnimationFrame', (f: (t: number) => void) => (frames.push(f), frames.length))
  vi.stubGlobal('cancelAnimationFrame', () => {})
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('a spin seen later', () => {
  it('is the settled line straight away: who, which wheel, the winner with its colour', () => {
    show(line())
    expect(document.querySelector('.wheel-card')).toBeNull()
    const text = document.querySelector('.wheel-line')!.textContent!.replace(/\s+/g, ' ')
    expect(text).toContain('alex spun a quick wheel → braai')
    expect(document.querySelector<HTMLElement>('.wheel-line .swatch')!.style.background).toMatch(/hsl\(180|rgb\(41, 255, 255\)/)
    expect(frames).toHaveLength(0)
  })

  it('names a saved wheel', () => {
    show(line({ name: 'Dinner', wheel_id: 'W1' }))
    expect(document.querySelector('.wheel-line')!.textContent).toContain('spun Dinner →')
  })

  it('Spin again and Remove winner & spin ask the server to spin from this line', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'spin_again' ? { ...line(), id: 'L2' } : undefined))
    show(line())
    const buttons = [...document.querySelectorAll<HTMLButtonElement>('.wheel-line .link-btn')]
    expect(buttons.map((b) => b.textContent!.trim())).toEqual(['Spin again', 'Remove winner & spin', 'Save as wheel'])
    buttons[0].click()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('spin_again', { messageId: 'L1', removeWinner: false }))
    await vi.waitFor(() => expect(buttons[1].disabled).toBe(false))
    // your own spin's line shows at once, spinning
    expect(app.state.messages['C1']?.some((m) => m.id === 'L2')).toBe(true)
    buttons[1].click()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('spin_again', { messageId: 'L1', removeWinner: true }))
  })

  it('two options: no Remove winner (one would be left); a saved wheel: no Save as wheel', () => {
    show(line({ options: ['a', 'b'], winner: 0, name: 'Coin', wheel_id: 'W1' }))
    expect([...document.querySelectorAll('.wheel-line .link-btn')].map((b) => b.textContent!.trim())).toEqual(['Spin again'])
  })

  it('a refusal shows on the line', async () => {
    invoke.mockRejectedValueOnce('one spin every few seconds; try again in a moment')
    show(line())
    document.querySelector<HTMLButtonElement>('.wheel-line .link-btn')!.click()
    await vi.waitFor(() => expect(document.querySelector('.wheel-line [role="alert"]')?.textContent).toContain('one spin every few seconds'))
  })

  it('Save as wheel keeps the options: Server or Just me, then a name', async () => {
    invoke.mockImplementation(async (cmd: string, args?: unknown) =>
      cmd === 'create_wheel' ? { id: 'W9', scope: 'personal', server_id: null, owner_id: 'U1', name: 'Food', options: OPTIONS, updated_at: '' } : (void args, undefined))
    show(line())
    ;[...document.querySelectorAll<HTMLButtonElement>('.wheel-line .link-btn')].at(-1)!.click()
    flushSync()
    const form = document.querySelector('form[aria-label="Save as wheel"]')!
    form.querySelector<HTMLButtonElement>('[role="radio"]:nth-child(2)')!.click() // Just me
    const name = form.querySelector<HTMLInputElement>('input')!
    name.value = 'Food'
    name.dispatchEvent(new Event('input'))
    flushSync()
    form.querySelector<HTMLButtonElement>('button[type="submit"]')!.click()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('create_wheel', { request: { name: 'Food', options: OPTIONS } }))
    await vi.waitFor(() => expect(document.querySelector('.wheel-line')!.textContent).toContain('Saved as Food'))
  })
})

describe('a spin as it happens', () => {
  it('spins to the server angle, lands on its winner for everyone, ticks, then folds into the line', () => {
    const m = line()
    noteSpin(m)
    show(m)
    expect(document.querySelector('.wheel-card')!.textContent).toContain('alex is spinning a quick wheel…')
    runFrame(0)
    expect(rotation()).toBe(0)
    for (let t = 16; t <= SPIN_MS + 16; t += 16) runFrame(t)
    const end = rotation()
    expect(winnerAt(end, OPTIONS.length)).toBe(2)
    expect(((end % 360) + 360) % 360).toBeCloseTo(135)
    expect(document.querySelector('.wheel-card')!.textContent).toContain('braai')
    expect(document.querySelectorAll('.slice.dim')).toHaveLength(3)
    expect(ticks()).toBeGreaterThan(3)
    vi.advanceTimersByTime(HOLD_MS)
    flushSync()
    expect(document.querySelector('.wheel-card')).toBeNull()
    expect(document.querySelector('.wheel-line')!.textContent!.replace(/\s+/g, ' ')).toContain('→ braai')
  })

  it('under reduced motion: a short spin, no tick', () => {
    motion.reducedMotion = true
    const m = line()
    noteSpin(m)
    show(m)
    runFrame(0)
    runFrame(QUIET_SPIN_MS / 2)
    runFrame(QUIET_SPIN_MS)
    expect(QUIET_SPIN_MS).toBeLessThanOrEqual(500)
    expect(winnerAt(rotation(), OPTIONS.length)).toBe(2)
    expect(frames).toHaveLength(0) // done
    expect(ticks()).toBe(0)
  })

  it('in an unfocused window: quiet too', () => {
    windowFocus.focused = false
    const m = line()
    noteSpin(m)
    show(m)
    for (let t = 0; t <= QUIET_SPIN_MS; t += 16) runFrame(t)
    runFrame(QUIET_SPIN_MS + 16)
    expect(frames).toHaveLength(0)
    expect(ticks()).toBe(0)
  })

  it('a spin that arrived over a second ago mounts settled (opened the chat mid-spin)', () => {
    const m = line()
    noteSpin(m, Date.now() - 1500)
    show(m)
    expect(document.querySelector('.wheel-card')).toBeNull()
    expect(document.querySelector('.wheel-line')).not.toBeNull()
  })
})
