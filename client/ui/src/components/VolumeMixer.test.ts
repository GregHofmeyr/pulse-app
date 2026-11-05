// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Sound } from '../lib/protocol/Sound'

const invoke = vi.hoisted(() => vi.fn(async (_c: string, _a?: unknown): Promise<unknown> => undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

import VolumeMixer from './VolumeMixer.svelte'
import { defaultMixer, mixGain } from '../lib/mixer'
import { mixer, reloadMixer } from '../lib/mixer.svelte'
import { reloadMixerCollapsed } from '../lib/mixerCollapsed.svelte'
import { setSoundboardVolume } from '../lib/soundboard.svelte'
import { app } from '../lib/store.svelte'
import { emptyState, type AppState } from '../lib/state'

const me = { id: 'ME', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const snd = (id: string, name: string): Sound => ({
  id, server_id: 'S1', name, emoji: null, emoji_id: null, media_hash: 'h' + id, volume: 120, duration_ms: 1000, added_by: null,
  created_at: '2026-10-01T00:00:00.000Z', plays: 0,
})
function withSounds(sounds: Sound[]) {
  app.state = {
    ...emptyState(), me,
    servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
    members: { S1: [{ user: me }] } as unknown as AppState['members'],
    sounds: Object.fromEntries(sounds.map((s) => [s.id, s])),
  } as AppState
}

let cmp: ReturnType<typeof mount> | undefined
const show = () => {
  cmp = mount(VolumeMixer, { target: document.body })
  flushSync()
}
const slider = (label: string) => document.querySelector<HTMLInputElement>(`input[type=range][aria-label="${label}"]`)!
const move = (label: string, v: number) => {
  const r = slider(label)
  r.value = String(v)
  r.dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}
const button = (label: string) => document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!
const calls = (c: string) => invoke.mock.calls.filter((x) => x[0] === c).map((x) => x[1])
const stored = () => JSON.parse(localStorage.getItem('pulse.mixer')!)
const section = (title: string) => [...document.querySelectorAll<HTMLElement>('.mix-section')].find((s) => s.querySelector('h4')?.textContent === title)!

beforeEach(() => {
  localStorage.clear()
  reloadMixer()
  reloadMixerCollapsed()
  invoke.mockClear()
  withSounds([])
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('VolumeMixer', () => {
  it('shows the sections in order under one heading', () => {
    show()
    expect(document.querySelector('h3')?.textContent).toBe('Volume mixer')
    expect([...document.querySelectorAll('.mix-section h4')].map((h) => h.textContent)).toEqual(
      ['Calls', 'Call sounds', 'Stream sounds', 'Notifications', 'Media'])
  })

  it('every notch is a labelled slider showing its percent; masters for all but Media', () => {
    show()
    for (const l of ['Calls', 'Call sounds', 'Stream sounds', 'Notifications']) {
      const r = slider(`${l} volume`)
      expect([r.min, r.max, r.value]).toEqual(['0', '200', '100'])
    }
    expect(slider('Media volume')).toBeNull()
    for (const l of ['Voices', 'Streams', 'Soundboard', 'Join', 'Leave', 'Mute', 'Unmute', 'Deafen', 'Undeafen',
      'Someone goes live', 'A stream ends', 'Message ding', 'Badge unlocked']) {
      expect(slider(l).max, l).toBe('200')
    }
    expect(slider('Clips').max).toBe('100')
    expect(slider('Join').getAttribute('aria-valuetext')).toBe('100%')
  })

  it('moving a slider saves it and shows the new percent; 0 reads as off', () => {
    show()
    move('Call sounds volume', 35)
    expect(stored().callSounds.master).toBe(35)
    expect(section('Call sounds').textContent).toContain('35%')
    move('Message ding', 0)
    expect(stored().notifications.message).toBe(0)
    expect(slider('Message ding').getAttribute('aria-valuetext')).toBe('Off')
  })

  it('Calls sliders reach the core', () => {
    show()
    move('Voices', 60)
    expect(calls('set_call_mix')).toEqual([{ callsPercent: 100, voicesPercent: 60, streamsPercent: 100 }])
  })

  it('▶ plays a sound at its section × its own level', () => {
    show()
    move('Call sounds volume', 50)
    move('Join', 150)
    button('Test Join').click()
    expect(calls('play_sound')).toEqual([{ name: 'join', set: 'classic', volume: mixGain(50, 150), output: null }])
  })

  it('▶ on Ringing plays one ring at Call sounds × Ringing (#50)', () => {
    show()
    move('Call sounds volume', 50)
    move('Ringing', 120)
    button('Test Ringing').click()
    expect(calls('play_sound')).toEqual([{ name: 'ringing', set: 'classic', volume: mixGain(50, 120), output: null }])
  })

  it('a test button is disabled while its level is off', () => {
    withSounds([snd('a', 'Airhorn')])
    show()
    move('Badge unlocked', 0)
    expect(button('Test Badge unlocked').disabled).toBe(true)
    expect(button('Test Badge unlocked').title).toBe('Off')
    expect(button('Test Message ding').disabled).toBe(false)
    move('Notifications volume', 0)
    expect(button('Test Message ding').disabled).toBe(true)
    expect(button('Test Soundboard').disabled).toBe(false)
    move('Calls volume', 0)
    expect(button('Test Soundboard').disabled).toBe(true)
  })

  it('only sound notches and Soundboard have a test button', () => {
    show()
    expect(button('Test Voices')).toBeNull()
    expect(button('Test Streams')).toBeNull()
    expect(button('Test Clips')).toBeNull()
    expect(button('Test Someone goes live')).not.toBeNull()
  })

  it('▶ Soundboard plays the first sound at Calls × Soundboard, and is disabled with no sounds', () => {
    show()
    expect(button('Test Soundboard').disabled).toBe(true)
    unmount(cmp!)
    withSounds([snd('b', 'Bruh'), snd('a', 'Airhorn')])
    show()
    move('Calls volume', 80)
    move('Soundboard', 140)
    button('Test Soundboard').click()
    expect(calls('soundboard_play')).toEqual([{ hash: 'ha', callsPercent: 80, soundboardPercent: 140, soundPercent: 120, output: null, soundId: null }])
  })

  it('Reset to defaults puts back only that section', () => {
    show()
    move('Voices', 10)
    move('Leave', 0)
    move('Clips', 30)
    section('Call sounds').querySelector<HTMLButtonElement>('button.reset')!.click()
    flushSync()
    expect(stored().callSounds).toEqual(defaultMixer().callSounds)
    expect(slider('Leave').value).toBe('100')
    expect(stored().calls.voices).toBe(10)
    expect(stored().media.clips).toBe(30)
    section('Media').querySelector<HTMLButtonElement>('button.reset')!.click()
    flushSync()
    expect(slider('Clips').value).toBe('100')
  })

  it('the Reset buttons say which section they reset', () => {
    show()
    expect(button('Reset Calls to defaults')).not.toBeNull()
    expect(button('Reset Media to defaults')).not.toBeNull()
  })

  it('stays in step with the soundboard popover’s slider', () => {
    show()
    setSoundboardVolume(45)
    flushSync()
    expect(slider('Soundboard').value).toBe('45')
    move('Soundboard', 170)
    expect(mixer.calls.soundboard).toBe(170)
  })
})

describe('VolumeMixer collapsible sections (#164)', () => {
  const toggle = (title: string) => section(title).querySelector<HTMLButtonElement>('h4 button')!
  const body = (title: string) => document.getElementById(toggle(title).getAttribute('aria-controls')!)!

  it('every section starts open, with a chevron toggle in its heading', () => {
    show()
    for (const t of ['Calls', 'Call sounds', 'Stream sounds', 'Notifications', 'Media']) {
      expect(toggle(t).getAttribute('aria-expanded'), t).toBe('true')
      expect(toggle(t).querySelector('svg'), t).not.toBeNull()
      expect(body(t).hidden, t).toBe(false)
    }
  })

  it('collapsing hides only its rows; the master slider and Reset stay in the header', () => {
    show()
    toggle('Calls').click()
    flushSync()
    expect(toggle('Calls').getAttribute('aria-expanded')).toBe('false')
    expect(body('Calls').hidden).toBe(true)
    expect(body('Calls').contains(slider('Voices'))).toBe(true)
    expect(slider('Calls volume').closest('[hidden]')).toBeNull()
    expect(button('Reset Calls to defaults').closest('[hidden]')).toBeNull()
    expect(body('Call sounds').hidden).toBe(false)
    toggle('Calls').click()
    flushSync()
    expect(body('Calls').hidden).toBe(false)
  })

  it('remembers which sections are closed on this device', () => {
    show()
    toggle('Media').click()
    toggle('Stream sounds').click()
    flushSync()
    unmount(cmp!)
    cmp = undefined
    reloadMixerCollapsed()
    show()
    expect(body('Media').hidden).toBe(true)
    expect(body('Stream sounds').hidden).toBe(true)
    expect(body('Calls').hidden).toBe(false)
  })

  it('a garbled saved value opens everything', () => {
    localStorage.setItem('pulse.mixerCollapsed', '{nope')
    reloadMixerCollapsed()
    show()
    expect(body('Calls').hidden).toBe(false)
  })
})
