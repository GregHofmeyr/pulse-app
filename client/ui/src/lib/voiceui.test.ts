import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { readdirSync } from 'node:fs'
import {
  soundFor, volumeLabel, clampVolume, migrateAudioConfig, defaultAudioConfig, playSound,
  SOUND_NAMES, SOUND_SETS, SOUND_LABELS, PREVIEW, PREVIEW_GAP_MS, defaultSoundPrefs,
  normalizeSoundPrefs, loadSoundPrefs, saveSoundPrefs, previewSet, stopPreview,
} from './voiceui'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => {}) }))

describe('voice ui helpers', () => {
  it('volumeLabel', () => {
    expect(volumeLabel(100)).toBe('100%')
    expect(volumeLabel(0)).toBe('0%') // not 'Muted': that reads as their own mute
    expect(volumeLabel(250)).toBe('200%')
  })

  it('clampVolume', () => {
    expect(clampVolume(-5)).toBe(0)
    expect(clampVolume(140.6)).toBe(141)
    expect(clampVolume(999)).toBe(200)
  })

  it('soundFor picks the transition sound', () => {
    const off = { muted: false, deafened: false }
    const muted = { muted: true, deafened: false }
    const deaf = { muted: true, deafened: true }
    expect(soundFor(off, muted)).toBe('mute')
    expect(soundFor(muted, off)).toBe('unmute')
    expect(soundFor(off, deaf)).toBe('deafen')
    expect(soundFor(muted, deaf)).toBe('deafen')
    expect(soundFor(deaf, muted)).toBe('undeafen')
    expect(soundFor(deaf, off)).toBe('undeafen')
    expect(soundFor(off, off)).toBeNull()
  })
})

import { transitionSounds } from './voiceui'

describe('transitionSounds', () => {
  const off = { muted: false, deafened: false }
  const at = (channelId: string | null, connection: string, controls = off) => ({ channelId, connection, controls })
  it('plays join when we become connected (after a connecting step)', () => {
    expect(transitionSounds(at(null, 'disconnected'), at('V1', 'connecting'))).toEqual([])
    expect(transitionSounds(at('V1', 'connecting'), at('V1', 'connected'))).toEqual(['join'])
  })
  it('plays leave only when we were connected', () => {
    expect(transitionSounds(at('V1', 'connected'), at(null, 'disconnected'))).toEqual(['leave'])
    expect(transitionSounds(at('V1', 'connecting'), at(null, 'disconnected'))).toEqual([])
  })
  it('reconnecting back to connected is not a join', () => {
    expect(transitionSounds(at('V1', 'reconnecting'), at('V1', 'connected'))).toEqual([])
  })
  it('switching channels plays join for the new one', () => {
    expect(transitionSounds(at('V1', 'connected'), at('V2', 'connected'))).toEqual(['join'])
  })
  it('mute toggles still sound', () => {
    expect(transitionSounds(at('V1', 'connected'), at('V1', 'connected', { muted: true, deafened: false }))).toEqual(['mute'])
  })
})

describe('migrateAudioConfig', () => {
  it('maps the old noise toggle off to level off', () => {
    const c = migrateAudioConfig({ noise_suppress: false, sensitivity: 0.05 })
    expect(c.noise_suppression).toBe('off')
    expect(c.sensitivity).toBe(0.05)
    expect('noise_suppress' in c).toBe(false)
  })
  it('gives old toggle-on users the default level and auto sensitivity', () => {
    const c = migrateAudioConfig({ noise_suppress: true })
    expect(c.noise_suppression).toBe(defaultAudioConfig.noise_suppression)
    expect(c.auto_sensitivity).toBe(true)
  })
  it('keeps an explicit new level', () => {
    expect(migrateAudioConfig({ noise_suppression: 'standard', noise_suppress: false }).noise_suppression).toBe('standard')
  })
})

function stubStorage(initial: Record<string, string> = {}) {
  const store = { ...initial }
  vi.stubGlobal('localStorage', {
    getItem: (k: string) => store[k] ?? null,
    setItem: (k: string, v: string) => { store[k] = v },
  })
  return store
}

async function captureInvokes() {
  const { invoke } = await import('@tauri-apps/api/core')
  const calls: unknown[] = []
  vi.mocked(invoke).mockImplementation(async (cmd, args) => { calls.push([cmd, args]) })
  return calls
}

describe('playSound', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('sends the set, volume and the output chosen in settings', async () => {
    const calls = await captureInvokes()
    stubStorage({
      'pulse.audio': JSON.stringify({ output: 'Headphones' }),
      'pulse.sounds': JSON.stringify({ set: 'goofy', volume_pct: 40, off: [] }),
    })
    playSound('join')
    expect(calls).toEqual([['play_sound', { name: 'join', set: 'goofy', volume: 0.4, output: 'Headphones' }]])
  })

  it('uses the defaults when nothing is saved', async () => {
    const calls = await captureInvokes()
    stubStorage()
    playSound('live')
    expect(calls).toEqual([['play_sound', { name: 'live', set: 'classic', volume: 1, output: null }]])
  })

  it('plays nothing for a switched-off event', async () => {
    const calls = await captureInvokes()
    stubStorage({ 'pulse.sounds': JSON.stringify({ set: 'classic', volume_pct: 100, off: ['message'] }) })
    playSound('message')
    expect(calls).toEqual([])
  })

  it('plays nothing at volume 0', async () => {
    const calls = await captureInvokes()
    stubStorage({ 'pulse.sounds': JSON.stringify({ set: 'classic', volume_pct: 0, off: [] }) })
    playSound('join')
    expect(calls).toEqual([])
  })
})

describe('sound prefs', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('defaults', () => {
    expect(defaultSoundPrefs).toEqual({ set: 'classic', volume_pct: 100, off: [] })
    expect(normalizeSoundPrefs(undefined)).toEqual(defaultSoundPrefs)
    expect(normalizeSoundPrefs(null)).toEqual(defaultSoundPrefs)
    expect(normalizeSoundPrefs('goofy')).toEqual(defaultSoundPrefs)
    expect(normalizeSoundPrefs(7)).toEqual(defaultSoundPrefs)
    expect(normalizeSoundPrefs({})).toEqual(defaultSoundPrefs)
  })

  it('an unknown set falls back to classic', () => {
    expect(normalizeSoundPrefs({ set: 'nope' }).set).toBe('classic')
    expect(normalizeSoundPrefs({ set: 5 }).set).toBe('classic')
    expect(normalizeSoundPrefs({ set: 'minimal-wood' }).set).toBe('minimal-wood')
  })

  it('volume is rounded and clamped to 0-100', () => {
    const v = (volume_pct: unknown) => normalizeSoundPrefs({ volume_pct }).volume_pct
    expect(v(55.6)).toBe(56)
    expect(v(-4)).toBe(0)
    expect(v(250)).toBe(100)
    expect(v(0)).toBe(0)
    expect(v(NaN)).toBe(100)
    expect(v(Infinity)).toBe(100)
    expect(v('50')).toBe(100)
    expect(v(undefined)).toBe(100)
  })

  it('off keeps only known names, deduped', () => {
    expect(normalizeSoundPrefs({ off: ['mute', 'bogus', 'mute', 3, null, 'live_end'] }).off).toEqual(['mute', 'live_end'])
    expect(normalizeSoundPrefs({ off: 'mute' }).off).toEqual([])
    expect(normalizeSoundPrefs({ off: { 0: 'mute' } }).off).toEqual([])
  })

  it('round-trips through localStorage', () => {
    stubStorage()
    const p = { set: 'twostep-warm' as const, volume_pct: 30, off: ['message' as const] }
    saveSoundPrefs(p)
    expect(loadSoundPrefs()).toEqual(p)
  })

  it('junk in storage gives the defaults or a cleaned value', () => {
    stubStorage({ 'pulse.sounds': '{not json' })
    expect(loadSoundPrefs()).toEqual(defaultSoundPrefs)
    stubStorage({ 'pulse.sounds': JSON.stringify({ set: 'goofy', off: ['x', 'join'], extra: 1 }) })
    expect(loadSoundPrefs()).toEqual({ set: 'goofy', volume_pct: 100, off: ['join'] })
  })
})

describe('set preview', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => {
    stopPreview()
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  const names = (calls: unknown[]) => (calls as [string, { name: string }][]).map((c) => c[1].name)

  it('plays PREVIEW in order, 450 ms apart, with the given set', async () => {
    const calls = await captureInvokes()
    stubStorage({ 'pulse.sounds': JSON.stringify({ set: 'classic', volume_pct: 50, off: ['mute', 'join'] }) })
    expect(PREVIEW_GAP_MS).toBe(450)
    previewSet('minimal-glass')
    expect(names(calls)).toEqual(['join'])
    for (let i = 1; i < PREVIEW.length; i++) {
      vi.advanceTimersByTime(PREVIEW_GAP_MS)
      expect(names(calls)).toEqual(PREVIEW.slice(0, i + 1))
    }
    expect(calls[0]).toEqual(['play_sound', { name: 'join', set: 'minimal-glass', volume: 0.5, output: null }])
    vi.advanceTimersByTime(5000)
    expect(calls).toHaveLength(PREVIEW.length)
  })

  it('a second preview cancels the first, and stopPreview cancels', async () => {
    const calls = await captureInvokes()
    stubStorage()
    previewSet('goofy')
    vi.advanceTimersByTime(PREVIEW_GAP_MS)
    previewSet('twostep-deep')
    vi.advanceTimersByTime(10_000)
    const sets = (calls as [string, { set: string }][]).map((c) => c[1].set)
    expect(sets).toEqual(['goofy', 'goofy', ...Array(PREVIEW.length).fill('twostep-deep')])

    calls.length = 0
    previewSet('classic')
    stopPreview()
    vi.advanceTimersByTime(10_000)
    expect(names(calls)).toEqual(['join'])
  })

  it('volume 0 plays nothing', async () => {
    const calls = await captureInvokes()
    stubStorage({ 'pulse.sounds': JSON.stringify({ volume_pct: 0 }) })
    previewSet('classic')
    vi.advanceTimersByTime(10_000)
    expect(calls).toEqual([])
  })
})

describe('sound catalogue', () => {
  const soundsDir = new URL('../../../src-tauri/sounds/', import.meta.url)

  it('set ids match the directories on disk', () => {
    const dirs = readdirSync(soundsDir, { withFileTypes: true }).filter((e) => e.isDirectory()).map((e) => e.name)
    expect(SOUND_SETS.map((s) => s.id).sort()).toEqual(dirs.sort())
  })

  it('set order, shown names and blurbs', () => {
    expect(SOUND_SETS.map((s) => s.label)).toEqual([
      'Classic', 'Two-step · Bright', 'Two-step · Warm', 'Two-step · Deep',
      'Minimal · Soft', 'Minimal · Wood', 'Minimal · Glass', 'Goofy',
    ])
    expect(SOUND_SETS.map((s) => s.id)[0]).toBe('classic')
    for (const s of SOUND_SETS) expect(s.blurb.length).toBeGreaterThan(10)
  })

  it('event names match the classic files, with labels', () => {
    const files = readdirSync(new URL('classic/', soundsDir)).map((f) => f.replace(/\.wav$/, ''))
    expect([...SOUND_NAMES].sort()).toEqual(files.sort())
    expect(SOUND_NAMES).toEqual(['join', 'leave', 'mute', 'unmute', 'deafen', 'undeafen', 'message', 'live', 'live_end'])
    expect(SOUND_NAMES.map((n) => SOUND_LABELS[n])).toEqual([
      'Join a call', 'Leave a call', 'Mute', 'Unmute', 'Deafen', 'Undeafen',
      'New message', 'Someone goes live', 'A stream ends',
    ])
  })
})

describe('sounds outside a call', () => {
  it('the mute hotkey still plays its sound when not in voice (a quick "is my audio working?")', async () => {
    const { transitionSounds } = await import('./voiceui')
    const idle = { channelId: null, connection: 'disconnected' as const, controls: { muted: false, deafened: false } }
    const muted = { ...idle, controls: { muted: true, deafened: false } }
    expect(transitionSounds(idle, muted)).toEqual(['mute'])
    expect(transitionSounds(muted, idle)).toEqual(['unmute'])
  })
})
