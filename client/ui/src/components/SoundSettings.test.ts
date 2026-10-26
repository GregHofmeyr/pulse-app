// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invoke(cmd, args) }))

import SoundSettings from './SoundSettings.svelte'
import { SOUND_SETS } from '../lib/voiceui'

const LABELS = ['Classic', 'Two-step · Bright', 'Two-step · Warm', 'Two-step · Deep', 'Minimal · Soft', 'Minimal · Wood', 'Minimal · Glass', 'Goofy']
let cmp: ReturnType<typeof mount> | undefined
const show = () => {
  cmp = mount(SoundSettings, { target: document.body })
  flushSync()
}
const radios = () => [...document.querySelectorAll<HTMLElement>('[role=radiogroup][aria-label="Sound set"] [role=radio]')]
const radio = (label: string) => radios().find((r) => r.textContent?.includes(label))!
const stored = () => JSON.parse(localStorage.getItem('pulse.sounds') ?? 'null')
const plays = () => invoke.mock.calls.filter(([c]) => c === 'play_sound').map(([, a]) => a as Record<string, unknown>)

beforeEach(() => {
  vi.useFakeTimers()
  invoke.mockClear()
  localStorage.clear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('SoundSettings', () => {
  it('renders the 8 sets with Classic checked by default', () => {
    show()
    expect(radios().length).toBe(8)
    expect(radios().map((r, i) => r.textContent?.includes(LABELS[i]))).toEqual(Array(8).fill(true))
    expect(radios().map((r) => r.getAttribute('aria-checked'))).toEqual(['true', ...Array(7).fill('false')])
    expect(document.querySelector('h3')?.textContent).toBe('Sounds')
  })

  it('clicking a row selects and saves it', () => {
    show()
    radio('Two-step · Warm').click()
    flushSync()
    expect(radio('Two-step · Warm').getAttribute('aria-checked')).toBe('true')
    expect(radio('Classic').getAttribute('aria-checked')).toBe('false')
    expect(stored().set).toBe('twostep-warm')
  })

  it('Space and Enter on a row select it', () => {
    show()
    radio('Minimal · Wood').dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true }))
    flushSync()
    expect(stored().set).toBe('minimal-wood')
    radio('Goofy').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    flushSync()
    expect(stored().set).toBe('goofy')
  })

  it('the preview button plays the set at once and leaves the selection alone', () => {
    show()
    document.querySelector<HTMLButtonElement>('button[aria-label="Preview Goofy"]')!.click()
    expect(plays()[0]).toMatchObject({ set: 'goofy', name: 'join' })
    flushSync()
    expect(radio('Classic').getAttribute('aria-checked')).toBe('true')
    expect(stored()).toBeNull()
  })

  it('the volume slider saves and shows the percentage', () => {
    show()
    const input = document.querySelector<HTMLInputElement>('input[type=range][aria-label="Sound volume"]')!
    expect(input.min).toBe('0')
    expect(input.max).toBe('100')
    input.value = '40'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    input.dispatchEvent(new Event('change', { bubbles: true }))
    flushSync()
    expect(stored().volume_pct).toBe(40)
    expect(document.body.textContent).toContain('40%')
  })

  it('event switches add to and remove from off', () => {
    show()
    const sw = () => document.querySelector<HTMLButtonElement>('button[aria-label="A stream ends"]')!
    expect(sw().getAttribute('aria-pressed')).toBe('true')
    sw().click()
    flushSync()
    expect(stored().off).toEqual(['live_end'])
    expect(sw().getAttribute('aria-pressed')).toBe('false')
    sw().click()
    flushSync()
    expect(stored().off).toEqual([])
  })

  it('unmounting stops a running preview', () => {
    show()
    document.querySelector<HTMLButtonElement>('button[aria-label="Preview Classic"]')!.click()
    expect(plays().length).toBe(1)
    unmount(cmp!)
    cmp = undefined
    vi.advanceTimersByTime(5000)
    expect(plays().length).toBe(1)
  })

  it('is a roving-tabindex radiogroup: one tab stop, arrows select, save, focus and wrap', () => {
    show()
    expect(radios().filter((r) => r.tabIndex === 0).length).toBe(1)
    expect(radio('Classic').tabIndex).toBe(0)
    const key = (el: HTMLElement, k: string) => el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }))
    key(radio('Classic'), 'ArrowDown')
    flushSync()
    expect(radio('Two-step · Bright').getAttribute('aria-checked')).toBe('true')
    expect(stored().set).toBe('twostep-bright')
    expect(document.activeElement).toBe(radio('Two-step · Bright'))
    expect(radios().filter((r) => r.tabIndex === 0)).toEqual([radio('Two-step · Bright')])
    key(radio('Two-step · Bright'), 'ArrowUp')
    flushSync()
    key(radio('Classic'), 'ArrowUp')
    flushSync()
    expect(stored().set).toBe('goofy')
    expect(document.activeElement).toBe(radio('Goofy'))
    key(radio('Goofy'), 'ArrowRight')
    flushSync()
    expect(stored().set).toBe('classic')
  })

  it('ArrowLeft from Classic wraps to Goofy: selected, saved and focused', () => {
    show()
    radio('Classic').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true }))
    flushSync()
    expect(radio('Goofy').getAttribute('aria-checked')).toBe('true')
    expect(radio('Classic').getAttribute('aria-checked')).toBe('false')
    expect(stored().set).toBe('goofy')
    expect(document.activeElement).toBe(radio('Goofy'))
  })

  it('a key named like an Object.prototype member does nothing and does not throw', () => {
    show()
    const errors: unknown[] = []
    const onError = (e: ErrorEvent) => {
      errors.push(e.error)
      e.preventDefault()
    }
    window.addEventListener('error', onError)
    try {
      for (const k of ['toString', 'constructor', 'hasOwnProperty', '__proto__']) {
        const ev = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true })
        radio('Classic').dispatchEvent(ev)
        flushSync()
        expect(ev.defaultPrevented, k).toBe(false)
      }
    } finally {
      window.removeEventListener('error', onError)
    }
    expect(errors).toEqual([])
    expect(stored()).toBeNull()
    expect(radio('Classic').getAttribute('aria-checked')).toBe('true')
  })

  it('names each radio by its label and describes it with the blurb', () => {
    show()
    const r = radio('Goofy')
    expect(r.getAttribute('aria-label')).toBe('Goofy')
    const desc = document.getElementById(r.getAttribute('aria-describedby')!)
    expect(desc?.textContent).toBe(SOUND_SETS.find((s) => s.id === 'goofy')!.blurb)
    expect(new Set(radios().map((x) => x.getAttribute('aria-describedby'))).size).toBe(8)
  })

  it('keeps the volume hint outside the labelled control', () => {
    show()
    const input = document.querySelector('input[aria-label="Sound volume"]')!
    expect(input.closest('label')).toBeNull()
  })
})
