// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invoke(cmd, args) }))

import { mixer, pushCallMix, reloadMixer, resetMixerSection, setClipsMuted, setNotch } from './mixer.svelte'
import { MIXER_KEY, defaultMixer } from './mixer'

const stored = () => JSON.parse(localStorage.getItem(MIXER_KEY)!)
const settle = () => new Promise((r) => setTimeout(r, 0))
const callMixes = () => invoke.mock.calls.filter(([c]) => c === 'set_call_mix').map(([, a]) => a)

beforeEach(async () => {
  await settle() // the last test's push has landed
  localStorage.clear()
  reloadMixer()
  invoke.mockClear()
})

describe('mixer state', () => {
  it('setNotch clamps, rounds, saves', () => {
    setNotch('callSounds', 'join', 250.4)
    expect(mixer.callSounds.join).toBe(200)
    setNotch('notifications', 'master', 33.6)
    expect(stored().notifications.master).toBe(34)
    setNotch('media', 'clips', 180)
    expect(mixer.media.clips).toBe(100) // a <video> can't go above its own full volume
    setNotch('calls', 'voices', -5)
    expect(mixer.calls.voices).toBe(0)
  })

  it('an unknown notch is ignored', () => {
    setNotch('calls', 'nope', 10)
    setNotch('calls', '__proto__', 10)
    expect(stored()).toEqual(defaultMixer())
  })

  it('Calls changes go to the core; the soundboard notch and other sections do not', async () => {
    setNotch('calls', 'master', 50)
    await settle()
    setNotch('calls', 'streams', 150)
    await settle()
    setNotch('calls', 'soundboard', 20)
    setNotch('callSounds', 'master', 20)
    await settle()
    expect(callMixes()).toEqual([
      { callsPercent: 50, voicesPercent: 100, streamsPercent: 100 },
      { callsPercent: 50, voicesPercent: 100, streamsPercent: 150 },
    ])
  })

  it('pushCallMix sends the current Calls levels', async () => {
    mixer.calls.voices = 70
    await pushCallMix()
    expect(callMixes()).toEqual([{ callsPercent: 100, voicesPercent: 70, streamsPercent: 100 }])
  })

  it('reset puts one section back and tells the core when it is Calls', async () => {
    setNotch('calls', 'voices', 10)
    setNotch('callSounds', 'leave', 0)
    await settle()
    invoke.mockClear()
    resetMixerSection('callSounds')
    expect(mixer.callSounds.leave).toBe(100)
    expect(mixer.calls.voices).toBe(10)
    expect(callMixes()).toEqual([])
    resetMixerSection('calls')
    expect(stored().calls).toEqual(defaultMixer().calls)
    await settle()
    expect(callMixes()).toEqual([{ callsPercent: 100, voicesPercent: 100, streamsPercent: 100 }])
  })

  it('a drag sends one push at a time and ends on the latest levels', async () => {
    const pending: Array<() => void> = []
    invoke.mockImplementation((c: string) => (c === 'set_call_mix' ? new Promise<undefined>((r) => pending.push(() => r(undefined))) : Promise.resolve(undefined)))
    setNotch('calls', 'voices', 10)
    setNotch('calls', 'voices', 20)
    setNotch('calls', 'voices', 30)
    expect(callMixes()).toEqual([{ callsPercent: 100, voicesPercent: 10, streamsPercent: 100 }])
    pending.shift()!()
    await vi.waitFor(() => expect(callMixes()).toHaveLength(2))
    expect(callMixes()[1]).toEqual({ callsPercent: 100, voicesPercent: 30, streamsPercent: 100 })
    pending.shift()!()
    await new Promise((r) => setTimeout(r, 0))
    expect(callMixes()).toHaveLength(2) // nothing newer to send
    invoke.mockImplementation(async () => undefined)
  })

  it('a failed push does not stop the next one', async () => {
    invoke.mockImplementationOnce(async () => { throw new Error('no') })
    setNotch('calls', 'streams', 40)
    await new Promise((r) => setTimeout(r, 0))
    setNotch('calls', 'streams', 50)
    await vi.waitFor(() => expect(callMixes()).toHaveLength(2))
    expect(callMixes()[1]).toEqual({ callsPercent: 100, voicesPercent: 100, streamsPercent: 50 })
  })

  it('clips mute is kept with the record', () => {
    setClipsMuted(true)
    expect(stored().media.clipsMuted).toBe(true)
  })
})
