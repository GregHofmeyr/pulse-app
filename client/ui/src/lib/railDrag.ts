// Drag to reorder the server tabs (#185), hand-built on pointer events like the channel list's drag (lib/drag.ts),
// whose slot and shift rules it reuses on the horizontal axis. A tab is any `[data-server]` element inside the node
// (or `[data-<item>]`: the profile editor's badge row, #467, drags `[data-badge]`).
//
// Smooth under load (#103): every tab is measured once when the drag starts; each frame then only writes
// `transform`s. A press becomes a drag only past DRAG_THRESHOLD, so a click still opens the server and right-click
// still opens its menu; the click a drag ends with is swallowed. Escape, a lost pointer or a drop that changes
// nothing ease everything back. A real drop renders the new order and clears the transforms in the same frame
// (`settling` turns transitions off for it), so nothing slides from the wrong place.
import { flushSync } from 'svelte'
import { DRAG_THRESHOLD, shiftsFor, slotFor } from './drag'

export type RailDragOptions = {
  /** `id` was dropped at `index` among the tabs (counted without it). Not called for a drop where it started. */
  onDrop: (id: string, index: number) => void
  /** No dragging (e.g. a single server). */
  disabled?: boolean
  /** The data attribute that marks an item and holds its id (default `server`: `[data-server]`). */
  item?: string
}

type Drag = {
  id: string
  pointerId: number
  startX: number
  tab: HTMLElement
  /** set once it moves past the threshold */
  live: null | { tabs: HTMLElement[]; from: number; others: { top: number; height: number }[]; centre: number; step: number; slot: number }
}

export function railDrag(node: HTMLElement, options: RailDragOptions) {
  let opts = options
  let drag: Drag | null = null

  const attr = () => opts.item ?? 'server'
  const tabs = () => [...node.querySelectorAll<HTMLElement>(`[data-${attr()}]`)]

  function clear() {
    // the lifted tab loses `lifted` first, so it eases back with the others on a cancel
    node.classList.remove('dragging')
    drag?.tab.classList.remove('lifted')
    for (const t of tabs()) t.style.transform = ''
    drag = null
    window.removeEventListener('keydown', escape, true)
  }

  function start(d: Drag) {
    const all = tabs()
    const from = all.indexOf(d.tab)
    // measured once: left edges along the rail stand in for drag.ts's tops
    const boxes = all.map((t) => t.getBoundingClientRect())
    const me = boxes[from]
    const others = boxes.filter((_, i) => i !== from).map((b) => ({ top: b.left, height: b.width }))
    // the room one tab takes: its width plus the gap to the next
    const next = boxes[from + 1] ?? boxes[from - 1]
    const gap = next ? Math.max(0, next.left > me.left ? next.left - me.right : me.left - next.right) : 0
    d.live = { tabs: all, from, others, centre: me.left + me.width / 2, step: me.width + gap, slot: from }
    node.classList.add('dragging')
    d.tab.classList.add('lifted')
    try {
      d.tab.setPointerCapture(d.pointerId)
    } catch {
      // a synthetic pointer (tests) can't be captured: the node still gets its moves
    }
    window.addEventListener('keydown', escape, true)
  }

  function move(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return
    const dx = e.clientX - drag.startX
    if (!drag.live) {
      if (Math.abs(dx) < DRAG_THRESHOLD) return
      start(drag)
    }
    const l = drag.live!
    l.slot = slotFor(l.others, l.centre + dx)
    const shifts = shiftsFor(l.from, l.slot, l.step, l.tabs.length)
    l.tabs.forEach((t, i) => (t.style.transform = i === l.from ? `translateX(${dx}px)` : shifts[i] ? `translateX(${shifts[i]}px)` : ''))
  }

  function up(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return
    const d = drag
    if (!d.live) {
      drag = null
      return
    }
    // the click this press ends with is the drag's, not a tab click
    window.addEventListener('click', swallow, { capture: true, once: true })
    setTimeout(() => window.removeEventListener('click', swallow, { capture: true }), 0)
    if (d.live.slot === d.live.from) return clear()
    node.classList.add('settling')
    clear()
    opts.onDrop(d.id, d.live.slot)
    flushSync()
    requestAnimationFrame(() => node.classList.remove('settling'))
  }

  function swallow(e: MouseEvent) {
    e.preventDefault()
    e.stopPropagation()
  }

  function escape(e: KeyboardEvent) {
    if (e.key !== 'Escape' || !drag?.live) return
    e.preventDefault()
    e.stopPropagation()
    clear()
  }

  function down(e: PointerEvent) {
    if (opts.disabled || e.button !== 0 || drag) return
    const tab = (e.target as Element).closest?.<HTMLElement>(`[data-${attr()}]`)
    if (!tab || !node.contains(tab)) return
    drag = { id: tab.getAttribute(`data-${attr()}`)!, pointerId: e.pointerId, startX: e.clientX, tab, live: null }
  }

  function cancel(e: PointerEvent) {
    if (drag && e.pointerId === drag.pointerId) clear()
  }

  node.addEventListener('pointerdown', down)
  node.addEventListener('pointermove', move)
  node.addEventListener('pointerup', up)
  node.addEventListener('pointercancel', cancel)
  return {
    update(o: RailDragOptions) {
      opts = o
    },
    destroy() {
      clear()
      window.removeEventListener('click', swallow, { capture: true })
      node.removeEventListener('pointerdown', down)
      node.removeEventListener('pointermove', move)
      node.removeEventListener('pointerup', up)
      node.removeEventListener('pointercancel', cancel)
    },
  }
}
