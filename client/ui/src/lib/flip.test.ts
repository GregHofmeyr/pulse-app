// @vitest-environment jsdom
// FLIP glides (#181): a box that jumped (the stream mini preview snapping to a corner) slides from where it was.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { DUR_EXIT, DUR_SLOW, EASE, EASE_EXIT, glideFrom, growFromCorner } from './flip'
import { motion } from './animate.svelte'
import { windowFocus } from './windowFocus.svelte'

const box = (left: number, top: number) => ({ left, top }) as DOMRect

describe('glideFrom', () => {
  afterEach(() => {
    motion.reducedMotion = false
    motion.streaming = false
    windowFocus.focused = true
  })

  it('slides from the old place to the new one, transform only, on the slow motion token', () => {
    const el = document.createElement('div')
    el.animate = vi.fn() as never
    windowFocus.focused = true
    glideFrom(el, box(100, 60), box(16, 68))
    expect(el.animate).toHaveBeenCalledWith([{ translate: '84px -8px' }, { translate: '0px 0px' }], { duration: DUR_SLOW, easing: EASE })
  })

  it('the tokens are app.css’s', () => {
    const css = readFileSync(resolve(__dirname, '../app.css'), 'utf8')
    expect(css).toContain(`--dur-slow: ${DUR_SLOW}ms;`)
    expect(css).toContain(`--ease: ${EASE};`)
  })

  it('nothing moves when it did not move, under reduced motion, while live or in the background', () => {
    const el = document.createElement('div')
    el.animate = vi.fn() as never
    windowFocus.focused = true
    glideFrom(el, box(16, 68), box(16, 68))
    motion.reducedMotion = true
    glideFrom(el, box(100, 60), box(16, 68))
    motion.reducedMotion = false
    motion.streaming = true
    glideFrom(el, box(100, 60), box(16, 68))
    motion.streaming = false
    windowFocus.focused = false
    glideFrom(el, box(100, 60), box(16, 68))
    expect(el.animate).not.toHaveBeenCalled()
  })

  it('an engine without element.animate just lands', () => {
    const el = document.createElement('div')
    expect(() => glideFrom(el, box(100, 60), box(16, 68))).not.toThrow()
  })
})

describe('growFromCorner (#288: the Spotify pill grows into its card)', () => {
  afterEach(() => {
    motion.reducedMotion = false
    motion.streaming = false
    windowFocus.focused = true
  })
  const card = () => {
    const el = document.createElement('div')
    el.getBoundingClientRect = () => ({ width: 300, height: 120 }) as DOMRect
    el.animate = vi.fn(() => ({}) as Animation) as never
    return el
  }
  const pill = { width: 200, height: 34 }

  it('reveals the card from the pill-sized box in its bottom-right corner, clip only, on the slow token; back on the exit token', () => {
    const el = card()
    windowFocus.focused = true
    expect(growFromCorner(el, pill)).not.toBeNull()
    expect(el.animate).toHaveBeenCalledWith(
      [{ clipPath: 'inset(86px 0px 0px 100px round 16px)' }, { clipPath: 'inset(0px 0px 0px 0px round 12px)' }],
      { duration: DUR_SLOW, easing: EASE },
    )
    growFromCorner(el, pill, true)
    expect(el.animate).toHaveBeenLastCalledWith(
      [{ clipPath: 'inset(0px 0px 0px 0px round 12px)' }, { clipPath: 'inset(86px 0px 0px 100px round 16px)' }],
      { duration: DUR_EXIT, easing: EASE_EXIT, fill: 'forwards' },
    )
  })

  it('the exit tokens are app.css’s', () => {
    const css = readFileSync(resolve(__dirname, '../app.css'), 'utf8')
    expect(css).toContain(`--dur-exit: ${DUR_EXIT}ms;`)
    expect(css).toContain(`--ease-exit: ${EASE_EXIT};`)
  })

  it('instant under reduced motion, while live, in the background, unlaid out or without element.animate', () => {
    const el = card()
    motion.reducedMotion = true
    expect(growFromCorner(el, pill)).toBeNull()
    motion.reducedMotion = false
    motion.streaming = true
    expect(growFromCorner(el, pill)).toBeNull()
    motion.streaming = false
    windowFocus.focused = false
    expect(growFromCorner(el, pill)).toBeNull()
    windowFocus.focused = true
    expect(growFromCorner(el, { width: 0, height: 0 })).toBeNull()
    expect(el.animate).not.toHaveBeenCalled()
    expect(growFromCorner(document.createElement('div'), pill)).toBeNull()
  })
})
