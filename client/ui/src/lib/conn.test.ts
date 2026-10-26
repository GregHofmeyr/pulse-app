import { describe, expect, it } from 'vitest'
import { connLabel } from './conn'

describe('connLabel', () => {
  it('says why a refused connection keeps retrying', () => {
    expect(connLabel('too_many_devices')).toBe('Too many devices connected — close Pulse on another device')
    expect(connLabel({ rate_limited: 7 })).toBe('Too many attempts, retrying in 7s…')
  })
  it('keeps the plain wording for ordinary drops', () => {
    expect(connLabel('reconnecting')).toBe('Reconnecting…')
    expect(connLabel('connecting')).toBe('Connecting…')
  })
})
