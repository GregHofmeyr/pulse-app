// @vitest-environment jsdom
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { afterEach, describe, expect, it } from 'vitest'
import { DUR_EXIT, exitFade } from './exit'
import { motion } from './animate.svelte'
import { windowFocus } from './windowFocus.svelte'

const withAnimate = () => {
  const el = document.createElement('div')
  ;(el as unknown as { animate: () => void }).animate = () => {}
  return el
}

afterEach(() => {
  motion.reducedMotion = false
  motion.streaming = false
  windowFocus.focused = true
})

describe('exitFade (#173)', () => {
  it('fades opacity out over --dur-exit', () => {
    const t = exitFade(withAnimate())
    expect(t.duration).toBe(DUR_EXIT)
    expect(t.css!(0.5, 0.5)).toBe('opacity: 0.5')
    expect(readFileSync(resolve(__dirname, '../app.css'), 'utf8')).toContain(`--dur-exit: ${DUR_EXIT}ms;`)
  })

  it('goes at once under reduced motion, while live, in the background, or without element.animate', () => {
    motion.reducedMotion = true
    expect(exitFade(withAnimate()).duration).toBe(0)
    motion.reducedMotion = false
    motion.streaming = true
    expect(exitFade(withAnimate()).duration).toBe(0)
    motion.streaming = false
    windowFocus.focused = false
    expect(exitFade(withAnimate()).duration).toBe(0)
    windowFocus.focused = true
    expect(exitFade(document.createElement('div')).duration).toBe(document.createElement('div').animate ? DUR_EXIT : 0)
  })
})
