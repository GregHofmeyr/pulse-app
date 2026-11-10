// Svelte actions shared by popovers and menus.

/** The `[data-menu-toggle]` the current press or click is on (null when it's on none). A key press clears it, so a
 *  menu a key opens without a click (Shift+F10, ContextMenu) doesn't take the last clicked toggle as its own. */
let pressedToggle: Element | null = null
let tracking = false
function trackToggles() {
  if (tracking || typeof window === 'undefined') return
  tracking = true
  const on = (e: Event) => (pressedToggle = (e.target as Element | null)?.closest?.('[data-menu-toggle]') ?? null)
  window.addEventListener('pointerdown', on, true)
  window.addEventListener('click', on, true) // Enter / Space on a toggle clicks it with no press
  window.addEventListener('keydown', () => (pressedToggle = null), true)
}
trackToggles()

/** Calls `onOutside` on any pointer press outside `node` (or Escape), with the event that did it.
 *  A press on the `[data-menu-toggle]` that opened it is ignored: that button toggles the menu itself. A press on any
 *  other toggle closes it like any outside press, so only one menu is open at a time (#192). */
export function clickOutside(node: HTMLElement, onOutside: (e: Event) => void) {
  let cb = onOutside
  const own = pressedToggle
  const down = (e: PointerEvent) => {
    const t = e.target as Element
    if (node.contains(t)) return
    if (own && own.contains(t)) return
    cb(e)
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
 * `inert` while it's open, from just after its first frame (mark a click-to-close backdrop `data-backdrop` to
 * spare it); popovers rendered inside the modal stay usable. A modal opened over another (e.g. an image inside a dialog)
 * takes Escape and Tab until it goes.
 */
export function modal(node: HTMLElement, onClose: () => void) {
  let cb = onClose
  const before = document.activeElement as HTMLElement | null
  const items = () => [...node.querySelectorAll<HTMLElement>(FOCUSABLE)]
  ;(items()[0] ?? node).focus()
  // Inert lands just after the modal's first frame (#238): it restyles everything behind it, a whole-app style
  // recalc that made the open frame long. Until then the focus is already inside and the backdrop takes the pointer.
  // Only what we set gets cleared: something already inert (e.g. another modal's) stays that way.
  // Spared: the modal's own `[data-backdrop]` (click to close), and the app-wide tooltip, which
  // lives on <body> and also serves the modal's own controls.
  const inerted: Element[] = []
  const makeInert = () => {
    for (let el: Element = node; el.parentElement && el !== document.body; el = el.parentElement) {
      for (const sib of el.parentElement.children) {
        if (sib === el || sib.hasAttribute('inert') || sib.hasAttribute('data-backdrop')) continue
        if (sib instanceof HTMLScriptElement || sib.getAttribute('role') === 'tooltip') continue
        sib.setAttribute('inert', '')
        inerted.push(sib)
      }
    }
  }
  let inertTimer: ReturnType<typeof setTimeout> | undefined
  const inertFrame = requestAnimationFrame(() => (inertTimer = setTimeout(makeInert)))
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
      cancelAnimationFrame(inertFrame)
      clearTimeout(inertTimer)
      for (const el of inerted) el.removeAttribute('inert')
      // After clearing inert: an inert element can't take focus.
      if (before?.isConnected) before.focus()
    },
  }
}

/**
 * Arrow keys (and Home / End) for a `role="radiogroup"` of `role="radio"` buttons: moves focus to the
 * next radio and picks it, wrapping, as native radio buttons do. Pair it with a roving tabindex (the
 * checked radio 0, the rest -1) so Tab enters the group once.
 */
export function radioKeys(e: KeyboardEvent & { currentTarget: EventTarget & HTMLElement }) {
  const radios = [...e.currentTarget.querySelectorAll<HTMLElement>('[role="radio"]')]
  const i = radios.indexOf(document.activeElement as HTMLElement)
  if (i < 0 || radios.length === 0) return
  const n = radios.length
  const to = { ArrowRight: i + 1, ArrowDown: i + 1, ArrowLeft: i - 1, ArrowUp: i - 1, Home: 0, End: n - 1 }[e.key]
  if (to === undefined) return
  e.preventDefault()
  const next = radios[(to + n) % n]
  next.focus()
  next.click()
}

/** `use:narrowerThan={{ px, set }}`: tells `set` whether the element is narrower than `px`, at once and on every
 *  resize (#97: headers fold their search box and chips). Without ResizeObserver it never says narrow. */
export function narrowerThan(node: HTMLElement, opts: { px: number; set: (narrow: boolean) => void }) {
  let o = opts
  if (typeof ResizeObserver === 'undefined') {
    o.set(false)
    return { update: (n: typeof opts) => (o = n), destroy() {} }
  }
  // No synchronous clientWidth read here: it forced a layout on every channel switch (#173). A ResizeObserver reports
  // a newly observed element's size before the next paint, so the first frame is already right.
  const ro = new ResizeObserver((entries) => {
    const w = entries[entries.length - 1]?.contentRect.width ?? 0
    o.set(w > 0 && w < o.px)
  })
  ro.observe(node)
  return {
    update(n: typeof opts) {
      o = n
    },
    destroy() {
      ro.disconnect()
    },
  }
}
