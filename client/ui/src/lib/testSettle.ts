// Shared ways for a test to let pending work finish before it asserts. Not used by the app.
//
// Which one to use:
// - Work that ends in a timer or a chain of real promises (a mocked `invoke` resolving, then the component reacting):
//   `nextTask()` waits one macrotask, which runs every microtask queued before it. Real timers only.
// - Svelte state that settles over a few promise hops: `settle()` (tick-based, flushes between hops) or
//   `flushMicrotasks()` (bare promise hops, one flush at the end). Both take how many hops to wait.
// - Both of the above: `settleTask()`.
// - Under `vi.useFakeTimers()`: `settleTimers()`, since a real `setTimeout` never fires there.
import { flushSync, tick } from 'svelte'
import { vi } from 'vitest'

/** One macrotask: every microtask queued so far runs first. Hangs under fake timers; use `settleTimers()` there. */
export const nextTask = () => new Promise<void>((r) => setTimeout(r, 0))

/** `turns` Svelte ticks (a microtask, then a flush, each), then one more flush. */
export async function settle(turns = 5) {
  for (let i = 0; i < turns; i++) await tick()
  flushSync()
}

/** `settle(turns)` without the last flush, then a macrotask, then a flush. `settleTask(0)` is just task then flush. */
export async function settleTask(turns = 5) {
  for (let i = 0; i < turns; i++) await tick()
  await nextTask()
  flushSync()
}

/** `turns` bare microtask hops, with no flush: for code that awaits promises but doesn't touch Svelte state. */
export async function microtasks(turns = 5) {
  for (let i = 0; i < turns; i++) await Promise.resolve()
}

/** `turns` bare microtask hops, then one flush. */
export async function flushMicrotasks(turns = 5) {
  await microtasks(turns)
  flushSync()
}

/** For fake timers: flush, run every timer due now (and the promises they resolve), flush again. */
export async function settleTimers() {
  flushSync()
  await vi.advanceTimersByTimeAsync(0)
  flushSync()
}
