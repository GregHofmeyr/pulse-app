import { describe, expect, it } from 'vitest'
import { DEFAULT_PRESET, PRESETS, estimateBytes } from './clipPresets'

describe('clipPresets', () => {
  it('estimates match the agreed values', () => {
    expect(Math.abs(estimateBytes('light', 90_000) - 47_736_000)).toBeLessThanOrEqual(1)
    expect(Math.abs(estimateBytes('sharp', 90_000) - 93_636_000)).toBeLessThanOrEqual(1)
    expect(estimateBytes('light', 0)).toBe(0)
  })
  it('has only Light and Sharp, Light by default', () => {
    expect(Object.keys(PRESETS)).toEqual(['light', 'sharp'])
    expect(DEFAULT_PRESET).toBe('light')
  })
})
