// Svelte actions shared by popovers and menus.

/** Calls `onOutside` on any pointer press outside `node` (or Escape), with the event that did it.
 *  Presses on a `[data-menu-toggle]` element are ignored: that button toggles the menu itself. */
export function clickOutside(node: HTMLElement, onOutside: (e: Event) => void) {
  let cb = onOutside
  const down = (e: PointerEvent) => {
    const t = e.target as Element
    if (!node.contains(t) && !t.closest?.('[data-menu-toggle]')) cb(e)
  }
  const key = (e: KeyboardEvent) => {
    if (e.key !== 'Escape') return
    // handled: what sits under the menu (e.g. the full view) leaves it alone
    e.preventDefault()
    cb(e)
  }
  // Defer so the click that opened the menu doesn't immediately close it.
  const t = setTimeout(() => {
    window.addEventListener('pointerdown', down, true)
    window.addEventListener('keydown', key)
  })
  return {
    update(next: (e: Event) => void) {
      cb = next
    },
    destroy() {
      clearTimeout(t)
      window.removeEventListener('pointerdown', down, true)
      window.removeEventListener('keydown', key)
    },
  }
}

/** Open modals, oldest first: only the newest still on the page answers keys. */
const openModals: HTMLElement[] = []
function topModal(): HTMLElement | undefined {
  for (let i = openModals.length - 1; i >= 0; i--) if (openModals[i].isConnected) return openModals[i]
}

const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]):not([type=hidden]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

/**
 * Dialog behaviour for a modal's root (give it tabindex="-1"): focus moves in on open, Tab wraps
 * inside, Escape calls `onClose` (unless an inner control already took it), and focus returns to
 * what had it before when the modal goes away. Everything beside the modal's ancestor chain is
 * `inert` while it's open (mark a click-to-close backdrop `data-backdrop` to spare it); popovers
 * rendered inside the modal stay usable. A modal opened over another (e.g. an image inside a dialog)
 * takes Escape and Tab until it goes.
 */
export function modal(node: HTMLElement, onClose: () => void) {
  let cb = onClose
  const before = document.activeElement as HTMLElement | null
  // Only what we set gets cleared: something already inert (e.g. another modal's) stays that way.
  // Spared: the modal's own `[data-backdrop]` (click to close), and the app-wide tooltip, which
  // lives on <body> and also serves the modal's own controls.
  const inerted: Element[] = []
  for (let el: Element = node; el.parentElement && el !== document.body; el = el.parentElement) {
    for (const sib of el.parentElement.children) {
      if (sib === el || sib.hasAttribute('inert') || sib.hasAttribute('data-backdrop')) continue
      if (sib instanceof HTMLScriptElement || sib.getAttribute('role') === 'tooltip') continue
      sib.setAttribute('inert', '')
      inerted.push(sib)
    }
  }
  const items = () => [...node.querySelectorAll<HTMLElement>(FOCUSABLE)]
  ;(items()[0] ?? node).focus()
  // On window, in the bubble phase: Svelte delegates onkeydown to the mount root, so inner
  // handlers (an input's or Select's own Esc) have run, and can mark the event handled, by now.
  const key = (e: KeyboardEvent) => {
    if (topModal() !== node) return
    if (e.key === 'Escape') {
      if (!e.defaultPrevented) {
        e.preventDefault()
        cb()
      }
      return
    }
    if (e.key !== 'Tab') return
    const list = items()
    if (list.length === 0) return void e.preventDefault()
    const first = list[0]
    const last = list[list.length - 1]
    const at = document.activeElement
    if (!at || !node.contains(at)) {
      // focus fell out of the dialog (e.g. onto the page behind): bring it back
      e.preventDefault()
      ;(e.shiftKey ? last : first).focus()
    } else if (e.shiftKey && (at === first || at === node)) {
      e.preventDefault()
      last.focus()
    } else if (!e.shiftKey && (at === last || at === node)) {
      e.preventDefault()
      first.focus()
    }
  }
  window.addEventListener('keydown', key)
  openModals.push(node)
  return {
    update(next: () => void) {
      cb = next
    },
    destroy() {
      window.removeEventListener('keydown', key)
      const i = openModals.indexOf(node)
      if (i >= 0) openModals.splice(i, 1)
      for (const el of inerted) el.removeAttribute('inert')
      // After clearing inert: an inert element can't take focus.
      if (before?.isConnected) before.focus()
    },
  }
}
