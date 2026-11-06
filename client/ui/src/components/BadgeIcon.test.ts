// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it } from 'vitest'
import BadgeIcon from './BadgeIcon.svelte'
import { mediaUrl } from '../lib/emoji'

let cmp: ReturnType<typeof mount> | undefined
function show(props: Record<string, unknown>) {
  cmp = mount(BadgeIcon, { target: document.body, props: { icon: '💬', rarity: 'rare', tier: 1, tiers: 1, ...props } })
  flushSync()
  return document.querySelector('.badge-icon') as HTMLElement
}
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('BadgeIcon', () => {
  it('the ring takes the rarity colour', () => {
    const el = show({ rarity: 'epic' })
    expect(el.classList.contains('epic')).toBe(true)
    expect(el.style.getPropertyValue('--ring-color')).toBe('var(--rarity-epic)')
    expect(el.textContent).toContain('💬')
  })

  it('a numeral only for a badge with tiers', () => {
    expect(show({ tier: 1, tiers: 1 }).querySelector('.tier')).toBeNull()
    unmount(cmp!)
    cmp = undefined
    expect(show({ tier: 3, tiers: 5 }).querySelector('.tier')!.textContent).toBe('III')
  })

  it('first earner: gold and a sparkle', () => {
    expect(show({}).querySelector('.spark')).toBeNull()
    unmount(cmp!)
    cmp = undefined
    const el = show({ first: true })
    expect(el.classList.contains('first')).toBe(true)
    expect(el.querySelector('.spark')!.textContent).toBe('✦')
  })

  it('hidden reads ???, and NEW when unseen', () => {
    const el = show({ hidden: true, isNew: true })
    expect(el.textContent).toContain('???')
    expect(el.textContent).not.toContain('💬')
    expect(el.querySelector('.new')!.textContent).toBe('NEW')
  })

  it('without art it draws the emoji and no picture', () => {
    const el = show({ art: null })
    expect(el.querySelector('img')).toBeNull()
    expect(el.querySelector('.glyph')!.textContent).toBe('💬')
  })

  it('with art it draws the picture inside the ring instead of the emoji, at every size', () => {
    const hash = 'ab'.repeat(32)
    for (const size of ['case', 'show'] as const) {
      const el = show({ art: hash, size, rarity: 'mythical' })
      const img = el.querySelector('img.art') as HTMLImageElement
      expect(img).not.toBeNull()
      expect(img.getAttribute('src')).toBe(mediaUrl(hash))
      // decoration: the badge's label (or the text beside it) names it
      expect(img.getAttribute('alt')).toBe('')
      expect(el.querySelector('.glyph')).toBeNull()
      expect(el.textContent).not.toContain('💬')
      expect(el.classList.contains('has-art')).toBe(true)
      expect(el.style.getPropertyValue('--ring-color')).toBe('var(--rarity-mythical)')
      unmount(cmp!)
      cmp = undefined
    }
  })

  it('a hidden badge never shows art', () => {
    const el = show({ art: 'ab'.repeat(32), hidden: true })
    expect(el.querySelector('img')).toBeNull()
    expect(el.textContent).toContain('???')
  })

  it('a malformed art hash falls back to the emoji', () => {
    const el = show({ art: '../x' })
    expect(el.querySelector('img')).toBeNull()
    expect(el.textContent).toContain('💬')
  })

  it('the showcase size is smaller', () => {
    expect(show({ size: 'show' }).classList.contains('show')).toBe(true)
  })
})
