// Which sidebar sections (text, quotes, clips, voice) are collapsed, per server, on this device.
const KEY = 'pulse.collapsedSections'

function saved(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? '[]')
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : []
  } catch {
    return []
  }
}

/** `serverId:kind` for each collapsed section. */
export const sections = $state({ collapsed: saved() })

const key = (serverId: string, kind: string) => `${serverId}:${kind}`

export function isCollapsed(serverId: string, kind: string): boolean {
  return sections.collapsed.includes(key(serverId, kind))
}

export function toggleSection(serverId: string, kind: string): void {
  const k = key(serverId, kind)
  sections.collapsed = sections.collapsed.includes(k) ? sections.collapsed.filter((x) => x !== k) : [...sections.collapsed, k]
  try {
    localStorage.setItem(KEY, JSON.stringify(sections.collapsed))
  } catch {
    /* ignore */
  }
}

/** Read back from storage (tests: as after a restart). */
export function resetCollapsed(): void {
  sections.collapsed = saved()
}
