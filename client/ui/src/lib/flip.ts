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
  if (typeof el.animate !== 'function' || motion.reducedMotion || motion.streaming || !windowFocus.focused) return
  el.animate([{ translate: `${dx}px ${dy}px` }, { translate: '0px 0px' }], { duration: DUR_SLOW, easing: EASE })
}
