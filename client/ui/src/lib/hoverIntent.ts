// Hover intent for submenus (#173): a flyout opened by hover doesn't close the instant the pointer crosses another
// item of its menu, so a diagonal move from the item toward the flyout (over the next item) keeps it open. Leaving
// starts a short timer; arriving back on the item or in the flyout cancels it.

/** How long a hover-opened flyout waits before closing once the pointer is over another item. */
export const SUBMENU_CLOSE_DELAY = 200

/** The stage's Spotify pill (#288): how long the pointer rests on it, having moved, before it grows into its card. */
export const PILL_OPEN_DELAY = 200
/** How long the card waits once the pointer has left it before it shrinks back into the pill. */
export const PILL_CLOSE_GRACE = 300

export type CloseTimer = { schedule: (close: () => void) => void; cancel: () => void }

export function closeTimer(delay = SUBMENU_CLOSE_DELAY): CloseTimer {
  let t: ReturnType<typeof setTimeout> | undefined
  return {
    schedule(close) {
      clearTimeout(t)
      t = setTimeout(() => {
        t = undefined
        close()
      }, delay)
    },
    cancel() {
      clearTimeout(t)
      t = undefined
    },
  }
}

/** Quick peek (#619): how long the pointer rests on a build's card or row before its peek shows (spec §4.4: ~600 ms). */
export const PEEK_OPEN_DELAY = 600
/** How long the peek waits once the pointer has left both the card and the peek before it closes. */
export const PEEK_CLOSE_GRACE = 300

export type PeekHover = { enter: () => void; leave: () => void; stay: () => void; stop: () => void }

/** A card's peek by hover: `enter` the card starts the open delay, `leave` (card or peek) starts the close grace,
 *  `stay` (onto the peek) cancels it; `stop` cancels both (the card goes, or the peek was opened or closed another way). */
export function peekHover(set: (open: boolean) => void): PeekHover {
  const opening = closeTimer(PEEK_OPEN_DELAY)
  const closing = closeTimer(PEEK_CLOSE_GRACE)
  return {
    enter() {
      closing.cancel()
      opening.schedule(() => set(true))
    },
    leave() {
      opening.cancel()
      closing.schedule(() => set(false))
    },
    stay: () => closing.cancel(),
    stop() {
      opening.cancel()
      closing.cancel()
    },
  }
}
