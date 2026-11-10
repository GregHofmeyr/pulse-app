// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest'
import {
  MAX_GAIN, MIXER_KEY, MIXER_SECTIONS, defaultMixer, gainToPercent, loadMixer, migrateMixer, mixGain, normalizeMixer,
  percentToGain, resetSection, saveMixer, soundGain, soundMasterGain, soundSectionOf, ringtoneGain,
} from './mixer'
import { SOUND_NAMES } from './voiceui'

beforeEach(() => localStorage.clear())

describe('the gain curve (mirrors the core’s percent_to_gain)', () => {
  it('has the same anchors', () => {
    expect(percentToGain(0)).toBe(0)
    expect(percentToGain(100)).toBe(1)
    expect(percentToGain(200)).toBeCloseTo(3.981, 3)
    expect(percentToGain(1)).toBeCloseTo(0.01047, 4)
    expect(percentToGain(50)).toBeCloseTo(0.1, 4)
    expect(percentToGain(500)).toBe(percentToGain(200))
    expect(percentToGain(-3)).toBe(0)
  })
  it('gainToPercent inverts it, keeping loudness', () => {
    expect(gainToPercent(0)).toBe(0)
    expect(gainToPercent(1)).toBe(100)
    expect(gainToPercent(0.5)).toBe(85) // -6 dB
    expect(gainToPercent(percentToGain(137))).toBe(137)
    expect(gainToPercent(1e-9)).toBe(1) // audible stays above off
    expect(gainToPercent(100)).toBe(200)
    expect(gainToPercent(NaN)).toBe(0)
  })
  it('mixGain multiplies and caps at the safe maximum', () => {
    expect(mixGain(100, 100)).toBe(1)
    expect(mixGain(50, 100)).toBeCloseTo(0.1, 4)
    expect(mixGain(0, 200)).toBe(0)
    expect(mixGain(200, 200)).toBe(MAX_GAIN)
    expect(MAX_GAIN).toBeCloseTo(percentToGain(200), 4)
  })
})

describe('sections', () => {
  it('lists the spec’s sections and notches in order', () => {
    expect(MIXER_SECTIONS.map((s) => s.label)).toEqual(['Calls', 'Call sounds', 'Stream sounds', 'Notifications', 'Media'])
    expect(MIXER_SECTIONS.map((s) => s.notches.map((n) => n.label))).toEqual([
      ['Voices', 'Streams', 'Soundboard', 'Join sounds'],
      ['Join', 'Leave', 'Mute', 'Unmute', 'Deafen', 'Undeafen', 'Talking while muted', 'Ringing'],
      ['Someone goes live', 'A stream ends'],
      ['Message ding', 'Badge unlocked', 'Picker wheel tick', 'Picker wheel result'],
      ['Clips'],
    ])
    expect(MIXER_SECTIONS.map((s) => s.master)).toEqual([true, true, true, true, false])
  })
  it('every UI sound has exactly one notch', () => {
    for (const n of SOUND_NAMES) expect(soundSectionOf(n), n).toBeTruthy()
    const soundNotches = MIXER_SECTIONS.flatMap((s) => s.notches.filter((x) => x.test === 'sound').map((x) => x.key))
    expect([...soundNotches].sort()).toEqual([...SOUND_NAMES].sort())
  })
})

describe('prefs', () => {
  it('defaults: every notch at 100 %, clips unmuted', () => {
    const d = defaultMixer()
    expect(d).toEqual({
      v: 1,
      calls: { master: 100, voices: 100, streams: 100, soundboard: 100, join_sounds: 100 },
      callSounds: { master: 100, join: 100, leave: 100, mute: 100, unmute: 100, deafen: 100, undeafen: 100, muted_talk: 100, ringing: 100 },
      streamSounds: { master: 100, live: 100, live_end: 100 },
      notifications: { master: 100, message: 100, badge: 100, wheel_tick: 100, wheel_result: 100 },
      media: { clips: 100, clipsMuted: false },
    })
    expect(loadMixer()).toEqual(d)
  })

  it('a record saved before the muted cue (#252) has it on, at 100 %, and keeps the rest', () => {
    const before = defaultMixer() as unknown as { callSounds: Record<string, number> }
    delete before.callSounds.muted_talk
    before.callSounds.join = 40
    localStorage.setItem(MIXER_KEY, JSON.stringify(before))
    const p = loadMixer()
    expect(p.callSounds.muted_talk).toBe(100)
    expect(p.callSounds.join).toBe(40)
  })

  it('a record saved before the wheel result chime (#271) has it on, at 100 %, and keeps the rest', () => {
    const before = defaultMixer() as unknown as { notifications: Record<string, number> }
    delete before.notifications.wheel_result
    before.notifications.wheel_tick = 30
    localStorage.setItem(MIXER_KEY, JSON.stringify(before))
    const p = loadMixer()
    expect(p.notifications.wheel_result).toBe(100)
    expect(p.notifications.wheel_tick).toBe(30)
  })

  it('persists one versioned record', () => {
    const p = defaultMixer()
    p.callSounds.master = 30
    p.media.clips = 40
    saveMixer(p)
    expect(JSON.parse(localStorage.getItem(MIXER_KEY)!).v).toBe(1)
    expect(loadMixer()).toEqual(p)
  })

  it('migrates today’s keys: off sounds → 0, the old volume → the three sound masters (same loudness)', () => {
    localStorage.setItem('pulse.sounds', JSON.stringify({ set: 'goofy', volume_pct: 50, off: ['join', 'badge'] }))
    localStorage.setItem('pulse.soundboard', JSON.stringify({ volume_pct: 40 }))
    localStorage.setItem('pulse.clipVolume', JSON.stringify({ volume: 0.5, muted: true }))
    const p = loadMixer()
    expect([p.callSounds.master, p.streamSounds.master, p.notifications.master]).toEqual([85, 85, 85])
    expect(p.callSounds.join).toBe(0)
    expect(p.notifications.badge).toBe(0)
    expect(p.callSounds.leave).toBe(100)
    expect(p.calls.soundboard).toBe(40) // already on the dB curve
    expect(p.calls.master).toBe(100)
    expect(p.media).toEqual({ clips: 85, clipsMuted: true })
    // migrated once: the record is written, the old keys left alone
    expect(JSON.parse(localStorage.getItem(MIXER_KEY)!)).toEqual(p)
    expect(JSON.parse(localStorage.getItem('pulse.sounds')!).set).toBe('goofy')
  })

  it('an old sound volume of 0 makes all three masters 0, not 1 %', () => {
    localStorage.setItem('pulse.sounds', JSON.stringify({ volume_pct: 0, off: [] }))
    const p = loadMixer()
    expect([p.callSounds.master, p.streamSounds.master, p.notifications.master]).toEqual([0, 0, 0])
  })

  it('an old full volume and clip volume carry over as 100 %', () => {
    expect(migrateMixer({ sounds: { volume_pct: 100, off: [] }, soundboard: { volume_pct: 100 }, clipVolume: { volume: 1, muted: false } }))
      .toEqual(defaultMixer())
  })

  it('a newer record (after a downgrade) is read as the old keys say, and left in place', () => {
    localStorage.setItem('pulse.sounds', JSON.stringify({ volume_pct: 100, off: ['live'] }))
    localStorage.setItem(MIXER_KEY, JSON.stringify({ v: 2, calls: { master: 5 } }))
    const p = loadMixer()
    expect(p.v).toBe(1)
    expect(p.calls.master).toBe(100)
    expect(p.streamSounds.live).toBe(0)
    expect(JSON.parse(localStorage.getItem(MIXER_KEY)!).v).toBe(2) // not overwritten on load
  })

  it('a corrupt record falls back to the defaults, not to the pre-mixer settings', () => {
    localStorage.setItem('pulse.sounds', JSON.stringify({ volume_pct: 100, off: ['live'] }))
    localStorage.setItem(MIXER_KEY, '{not json')
    expect(loadMixer()).toEqual(defaultMixer())
    localStorage.setItem(MIXER_KEY, JSON.stringify({ v: 'one' }))
    expect(loadMixer()).toEqual(defaultMixer())
  })

  it('cleans a corrupt record field by field', () => {
    localStorage.setItem(MIXER_KEY, JSON.stringify({
      v: 1, calls: { master: '50', voices: 999, streams: -4, soundboard: 37.6 }, callSounds: null,
      notifications: { message: NaN, badge: 12 }, media: { clips: 150, clipsMuted: 'yes' },
    }))
    const p = loadMixer()
    // a record from before join sounds (#121) gains the notch at 100 %
    expect(p.calls).toEqual({ master: 100, voices: 200, streams: 0, soundboard: 38, join_sounds: 100 })
    expect(p.callSounds).toEqual(defaultMixer().callSounds)
    expect(p.notifications).toEqual({ master: 100, message: 100, badge: 12, wheel_tick: 100, wheel_result: 100 })
    expect(p.media).toEqual({ clips: 100, clipsMuted: false })
    expect(normalizeMixer('nope')).toBeNull()
    expect(normalizeMixer([1])).toBeNull()
  })

  it('unreadable storage falls back to the defaults', () => {
    localStorage.setItem(MIXER_KEY, '{not json')
    expect(loadMixer()).toEqual(defaultMixer())
  })
})

describe('sound gains', () => {
  it('section master × item', () => {
    const p = defaultMixer()
    p.callSounds.master = 50
    p.callSounds.join = 200
    expect(soundGain(p, 'join')).toBeCloseTo(percentToGain(50) * percentToGain(200), 6)
    expect(soundGain(p, 'message')).toBe(1)
    expect(soundMasterGain(p, 'join')).toBeCloseTo(percentToGain(50), 6)
    p.notifications.message = 0
    expect(soundGain(p, 'message')).toBe(0)
    p.streamSounds.master = 0
    expect(soundGain(p, 'live')).toBe(0)
    expect(soundGain(p, 'live_end')).toBe(0)
  })
  it('maps each sound to its section', () => {
    expect(soundSectionOf('undeafen')).toBe('callSounds')
    expect(soundSectionOf('muted_talk')).toBe('callSounds')
    expect(soundSectionOf('live_end')).toBe('streamSounds')
    expect(soundSectionOf('message')).toBe('notifications')
    expect(soundSectionOf('badge')).toBe('notifications')
    expect(soundSectionOf('wheel_result')).toBe('notifications')
  })
})

describe('resetSection', () => {
  it('resets only that section', () => {
    const p = defaultMixer()
    p.calls.voices = 20
    p.callSounds.master = 10
    p.callSounds.mute = 0
    p.media = { clips: 30, clipsMuted: true }
    const r = resetSection(p, 'callSounds')
    expect(r.callSounds).toEqual(defaultMixer().callSounds)
    expect(r.calls.voices).toBe(20)
    expect(r.media).toEqual({ clips: 30, clipsMuted: true })
    expect(resetSection(r, 'media').media).toEqual({ clips: 100, clipsMuted: false })
  })
})

describe('the Ringing notch (#50)', () => {
  beforeEach(() => localStorage.clear())
  it('sits under Call sounds and scales the ringtone with that master', () => {
    const section = MIXER_SECTIONS.find((s) => s.id === 'callSounds')!
    expect(section.notches.at(-1)).toMatchObject({ key: 'ringing', label: 'Ringing', test: 'ring', max: 200 })
    const p = defaultMixer()
    expect(ringtoneGain(p)).toBe(1)
    p.callSounds.ringing = 50
    expect(ringtoneGain(p)).toBeCloseTo(percentToGain(50), 6)
    p.callSounds.master = 200
    expect(ringtoneGain(p)).toBeCloseTo(percentToGain(200) * percentToGain(50), 6)
    p.callSounds.ringing = 0
    expect(ringtoneGain(p)).toBe(0)
  })
  it('a record saved before it existed rings at 100 %', () => {
    const { ringing: _r, ...before } = defaultMixer().callSounds
    localStorage.setItem(MIXER_KEY, JSON.stringify({ ...defaultMixer(), callSounds: { ...before, master: 40 } }))
    const p = loadMixer()
    expect(p.callSounds.ringing).toBe(100)
    expect(p.callSounds.master).toBe(40)
  })
})
