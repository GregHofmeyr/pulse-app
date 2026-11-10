// The side columns' chosen widths (#160), on this device. null: the default. `chat`: the right column while it holds a
// voice channel's chat (#226), which keeps its own width (the members list's stays as it was).
// `strip`: the chat strip beside a stream in full view (#63), with its own width again.
import type { SavedWidths, Side } from './columns'

const KEY = 'pulse.columns'

export type Column = Side | 'chat' | 'strip'
export type StoredWidths = SavedWidths & { chat: number | null; strip: number | null }

const width = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) && v > 0 ? Math.round(v) : null)

function saved(): StoredWidths {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? '{}')
    const o = v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {}
    return { left: width(o.left), right: width(o.right), chat: width(o.chat), strip: width(o.strip) }
  } catch {
    return { left: null, right: null, chat: null, strip: null }
  }
}

export const columns = $state<StoredWidths>(saved())

function store() {
  try {
    localStorage.setItem(KEY, JSON.stringify(columns))
  } catch {
    /* ignore */
  }
}

/** A drag or key moved a column. `save`: write it down (a drag saves once, when it ends). */
export function setColumn(side: Column, w: number, save = true): void {
  columns[side] = Math.round(w)
  if (save) store()
}

/** Back to the default (a double-click on its handle). */
export function resetColumn(side: Column): void {
  columns[side] = null
  store()
}

/** Write the current widths down (the end of a drag). */
export function saveColumns(): void {
  store()
}

/** Read back from storage (tests: as after a restart). */
export function reloadColumns(): void {
  Object.assign(columns, saved())
}
