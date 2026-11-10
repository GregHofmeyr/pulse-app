// @vitest-environment jsdom
// Saved wheels' changes (#158): sent one at a time in order, only the newest answer shown, a refusal reloads the
// list; a spin waits for them. And the live-spin window.
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Wheel } from './protocol/Wheel'

const w: Wheel = { id: 'W1', scope: 'server', server_id: 'S1', owner_id: null, name: 'Games', options: ['a', 'b'], updated_at: '' }
let answers: ((v: unknown) => void)[] = []
let fails: ((e: unknown) => void)[] = []
const invoke = vi.fn(async (cmd: string, _args?: unknown): Promise<unknown> => {
  if (cmd === 'update_wheel') return new Promise((ok, no) => (answers.push(ok), fails.push(no)))
  if (cmd === 'list_server_wheels') return [{ ...w, options: ['from', 'server'] }]
  return undefined
})
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import {
  handleWheelEvent, holdQuietSpin, noteSpin, onOwnSpin, quietSpins, resetWheels, saveWheel, savesSettled, setSpinQuietly,
  spinAgain, spinQuietly, spinWheel, spunLive, wheelPrefs, wheels,
} from './wheels.svelte'
import { QUIET_CARD_MS, QUIET_LINGER_MS } from './wheel'

const flush = () => new Promise((r) => setTimeout(r, 0))

beforeEach(() => {
  resetWheels()
  invoke.mockReset() // back to the implementation above: a test that swaps it in must not leave it to the next
  answers = []
  fails = []
  wheels.server = { S1: [w] }
})

describe('saving a saved wheel', () => {
  it('sends changes one at a time, in order, and keeps the newest on screen', async () => {
    const first = saveWheel(w, { options: ['a', 'b', 'c'] })
    const second = saveWheel({ ...w, options: ['a', 'b', 'c'] }, { options: ['a', 'b', 'c', 'd'] })
    expect(wheels.server.S1[0].options).toEqual(['a', 'b', 'c', 'd'])
    await flush()
    expect(invoke.mock.calls.filter(([c]) => c === 'update_wheel')).toHaveLength(1) // the second waits
    answers[0]({ ...w, options: ['a', 'b', 'c'] })
    expect(await first).toBe('')
    expect(wheels.server.S1[0].options).toEqual(['a', 'b', 'c', 'd']) // the older answer doesn't undo the newer change
    await flush()
    answers[1]({ ...w, options: ['a', 'b', 'c', 'd'] })
    expect(await second).toBe('')
    expect(wheels.server.S1[0].options).toEqual(['a', 'b', 'c', 'd'])
  })

  it('a refusal says why and fetches the list fresh', async () => {
    const r = saveWheel(w, { name: '' })
    await flush()
    fails[0]('wheel names are 1-40 characters')
    expect(await r).toBe('wheel names are 1-40 characters')
    expect(wheels.server.S1[0].options).toEqual(['from', 'server'])
  })

  it('a spin can wait for every change to land', async () => {
    let done = false
    saveWheel(w, { options: ['x', 'y'] })
    const settled = savesSettled().then(() => (done = true))
    await flush()
    expect(done).toBe(false)
    answers[0]({ ...w, options: ['x', 'y'] })
    await settled
    expect(done).toBe(true)
  })
})

describe('quiet spins (#270)', () => {
  const q = (id: string, channel_id = 'C1') => ({ id, channel_id, spin: { spinner_id: 'U1', wheel_id: null, name: null, options: ['a', 'b'], winner: 1, angle: 100 } })

  it('one arriving (or our own answer) shows in its chat once, then goes on its own', () => {
    vi.useFakeTimers()
    try {
      handleWheelEvent({ t: 'WheelSpunQuietly', d: { spin: q('Q1') } })
      handleWheelEvent({ t: 'WheelSpunQuietly', d: { spin: q('Q1') } })
      handleWheelEvent({ t: 'WheelSpunQuietly', d: { spin: q('Q2', 'C2') } })
      expect(quietSpins.C1.map((c) => c.spin.id)).toEqual(['Q1'])
      expect(quietSpins.C2.map((c) => c.spin.id)).toEqual(['Q2'])
      vi.advanceTimersByTime(QUIET_CARD_MS - 1)
      expect(quietSpins.C1).toHaveLength(1)
      vi.advanceTimersByTime(1)
      expect(quietSpins.C1 ?? []).toEqual([])
    } finally {
      vi.useRealTimers()
    }
  })

  it('stays while it is held (hovered), and goes a moment after it is let go', () => {
    vi.useFakeTimers()
    try {
      handleWheelEvent({ t: 'WheelSpunQuietly', d: { spin: q('Q1') } })
      holdQuietSpin('Q1', true)
      vi.advanceTimersByTime(QUIET_CARD_MS * 3)
      expect(quietSpins.C1).toHaveLength(1)
      holdQuietSpin('Q1', false)
      vi.advanceTimersByTime(QUIET_LINGER_MS)
      expect(quietSpins.C1 ?? []).toEqual([])
    } finally {
      vi.useRealTimers()
    }
  })

  it('spinning quietly shows ours at once and tells the chat we spun', async () => {
    invoke.mockImplementationOnce(async () => q('Q7'))
    const spun: string[] = []
    const off = onOwnSpin((c) => spun.push(c))
    await spinQuietly('C1', { options: ['a', 'b'] })
    off()
    expect(invoke).toHaveBeenCalledWith('spin_wheel_quietly', { channelId: 'C1', request: { options: ['a', 'b'] } })
    expect(quietSpins.C1.map((c) => c.spin.id)).toEqual(['Q7'])
    expect(spun).toEqual(['C1'])
  })

  it('"Spin quietly" is remembered on this device', () => {
    setSpinQuietly(true)
    expect(wheelPrefs.quiet).toBe(true)
    expect(localStorage.getItem('pulse.wheelSpinQuietly')).toBe('true')
    setSpinQuietly(false)
    expect(wheelPrefs.quiet).toBe(false)
  })
})

describe('your own spins (#270: the chat jumps to them)', () => {
  it('a spin you start tells the chat; listeners can stop listening', async () => {
    const line = { id: 'L9', channel_id: 'C3', wheel: { spinner_id: 'U1', wheel_id: null, name: null, options: ['a', 'b'], winner: 0, angle: 10 } }
    invoke.mockImplementation(async (cmd: string) => (cmd === 'spin_wheel' || cmd === 'spin_again' ? line : undefined))
    const spun: string[] = []
    const off = onOwnSpin((c) => spun.push(c))
    await spinWheel('C3', { options: ['a', 'b'] })
    await spinAgain('L1', false)
    off()
    await spinWheel('C3', { options: ['a', 'b'] })
    expect(spun).toEqual(['C3', 'C3'])
  })
})

describe('live spins', () => {
  it('a spin is live for a second after it reaches this client', () => {
    const m = { id: 'L1', wheel: { spinner_id: null, wheel_id: null, name: null, options: ['a', 'b'], winner: 0, angle: 10 } } as never
    noteSpin(m, 1000)
    expect(spunLive('L1', 1500)).toBe(true)
    expect(spunLive('L1', 2100)).toBe(false)
    expect(spunLive('L2', 1500)).toBe(false)
  })
})
