// @vitest-environment jsdom
// FLIP glides (#181): a box that jumped (the stream mini preview snapping to a corner) slides from where it was.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { DUR_SLOW, EASE, glideFrom } from './flip'
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
