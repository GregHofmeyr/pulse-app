// A builds channel's look on this device (#617): Gallery or List, and whether its right panel is open (open the first
// time), per channel. Saved in `pulse.builds`; storage that's missing, blocked or holds junk reads as the defaults.

export type BuildView = 'gallery' | 'list'
type Pref = { view?: BuildView; panel?: boolean }

const KEY = 'pulse.builds'

function load(): Record<string, Pref> {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? '{}')
    return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, Pref>) : {}
  } catch {
    return {}
  }
}

function save(channelId: string, change: Pref) {
  try {
    const all = load()
    localStorage.setItem(KEY, JSON.stringify({ ...all, [channelId]: { ...all[channelId], ...change } }))
  } catch {
    // not saved: this visit still shows it
  }
}

export function viewOf(channelId: string): BuildView {
  const v = load()[channelId]?.view
  return v === 'list' ? 'list' : 'gallery'
}

export function setView(channelId: string, view: BuildView) {
  save(channelId, { view })
}

export function panelOpen(channelId: string): boolean {
  return load()[channelId]?.panel !== false
}

export function setPanelOpen(channelId: string, open: boolean) {
  save(channelId, { panel: open })
}

// A build page's Comments column (#618 testing): shown until hidden, on this device for every builds channel alike.
// Saved in `pulse.buildComments` as `shown` or `hidden`; anything else, or storage that throws, reads as shown.
const COMMENTS_KEY = 'pulse.buildComments'

export function commentsShown(): boolean {
  try {
    return localStorage.getItem(COMMENTS_KEY) !== 'hidden'
  } catch {
    return true
  }
}

export function setCommentsShown(shown: boolean) {
  try {
    localStorage.setItem(COMMENTS_KEY, shown ? 'shown' : 'hidden')
  } catch {
    // not saved: this visit still shows it
  }
}
