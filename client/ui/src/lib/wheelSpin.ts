// Running a wheel's spin on screen (#158, #270): shared by a spin's line, a quiet spin's card and a Just me wheel's
// private spin in the popover. Turns the wheel to `angle` (lib/wheel.ts rotationAt), ticking as each slice passes
// the pointer (never when `quiet`: reduced motion, streaming or an unfocused window), then says it landed.
import { motion } from './animate.svelte'
import { playSound } from './voiceui'
import { rotationAt, spinDuration, TICK_GAP_MS, winnerAt } from './wheel'
import { windowFocus } from './windowFocus.svelte'

/** Whether a spin starting now should be the short, silent one. */
export const quietNow = (): boolean => motion.reducedMotion || motion.streaming || !windowFocus.focused

/** Spin a wheel of `count` slices to `angle`. Returns a stop function (an unmount mid-spin). */
export function runSpin(o: { angle: number; count: number; quiet: boolean; onRotate: (deg: number) => void; onLanded: () => void }): () => void {
  const duration = spinDuration(o.quiet)
  let start: number | null = null
  let raf = 0
  let slice = winnerAt(0, o.count)
  let lastTick = -Infinity
  const frame = (t: number) => {
    start ??= t
    const elapsed = t - start
    const rotation = rotationAt(elapsed, o.angle, o.quiet)
    o.onRotate(rotation)
    const now = winnerAt(rotation, o.count)
    if (now !== slice) {
      slice = now
      if (!o.quiet && t - lastTick >= TICK_GAP_MS) {
        lastTick = t
        playSound('wheel_tick')
      }
    }
    if (elapsed < duration) raf = requestAnimationFrame(frame)
    else o.onLanded()
  }
  raf = requestAnimationFrame(frame)
  return () => cancelAnimationFrame(raf)
}
