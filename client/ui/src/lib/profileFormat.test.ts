import { describe, expect, it } from 'vitest'
import { badgeInfo } from './badges'
import { formatMemberSince, formatVoice } from './profileFormat'

describe('profileFormat', () => {
  it('formats voice time', () => {
    expect(formatVoice(0)).toBe('0 min')
    expect(formatVoice(1800)).toBe('30 min')
    expect(formatVoice(9000)).toBe('2.5 h')
    expect(formatVoice(514800)).toBe('143 h')
  })
  it('formats member since', () => {
    expect(formatMemberSince('2026-10-03T12:00:00Z')).toBe('Oct 3, 2026')
  })
  it('unknown badge keys are title-cased with the trophy', () => {
    expect(badgeInfo('early_bird')).toEqual({ name: 'Early Bird', icon: 'trophy' })
  })
})
