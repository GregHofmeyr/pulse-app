// Speaking rings (#651): the eleven styles and nine colours, keyed as the server keys them.
import { describe, expect, it } from 'vitest'
import { contrast } from './color'
import type { User } from './protocol/User'
import { effectiveStyle, RING_COLORS, RING_STYLES, ringColorOf, ringState, ringStyleOf, speakCss, watchesLevels, type RingPlace, type RingStyle } from './speakingRing'

const user = (extra: Partial<User> = {}, colors: string[] = []): User => ({
  id: 'U1',
  username: 'alex',
  display_name: 'alex',
  avatar_hash: null,
  avatar_still: null,
  style: { font: null, colors },
  ...extra,
})

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

  it('unknown_absent_or_emptied_users_are_classic_green', () => {
    expect(ringStyleOf(null)).toBe('classic')
    expect(ringColorOf(null)).toBe('green')
    expect(ringStyleOf(user({ ring_style: 'sparkle', ring_color: 'red' }))).toBe('classic')
    expect(ringColorOf(user({ ring_style: 'sparkle', ring_color: 'red' }))).toBe('green')
    // what a blocked viewer gets (§6.4): no fields at all
    expect(ringStyleOf(user())).toBe('classic')
    expect(ringColorOf(user())).toBe('green')
    expect(ringStyleOf(user({ ring_style: 'bars', ring_color: 'purple' }))).toBe('bars')
    expect(ringColorOf(user({ ring_style: 'bars', ring_color: 'purple' }))).toBe('purple')
  })

  it('a_palette_colour_is_its_theme_token', () => {
    expect(speakCss(user(), 'gold', '#1b1c21')).toBe('var(--speak-gold)')
    expect(speakCss(null, 'green', '#1b1c21')).toBe('var(--speak-green)')
  })

  it('profile_colour_is_the_first_name_colour_or_green', () => {
    expect(speakCss(user({}, ['#ffc4a3', '#ff7fae']), 'profile', '#1b1c21')).toBe('#ffc4a3')
    expect(speakCss(user({}, []), 'profile', '#1b1c21')).toBe('var(--speak-green)')
    expect(speakCss(null, 'profile', '#1b1c21')).toBe('var(--speak-green)')
    expect(speakCss(user({}, ['not-a-colour']), 'profile', '#1b1c21')).toBe('var(--speak-green)')
  })

  it('profile_colour_is_made_readable_on_the_theme', () => {
    const c = speakCss(user({}, ['#ffe58f']), 'profile', '#f4f4f6')
    expect(c).toMatch(/^#[0-9a-f]{6}$/i)
    expect(contrast(c, '#f4f4f6')).toBeGreaterThanOrEqual(3)
  })

  it('ring_state_table', () => {
    const base = { lit: true, place: 'stage' as RingPlace, style: 'classic' as RingStyle, reducedMotion: false, streaming: false, animating: true }
    expect(ringState({ ...base, lit: false })).toBe('off')
    expect(ringState(base)).toBe('on')
    expect(ringState({ ...base, style: 'bars' })).toBe('on')
    expect(ringState({ ...base, reducedMotion: true })).toBe('still')
    expect(ringState({ ...base, streaming: true })).toBe('still')
    expect(ringState({ ...base, animating: false })).toBe('still')
    expect(ringState({ ...base, place: 'list' })).toBe('still')
    expect(ringState({ ...base, place: 'list', lit: false })).toBe('off')
  })

  it('classic_for_all_forces_the_style_but_keeps_the_colour', () => {
    expect(effectiveStyle('bars', { classicForAll: true })).toBe('classic')
    expect(effectiveStyle('bars', { classicForAll: false })).toBe('bars')
    // the colour comes from the user, untouched by the setting
    expect(ringColorOf(user({ ring_style: 'bars', ring_color: 'purple' }))).toBe('purple')
  })

  it('only_visible_reactive_effective_styles_watch_levels', () => {
    const o = { reducedMotion: false, streaming: false, place: 'stage' as RingPlace }
    for (const s of RING_STYLES) expect(watchesLevels(s.key, o), s.key).toBe(s.reactive)
    expect(watchesLevels('bars', { ...o, reducedMotion: true })).toBe(false)
    expect(watchesLevels('bars', { ...o, streaming: true })).toBe(false)
    expect(watchesLevels('bars', { ...o, place: 'list' })).toBe(false)
    expect(watchesLevels('bars', { ...o, place: 'preview' })).toBe(false)
    expect(watchesLevels('dots', { ...o, place: 'band' })).toBe(true)
    expect(watchesLevels('liquid', { ...o, place: 'strip' })).toBe(true)
  })
})
