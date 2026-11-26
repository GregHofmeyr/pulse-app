// Speaking rings (#651): the eleven styles and nine colours, keyed as the server keys them.
import { describe, expect, it } from 'vitest'
import { RING_COLORS, RING_STYLES } from './speakingRing'

describe('speaking rings (#651)', () => {
  /** A literal copy of RING_STYLES / RING_COLORS in server/src/profiles/mod.rs: change one side and both tests fail. */
  it('the_eleven_styles_and_nine_colours_keyed_as_the_server_keys_them', () => {
    expect(RING_STYLES.map((s) => s.key)).toEqual([
      'classic',
      'glow',
      'pulse',
      'ripple',
      'sweep',
      'orbit',
      'halo',
      'viewfinder',
      'liquid',
      'dots',
      'bars',
    ])
    expect(RING_STYLES.map((s) => s.label)).toEqual([
      'Classic',
      'Soft glow',
      'Pulse',
      'Ripple',
      'Sweep',
      'Orbit',
      'Halo',
      'Viewfinder',
      'Liquid',
      'Wave of dots',
      'Bars round the ring',
    ])
    expect(RING_STYLES.filter((s) => s.reactive).map((s) => s.key)).toEqual(['liquid', 'dots', 'bars'])
    expect(RING_COLORS.map((c) => c.key)).toEqual(['green', 'blue', 'gold', 'pink', 'cyan', 'orange', 'white', 'purple', 'profile'])
    expect(RING_COLORS.map((c) => c.label)).toEqual([
      'Green',
      'Pulse blue',
      'Gold',
      'Pink',
      'Cyan',
      'Orange',
      'White',
      'Purple',
      'Profile colour',
    ])
  })
})
