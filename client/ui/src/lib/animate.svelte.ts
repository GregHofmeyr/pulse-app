// When an animated custom emoji plays: only in a focused window, only on-screen, only if the
// "Animate emoji" setting allows it, and never under the OS reduced-motion preference. Otherwise
// its stored first frame shows. Both images are immutable-cached, so swapping src is cheap.
import { untrack } from 'svelte'
import type { Attachment } from 'svelte/attachments'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { mediaUrl } from './emoji'

export type AnimateMode = 'always' | 'hover' | 'never'
export const ANIMATE_MODES: AnimateMode[] = ['always', 'hover', 'never']

export function shouldAnimate(s: { mode: AnimateMode; focused: boolean; visible: boolean; hovered: boolean; reducedMotion: boolean }): boolean {
  if (!s.focused || !s.visible || s.reducedMotion) return false
  return s.mode === 'always' || (s.mode === 'hover' && s.hovered)
}

const MODE_KEY = 'pulse.animateEmoji'

function savedMode(): AnimateMode {
  try {
    const v = localStorage.getItem(MODE_KEY)
    return ANIMATE_MODES.includes(v as AnimateMode) ? (v as AnimateMode) : 'always'
  } catch {
    return 'always'
  }
}

/** The one shared source every animated emoji follows. */
export const motion = $state({ mode: savedMode(), focused: true, reducedMotion: false })

/** The "Animate emoji" setting (this device only, like the skin tone). */
export function setAnimateMode(m: AnimateMode): void {
  motion.mode = m
  try {
    localStorage.setItem(MODE_KEY, m)
  } catch {
    /* ignore */
  }
}

let watching = false

/** Starts following window focus and reduced motion, once for the app's lifetime. */
export function watchMotion(): void {
  if (watching) return
  watching = true
  motion.focused = document.hasFocus()
  window.addEventListener('focus', () => (motion.focused = true))
  window.addEventListener('blur', () => (motion.focused = false))
  // The webview may not get DOM focus/blur when the OS window changes focus (WebKitGTK, WebView2
  // with focus inside a native control); the window's own event always comes. Last one wins.
  if ('__TAURI_INTERNALS__' in window) {
    try {
      void getCurrentWindow()
        .onFocusChanged(({ payload }) => (motion.focused = payload))
        .catch(() => {})
    } catch {
      /* DOM events only */
    }
  }
  const mq = typeof window.matchMedia === 'function' ? window.matchMedia('(prefers-reduced-motion: reduce)') : null
  if (mq) {
    motion.reducedMotion = mq.matches
    mq.addEventListener('change', (e) => (motion.reducedMotion = e.matches))
  }
}

// One IntersectionObserver for every animated emoji on screen.
let io: IntersectionObserver | null | undefined
const onSeen = new Map<Element, (visible: boolean) => void>()

function observe(el: Element, cb: (visible: boolean) => void): () => void {
  if (io === undefined) {
    io = typeof IntersectionObserver === 'function'
      ? new IntersectionObserver((entries) => {
          for (const e of entries) onSeen.get(e.target)?.(e.isIntersecting)
        })
      : null
  }
  if (!io) {
    cb(true) // can't tell: treat as on-screen
    return () => {}
  }
  onSeen.set(el, cb)
  io.observe(el)
  return () => {
    onSeen.delete(el)
    io?.unobserve(el)
  }
}

/**
 * Keeps `img`'s src on `anim` or `still` by the rule above. Hover counts over the button holding
 * the image (a reaction chip, picker cell or autocomplete row), else the image itself; `hovered`
 * overrides that. Returns the cleanup.
 */
export function wireEmoji(img: HTMLImageElement, anim: string, still: string, hovered?: () => boolean): () => void {
  return untrack(() => {
    watchMotion()
    let visible = $state(false)
    let pointer = $state(false)
    const target = img.closest('button') ?? img
    const enter = () => (pointer = true)
    const leave = () => (pointer = false)
    if (!hovered) {
      target.addEventListener('pointerenter', enter)
      target.addEventListener('pointerleave', leave)
    }
    const unobserve = observe(img, (v) => (visible = v))
    const stop = $effect.root(() => {
      $effect(() => {
        const on = shouldAnimate({ ...motion, visible, hovered: hovered ? hovered() : pointer })
        const src = on ? anim : still
        if (img.getAttribute('src') !== src) img.setAttribute('src', src)
      })
    })
    return () => {
      stop()
      unobserve()
      target.removeEventListener('pointerenter', enter)
      target.removeEventListener('pointerleave', leave)
    }
  })
}

const isAnimated = (hash: string | null, still: string | null): still is string => !!hash && !!still && still !== hash

/** The src to render a custom emoji with: its still first frame if it has one. */
export function stillSrc(e: { hash: string | null; still_hash: string | null }): string {
  return mediaUrl(e.still_hash ?? e.hash!)
}

/** `{@attach}` for one custom emoji <img> rendered with `stillSrc`; a no-op for still emoji. */
export function animatedEmoji(e: { hash: string | null; still_hash: string | null }, hovered?: () => boolean): Attachment<HTMLImageElement> {
  const { hash, still_hash } = e
  return (img) => {
    if (isAnimated(hash, still_hash)) return wireEmoji(img, mediaUrl(hash!), mediaUrl(still_hash), hovered)
  }
}

/** `{@attach}` for rendered message HTML: wires each `img[data-anim]` in it. Pass the HTML so a
 * new render re-wires. */
export function animateEmojiIn(_html: string): Attachment<HTMLElement> {
  return (node) => {
    const stops = Array.from(node.querySelectorAll<HTMLImageElement>('img[data-anim]'), (img) =>
      wireEmoji(img, mediaUrl(img.dataset.anim!), img.getAttribute('src')!),
    )
    return () => stops.forEach((s) => s())
  }
}
