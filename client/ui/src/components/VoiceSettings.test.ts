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
      expect(fill.style.transform).toBe('scaleX(0)')
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
    expect(radios().map((r) => r.textContent)).toEqual(['Off', 'Standard', 'Strong', 'Strong (low delay)'])
    radios()[2].click()
    flushSync()
    await Promise.resolve()
    expect(radios().map((r) => r.getAttribute('aria-checked'))).toEqual(['false', 'false', 'true', 'false'])
    const applied = invoke.mock.calls.filter(([cmd]) => cmd === 'set_audio_config').at(-1)?.[1] as { config?: { noise_suppression?: string } } | undefined
    expect(JSON.stringify(applied)).toContain('"noise_suppression":"strong"')
    unmount(c)
    document.body.innerHTML = ''
  })
})

describe('VoiceSettings Strong (low delay) (#77)', () => {
  type Status = { state: string; downloaded?: number; total?: number | null; error?: string }
  let status: Status
  let finish: ((s: Status) => void) | null
  beforeEach(() => {
    invoke.mockClear()
    localStorage.clear()
    status = { state: 'missing' }
    finish = null
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'model_status') return status
      if (cmd === 'model_download') {
        status = { state: 'downloading', downloaded: 0, total: null }
        return new Promise<Status>((resolve) => (finish = (s) => resolve((status = s))))
      }
      return undefined
    })
  })
  const settle = async () => {
    for (let i = 0; i < 6; i++) await Promise.resolve()
    flushSync()
  }
  const radios = () => [...document.querySelectorAll<HTMLButtonElement>('[role=radiogroup][aria-label="Noise suppression"] [role=radio]')]
  const calls = (name: string) => invoke.mock.calls.filter(([cmd]) => cmd === name)
  const text = () => document.body.textContent ?? ''
  const done = (c: ReturnType<typeof mount>) => {
    unmount(c)
    document.body.innerHTML = ''
    localStorage.clear()
    invoke.mockImplementation(async () => undefined)
  }

  it('picking it applies it, starts the download, shows progress, and the line goes once ready', async () => {
    const c = mount(VoiceSettings, { target: document.body })
    await settle()
    expect(text()).not.toContain('Less delay')
    radios()[3].click()
    await settle()
    expect(radios().map((r) => r.getAttribute('aria-checked'))).toEqual(['false', 'false', 'false', 'true'])
    const applied = calls('set_audio_config').at(-1)?.[1]
    expect(JSON.stringify(applied)).toContain('"noise_suppression":"strong_low_delay"')
    expect(text()).toContain('Less delay, uses more CPU and memory.')
    expect(calls('model_download')).toHaveLength(1)
    expect(document.querySelector('[data-model-state]')?.textContent).toContain('Downloading…')
    finish!({ state: 'ready' })
    await settle()
    expect(document.querySelector('[data-model-state]')).toBeNull()
    expect(text()).toContain('Less delay, uses more CPU and memory.')
    done(c)
  })

  it('a failed download says why, and Retry downloads again', async () => {
    const c = mount(VoiceSettings, { target: document.body })
    await settle()
    radios()[3].click()
    await settle()
    finish!({ state: 'failed', error: "couldn't reach the server" })
    await settle()
    const line = document.querySelector('[data-model-state="failed"]')
    expect(line?.textContent).toContain("Couldn't download it: couldn't reach the server")
    const retry = [...line!.querySelectorAll('button')].find((b) => b.textContent === 'Retry')!
    retry.click()
    await settle()
    expect(calls('model_download')).toHaveLength(2)
    expect(document.querySelector('[data-model-state]')?.textContent).toContain('Downloading…')
    done(c)
  })

  it('already downloaded: picking it downloads nothing', async () => {
    status = { state: 'ready' }
    const c = mount(VoiceSettings, { target: document.body })
    await settle()
    radios()[3].click()
    await settle()
    expect(calls('model_download')).toHaveLength(0)
    expect(document.querySelector('[data-model-state]')).toBeNull()
    done(c)
  })

  it('chosen earlier but not there (deleted, or damaged): it says so, with a Download button', async () => {
    localStorage.setItem('pulse.audio', JSON.stringify({ noise_suppression: 'strong_low_delay' }))
    const c = mount(VoiceSettings, { target: document.body })
    await settle()
    const line = document.querySelector('[data-model-state="missing"]')
    expect(line?.textContent).toContain('Not downloaded yet')
    expect(calls('model_download'), 'not without asking').toHaveLength(0)
    ;[...line!.querySelectorAll('button')].find((b) => b.textContent === 'Download')!.click()
    await settle()
    expect(calls('model_download')).toHaveLength(1)
    done(c)
  })
})

describe('VoiceSettings echo cancellation (#76)', () => {
  beforeEach(() => {
    invoke.mockClear()
    localStorage.clear()
  })

  it('Auto / On / Off, Auto by default; choosing one applies it', async () => {
    const c = mount(VoiceSettings, { target: document.body })
    flushSync()
    const radios = () => [...document.querySelectorAll<HTMLButtonElement>('[role=radiogroup][aria-label="Echo cancellation"] [role=radio]')]
    expect(radios().map((r) => r.textContent)).toEqual(['Auto', 'On', 'Off'])
    expect(radios().map((r) => r.getAttribute('aria-checked'))).toEqual(['true', 'false', 'false'])
    expect(document.body.textContent).toContain('Auto turns it off on headphones and headsets')
    // no on/off switch for it any more
    expect(document.querySelector('button.switch[aria-label="Echo cancellation"]')).toBeNull()
    radios()[2].click()
    flushSync()
    await Promise.resolve()
    expect(radios().map((r) => r.getAttribute('aria-checked'))).toEqual(['false', 'false', 'true'])
    const applied = invoke.mock.calls.filter(([cmd]) => cmd === 'set_audio_config').at(-1)?.[1]
    expect(JSON.stringify(applied)).toContain('"echo_mode":"off"')
    expect(JSON.stringify(applied)).not.toContain('echo_cancel')
    unmount(c)
    document.body.innerHTML = ''
  })

  it('under Auto, what Auto decided for the output, from the core (#172)', () => {
    voice.echo = { mode: 'auto', running: false, output: 'headphones' }
    const c = mount(VoiceSettings, { target: document.body })
    flushSync()
    const line = () => document.querySelector('[data-echo-decision]')?.textContent?.trim() ?? null
    expect(line()).toBe('Off for now: your output is headphones or a headset')
    voice.echo = { mode: 'auto', running: true, output: 'unknown' }
    flushSync()
    expect(line()).toBe("On: your output doesn't say it's headphones")
    const radios = () => [...document.querySelectorAll<HTMLButtonElement>('[role=radiogroup][aria-label="Echo cancellation"] [role=radio]')]
    radios()[1].click()
    flushSync()
    expect(line(), 'On chosen: nothing to explain').toBeNull()
    unmount(c)
    document.body.innerHTML = ''
    voice.echo = null
  })
})

describe('VoiceSettings helper text (#163)', () => {
  it('input gain, input sensitivity and noise suppression keep their labels and controls but no description', () => {
    const c = mount(VoiceSettings, { target: document.body })
    flushSync()
    const text = document.body.textContent ?? ''
    for (const gone of ['boosts your voice', 'hears you speaking', 'right of the handle', 'to see the bar move', 'to see your level', 'your mic as it is', 'removes fans', 'Uses more CPU'])
      expect(text).not.toContain(gone)
    for (const kept of ['INPUT GAIN', 'INPUT SENSITIVITY', 'NOISE SUPPRESSION', 'Automatically determine sensitivity']) expect(text).toContain(kept)
    expect(document.querySelector('[role=radiogroup][aria-label="Noise suppression"]')).not.toBeNull()
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
