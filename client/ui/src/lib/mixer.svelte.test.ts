// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invoke(cmd, args) }))

import { mixer, pushCallMix, reloadMixer, resetMixerSection, setClipsMuted, setNotch } from './mixer.svelte'
import { MIXER_KEY, defaultMixer } from './mixer'

const stored = () => JSON.parse(localStorage.getItem(MIXER_KEY)!)
const callMixes = () => invoke.mock.calls.filter(([c]) => c === 'set_call_mix').map(([, a]) => a)

beforeEach(() => {
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

  it('Calls changes go to the core; the soundboard notch and other sections do not', () => {
    setNotch('calls', 'master', 50)
    setNotch('calls', 'streams', 150)
    setNotch('calls', 'soundboard', 20)
    setNotch('callSounds', 'master', 20)
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

  it('reset puts one section back and tells the core when it is Calls', () => {
    setNotch('calls', 'voices', 10)
    setNotch('callSounds', 'leave', 0)
    invoke.mockClear()
    resetMixerSection('callSounds')
    expect(mixer.callSounds.leave).toBe(100)
    expect(mixer.calls.voices).toBe(10)
    expect(callMixes()).toEqual([])
    resetMixerSection('calls')
    expect(stored().calls).toEqual(defaultMixer().calls)
    expect(callMixes()).toEqual([{ callsPercent: 100, voicesPercent: 100, streamsPercent: 100 }])
  })

  it('clips mute is kept with the record', () => {
    setClipsMuted(true)
    expect(stored().media.clipsMuted).toBe(true)
  })
})
