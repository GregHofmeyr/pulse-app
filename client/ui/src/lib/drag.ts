// Drag to reorder the channel list (#55, #54), hand-built on pointer events. It reads the DOM ChannelList renders:
// `.list` > `.cat.top` (rows) and `.cat[data-category]` (a `.label` heading, then rows); a row is
// `.row[data-channel]`, and what follows it as its own (a voice row's chat icon, `.row-side`, and its `.occupant`s) moves
// with it.
//
// Smooth under load (#103): every box is measured once when the drag starts (and once more if a hovered section
// opens); each frame then only writes `transform`s, so nothing re-runs layout while the pointer moves. The drop
// renders the new order and clears the transforms in the same frame (`settling` turns transitions off for it), so
// nothing jumps; the dropped item then glides from where it was let go into its place. A drop that changes nothing,
// and a cancel (Escape, or the list changing under the drag: `cancelDrag`), ease everything back instead.
//
// Near the list's top or bottom edge the list scrolls (#193): a frame loop of its own, its speed in proportion to how
// close the pointer is, eased in and out (straight to speed under reduced motion). Each of its frames is one
// scrollTop write plus the usual transforms; the dragged item keeps to the pointer. A drop or Escape stops it.
import { flushSync } from 'svelte'
import { motion } from './animate.svelte'

export type Drop =
  | { kind: 'channel'; id: string; to: { category: string | null; index: number } }
  | { kind: 'category'; id: string; index: number }

/** A box along the list, in pixels from the list's top. */
export type Rect = { top: number; height: number }

/** Movement before a press becomes a drag: less is a click. */
export const DRAG_THRESHOLD = 4
/** Hovering a collapsed section this long while dragging a channel opens it. */
export const HOVER_OPEN_MS = 600

/** How close to the list's top or bottom edge, in pixels, the pointer starts it scrolling while dragging (#193). */
export const EDGE_ZONE = 48
/** The edge scroll's top speed, in pixels per millisecond (with the pointer on or past the edge). */
export const EDGE_MAX_SPEED = 1.2
/** How quickly the edge scroll eases to a new speed (time constants, ms): a gentle start, a quick stop, so it barely
 *  coasts once the pointer leaves the edge. */
const EDGE_EASE_IN_MS = 90
const EDGE_EASE_OUT_MS = 30

/** How fast the list scrolls with the pointer at `y` (from the top of its `view`-high visible box), in pixels per
 *  millisecond: up (negative) in the top edge zone, down in the bottom one, in proportion to how far into the zone
 *  the pointer is, at full speed on or past the edge. A short list keeps a still middle (the zones never overlap). */
export function edgeSpeed(y: number, view: number): number {
  const zone = Math.min(EDGE_ZONE, view / 3)
  if (zone <= 0) return 0
  const into = (d: number) => Math.min(1, Math.max(0, (zone - d) / zone))
  const up = into(y)
  if (up > 0) return -EDGE_MAX_SPEED * up
  return EDGE_MAX_SPEED * into(view - y)
}

/** The next frame's speed: eased from `v` toward `target` over `dt` ms, or straight to it under reduced motion
 *  (it still scrolls, without the ramp). Small enough is a stop. */
export function easeSpeed(v: number, target: number, dt: number, reduced: boolean): number {
  if (reduced) return target
  const tau = Math.abs(target) < Math.abs(v) || Math.sign(target) !== Math.sign(v) ? EDGE_EASE_OUT_MS : EDGE_EASE_IN_MS
  const next = v + (target - v) * (1 - Math.exp(-dt / tau))
  return Math.abs(next - target) < 0.005 ? target : next
}

/** The slot the pointer at `y` is over: how many of `items` have their middle above it. */
export function slotFor(items: Rect[], y: number): number {
  let n = 0
  for (const r of items) if (r.top + r.height / 2 < y) n++
  return n
}

/** How far each of `count` items moves while the item at `fromIndex` (-1: one from elsewhere) is over `toSlot`
 *  (counted without it; -1: over another list): the ones after it close its gap (-height), the ones from the slot
 *  on make room (+height). The lifted item itself gets 0; it follows the pointer. */
export function shiftsFor(fromIndex: number, toSlot: number, height: number, count: number): number[] {
  return Array.from({ length: count }, (_, i) => {
    if (i === fromIndex) return 0
    const after = fromIndex >= 0 && i > fromIndex
    const j = after ? i - 1 : i
    return (after ? -height : 0) + (toSlot >= 0 && j >= toSlot ? height : 0)
  })
}

type Kind = 'item' | 'head' | 'top' | 'cat' | 'other'
/** One thing along the list: a channel (its row and occupants), a heading, a whole section, or anything else. */
type Entry = Rect & { els: HTMLElement[]; kind: Kind; list: string | null; id: string; collapsed: boolean }

type Opts = {
  enabled: () => boolean
  /** false: the drop changes nothing (it eases back like a cancel) */
  onDrop: (d: Drop) => boolean
  onHoverSection: (key: string) => void
  /** a drag began (after its threshold) */
  onStart?: () => void
  /** it ended: dropped (after `onDrop`) or cancelled */
  onEnd?: () => void
}

/** Each list's way to cancel its drag from outside (`cancelDrag`). */
const cancels = new WeakMap<HTMLElement, () => void>()

/** Cancel `list`'s drag, if one is under way: everything eases back, nothing is dropped (the list it measured has
 *  changed under it, #54). */
export function cancelDrag(list: HTMLElement): void {
  cancels.get(list)?.()
}

const LINE = 'drop-line'
const translate = (y: number) => `translate3d(0, ${y}px, 0)`

export function sortableList(list: HTMLElement, opts: Opts): { update(o: Opts): void; destroy(): void } {
  let o = opts
  type Press = { pointerId: number; x: number; y: number; kind: 'channel' | 'category'; id: string }
  let press: Press | null = null
  let drag: {
    kind: 'channel' | 'category'
    id: string
    pointerId: number
    entries: Entry[]
    from: number
    /** the dragged item's height plus the gap after it: what the others move by */
    h: number
    grab: number
    y: number
    origin: number
    scroll: number
    /** the list's visible height, and how far it can scroll (measured with nothing moved) */
    view: number
    maxScroll: number
    /** the room below the last item (padding), measured once */
    extra: number
    /** the edge scroll's speed now, px/ms */
    v: number
    line: HTMLElement
    dest: { list: string | null; index: number; slot: number } | null
    hover: { id: string; timer: ReturnType<typeof setTimeout> } | null
  } | null = null
  /** Escape ended the drag but the button is still down: its release mustn't click. */
  let released = false
  let raf = 0
  /** transform each element was given, so a re-measure can take it back out */
  const moved = new Map<HTMLElement, number>()
  let cleanup: (() => void) | null = null
  let swallowing: ((e: MouseEvent) => void) | null = null

  // ---- measuring (only at the start, and once after a hovered section opens) ----

  const boxOf = (els: HTMLElement[], origin: number, scroll: number): Rect => {
    const a = els[0].getBoundingClientRect()
    const b = els[els.length - 1].getBoundingClientRect()
    const top = a.top - (moved.get(els[0]) ?? 0) - origin + scroll
    return { top, height: b.bottom - (moved.get(els[els.length - 1]) ?? 0) - origin + scroll - top }
  }
  const entry = (els: HTMLElement[], kind: Kind, list: string | null = null, id = '', collapsed = false) =>
    ({ els, kind, list, id, collapsed, top: 0, height: 0 }) as Entry
  const children = (el: Element) => [...el.children].filter((c) => !c.classList.contains(LINE)) as HTMLElement[]

  function readEntries(kind: 'channel' | 'category'): Entry[] {
    const out: Entry[] = []
    for (const el of children(list)) {
      const isTop = el.matches('.cat.top')
      const cat = el.matches('.cat[data-category]') ? el.dataset.category! : null
      if (isTop && el.childElementCount === 0) continue
      if (kind === 'category') {
        out.push(isTop ? entry([el], 'top') : cat ? entry([el], 'cat', cat, cat) : entry([el], 'other'))
        continue
      }
      if (!isTop && !cat) {
        out.push(entry([el], 'other'))
        continue
      }
      let cur: Entry | null = null
      for (const k of children(el)) {
        if (k.matches('.row[data-channel]')) out.push((cur = entry([k], 'item', cat, k.dataset.channel!)))
        else if (k.matches('.occupant, .row-side') && cur) cur.els.push(k)
        else {
          cur = null
          const head = cat && k.matches('.label')
          const shut = head && k.querySelector('button.section')?.getAttribute('aria-expanded') === 'false'
          out.push(head ? entry([k], 'head', cat, cat, !!shut) : entry([k], 'other'))
        }
      }
    }
    return out
  }

  function measure(d: NonNullable<typeof drag>): void {
    const r = list.getBoundingClientRect()
    d.origin = r.top + list.clientTop
    d.scroll = list.scrollTop
    d.entries = readEntries(d.kind)
    for (const e of d.entries) Object.assign(e, boxOf(e.els, d.origin, d.scroll))
    // how far the edge scroll may go, from the untransformed boxes: transforms could stretch scrollHeight mid-drag
    d.view = list.clientHeight
    const bottom = d.entries.reduce((m, e) => Math.max(m, e.top + e.height), 0)
    if (drag !== d) d.extra = Math.max(0, list.scrollHeight - bottom)
    d.maxScroll = Math.max(0, bottom + d.extra - d.view)
    // measured again mid-drag (a hovered section opened): the rows that just appeared ease like the rest
    if (drag === d) for (const e of d.entries) for (const el of e.els) el.classList.add('shift')
    d.from =d.entries.findIndex((e) => (d.kind === 'channel' ? e.kind === 'item' : e.kind === 'cat') && e.id === d.id)
    const me = d.entries[d.from]
    if (!me) return
    const next = d.entries[d.from + 1]
    const prev = d.entries[d.from - 1]
    d.h = next ? next.top - me.top : prev ? me.top + me.height - (prev.top + prev.height) : me.height
    d.h = Math.max(d.h, me.height)
  }

  // ---- each frame: work out the slot, then write transforms ----

  const pointerY = (d: NonNullable<typeof drag>) => d.y - d.origin + d.scroll

  function frame(): void {
    const d = drag
    if (!d) return
    const me = d.entries[d.from]
    if (!me || !me.els[0].isConnected) return cancel()
    const y = pointerY(d)
    const others = d.entries.filter((_, i) => i !== d.from)
    let slot: number
    let dest: { list: string | null; index: number }
    let start: { list: string | null; index: number }
    if (d.kind === 'channel') {
      slot = slotFor(others, y)
      dest = { list: null, index: 0 }
      for (let i = slot - 1; i >= 0; i--) {
        const e = others[i]
        if (e.kind === 'item') {
          dest = { list: e.list, index: others.slice(0, i).filter((x) => x.kind === 'item' && x.list === e.list).length + 1 }
          break
        }
        if (e.kind === 'head') {
          dest = { list: e.list, index: 0 }
          break
        }
      }
      start = { list: me.list, index: d.entries.slice(0, d.from).filter((x) => x.kind === 'item' && x.list === me.list).length }
      hoverSection(d, dest.list)
    } else {
      // between sections: never above the uncategorised area, never below what follows the last category
      const cats = others.filter((e) => e.kind === 'cat')
      const k = slotFor(cats, y)
      slot = !cats.length ? d.from : k < cats.length ? others.indexOf(cats[k]) : others.indexOf(cats[cats.length - 1]) + 1
      dest = { list: null, index: k }
      start = { list: null, index: d.entries.slice(0, d.from).filter((x) => x.kind === 'cat').length }
    }
    const same = dest.list === start.list && dest.index === start.index
    d.dest = { ...dest, slot: same ? -1 : slot }

    const shifts = shiftsFor(d.from, slot, d.h, d.entries.length)
    d.entries.forEach((e, i) => {
      for (const el of e.els) set(el, i === d.from ? y - d.grab - me.top : shifts[i])
    })
    // the line: where the dragged item's top will land
    const gap = d.h - me.height
    let land: number
    if (slot > 0) {
      const prev = others[slot - 1]
      const pi = d.entries.indexOf(prev)
      land = prev.top + prev.height + shifts[pi] + gap
    } else land = Math.min(...d.entries.map((e) => e.top))
    d.line.style.transform = translate(Math.round(land - gap / 2 - 1))
  }

  function set(el: HTMLElement, y: number): void {
    if (moved.get(el) === y && el.style.transform) return
    moved.set(el, y)
    el.style.transform = translate(y)
  }

  function schedule(): void {
    if (raf) return
    raf = requestAnimationFrame(() => {
      raf = 0
      frame()
    })
  }

  // ---- near an edge, the list scrolls (#193) ----

  let edgeRaf = 0
  let edgeT = 0

  /** Start the edge scroll if the pointer is in an edge zone (or it is still easing to a stop). */
  function edge(): void {
    const d = drag
    if (!d || edgeRaf) return
    if (!d.v && !edgeSpeed(d.y - d.origin, d.view)) return
    edgeT = 0
    edgeRaf = requestAnimationFrame(edgeFrame)
  }

  function edgeFrame(t: number): void {
    edgeRaf = 0
    const d = drag
    if (!d) return
    const dt = edgeT ? Math.min(32, Math.max(0, t - edgeT)) : 16
    edgeT = t
    d.v = easeSpeed(d.v, edgeSpeed(d.y - d.origin, d.view), dt, motion.reducedMotion)
    if (!d.v) return
    const step = d.v * dt
    const next = Math.min(d.maxScroll, Math.max(0, d.scroll + (Math.abs(step) < 1 ? Math.sign(step) : step)))
    // at the end it goes: stop (the next pointer move starts it again if there's room)
    if (next === d.scroll) {
      d.v = 0
      return
    }
    list.scrollTop = next
    d.scroll = next
    // this frame's transforms, for the new scroll (instead of a second pass for its scroll event)
    if (raf) cancelAnimationFrame(raf)
    raf = 0
    frame()
    if (drag === d) edgeRaf = requestAnimationFrame(edgeFrame)
  }

  // ---- hovering a collapsed section opens it ----

  function hoverSection(d: NonNullable<typeof drag>, cat: string | null): void {
    const shut = cat !== null && d.entries.some((e) => e.kind === 'head' && e.list === cat && e.collapsed)
    if (d.hover && (!shut || d.hover.id !== cat)) {
      clearTimeout(d.hover.timer)
      d.hover = null
    }
    if (!shut || d.hover) return
    d.hover = {
      id: cat,
      timer: setTimeout(() => {
        if (drag !== d) return
        d.hover = null
        o.onHoverSection(`cat:${cat}`)
        flushSync()
        // once it has rendered, measure again (the opened rows push everything after them down)
        requestAnimationFrame(() => {
          if (drag !== d) return
          measure(d)
          frame()
        })
      }, HOVER_OPEN_MS),
    }
  }

  // ---- start, drop, cancel ----

  function start(p: Press, e: PointerEvent): boolean {
    finishCleanup()
    const line = document.createElement('div')
    line.className = LINE
    line.setAttribute('aria-hidden', 'true')
    const d: NonNullable<typeof drag> = {
      kind: p.kind, id: p.id, pointerId: p.pointerId, entries: [], from: -1, h: 0, grab: 0, y: e.clientY,
      origin: 0, scroll: 0, view: 0, maxScroll: 0, extra: 0, v: 0, line, dest: null, hover: null,
    }
    measure(d)
    const me = d.entries[d.from]
    if (!me) return false
    drag = d
    try {
      list.setPointerCapture(p.pointerId)
    } catch {
      // not a live pointer (tests, or it went away): the window listeners still follow it
    }
    d.grab = p.y - d.origin + d.scroll - me.top
    list.classList.add('dragging')
    for (const x of d.entries) for (const el of x.els) el.classList.add('shift')
    for (const el of me.els) el.classList.add('lifted')
    window.getSelection?.()?.removeAllRanges()
    // placed before it shows, so it doesn't slide in from the top
    line.style.transform = translate(me.top)
    list.append(line)
    o.onStart?.()
    return true
  }

  /** Take everything the drag put on the page off again; `lifted` items ease home first (`returning`). */
  function release(d: NonNullable<typeof drag>): HTMLElement[] {
    if (d.hover) clearTimeout(d.hover.timer)
    if (raf) cancelAnimationFrame(raf)
    raf = 0
    if (edgeRaf) cancelAnimationFrame(edgeRaf)
    edgeRaf = 0
    drag = null
    d.line.remove()
    list.classList.remove('dragging')
    try {
      if (list.hasPointerCapture?.(d.pointerId)) list.releasePointerCapture(d.pointerId)
    } catch {
      // already gone
    }
    o.onEnd?.()
    return d.entries.flatMap((e) => e.els)
  }

  function cancel(): void {
    const d = drag
    if (!d) return
    const els = release(d)
    const lifted = d.entries[d.from]?.els ?? []
    // nothing is out of place (let go where it started): no transition will run, so there is nothing to wait for
    const away = [...moved.values()].some((y) => y !== 0)
    for (const el of els) el.style.transform = ''
    moved.clear()
    const done = () => {
      for (const el of els) el.classList.remove('shift', 'lifted', 'returning')
    }
    if (!away) return done()
    for (const el of lifted) el.classList.add('returning')
    afterGlide(lifted, done)
  }

  function drop(): void {
    const d = drag
    if (!d) return
    frame()
    if (!d.dest || d.dest.slot < 0 || drag !== d) return cancel()
    const me = d.entries[d.from]
    const at = pointerY(d) - d.grab
    const dest = d.dest
    // (nothing renders before the flush below)
    const changed = o.onDrop(d.kind === 'channel' ? { kind: 'channel', id: d.id, to: { category: dest.list, index: dest.index } } : { kind: 'category', id: d.id, index: dest.index })
    // a drop that changes nothing has no new order to render into: it eases back
    if (!changed) return cancel()
    list.classList.add('settling')
    const els = release(d)
    // the new order renders now, and in the same frame nothing is moved any more
    flushSync()
    for (const el of els) {
      el.style.transform = ''
      el.classList.remove('shift', 'lifted', 'returning')
    }
    moved.clear()
    // the dropped item: drawn where it was let go, then gliding into its new place
    const landed = find(d.kind, d.id)
    if (landed.length && me) {
      const r = list.getBoundingClientRect()
      const top = landed[0].getBoundingClientRect().top - (r.top + list.clientTop) + list.scrollTop
      const off = Math.round(at - top)
      if (off) {
        for (const el of landed) {
          el.classList.add('lifted')
          el.style.transform = translate(off)
        }
        // the start of the glide is computed before transitions come back on
        void getComputedStyle(landed[0]).transform
      }
    }
    requestAnimationFrame(() => {
      list.classList.remove('settling')
      if (!landed.length) return
      for (const el of landed) {
        el.classList.add('returning')
        el.style.transform = ''
      }
      afterGlide(landed, () => {
        for (const el of landed) el.classList.remove('lifted', 'returning')
      })
    })
  }

  /** The element(s) an item is drawn with now: a section, or a row and the occupants after it. */
  function find(kind: 'channel' | 'category', id: string): HTMLElement[] {
    if (kind === 'category') {
      const s = list.querySelector<HTMLElement>(`.cat[data-category="${CSS.escape(id)}"]`)
      return s ? [s] : []
    }
    const row = list.querySelector<HTMLElement>(`.row[data-channel="${CSS.escape(id)}"]`)
    if (!row) return []
    const out = [row]
    for (let el = row.nextElementSibling; el?.matches('.occupant, .row-side'); el = el.nextElementSibling) out.push(el as HTMLElement)
    return out
  }

  /** `done` once `els[0]` has eased its transform home (or soon anyway, if no transition runs). */
  function afterGlide(els: HTMLElement[], done: () => void): void {
    finishCleanup()
    const el = els[0]
    let timer: ReturnType<typeof setTimeout> | undefined
    const end = (e?: TransitionEvent) => {
      if (e && (e.target !== el || e.propertyName !== 'transform')) return
      el?.removeEventListener('transitionend', end)
      clearTimeout(timer)
      cleanup = null
      done()
    }
    cleanup = () => end()
    el?.addEventListener('transitionend', end)
    // a fallback well past the longest motion token
    timer = setTimeout(end, 1000)
  }
  function finishCleanup(): void {
    cleanup?.()
  }

  /** The click a drag's release makes isn't a click on what's under it. */
  function swallowClick(): void {
    stopSwallowing()
    const fn = (e: MouseEvent) => {
      e.preventDefault()
      e.stopPropagation()
      stopSwallowing()
    }
    swallowing = fn
    window.addEventListener('click', fn, true)
    // a release outside the list makes no click: don't keep waiting for one
    setTimeout(() => {
      if (swallowing === fn) stopSwallowing()
    }, 0)
  }
  function stopSwallowing(): void {
    if (swallowing) window.removeEventListener('click', swallowing, true)
    swallowing = null
  }

  // ---- events ----

  function onDown(e: PointerEvent): void {
    // a new press: whatever the last drag's release clicked has been and gone
    stopSwallowing()
    if (e.button !== 0 || press || drag || !o.enabled()) return
    const t = e.target as Element | null
    if (!t || t.closest('.add, input, textarea, select, [contenteditable], .occupant, .row-side')) return
    const row = t.closest<HTMLElement>('.row[data-channel]')
    const section = row?.closest('.cat')
    if (row && section && list.contains(row) && (section.matches('.top') || section.matches('[data-category]'))) {
      press = { pointerId: e.pointerId, x: e.clientX, y: e.clientY, kind: 'channel', id: row.dataset.channel! }
    } else {
      const head = t.closest<HTMLElement>('[data-category] > .label')
      if (!head || !list.contains(head) || head.classList.contains('renaming')) return
      press = { pointerId: e.pointerId, x: e.clientX, y: e.clientY, kind: 'category', id: (head.parentElement as HTMLElement).dataset.category! }
    }
    released = false
    listen(true)
  }

  function onMove(e: PointerEvent): void {
    if (drag) {
      if (e.pointerId !== drag.pointerId) return
      drag.y = e.clientY
      schedule()
      edge()
      return
    }
    if (!press || e.pointerId !== press.pointerId) return
    if (Math.hypot(e.clientX - press.x, e.clientY - press.y) < DRAG_THRESHOLD) return
    const p = press
    press = null
    if (start(p, e)) {
      schedule()
      edge()
    } else listen(false)
  }

  function onUp(e: PointerEvent): void {
    if (drag && e.pointerId === drag.pointerId) {
      drag.y = e.clientY
      drop()
      swallowClick()
    } else if (released) swallowClick()
    press = null
    released = false
    listen(false)
  }

  function onCancel(): void {
    press = null
    released = false
    cancel()
    listen(false)
  }

  function onLost(): void {
    if (drag) onCancel()
  }

  function onKey(e: KeyboardEvent): void {
    if (e.key !== 'Escape') return
    if (press) {
      press = null
      listen(false)
      return
    }
    if (!drag) return
    e.preventDefault()
    e.stopPropagation()
    cancel()
    // the button is still down: its release is the end of the drag, not a click
    released = true
  }

  function onScroll(): void {
    if (!drag) return
    drag.scroll = list.scrollTop
    schedule()
  }

  let listening = false
  function listen(on: boolean): void {
    if (on === listening) return
    listening = on
    const f = on ? window.addEventListener.bind(window) : window.removeEventListener.bind(window)
    f('pointermove', onMove as EventListener)
    f('pointerup', onUp as EventListener)
    f('pointercancel', onCancel)
    f('keydown', onKey as EventListener, true)
  }

  list.addEventListener('pointerdown', onDown)
  list.addEventListener('lostpointercapture', onLost)
  list.addEventListener('scroll', onScroll, { passive: true })
  cancels.set(list, () => {
    if (!drag) return
    cancel()
    // the button is still down: its release is the end of the drag, not a click
    released = true
  })

  return {
    update(next: Opts) {
      o = next
    },
    destroy() {
      cancels.delete(list)
      if (drag) release(drag)
      finishCleanup()
      stopSwallowing()
      listen(false)
      list.removeEventListener('pointerdown', onDown)
      list.removeEventListener('lostpointercapture', onLost)
      list.removeEventListener('scroll', onScroll)
    },
  }
}
