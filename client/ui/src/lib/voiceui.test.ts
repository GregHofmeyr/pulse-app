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
