// Drag a conversation onto a section of your list (#419): one of your categories, Pinned, or the main list. Within a
// section conversations stay newest first, so a drag only changes where one sits, never its order. Hand-built on
// pointer events like the rail's drag (lib/railDrag.ts): a press becomes a drag only past DRAG_THRESHOLD, so a click
// still opens the conversation and right-click still opens its menu; the click a drag ends with is swallowed. The
// lifted row follows the pointer; the section under it is marked (`drop-here` on its `[data-drop]` element). Escape
// or a lost pointer puts it back. The right-click menu's Move to category stays the keyboard's way.
import { DRAG_THRESHOLD } from './drag'

/** Where a conversation can be dropped: a category of yours (by id), Pinned, or the main list. */
export type DropTarget = { kind: 'category'; id: string } | { kind: 'pinned' } | { kind: 'rest' }

export type Placement = { pinned: boolean; category_id: string | null }

/** A `[data-drop]` value as a target: `category:<id>`, `pinned` or `rest`. */
export function parseDrop(v: string | null | undefined): DropTarget | null {
  if (v === 'pinned' || v === 'rest') return { kind: v }
  if (v?.startsWith('category:') && v.length > 9) return { kind: 'category', id: v.slice(9) }
  return null
}

/** The placement a drop asks for, or null when it changes nothing. Into a category: shown there, so not pinned
 *  (Pinned wins over a category). Onto Pinned: pinned, its category kept. Onto the main list: out of both. */
export function dropPlacement(now: Placement, target: DropTarget): Placement | null {
  const next: Placement =
    target.kind === 'category' ? { pinned: false, category_id: target.id }
    : target.kind === 'pinned' ? { pinned: true, category_id: now.category_id }
    : { pinned: false, category_id: null }
  return next.pinned === now.pinned && next.category_id === now.category_id ? null : next
}

export type ConvDragOptions = {
  /** `id` was dropped on `target`. Not called for a drop outside every section. */
  onDrop: (id: string, target: DropTarget) => void
  /** A drag started (true) or ended (false), to show the drop sections that are empty. */
  onLift?: (lifted: boolean) => void
  /** No dragging (nowhere to drop: no categories and nothing pinned). */
  disabled?: boolean
  /** What's under a point, top first (`document.elementsFromPoint`; tests pass their own). */
  hitTest?: (x: number, y: number) => Element[]
}

type Drag = { id: string; pointerId: number; startX: number; startY: number; row: HTMLElement; live: boolean; over: HTMLElement | null }

export function convDrag(node: HTMLElement, options: ConvDragOptions) {
  let opts = options
  let drag: Drag | null = null

  const hit = (x: number, y: number) => (opts.hitTest ?? ((a: number, b: number) => document.elementsFromPoint?.(a, b) ?? []))(x, y)

  function mark(el: HTMLElement | null) {
    if (!drag || drag.over === el) return
    drag.over?.classList.remove('drop-here')
    el?.classList.add('drop-here')
    drag.over = el
  }

  function clear() {
    if (drag?.live) opts.onLift?.(false)
    if (drag) {
      drag.row.classList.remove('lifted')
      drag.row.style.transform = ''
      drag.over?.classList.remove('drop-here')
    }
    node.classList.remove('dragging')
    drag = null
    window.removeEventListener('keydown', escape, true)
  }

  function start(d: Drag) {
    d.live = true
    node.classList.add('dragging')
    d.row.classList.add('lifted')
    try {
      d.row.setPointerCapture(d.pointerId)
    } catch {
      // a synthetic pointer (tests) can't be captured: the node still gets its moves
    }
    window.addEventListener('keydown', escape, true)
    opts.onLift?.(true)
  }

  function move(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return
    const dx = e.clientX - drag.startX
    const dy = e.clientY - drag.startY
    if (!drag.live) {
      if (Math.hypot(dx, dy) < DRAG_THRESHOLD) return
      start(drag)
    }
    drag.row.style.transform = `translate(${dx}px, ${dy}px)`
    // what's under the lifted row (it's on top, and keeps the pointer capture, so it isn't made pointer-transparent)
    const row = drag.row
    const below = hit(e.clientX, e.clientY).find((el) => !row.contains(el))
    const under = below?.closest<HTMLElement>('[data-drop]') ?? null
    mark(under && node.contains(under) ? under : null)
  }

  function up(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return
    const d = drag
    if (!d.live) {
      drag = null
      return
    }
    // the click this press ends with is the drag's, not a click on the conversation
    window.addEventListener('click', swallow, { capture: true, once: true })
    setTimeout(() => window.removeEventListener('click', swallow, { capture: true }), 0)
    const target = parseDrop(d.over?.dataset.drop)
    clear()
    if (target) opts.onDrop(d.id, target)
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
    const row = (e.target as Element).closest?.<HTMLElement>('[data-conv]')
    if (!row || !node.contains(row)) return
    drag = { id: row.dataset.conv!, pointerId: e.pointerId, startX: e.clientX, startY: e.clientY, row, live: false, over: null }
  }

  function cancel(e: PointerEvent) {
    if (drag && e.pointerId === drag.pointerId) clear()
  }

  node.addEventListener('pointerdown', down)
  node.addEventListener('pointermove', move)
  node.addEventListener('pointerup', up)
  node.addEventListener('pointercancel', cancel)
  return {
    update(o: ConvDragOptions) {
      opts = o
      if (o.disabled && drag) clear()
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
