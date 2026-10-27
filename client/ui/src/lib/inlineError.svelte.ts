// A failed action's error line on one message: shown for a few seconds, a newer one restarts the clock.

/** How long a failed delete/reaction error stays on its message. */
export const INLINE_ERROR_MS = 5_000

export class InlineError {
  text = $state('')
  #timer: ReturnType<typeof setTimeout> | undefined
  // A reaction can fail after the message has gone from the screen: no timer for a component that's gone.
  #alive = true

  show = (msg: string): void => {
    if (!this.#alive) return
    clearTimeout(this.#timer)
    this.text = msg
    this.#timer = setTimeout(() => (this.text = ''), INLINE_ERROR_MS)
  }

  /** The owner went away: stop the clock and ignore late errors. */
  stop = (): void => {
    this.#alive = false
    clearTimeout(this.#timer)
  }
}
