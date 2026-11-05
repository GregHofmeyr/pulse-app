// Hover intent for submenus (#173): a flyout opened by hover doesn't close the instant the pointer crosses another
// item of its menu, so a diagonal move from the item toward the flyout (over the next item) keeps it open. Leaving
// starts a short timer; arriving back on the item or in the flyout cancels it.

/** How long a hover-opened flyout waits before closing once the pointer is over another item. */
export const SUBMENU_CLOSE_DELAY = 200

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
