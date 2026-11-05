// Which volume mixer sections are collapsed (#164), on this device. All open unless closed here.
const KEY = 'pulse.mixerCollapsed'

function saved(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? '[]')
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : []
  } catch {
    return []
  }
}

export const mixerCollapsed = $state({ ids: saved() })

export function toggleMixerSection(id: string): void {
  mixerCollapsed.ids = mixerCollapsed.ids.includes(id) ? mixerCollapsed.ids.filter((x) => x !== id) : [...mixerCollapsed.ids, id]
  try {
    localStorage.setItem(KEY, JSON.stringify(mixerCollapsed.ids))
  } catch {
    /* ignore */
  }
}

/** Read back from storage (tests: as after a restart). */
export function reloadMixerCollapsed(): void {
  mixerCollapsed.ids = saved()
}
