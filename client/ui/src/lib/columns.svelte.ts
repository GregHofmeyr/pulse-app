// The side columns' chosen widths (#160), on this device. null: the default.
import type { SavedWidths, Side } from './columns'

const KEY = 'pulse.columns'

const width = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) && v > 0 ? Math.round(v) : null)

function saved(): SavedWidths {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? '{}')
    const o = v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {}
    return { left: width(o.left), right: width(o.right) }
  } catch {
    return { left: null, right: null }
  }
}

export const columns = $state<SavedWidths>(saved())

function store() {
  try {
    localStorage.setItem(KEY, JSON.stringify(columns))
  } catch {
    /* ignore */
  }
}

/** A drag or key moved a column. `save`: write it down (a drag saves once, when it ends). */
export function setColumn(side: Side, w: number, save = true): void {
  columns[side] = Math.round(w)
  if (save) store()
}

/** Back to the default (a double-click on its handle). */
export function resetColumn(side: Side): void {
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
