// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invoke(cmd, args) }))

import VoiceSettings from './VoiceSettings.svelte'
import { voice } from '../lib/voice.svelte'

const meterCalls = () => invoke.mock.calls.filter(([cmd]) => cmd === 'watch_voice_meters').map(([, a]) => a)

describe('VoiceSettings meter subscription', () => {
  beforeEach(() => invoke.mockClear())

  it('asks for voice levels while open and stops when closed', () => {
    const c = mount(VoiceSettings, { target: document.body })
    flushSync()
    expect(meterCalls()).toEqual([{ on: true }])
    unmount(c)
    expect(meterCalls()).toEqual([{ on: true }, { on: false }])
  })

  it('every open is paired with one close, so reopening keeps the meter alive', () => {
    for (let i = 0; i < 3; i++) {
      const c = mount(VoiceSettings, { target: document.body })
      flushSync()
      unmount(c)
    }
    expect(meterCalls()).toEqual([{ on: true }, { on: false }, { on: true }, { on: false }, { on: true }, { on: false }])
  })

  it('reopening starts the meter from zero, not from the level last seen before it closed', () => {
    voice.levels = { mic: 0.2, speaker: 0.5 }
    voice.gateOpen = true
    const c = mount(VoiceSettings, { target: document.body })
    try {
      flushSync()
      const fill = document.querySelector<HTMLElement>('.meter .fill')!
      expect(fill.style.width).toBe('0%')
      expect(fill.classList.contains('over')).toBe(false)
    } finally {
      unmount(c)
    }
  })
})

describe('VoiceSettings noise suppression', () => {
  beforeEach(() => invoke.mockClear())

  it('choosing a level marks it and applies it', async () => {
    const c = mount(VoiceSettings, { target: document.body })
    flushSync()
    const radios = () => [...document.querySelectorAll<HTMLButtonElement>('[role=radiogroup][aria-label="Noise suppression"] [role=radio]')]
    expect(radios().map((r) => r.textContent)).toEqual(['Off', 'Standard', 'Strong'])
    radios()[2].click()
    flushSync()
    await Promise.resolve()
    expect(radios().map((r) => r.getAttribute('aria-checked'))).toEqual(['false', 'false', 'true'])
    const applied = invoke.mock.calls.filter(([cmd]) => cmd === 'set_audio_config').at(-1)?.[1] as { config?: { noise_suppression?: string } } | undefined
    expect(JSON.stringify(applied)).toContain('"noise_suppression":"strong"')
    unmount(c)
    document.body.innerHTML = ''
  })
})

describe('VoiceSettings sounds', () => {
  it('renders the Sounds section', () => {
    const c = mount(VoiceSettings, { target: document.body })
    flushSync()
    expect(document.querySelectorAll('h2').length).toBe(1)
    expect([...document.querySelectorAll('h3')].map((h) => h.textContent)).toContain('Sounds')
    expect(document.querySelector('[role=radiogroup][aria-label="Sound set"]')).not.toBeNull()
    unmount(c)
    document.body.innerHTML = ''
  })
})

describe('VoiceSettings on Linux', () => {
  const ua = Object.getOwnPropertyDescriptor(window.navigator, 'userAgent')
  const asLinux = () => Object.defineProperty(window.navigator, 'userAgent', { value: 'Mozilla/5.0 (X11; Linux x86_64)', configurable: true })
  const restore = () => {
    if (ua) Object.defineProperty(window.navigator, 'userAgent', ua)
    else delete (window.navigator as { userAgent?: string }).userAgent
  }
  const devices = (inputs: string[], outputs: string[]) =>
    invoke.mockImplementation(async (cmd: string) =>
      cmd === 'list_audio_devices'
        ? { inputs: inputs.map((name) => ({ name, is_default: false })), outputs: outputs.map((name) => ({ name, is_default: false })) }
        : undefined,
    )
  const settle = async () => {
    for (let i = 0; i < 3; i++) await Promise.resolve()
    flushSync()
  }
  const hint = () => [...document.querySelectorAll('.hint')].find((p) => p.textContent?.includes('system sound settings'))

  it('lists the sound server devices and drops the system-settings hint', async () => {
    asLinux()
    devices(['Laptop Mic'], ['Earbuds'])
    const c = mount(VoiceSettings, { target: document.body })
    await settle()
    expect(hint()).toBeUndefined()
    unmount(c)
    document.body.innerHTML = ''
    restore()
    invoke.mockImplementation(async () => undefined)
  })

  it('without a sound server (only ALSA default) the hint still says to use system settings', async () => {
    asLinux()
    devices(['default'], ['default'])
    const c = mount(VoiceSettings, { target: document.body })
    await settle()
    expect(hint()).toBeDefined()
    unmount(c)
    document.body.innerHTML = ''
    restore()
    invoke.mockImplementation(async () => undefined)
  })
})
