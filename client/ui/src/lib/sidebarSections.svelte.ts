// Which sidebar sections are collapsed, on this device. Keys are a category's `cat:<id>` (#54), or an older server's
// kind section `serverId:kind` (text, quotes, clips, voice).
import type { ServerLayout } from './protocol/ServerLayout'
import { onAppReady } from './store.svelte'

const KEY = 'pulse.collapsedSections'

function saved(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? '[]')
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : []
  } catch {
    return []
  }
}

/** The key of each collapsed section. */
export const sections = $state({ collapsed: saved() })

function store(collapsed: string[]): void {
  sections.collapsed = collapsed
  try {
    localStorage.setItem(KEY, JSON.stringify(collapsed))
  } catch {
    /* ignore */
  }
}

export function isCollapsed(key: string): boolean {
  return sections.collapsed.includes(key)
}

export function setCollapsed(key: string, on: boolean): void {
  if (isCollapsed(key) === on) return
  store(on ? [...sections.collapsed, key] : sections.collapsed.filter((x) => x !== key))
}

export function toggleSection(key: string): void {
  setCollapsed(key, !isCollapsed(key))
}

/** A kind section collapsed before categories stays collapsed: `serverId:kind` becomes `cat:<id>` of that server's
 *  category made for that kind (`default_kind`, so a rename since doesn't matter). Other keys are left alone. */
export function carryOverCollapsed(layouts: ServerLayout[]): void {
  const to = new Map<string, string>()
  for (const l of layouts) for (const c of l.categories) if (c.default_kind) to.set(`${l.server_id}:${c.default_kind}`, `cat:${c.id}`)
  if (!sections.collapsed.some((k) => to.has(k))) return
  const next: string[] = []
  for (const k of sections.collapsed) {
    const key = to.get(k) ?? k
    if (!next.includes(key)) next.push(key)
  }
  store(next)
}

onAppReady((r) => carryOverCollapsed(r.layouts ?? []))

/** Read back from storage (tests: as after a restart). */
export function resetCollapsed(): void {
  sections.collapsed = saved()
}
