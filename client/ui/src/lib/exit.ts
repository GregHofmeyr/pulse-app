// A short exit for things that otherwise blink away (#173): toasts and notices fade out over --dur-exit, easing in.
// Use as `out:exitFade`. Svelte runs it with the Web Animations API, which the .streaming / .unfocused CSS pause
// doesn't reach, so it checks the same conditions itself: while you're live, the window is in the background, or
// reduced motion is on (or the engine has no element.animate), the element goes at once.
import type { TransitionConfig } from 'svelte/transition'
import { motion } from './animate.svelte'
import { windowFocus } from './windowFocus.svelte'

/** app.css --dur-exit, in ms (motionCss.test.ts keeps them equal). */
export const DUR_EXIT = 100

/** cubic-bezier(.4, 0, 1, 1) (--ease-exit), near enough: an ease-in. */
const easeIn = (t: number) => t * t

export function exitFade(node: Element): TransitionConfig {
  if (typeof (node as HTMLElement).animate !== 'function' || motion.reducedMotion || motion.streaming || !windowFocus.focused) return { duration: 0 }
  return { duration: DUR_EXIT, easing: easeIn, css: (t) => `opacity: ${t}` }
}
