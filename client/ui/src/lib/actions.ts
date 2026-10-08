// Svelte actions shared by popovers and menus.

/** Calls `onOutside` on any pointer press outside `node` (or Escape). Presses on a
 *  `[data-menu-toggle]` element are ignored: that button toggles the menu itself. */
export function clickOutside(node: HTMLElement, onOutside: () => void) {
  let cb = onOutside
  const down = (e: PointerEvent) => {
    const t = e.target as Element
    if (!node.contains(t) && !t.closest?.('[data-menu-toggle]')) cb()
  }
  const key = (e: KeyboardEvent) => {
    if (e.key === 'Escape') cb()
  }
  // Defer so the click that opened the menu doesn't immediately close it.
  const t = setTimeout(() => {
    window.addEventListener('pointerdown', down, true)
    window.addEventListener('keydown', key)
  })
  return {
    update(next: () => void) {
      cb = next
    },
    destroy() {
      clearTimeout(t)
      window.removeEventListener('pointerdown', down, true)
      window.removeEventListener('keydown', key)
    },
  }
}
