import { describe, expect, it } from 'vitest'
import { formatMemberSince, formatVoice } from './profileFormat'

describe('profileFormat', () => {
  it('formats voice time', () => {
    expect(formatVoice(0)).toBe('0 min')
    expect(formatVoice(1800)).toBe('30 min')
    expect(formatVoice(3570)).toBe('59 min')
    expect(formatVoice(3599)).toBe('59 min')
    expect(formatVoice(3600)).toBe('1.0 h')
    expect(formatVoice(9000)).toBe('2.5 h')
    expect(formatVoice(514800)).toBe('143 h')
  })
  it('formats member since', () => {
    expect(formatMemberSince('2026-10-03T12:00:00Z')).toBe('Oct 3, 2026')
  })
})
