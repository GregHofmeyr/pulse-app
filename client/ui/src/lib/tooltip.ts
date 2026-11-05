// App-wide themed tooltips. Any button with an aria-label and no visible text (i.e. icon buttons)
// gets one automatically; `data-tip="…"` overrides or adds one to anything else. A target whose text arrives later
// (the reaction chips' reactor names, #173) carries `data-tip-live`: it takes part in the delay and the handoff like
// any other, and its tip shows, or changes, the moment its `data-tip` is set.
type Rect = { left: number; top: number; width: number; height: number }
type Size = { width: number; height: number }

const GAP = 8
const MARGIN = 8
const DELAY_MS = 350
/** How long a tip outlives the pointer leaving its target, for the next target to take it over. */
const HANDOFF_MS = 100
/** How long after a tip closes the next one shows without the delay. */
const WARM_MS = 300

/** The hover delay, shared with a live tip that fetches its text (ReactionRow waits it before asking the server). */
export const tipTiming = { delay: DELAY_MS }
let warmNow: () => boolean = () => false
/** Whether a tip is up or just closed: the next one shows at once, so a live tip fetches at once too. */
export const tipIsWarm = (): boolean => warmNow()

/** Above the target, centred; below if there's no room; always inside the viewport. */
export function placeTip(target: Rect, tip: Size, viewport: Size): { left: number; top: number; below: boolean } {
  const below = target.top - tip.height - GAP < MARGIN
  const top = below ? target.top + target.height + GAP : target.top - tip.height - GAP
  const centred = target.left + target.width / 2 - tip.width / 2
  const left = Math.min(Math.max(centred, MARGIN), viewport.width - tip.width - MARGIN)
  return { left: Math.round(left), top: Math.round(top), below }
}

function labelFor(el: Element): string | null {
  const explicit = el.getAttribute('data-tip')
  if (explicit) return explicit
  if (el.tagName === 'BUTTON' && el.getAttribute('aria-label') && !el.textContent?.trim()) return el.getAttribute('aria-label')
  return null
}

export function installTooltips(): () => void {
  const tip = document.createElement('div')
  tip.className = 'pulse-tooltip'
  tip.setAttribute('role', 'tooltip')
  document.body.appendChild(tip)
  let timer: ReturnType<typeof setTimeout> | undefined
  let hideTimer: ReturnType<typeof setTimeout> | undefined
  let current: Element | null = null
  /** The target whose delay has run out: its tip shows as soon as it has text. */
  let due: Element | null = null
  /** Until when a new tip skips the delay: a tip was just up (#159). */
  let warmUntil = 0
  const shown = () => tip.classList.contains('show')

  const hide = () => {
    clearTimeout(timer)
    clearTimeout(hideTimer)
    if (shown()) warmUntil = Date.now() + WARM_MS
    current = null
    due = null
    tip.classList.remove('show')
  }
  const show = (el: Element, text: string) => {
    tip.textContent = text
    tip.classList.add('show')
    const p = placeTip(el.getBoundingClientRect(), tip.getBoundingClientRect(), { width: window.innerWidth, height: window.innerHeight })
    tip.style.left = `${p.left}px`
    tip.style.top = `${p.top}px`
  }
  // A mouse press hides the tip, then focuses what it pressed: that focus must not show it again at
  // once (#103: clicking ⋯ on a clip popped its tip up with the menu). Keyboard focus still does.
  let pressing = false
  const press = () => {
    pressing = true
    hide()
  }
  const release = () => (pressing = false)
  const enter = (e: Event) => {
    if (e.type === 'focusin' && pressing) return
    const el = (e.target as Element | null)?.closest?.('[data-tip], [data-tip-live], button[aria-label]')
    if (el && el === current) {
      clearTimeout(hideTimer) // back on it within the grace: it stays
      return
    }
    if (!el) return
    const text = labelFor(el)
    const live = el.hasAttribute('data-tip-live')
    if (!text && !live) return
    clearTimeout(timer)
    clearTimeout(hideTimer)
    current = el
    due = null
    // An open tip hands over: new text and place, no fade out and back in (#159: across a message's
    // quick actions every crossing flashed it). One that just closed comes back without the delay.
    if (shown()) {
      due = el
      if (text) return show(el, text)
      tip.classList.remove('show') // a live tip with no text yet: it shows when its text arrives
      warmUntil = Date.now() + WARM_MS
      return
    }
    const delay = e.type === 'focusin' || Date.now() < warmUntil ? 0 : tipTiming.delay
    timer = setTimeout(() => {
      if (current !== el) return
      due = el
      const now = labelFor(el) // read when it shows: a live tip may have its text by then
      if (now) show(el, now)
    }, delay)
  }
  // A live tip's text arriving or changing: shown in place once its delay has run.
  const observer = typeof MutationObserver === 'function'
    ? new MutationObserver((records) => {
        for (const r of records) {
          if (r.target !== current || due !== current) continue
          const text = labelFor(current)
          if (text) show(current, text)
          else if (shown()) tip.classList.remove('show')
        }
      })
    : null
  observer?.observe(document.body, { subtree: true, attributes: true, attributeFilter: ['data-tip'] })
  warmNow = () => shown() || Date.now() < warmUntil
  // A click from the keyboard (Enter, Space) leaves the tip up: a toggle's label that changes with it
  // ("Show members" → "Hide members") is read again once the click is handled. A mouse press hid it.
  const clicked = () => {
    const el = current
    if (el) setTimeout(() => {
      const text = current === el && tip.classList.contains('show') ? labelFor(el) : null
      if (text) show(el, text)
    }, 0)
  }
  // Leaving waits a moment, so the button the pointer lands on next takes the tip over (its mouseover
  // comes right after this mouseout) instead of it fading out first.
  const leave = (e: Event) => {
    if (!current || (current as Element).contains((e as MouseEvent).relatedTarget as Node | null)) return
    clearTimeout(timer)
    clearTimeout(hideTimer)
    hideTimer = setTimeout(hide, HANDOFF_MS)
  }

  document.addEventListener('mouseover', enter)
  document.addEventListener('focusin', enter)
  document.addEventListener('mouseout', leave)
  document.addEventListener('focusout', hide)
  document.addEventListener('mousedown', press)
  // a release outside the window never arrives here: any key ends the press too
  document.addEventListener('mouseup', release)
  document.addEventListener('keydown', release)
  document.addEventListener('click', clicked)
  return () => {
    document.removeEventListener('mouseover', enter)
    document.removeEventListener('focusin', enter)
    document.removeEventListener('mouseout', leave)
    document.removeEventListener('focusout', hide)
    document.removeEventListener('mousedown', press)
    document.removeEventListener('mouseup', release)
    document.removeEventListener('keydown', release)
    document.removeEventListener('click', clicked)
    observer?.disconnect()
    warmNow = () => false
    tip.remove()
  }
}
