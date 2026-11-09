// @vitest-environment jsdom
// A picker wheel spin in the chat (#158, #271): live it spins to the server's angle and lands on its winner with a
// chime and the result view; the card stays until seen, then 20 s, then folds into the line (✕ at once, an hour at
// most); opened later it's the card (within the hour, unseen) or the line; Spin again, Remove winner & spin, Save as
// wheel.
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
import { noteSpin, resetWheels, spunLive } from '../lib/wheels.svelte'
import { QUIET_SPIN_MS, SPIN_MS, winnerAt } from '../lib/wheel'
import { CARD_MAX_MS, CARD_SEEN_MS, resetWheelCards } from '../lib/wheelCards'
import type { Message } from '../lib/protocol/Message'
import type { WheelSpin } from '../lib/protocol/WheelSpin'

const OPTIONS = ['pizza', 'sushi', 'braai', 'tacos']
const HOUR_AGO = () => new Date(Date.now() - CARD_MAX_MS - 60_000).toISOString()
/** A spin's line, spun over an hour ago unless `at` says (then its card has folded, seen or not). */
const line = (over: Partial<WheelSpin> = {}, at = HOUR_AGO()): Message => ({
  id: 'L1', channel_id: 'C1', author_id: null, kind: 'system', content: '🎡 alex spun a quick wheel → braai', reply_to_id: null,
  created_at: at, edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
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
const played = (name: string) => invoke.mock.calls.filter(([c, a]) => c === 'play_sound' && (a as { name: string }).name === name).length
const ticks = () => played('wheel_tick')
const chimes = () => played('wheel_result')
const card = () => document.querySelector<HTMLElement>('.wheel-card')
const resultView = () => document.querySelector('.wheel-card .result-view')
const inView = (on = true) => {
  FakeIntersectionObserver.set(card()!, on)
  flushSync()
}
const later = (ms: number) => {
  vi.advanceTimersByTime(ms)
  flushSync()
}
/** Spun just now, reaching this client now: it mounts spinning. */
const liveLine = (over: Partial<WheelSpin> = {}) => {
  const m = line(over, new Date().toISOString())
  noteSpin(m)
  return m
}
const land = () => {
  runFrame(0)
  for (let t = 16; t <= SPIN_MS + 16; t += 16) runFrame(t)
}

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
  localStorage.clear()
  resetWheelCards()
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

/** The line's action buttons (icon buttons, named by aria-label). */
const lineActs = () => [...document.querySelectorAll<HTMLButtonElement>('.wheel-line .acts button')]
const actNames = () => lineActs().map((b) => b.getAttribute('aria-label'))
const act = (name: string) => lineActs().find((b) => b.getAttribute('aria-label') === name)!

describe('a spin seen later (#273: the mini result card)', () => {
  it('is the settled line straight away: the winner with its colour, then who spun what, the count and the time', () => {
    show(line())
    expect(document.querySelector('.wheel-card')).toBeNull()
    const el = document.querySelector('.wheel-line')!
    expect(el.querySelector('.won')!.textContent!.trim()).toBe('braai')
    expect(el.querySelector<HTMLElement>('.swatch')!.style.background).toMatch(/hsl\(180|rgb\(41, 255, 255\)/)
    expect(el.querySelector('.sub')!.textContent!.replace(/\s+/g, ' ').trim()).toBe('alex spun a quick wheel · 4 options · 10:00')
    expect(frames).toHaveLength(0)
  })

  it('its thumbnail is the landed wheel: the same slices and colours, the winner full, the rest dimmed, under the pointer', () => {
    show(line())
    const thumb = document.querySelector('.wheel-line .wheel')!
    expect(thumb.getAttribute('style')).toMatch(/width: 40px/)
    const slices = [...thumb.querySelectorAll('.slice')]
    expect(slices.map((s) => s.classList.contains('dim'))).toEqual([true, true, false, true])
    expect(slices.map((s) => s.querySelector('path')!.getAttribute('fill'))).toEqual(['hsl(0, 100%, 58%)', 'hsl(90, 100%, 58%)', 'hsl(180, 100%, 58%)', 'hsl(270, 100%, 58%)'])
    // turned to the server's angle (slice 2 under the pointer); no labels at this size; a pointer and a small hub
    expect(thumb.querySelector('svg > g')!.getAttribute('transform')).toBe('rotate(135)')
    expect(thumb.querySelectorAll('text')).toHaveLength(0)
    expect(thumb.querySelector('.mini-pointer')).not.toBeNull()
    expect(thumb.querySelector('.mini-hub')).not.toBeNull()
    expect(thumb.querySelector('.result-view, button, .hub, .pointer')).toBeNull()
  })

  it('names a saved wheel', () => {
    show(line({ name: 'Dinner', wheel_id: 'W1' }))
    expect(document.querySelector('.wheel-line .sub')!.textContent).toContain('alex spun Dinner · 4 options')
  })

  it('the actions are 16 px icon buttons with tooltips, reachable by Tab', () => {
    show(line())
    expect(actNames()).toEqual(['Spin again', 'Remove winner & spin', 'Save as wheel'])
    for (const b of lineActs()) {
      expect(b.classList.contains('icon-btn')).toBe(true)
      expect(b.dataset.tip).toBe(b.getAttribute('aria-label'))
      expect(b.tabIndex).not.toBe(-1)
      expect(b.querySelector('svg')!.getAttribute('width')).toBe('16')
    }
  })

  it('Spin again and Remove winner & spin ask the server to spin from this line', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'spin_again' ? { ...line(), id: 'L2' } : undefined))
    show(line())
    act('Spin again').click()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('spin_again', { messageId: 'L1', removeWinner: false }))
    await vi.waitFor(() => expect(act('Remove winner & spin').disabled).toBe(false))
    // your own spin's line shows at once, spinning
    expect(app.state.messages['C1']?.some((m) => m.id === 'L2')).toBe(true)
    expect(spunLive('L2')).toBe(true)
    act('Remove winner & spin').click()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('spin_again', { messageId: 'L1', removeWinner: true }))
  })

  it('two options: no Remove winner (one would be left); a saved wheel: no Save as wheel', () => {
    show(line({ options: ['a', 'b'], winner: 0, name: 'Coin', wheel_id: 'W1' }))
    expect(actNames()).toEqual(['Spin again'])
    expect(document.querySelector('.wheel-line .sub')!.textContent).toContain('2 options')
  })

  it('a refusal shows on the line', async () => {
    invoke.mockRejectedValueOnce('one spin every few seconds; try again in a moment')
    show(line())
    act('Spin again').click()
    await vi.waitFor(() => expect(document.querySelector('.wheel-line [role="alert"]')?.textContent).toContain('one spin every few seconds'))
  })

  it('Save as wheel keeps the options: Server or Just me, then a name', async () => {
    invoke.mockImplementation(async (cmd: string, args?: unknown) =>
      cmd === 'create_wheel' ? { id: 'W9', scope: 'personal', server_id: null, owner_id: 'U1', name: 'Food', options: OPTIONS, updated_at: '' } : (void args, undefined))
    show(line())
    const save = act('Save as wheel')
    expect(save.getAttribute('aria-haspopup')).toBe('dialog')
    save.click()
    flushSync()
    expect(save.getAttribute('aria-expanded')).toBe('true')
    const form = document.querySelector('form[aria-label="Save as wheel"]')!
    form.querySelector<HTMLButtonElement>('[role="radio"]:nth-child(2)')!.click() // Just me
    const name = form.querySelector<HTMLInputElement>('input')!
    name.value = 'Food'
    name.dispatchEvent(new Event('input'))
    flushSync()
    form.querySelector<HTMLButtonElement>('button[type="submit"]')!.click()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('create_wheel', { request: { name: 'Food', options: OPTIONS } }))
    await vi.waitFor(() => expect(document.querySelector('.wheel-line')!.textContent).toContain('Saved as Food'))
    expect(actNames()).toEqual(['Spin again', 'Remove winner & spin'])
  })
})

describe('a spin as it happens', () => {
  it('spins to the server angle, lands on its winner for everyone, ticks, chimes, and shows the result view', () => {
    show(liveLine())
    expect(card()!.textContent).toContain('alex is spinning a quick wheel…')
    expect(resultView()).toBeNull()
    runFrame(0)
    expect(rotation()).toBe(0)
    for (let t = 16; t <= SPIN_MS + 16; t += 16) runFrame(t)
    const end = rotation()
    expect(winnerAt(end, OPTIONS.length)).toBe(2)
    expect(((end % 360) + 360) % 360).toBeCloseTo(135)
    expect(resultView()!.textContent!.trim()).toBe('braai')
    expect(card()!.textContent).toContain('alex spun a quick wheel')
    // the winning slice stays brighter under the fade
    expect(document.querySelectorAll('.slice.dim')).toHaveLength(3)
    expect(ticks()).toBeGreaterThan(3)
    expect(chimes()).toBe(1)
    // the hub gives way to the winner
    expect(document.querySelector('.wheel-card .hub')).toBeNull()
  })

  it('the card stays until it is seen, then CARD_SEEN_MS more, then folds into the line', () => {
    show(liveLine())
    land()
    // not in view (scrolled away): it waits
    later(10 * 60_000)
    expect(card()).not.toBeNull()
    inView()
    later(CARD_SEEN_MS - 1)
    expect(card()).not.toBeNull()
    later(1)
    expect(card()).toBeNull()
    expect(document.querySelector('.wheel-line .won')!.textContent!.trim()).toBe('braai')
  })

  it('in view while it spins counts once it lands, not before', () => {
    show(liveLine())
    inView()
    runFrame(0)
    runFrame(SPIN_MS / 2)
    later(CARD_SEEN_MS)
    land()
    expect(card()).not.toBeNull()
    later(CARD_SEEN_MS)
    expect(card()).toBeNull()
  })

  it('with Pulse in the background it chimes, and waits to be seen until Pulse is focused', () => {
    windowFocus.focused = false
    show(liveLine())
    land()
    expect(chimes()).toBe(1)
    inView()
    later(5 * 60_000)
    expect(card()).not.toBeNull()
    windowFocus.focused = true
    flushSync()
    later(CARD_SEEN_MS)
    expect(card()).toBeNull()
  })

  it('✕ (Dismiss) folds it at once, and it stays folded when the chat is opened again', () => {
    const m = liveLine()
    show(m)
    land()
    const x = document.querySelector<HTMLButtonElement>('.wheel-card button[aria-label="Dismiss"]')!
    expect(x.dataset.tip).toBe('Dismiss')
    x.click()
    flushSync()
    expect(card()).toBeNull()
    expect(document.querySelector('.wheel-line')).not.toBeNull()
    unmount(cmp!)
    resetWheels() // a reload: only what's stored is left
    resetWheelCards()
    show(m)
    expect(card()).toBeNull()
  })

  it('Spin again, Remove winner & spin and Save as wheel are on the card once it lands', () => {
    show(liveLine())
    expect(card()!.querySelectorAll('.link-btn')).toHaveLength(0)
    land()
    expect([...card()!.querySelectorAll('.link-btn')].map((b) => b.textContent!.trim())).toEqual(['Spin again', 'Remove winner & spin', 'Save as wheel'])
  })

  it('under reduced motion: a short spin, no tick, the result view (and the chime) at once', () => {
    motion.reducedMotion = true
    show(liveLine())
    runFrame(0)
    runFrame(QUIET_SPIN_MS / 2)
    runFrame(QUIET_SPIN_MS)
    expect(QUIET_SPIN_MS).toBeLessThanOrEqual(500)
    expect(winnerAt(rotation(), OPTIONS.length)).toBe(2)
    expect(frames).toHaveLength(0) // done
    expect(ticks()).toBe(0)
    expect(resultView()).not.toBeNull()
    expect(chimes()).toBe(1)
  })

  it('in an unfocused window: quiet too', () => {
    windowFocus.focused = false
    show(liveLine())
    for (let t = 0; t <= QUIET_SPIN_MS; t += 16) runFrame(t)
    runFrame(QUIET_SPIN_MS + 16)
    expect(frames).toHaveLength(0)
    expect(ticks()).toBe(0)
  })

  it('a spin that arrived over a second ago mounts landed: the card with the result view, no replay, no chime', () => {
    const m = line({}, new Date().toISOString())
    noteSpin(m, Date.now() - 1500)
    show(m)
    expect(resultView()!.textContent!.trim()).toBe('braai')
    expect(((rotation() % 360) + 360) % 360).toBeCloseTo(135)
    expect(frames).toHaveLength(0)
    expect(chimes()).toBe(0)
    inView()
    later(CARD_SEEN_MS)
    expect(card()).toBeNull()
  })

  it('opened within the hour but unseen: the card; it folds at the hour anyway', () => {
    show(line({}, new Date(Date.now() - CARD_MAX_MS + 5000).toISOString()))
    expect(resultView()).not.toBeNull()
    later(5000)
    expect(card()).toBeNull()
  })
})
