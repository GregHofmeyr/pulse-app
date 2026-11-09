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

import { noteSpin, resetWheels, saveWheel, savesSettled, spunLive, wheels } from './wheels.svelte'

const flush = () => new Promise((r) => setTimeout(r, 0))

beforeEach(() => {
  resetWheels()
  invoke.mockClear()
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

describe('live spins', () => {
  it('a spin is live for a second after it reaches this client', () => {
    const m = { id: 'L1', wheel: { spinner_id: null, wheel_id: null, name: null, options: ['a', 'b'], winner: 0, angle: 10 } } as never
    noteSpin(m, 1000)
    expect(spunLive('L1', 1500)).toBe(true)
    expect(spunLive('L1', 2100)).toBe(false)
    expect(spunLive('L2', 1500)).toBe(false)
  })
})
