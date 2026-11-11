// FLIP glides (#181): something that jumps to a new place (the stream mini preview snapping to a corner) is drawn
// where it was and slides home, transform only. Svelte isn't running it, so like exitFade it checks the motion rules
// itself: under reduced motion, while you're live or with the window in the background, it just lands.
import { motion } from './animate.svelte'
import { windowFocus } from './windowFocus.svelte'

/** app.css --dur-slow, in ms: the few larger moves. */
export const DUR_SLOW = 200
/** app.css --ease. */
export const EASE = 'cubic-bezier(.2, .8, .2, 1)'

/** Slide `el`, now laid out at `to`, from where it was drawn before (`from`). */
export function glideFrom(el: HTMLElement, from: Pick<DOMRect, 'left' | 'top'>, to: Pick<DOMRect, 'left' | 'top'>): void {
  const dx = Math.round(from.left - to.left)
  const dy = Math.round(from.top - to.top)
  if (!dx && !dy) return
  if (!mayMove(el)) return
  el.animate([{ translate: `${dx}px ${dy}px` }, { translate: '0px 0px' }], { duration: DUR_SLOW, easing: EASE })
}

/** app.css --dur-exit, in ms: exits, quicker than entrances. */
export const DUR_EXIT = 100
/** app.css --ease-exit. */
export const EASE_EXIT = 'cubic-bezier(.4, 0, 1, 1)'

/** Whether a scripted move may run on `el` now: never under reduced motion, while you're live or with the window in
 *  the background, as app.css's rules for transitions. */
function mayMove(el: HTMLElement): boolean {
  return typeof el.animate === 'function' && !motion.reducedMotion && !motion.streaming && windowFocus.focused
}

// the pill's corners (--radius-xl) and a menu surface's (--radius-lg)
const PILL_RADIUS = 16
const CARD_RADIUS = 12

/** Grow `el` (a card anchored by its bottom-right corner, laid out at full size) out of a `from`-sized pill in that
 *  corner (#288: the stage's Spotify pill opening into its card), clip-path only so nothing re-lays out; `back` shrinks
 *  it into the pill again (hold its last frame: the caller swaps the pill in when it finishes). Null when it just
 *  lands: the motion rules above, or nothing laid out to measure. */
export function growFromCorner(el: HTMLElement, from: { width: number; height: number }, back = false): Animation | null {
  if (!mayMove(el)) return null
  const to = el.getBoundingClientRect()
  if (!to.width || !to.height || !from.width || !from.height) return null
  const pill = `inset(${Math.max(0, Math.round(to.height - from.height))}px 0px 0px ${Math.max(0, Math.round(to.width - from.width))}px round ${PILL_RADIUS}px)`
  const card = `inset(0px 0px 0px 0px round ${CARD_RADIUS}px)`
  return back
    ? el.animate([{ clipPath: card }, { clipPath: pill }], { duration: DUR_EXIT, easing: EASE_EXIT, fill: 'forwards' })
    : el.animate([{ clipPath: pill }, { clipPath: card }], { duration: DUR_SLOW, easing: EASE })
}
