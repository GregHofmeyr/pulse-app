// Whether a clips channel's header shows its filter chips (this device only; shown by default).
// Clicking the channel's name hides them, to give a narrow window room.
const KEY = 'pulse.clipFilters'

function saved(): boolean {
  try {
    return localStorage.getItem(KEY) !== 'hidden'
  } catch {
    return true
  }
}

export const clipFilters = $state({ shown: saved() })

export function setClipFiltersShown(v: boolean): void {
  clipFilters.shown = v
  try {
    localStorage.setItem(KEY, v ? 'shown' : 'hidden')
  } catch {
    /* ignore */
  }
}
