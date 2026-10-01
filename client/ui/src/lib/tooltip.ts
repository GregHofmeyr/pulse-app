// App-wide themed tooltips. Any button with an aria-label and no visible text (i.e. icon buttons)
// gets one automatically; `data-tip="…"` overrides or adds one to anything else.
type Rect = { left: number; top: number; width: number; height: number }
type Size = { width: number; height: number }

const GAP = 8
const MARGIN = 8
const DELAY_MS = 350

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
  let current: Element | null = null

  const hide = () => {
    clearTimeout(timer)
    current = null
    tip.classList.remove('show')
  }
  const show = (el: Element, text: string) => {
    tip.textContent = text
    tip.classList.add('show')
    const p = placeTip(el.getBoundingClientRect(), tip.getBoundingClientRect(), { width: window.innerWidth, height: window.innerHeight })
    tip.style.left = `${p.left}px`
    tip.style.top = `${p.top}px`
  }
  const enter = (e: Event) => {
    const el = (e.target as Element | null)?.closest?.('[data-tip], button[aria-label]')
    if (!el || el === current) return
    const text = labelFor(el)
    if (!text) return
    hide()
    current = el
    timer = setTimeout(() => current === el && show(el, text), e.type === 'focusin' ? 0 : DELAY_MS)
  }
  const leave = (e: Event) => {
    if (current && !(current as Element).contains((e as MouseEvent).relatedTarget as Node | null)) hide()
  }

  document.addEventListener('mouseover', enter)
  document.addEventListener('focusin', enter)
  document.addEventListener('mouseout', leave)
  document.addEventListener('focusout', hide)
  document.addEventListener('mousedown', hide)
  return () => {
    document.removeEventListener('mouseover', enter)
    document.removeEventListener('focusin', enter)
    document.removeEventListener('mouseout', leave)
    document.removeEventListener('focusout', hide)
    document.removeEventListener('mousedown', hide)
    tip.remove()
  }
}
