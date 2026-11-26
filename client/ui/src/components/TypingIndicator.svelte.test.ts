// @vitest-environment jsdom
// The typing line under a chat (#652): one typer's own words or the plain line, a soft entrance, a crossfade when the
// words change, its height always held, and a fade only under reduced motion.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { flushSync, mount, unmount } from 'svelte'
import { compile } from 'svelte/compiler'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import TypingIndicator from './TypingIndicator.svelte'
import { typingLine, type Typer } from '../lib/typingLine'
import { DUR_EXIT, exitFade } from '../lib/exit'
import { motion } from '../lib/animate.svelte'
import { windowFocus } from '../lib/windowFocus.svelte'

const sam: Typer = { id: 'U2', name: 'sam', custom: 'cooking' }
const kim: Typer = { id: 'U3', name: 'kim', custom: 'barking' }
const jo: Typer = { id: 'U4', name: 'jo', custom: null }

describe('TypingIndicator', () => {
  let c: ReturnType<typeof mount> | undefined
  const props = $state<{ line: ReturnType<typeof typingLine> }>({ line: null })
  const show = (typers: Typer[]) => {
    props.line = typingLine(typers)
    flushSync()
  }
  const lines = () => [...document.querySelectorAll('.typing-line')]
  const text = () => document.querySelector('.typing')?.textContent?.trim()
  afterEach(() => {
    if (c) unmount(c)
    c = undefined
    props.line = null
    document.body.innerHTML = ''
  })
  const start = () => {
    c = mount(TypingIndicator, { target: document.body, props })
    flushSync()
  }

  it('is an empty, polite live region when nobody types (its height is still there)', () => {
    start()
    expect(document.querySelector('.typing')?.getAttribute('aria-live')).toBe('polite')
    expect(lines()).toHaveLength(0)
  })

  it('one typer with typing text: "<name> is <text>", the name bold', () => {
    start()
    show([sam])
    expect(text()).toBe('sam is cooking')
    expect(document.querySelector('.typing-line strong')?.textContent).toBe('sam')
  })

  it('one typer without: "<name> is typing…"', () => {
    start()
    show([jo])
    expect(text()).toBe('jo is typing…')
  })

  it("two typers: the plain line, whatever their typing text", () => {
    start()
    show([sam, kim])
    expect(text()).toBe('sam, kim are typing…')
  })

  it('appearing from nothing rises in; changed words crossfade in; the same words keep their element', () => {
    start()
    show([sam])
    const first = lines()[0]
    expect(first.classList.contains('rise')).toBe(true)
    show([sam]) // a repeat Typing: a new line object, the same words
    expect(lines()).toEqual([first])
    expect(first.classList.contains('rise')).toBe(true) // its entrance isn't restarted
    show([sam, kim])
    const second = lines().at(-1)!
    expect(second).not.toBe(first)
    expect(second.classList.contains('swap')).toBe(true)
    expect(second.classList.contains('rise')).toBe(false)
    show([kim])
    expect(text()).toBe('kim is barking')
    show([])
    expect(text()).toBe('')
    show([jo])
    expect(lines().at(-1)!.classList.contains('rise')).toBe(true)
  })
})

describe('leaving (#652 review)', () => {
  // an animation that never finishes: whatever waits for it stays on screen
  const hang = () => ({ onfinish: null, oncancel: null, cancel() {}, finish() {}, play() {}, pause() {}, currentTime: 0, finished: new Promise(() => {}) })
  let saved: unknown
  let c: ReturnType<typeof mount> | undefined
  const props = $state<{ line: ReturnType<typeof typingLine> }>({ line: null })
  beforeEach(() => {
    saved = (Element.prototype as unknown as { animate?: unknown }).animate
    ;(Element.prototype as unknown as { animate: unknown }).animate = hang
    windowFocus.focused = true
  })
  afterEach(() => {
    ;(Element.prototype as unknown as { animate?: unknown }).animate = saved
    if (c) unmount(c)
    c = undefined
    props.line = null
    document.body.innerHTML = ''
  })

  it('the line going still fades out (it stays while its fade runs)', () => {
    c = mount(TypingIndicator, { target: document.body, props })
    props.line = typingLine([sam])
    flushSync()
    props.line = null
    flushSync()
    expect(document.querySelectorAll('.typing-line')).toHaveLength(1)
  })

  it('the chat around it going (a channel switch, closing the chat) never waits for that fade', async () => {
    c = mount(TypingIndicator, { target: document.body, props })
    props.line = typingLine([sam])
    flushSync()
    const done = unmount(c, { outro: true })
    c = undefined
    flushSync()
    expect(document.querySelector('.typing')).toBeNull()
    await done
  })
})

describe('the leave fade', () => {
  const withAnimate = () => Object.assign(document.createElement('div'), { animate: () => {} })
  afterEach(() => {
    motion.reducedMotion = false
    motion.streaming = false
    windowFocus.focused = true
  })

  it('keeps its fade under reduced motion when asked (nothing moves), and still goes at once while live or unfocused', () => {
    motion.reducedMotion = true
    expect(exitFade(withAnimate()).duration).toBe(0)
    expect(exitFade(withAnimate(), { fadeWhenReduced: true }).duration).toBe(DUR_EXIT)
    motion.reducedMotion = false
    motion.streaming = true
    expect(exitFade(withAnimate(), { fadeWhenReduced: true }).duration).toBe(0)
    motion.streaming = false
    windowFocus.focused = false
    expect(exitFade(withAnimate(), { fadeWhenReduced: true }).duration).toBe(0)
  })

  it('the line leaves with it, a local outro (never |global: the chat being destroyed would wait for it)', () => {
    const src = readFileSync(resolve(__dirname, 'TypingIndicator.svelte'), 'utf8')
    expect(src).toMatch(/out:exitFade=\{\{ fadeWhenReduced: true \}\}/)
    expect(src).not.toMatch(/\|global/)
  })
})

describe('TypingIndicator CSS', () => {
  const css = compile(readFileSync(resolve(__dirname, 'TypingIndicator.svelte'), 'utf8'), { filename: 'TypingIndicator.svelte', css: 'external', cssHash: () => 's' }).css!.code.replace(/\/\*[\s\S]*?\*\//g, '')
  const rule = (sel: string) => css.match(new RegExp(`(?:^|\\n|\\})\\s*${sel.replace(/\./g, '\\.')}\\.s\\s*\\{([^}]*)\\}`))?.[1] ?? ''
  const reduced = css.match(/@media \(prefers-reduced-motion: reduce\)\s*\{([\s\S]*?\})\s*\}/)?.[1] ?? ''

  it('holds its height whether or not anyone types, and clips what it holds', () => {
    const box = rule('.typing')
    expect(box).toMatch(/height:\s*22px/)
    expect(box).toMatch(/position:\s*relative/)
    expect(box).toMatch(/overflow:\s*hidden/)
  })

  it('a long line is one line ending in an ellipsis; the lines overlap, so a change crossfades in place', () => {
    const line = rule('.typing-line')
    expect(line).toMatch(/position:\s*absolute/)
    expect(line).toMatch(/white-space:\s*nowrap/)
    expect(line).toMatch(/overflow:\s*hidden/)
    expect(line).toMatch(/text-overflow:\s*ellipsis/)
  })

  it('rises in: a fade and a few px up, landing with the small bounce, over --dur-slow', () => {
    expect(css.match(/\.typing-line\.s\.rise\s*\{([^}]*)\}/)?.[1]).toMatch(/animation:\s*typing-rise var\(--dur-slow\) var\(--ease-overshoot\)/)
    const kf = css.match(/@keyframes typing-rise\s*\{([\s\S]*?\})\s*\}/)?.[1] ?? ''
    expect(kf).toMatch(/opacity:\s*0/)
    expect(kf).toMatch(/transform:\s*translateY\(\d+px\)/)
  })

  it('changed words fade in over --dur, without moving', () => {
    expect(css.match(/\.typing-line\.s\.swap\s*\{([^}]*)\}/)?.[1]).toMatch(/animation:\s*fade-in var\(--dur\) var\(--ease\)/)
  })

  it('under reduced motion: a fade only, never a move', () => {
    expect(reduced).toMatch(/\.typing-line\.s\.rise,\s*\.typing-line\.s\.swap\s*\{\s*animation:\s*fade-in var\(--dur\) var\(--ease\) !important;?\s*\}/)
    expect(reduced).not.toMatch(/transform|typing-rise/)
  })

  it('is switched off, not paused, while live or in the background (paused it would sit invisible)', () => {
    expect(css).toMatch(/\.streaming \.typing-line\.s:not\(\.entered\),\s*\.unfocused \.typing-line\.s:not\(\.entered\)\s*\{\s*animation:\s*none !important;?\s*\}/)
  })

  it('colour tokens only', () => {
    expect(css).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i)
  })
})
