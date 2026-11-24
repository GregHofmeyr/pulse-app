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
    expect(document.body.textContent).toContain('Auto keeps it on for every output, headsets included')
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

  it('under Auto, a line only if the core reports it off (#172, #354)', () => {
    voice.echo = { mode: 'auto', running: false, output: 'headphones' }
    const c = mount(VoiceSettings, { target: document.body })
    flushSync()
    const line = () => document.querySelector('[data-echo-decision]')?.textContent?.trim() ?? null
    expect(line()).toBe('Off for now. Hearing echo complaints? Choose On')
    voice.echo = { mode: 'auto', running: true, output: 'headphones' }
    flushSync()
    expect(line(), 'on, as Auto always is now: the help text says it').toBeNull()
    voice.echo = { mode: 'auto', running: false, output: 'headphones' }
    flushSync()
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

describe('VoiceSettings even out voice volumes (#355)', () => {
  beforeEach(() => {
    invoke.mockClear()
    localStorage.clear()
  })

  it('on by default; turning it off applies and saves it', async () => {
    const c = mount(VoiceSettings, { target: document.body })
    flushSync()
    const sw = () => document.querySelector<HTMLButtonElement>('button.switch[aria-label="Even out voice volumes"]')!
    expect(sw().getAttribute('aria-pressed')).toBe('true')
    expect(document.body.textContent).toContain("Each person's own volume still applies on top")
    sw().click()
    flushSync()
    await Promise.resolve()
    expect(sw().getAttribute('aria-pressed')).toBe('false')
    const applied = invoke.mock.calls.filter(([cmd]) => cmd === 'set_audio_config').at(-1)?.[1]
    expect(JSON.stringify(applied)).toContain('"even_out_voices":false')
    expect(localStorage.getItem('pulse.audio')).toContain('"even_out_voices":false')
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
  it("a list that failed (the system took too long, #353) doesn't claim there's no sound server", async () => {
    asLinux()
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_audio_devices') throw 'the system took too long to list audio devices'
      return undefined
    })
    const c = mount(VoiceSettings, { target: document.body })
    await settle()
    expect(hint()).toBeUndefined()
    unmount(c)
    document.body.innerHTML = ''
    restore()
    invoke.mockImplementation(async () => undefined)
  })
})

describe('VoiceSettings mic test during a call', () => {
  beforeEach(() => {
    invoke.mockClear()
    invoke.mockImplementation(async () => undefined)
    voice.channelId = null
    voice.joining = null
  })
  const button = () => [...document.querySelectorAll<HTMLButtonElement>('.check button')][0]
  const hint = () => document.querySelector('[data-mic-test-hint]')?.textContent
  const settle = async () => {
    for (let i = 0; i < 3; i++) await Promise.resolve()
    flushSync()
  }

  it('in a call the test is off, with a hint to leave the call first', () => {
    voice.channelId = 'c1'
    const c = mount(VoiceSettings, { target: document.body })
    try {
      flushSync()
      expect(button().disabled).toBe(true)
      expect(hint()).toBe('Leave the call to test your mic.')
      button().click()
      expect(invoke.mock.calls.some(([cmd]) => cmd === 'start_mic_test')).toBe(false)
    } finally {
      unmount(c)
      document.body.innerHTML = ''
      voice.channelId = null
    }
  })

  it('while joining too', () => {
    voice.joining = 'c1'
    const c = mount(VoiceSettings, { target: document.body })
    try {
      flushSync()
      expect(button().disabled).toBe(true)
    } finally {
      unmount(c)
      document.body.innerHTML = ''
      voice.joining = null
    }
  })

  it('out of a call it runs; joining a call ends it (the core stops it) and the button comes back', async () => {
    const c = mount(VoiceSettings, { target: document.body })
    try {
      flushSync()
      expect(button().disabled).toBe(false)
      expect(hint()).toBe('Hear your mic the way others will.')
      button().click()
      await settle()
      expect(invoke.mock.calls.some(([cmd]) => cmd === 'start_mic_test')).toBe(true)
      expect(button().textContent).toBe('Stop')
      voice.channelId = 'c1'
      flushSync()
      expect(button().textContent).toBe("Let's check")
      expect(button().disabled).toBe(true)
    } finally {
      unmount(c)
      document.body.innerHTML = ''
      voice.channelId = null
    }
  })
})

describe('VoiceSettings record a mic sample (#583)', () => {
  beforeEach(() => {
    invoke.mockClear()
    voice.channelId = null
    voice.joining = null
  })
  const button = () => document.querySelector<HTMLButtonElement>('.sample button')!
  const hint = () => document.querySelector('[data-mic-sample-hint]')?.textContent
  const settle = async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve()
    flushSync()
  }

  it('needs the mic running: off outside a call and the test, with a hint', () => {
    invoke.mockImplementation(async () => undefined)
    const c = mount(VoiceSettings, { target: document.body })
    try {
      flushSync()
      expect(button().disabled).toBe(true)
      expect(hint()).toContain('Join a call')
    } finally {
      unmount(c)
      document.body.innerHTML = ''
    }
  })

  it('in a call it records, then shows where both files went and can show the folder', async () => {
    let finish: (v: unknown) => void = () => {}
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'record_mic_sample') return new Promise((resolve) => (finish = resolve))
      return undefined
    })
    voice.channelId = 'c1'
    const c = mount(VoiceSettings, { target: document.body })
    try {
      flushSync()
      expect(button().disabled).toBe(false)
      button().click()
      await settle()
      expect(button().textContent).toBe('Recording…')
      expect(button().disabled).toBe(true)
      finish({ raw: '/logs/mic-sample-raw.wav', sent: '/logs/mic-sample-sent.wav', folder: '/logs', seconds: 15 })
      await settle()
      const saved = document.querySelector('[data-mic-sample-saved]')!
      expect(saved.textContent).toContain('/logs')
      expect(saved.textContent).toContain('mic-sample-sent.wav')
      saved.querySelector('button')!.click()
      await settle()
      expect(invoke).toHaveBeenCalledWith('reveal_path', { path: '/logs/mic-sample-raw.wav' })
    } finally {
      unmount(c)
      document.body.innerHTML = ''
      voice.channelId = null
    }
  })

  it('a failure says why', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'record_mic_sample') throw 'No audio from your mic.'
      return undefined
    })
    voice.channelId = 'c1'
    const c = mount(VoiceSettings, { target: document.body })
    try {
      flushSync()
      button().click()
      await settle()
      expect(document.querySelector('[role=alert]')?.textContent).toBe('No audio from your mic.')
      expect(button().textContent).toBe('Record a mic sample')
    } finally {
      unmount(c)
      document.body.innerHTML = ''
      voice.channelId = null
    }
  })
})
