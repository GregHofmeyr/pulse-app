import { describe, expect, it } from 'vitest'
import { encodingFor } from './encodings'

describe('encodingFor', () => {
  it('matches the spec table and tunes games for motion', () => {
    expect(encodingFor(720, 60)).toEqual({ maxBitrate: 5_500_000, maxFramerate: 60, contentHint: 'motion', degradation: 'maintain-framerate' })
    expect(encodingFor(1080, 60).maxBitrate).toBe(8_000_000)
    expect(encodingFor(1440, 30).maxBitrate).toBe(7_500_000)
  })
  it('treats 15 fps as text', () => {
    expect(encodingFor(1080, 15)).toEqual({ maxBitrate: 2_500_000, maxFramerate: 15, contentHint: 'detail', degradation: 'maintain-resolution' })
  })
})
