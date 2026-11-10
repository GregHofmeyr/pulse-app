import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { readdirSync } from 'node:fs'
import {
  soundFor, volumeLabel, clampVolume, migrateAudioConfig, defaultAudioConfig, playSound,
  SOUND_NAMES, SOUND_SETS, SOUND_LABELS, PREVIEW, PREVIEW_GAP_MS, defaultSoundPrefs,
  normalizeSoundPrefs, loadSoundPrefs, saveSoundPrefs, previewSet, stopPreview,
  playSoundAt, playRingingAt, previewRingtone, RINGTONES,
} from './voiceui'
import { defaultMixer, mixGain, type MixerPrefs } from './mixer'

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

import { echoDecision, peerCallSound, serverVoiceSound, transitionSounds } from './voiceui'

describe('serverVoiceSound (#147)', () => {
  const c = (o: Partial<{ muted: boolean; deafened: boolean; server_muted: boolean; server_deafened: boolean }> = {}) => ({ muted: false, deafened: false, server_muted: false, server_deafened: false, ...o })
  it('server mute / deafen and their undo play the same cues as your own', () => {
    expect(serverVoiceSound(c(), { muted: true, deafened: false })).toBe('mute')
    expect(serverVoiceSound(c({ server_muted: true }), { muted: false, deafened: false })).toBe('unmute')
    expect(serverVoiceSound(c(), { muted: false, deafened: true })).toBe('deafen')
    expect(serverVoiceSound(c({ server_deafened: true }), { muted: false, deafened: false })).toBe('undeafen')
    expect(serverVoiceSound(c({ server_muted: true, server_deafened: true }), { muted: true, deafened: false })).toBe('undeafen')
  })
  it('nothing when what you hear and send stays the same', () => {
    expect(serverVoiceSound(c({ muted: true }), { muted: true, deafened: false }), 'already muted by you').toBeNull()
    expect(serverVoiceSound(c({ deafened: true }), { muted: false, deafened: true }), 'already deafened by you').toBeNull()
    expect(serverVoiceSound(c({ server_muted: true }), { muted: true, deafened: false })).toBeNull()
  })
})

describe('echoDecision (#172)', () => {
  it('under Auto, says what it decided and why', () => {
    expect(echoDecision({ mode: 'auto', running: false, output: 'headphones' }, 'auto')).toBe('Off for now: your output is headphones or a headset')
    expect(echoDecision({ mode: 'auto', running: true, output: 'other' }, 'auto')).toBe("On: your output isn't headphones or a headset")
    expect(echoDecision({ mode: 'auto', running: true, output: 'unknown' }, 'auto')).toBe("On: your output doesn't say it's headphones")
  })
  it('nothing for On or Off, before the core has said, or while the core still runs the old choice', () => {
    expect(echoDecision({ mode: 'on', running: true, output: 'headphones' }, 'on')).toBeNull()
    expect(echoDecision(null, 'auto')).toBeNull()
    expect(echoDecision({ mode: 'off', running: false, output: 'other' }, 'auto')).toBeNull()
  })
})

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

// #219: someone else joining or leaving the call you're in chirps too, so a join is noticed
describe('peerCallSound', () => {
  const ev = (t: 'VoiceJoined' | 'VoiceLeft', user = 'U2', channel = 'V1') =>
    ({ t, d: { channel_id: channel, user_id: user, flags: { muted: false, deafened: false } } }) as never
  const ctx = { myId: 'U1', channelId: 'V1', connection: 'connected' as const }
  it('someone joining your call plays join, leaving it plays leave', () => {
    expect(peerCallSound(ev('VoiceJoined'), ctx)).toBe('join')
    expect(peerCallSound(ev('VoiceLeft'), ctx)).toBe('leave')
  })
  it('your own join and leave are left to the connection (they sound once)', () => {
    expect(peerCallSound(ev('VoiceJoined', 'U1'), ctx)).toBeNull()
    expect(peerCallSound(ev('VoiceLeft', 'U1'), ctx)).toBeNull()
  })
  it('another channel, or no call of yours, is silent', () => {
    expect(peerCallSound(ev('VoiceJoined', 'U2', 'V2'), ctx)).toBeNull()
    expect(peerCallSound(ev('VoiceJoined'), { ...ctx, channelId: null })).toBeNull()
    expect(peerCallSound(ev('VoiceJoined'), { ...ctx, connection: 'connecting' })).toBeNull()
  })
  it('other events are silent', () => {
    expect(peerCallSound({ t: 'Typing', d: { channel_id: 'V1', user_id: 'U2' } } as never, ctx)).toBeNull()
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
  it('echo cancellation (#76): the old toggle off stays Off, on becomes Auto (the default); a saved mode wins', () => {
    const off = migrateAudioConfig({ echo_cancel: false })
    expect(off.echo_mode).toBe('off')
    expect('echo_cancel' in off).toBe(false)
    expect(migrateAudioConfig({ echo_cancel: true }).echo_mode).toBe('auto')
    expect(migrateAudioConfig({}).echo_mode).toBe('auto')
    expect(defaultAudioConfig.echo_mode).toBe('auto')
    expect(migrateAudioConfig({ echo_mode: 'on', echo_cancel: false }).echo_mode).toBe('on')
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

const mixerWith = (edit: (p: MixerPrefs) => void) => {
  const p = defaultMixer()
  edit(p)
  return JSON.stringify(p)
}

describe('playSound', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('sends the set, the mixer gain (section × sound) and the output chosen in settings', async () => {
    const calls = await captureInvokes()
    stubStorage({
      'pulse.audio': JSON.stringify({ output: 'Headphones' }),
      'pulse.sounds': JSON.stringify({ set: 'goofy' }),
      'pulse.mixer': mixerWith((p) => { p.callSounds.master = 50; p.callSounds.join = 200 }),
    })
    playSound('join')
    expect(calls).toEqual([['play_sound', { name: 'join', set: 'goofy', volume: mixGain(50, 200), output: 'Headphones' }]])
  })

  it('each sound uses its own section', async () => {
    const calls = await captureInvokes()
    stubStorage({ 'pulse.mixer': mixerWith((p) => { p.callSounds.master = 10; p.notifications.message = 150 }) })
    playSound('message')
    playSound('live')
    expect(calls).toEqual([
      ['play_sound', { name: 'message', set: 'classic', volume: mixGain(100, 150), output: null }],
      ['play_sound', { name: 'live', set: 'classic', volume: 1, output: null }],
    ])
  })

  it('uses the defaults when nothing is saved', async () => {
    const calls = await captureInvokes()
    stubStorage()
    playSound('live')
    expect(calls).toEqual([['play_sound', { name: 'live', set: 'classic', volume: 1, output: null }]])
  })

  it('plays nothing for a sound at 0 %, or its section at 0 %', async () => {
    const calls = await captureInvokes()
    stubStorage({ 'pulse.mixer': mixerWith((p) => { p.notifications.message = 0; p.callSounds.master = 0 }) })
    playSound('message')
    playSound('join')
    expect(calls).toEqual([])
  })

  it('a switched-off sound from before the mixer stays silent (migrated to 0 %)', async () => {
    const calls = await captureInvokes()
    stubStorage({ 'pulse.sounds': JSON.stringify({ set: 'classic', volume_pct: 100, off: ['message'] }) })
    playSound('message')
    expect(calls).toEqual([])
  })

  it('the old overall volume keeps its loudness', async () => {
    const calls = await captureInvokes()
    stubStorage({ 'pulse.sounds': JSON.stringify({ set: 'classic', volume_pct: 50, off: [] }) })
    playSound('join')
    expect((calls[0] as [string, { volume: number }])[1].volume).toBeCloseTo(0.5, 2)
  })

  it('playSoundAt plays at the given gain, even for a sound at 0 % elsewhere', async () => {
    const calls = await captureInvokes()
    stubStorage({ 'pulse.mixer': mixerWith((p) => { p.callSounds.join = 0 }) })
    playSoundAt('join', 2)
    playSoundAt('leave', 0)
    expect(calls).toEqual([['play_sound', { name: 'join', set: 'classic', volume: 2, output: null }]])
  })
})

describe('sound prefs', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('defaults', () => {
    expect(defaultSoundPrefs).toEqual({ set: 'classic', volume_pct: 100, off: [], ringtone: 'classic' })
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
    const p = { set: 'twostep-warm' as const, volume_pct: 30, off: ['message' as const], ringtone: 'chiptune' as const }
    saveSoundPrefs(p)
    expect(loadSoundPrefs()).toEqual(p)
  })

  it('junk in storage gives the defaults or a cleaned value', () => {
    stubStorage({ 'pulse.sounds': '{not json' })
    expect(loadSoundPrefs()).toEqual(defaultSoundPrefs)
    stubStorage({ 'pulse.sounds': JSON.stringify({ set: 'goofy', off: ['x', 'join'], extra: 1 }) })
    expect(loadSoundPrefs()).toEqual({ set: 'goofy', volume_pct: 100, off: ['join'], ringtone: 'classic' })
  })

  it('an unknown ringtone falls back to classic (#174)', () => {
    expect(normalizeSoundPrefs({ ringtone: 'nope' }).ringtone).toBe('classic')
    expect(normalizeSoundPrefs({ ringtone: 3 }).ringtone).toBe('classic')
    expect(normalizeSoundPrefs({ ringtone: 'marimba' }).ringtone).toBe('marimba')
    // choosing a ringtone leaves the set alone, and the other way round
    expect(normalizeSoundPrefs({ set: 'goofy', ringtone: 'phone' })).toMatchObject({ set: 'goofy', ringtone: 'phone' })
  })
})

describe('ringtones (#174)', () => {
  const ringtoneDir = new URL('../../../src-tauri/ringtone/', import.meta.url)
  afterEach(() => vi.unstubAllGlobals())

  it('the list, Classic first, each with a name and a blurb, one wav each on disk', () => {
    expect(RINGTONES.map((r) => r.id)).toEqual(['classic', 'marimba', 'chiptune', 'phone'])
    for (const r of RINGTONES) {
      expect(r.label.length).toBeGreaterThan(2)
      expect(r.blurb.length).toBeGreaterThan(10)
    }
    // Classic is the original ring.wav; the rest are named by id
    const files = readdirSync(ringtoneDir).map((f) => f.replace(/\.wav$/, '')).sort()
    expect(files).toEqual(['chiptune', 'marimba', 'phone', 'ring'])
  })

  it("the mixer's Ringing ▶ plays the chosen ringtone", async () => {
    const calls = await captureInvokes()
    stubStorage({ 'pulse.sounds': JSON.stringify({ set: 'goofy', ringtone: 'marimba' }) })
    playRingingAt(0.5)
    playRingingAt(0)
    expect(calls).toEqual([['play_sound', { name: 'ringing', set: 'goofy', volume: 0.5, output: null, ringtone: 'marimba' }]])
  })

  it('a preview plays one cycle of that ringtone at Call sounds × Ringing, not the chosen one', async () => {
    const calls = await captureInvokes()
    stubStorage({
      'pulse.sounds': JSON.stringify({ ringtone: 'classic' }),
      'pulse.mixer': mixerWith((p) => { p.callSounds.master = 50; p.callSounds.ringing = 120 }),
    })
    previewRingtone('phone')
    expect(calls).toEqual([['play_sound', { name: 'ringing', set: 'classic', volume: mixGain(50, 120), output: null, ringtone: 'phone' }]])
    calls.length = 0
    stubStorage({ 'pulse.mixer': mixerWith((p) => { p.callSounds.ringing = 0 }) })
    previewRingtone('phone')
    expect(calls, 'Ringing at 0 % previews nothing').toEqual([])
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
    stubStorage({ 'pulse.mixer': mixerWith((p) => { p.callSounds.master = 50; p.callSounds.join = 0; p.callSounds.mute = 0; p.streamSounds.master = 150 }) })
    expect(PREVIEW_GAP_MS).toBe(450)
    previewSet('minimal-glass')
    expect(names(calls)).toEqual(['join'])
    for (let i = 1; i < PREVIEW.length; i++) {
      vi.advanceTimersByTime(PREVIEW_GAP_MS)
      expect(names(calls)).toEqual(PREVIEW.slice(0, i + 1))
    }
    // each at its section's master: the per-sound notches don't apply to a preview
    expect(calls[0]).toEqual(['play_sound', { name: 'join', set: 'minimal-glass', volume: mixGain(50), output: null }])
    expect(calls[3]).toEqual(['play_sound', { name: 'live', set: 'minimal-glass', volume: mixGain(150), output: null }])
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

  it('sections at 0 % play nothing', async () => {
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
    expect(SOUND_NAMES).toEqual(['join', 'leave', 'mute', 'unmute', 'deafen', 'undeafen', 'muted_talk', 'message', 'live', 'live_end', 'badge', 'wheel_tick', 'wheel_result'])
    expect(SOUND_NAMES.map((n) => SOUND_LABELS[n])).toEqual([
      'Join a call', 'Leave a call', 'Mute', 'Unmute', 'Deafen', 'Undeafen', 'Talking while muted',
      'New message', 'Someone goes live', 'A stream ends', 'Badge unlocked', 'Picker wheel tick', 'Picker wheel result',
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
