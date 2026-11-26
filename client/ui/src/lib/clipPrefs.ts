// A clips channel's look on this device (#663): whether its right panel (games, people) is open, per channel; open
// the first time. Saved in `pulse.clips`. Someone who had hidden the old filter chips (`pulse.clipFilters`, every
// channel alike) starts closed until they choose for a channel. Storage that's missing, blocked or junk reads as open.

type Pref = { panel?: boolean }

const KEY = 'pulse.clips'
const LEGACY_KEY = 'pulse.clipFilters'

function load(): Record<string, Pref> {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? '{}')
    return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, Pref>) : {}
  } catch {
    return {}
  }
}

function legacyHidden(): boolean {
  try {
    return localStorage.getItem(LEGACY_KEY) === 'hidden'
  } catch {
    return false
  }
}

export function clipPanelOpen(channelId: string): boolean {
  const v = load()[channelId]?.panel
  return typeof v === 'boolean' ? v : !legacyHidden()
}

export function setClipPanelOpen(channelId: string, open: boolean) {
  try {
    const all = load()
    localStorage.setItem(KEY, JSON.stringify({ ...all, [channelId]: { ...all[channelId], panel: open } }))
  } catch {
    // not saved: this visit still shows it
  }
}
