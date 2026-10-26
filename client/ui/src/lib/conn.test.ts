import { describe, expect, it } from 'vitest'
import { connLabel, isRefusal } from './conn'

describe('connLabel', () => {
  it('says why a refused connection keeps retrying', () => {
    expect(connLabel('too_many_devices')).toBe('Too many devices connected — close Pulse on another device')
    expect(connLabel({ rate_limited: 7 }, 1000, 1000)).toBe('Too many attempts, retrying in 7s…')
  })
  it('counts the wait down and never shows a stale or zero number', () => {
    const c = { rate_limited: 7 }
    expect(connLabel(c, 1000, 3500)).toBe('Too many attempts, retrying in 5s…')
    expect(connLabel(c, 1000, 8000)).toBe('Connecting…')
    expect(connLabel(c, 1000, 20000)).toBe('Connecting…')
  })
  it('keeps the plain wording for ordinary drops', () => {
    expect(connLabel('reconnecting')).toBe('Reconnecting…')
    expect(connLabel('connecting')).toBe('Connecting…')
  })
  it('knows which states are refusals', () => {
    expect(isRefusal('too_many_devices')).toBe(true)
    expect(isRefusal({ rate_limited: 3 })).toBe(true)
    expect(isRefusal('reconnecting')).toBe(false)
  })
})
