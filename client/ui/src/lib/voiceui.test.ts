import { describe, expect, it } from 'vitest'
import { soundFor, volumeLabel, clampVolume } from './voiceui'

describe('voice ui helpers', () => {
  it('volumeLabel', () => {
    expect(volumeLabel(100)).toBe('100%')
    expect(volumeLabel(0)).toBe('Muted')
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
