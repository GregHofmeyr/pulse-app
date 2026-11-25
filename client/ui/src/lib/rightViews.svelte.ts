// The right panel's views (#638): search results, a voice channel's chat, the pinned build, the members list, and
// whatever comes next all share one column. More than one available shows a switcher (RightViewTabs) at its top.
// Which one shows: the person's last choice in this place (a server, or a DM / group) that's still available, else the
// one before it (so closing a view goes back), else the first available by the shell's rank. A view that arrives (a
// search run, the voice chat opened, a build pinned) claims the column: `claimRightView` from where it opens, applied
// by the shell's `settleRightViews` in the place it then shows. Nothing here knows any view by name.

import { untrack } from 'svelte'
import { onAppReady, onAppReset } from './store.svelte'

/** How long a claim waits for its view to become available (the chat opens a tick after its jump asks for it). */
const CLAIM_MS = 1000

/** place -> the views chosen there, oldest first (each once; the last is the current choice). Saved per account
 *  (`pulse.rightViews.<user id>`), so the choices survive a restart. */
const history = $state<Record<string, string[]>>({})
const KEY = 'pulse.rightViews'
/** The account the choices in memory are saved for. */
let owner: string | null = null
/** At most this many places are remembered (the most recently chosen in). */
const PLACES_KEPT = 100

/** The views available but not on screen (another tab chosen, or the column hidden), set by the shell: a view's own
 *  controls go by it (the stage's chat toggle brings its chat forward; Esc in the search box leaves hidden results). */
export const rightHidden = $state({ views: [] as string[] })
export const isRightViewHidden = (id: string): boolean => rightHidden.views.includes(id)

function load(userId: string) {
  for (const k of Object.keys(history)) delete history[k]
  try {
    const v: unknown = JSON.parse(localStorage.getItem(`${KEY}.${userId}`) ?? '{}')
    if (!v || typeof v !== 'object' || Array.isArray(v)) return
    for (const [place, list] of Object.entries(v as Record<string, unknown>)) {
      if (Array.isArray(list)) history[place] = list.filter((x): x is string => typeof x === 'string')
    }
  } catch {
    /* none kept */
  }
}

function save() {
  if (!owner) return
  try {
    const places = Object.keys(history).slice(-PLACES_KEPT)
    localStorage.setItem(`${KEY}.${owner}`, JSON.stringify(Object.fromEntries(places.map((p) => [p, history[p]]))))
  } catch {
    /* kept until a restart */
  }
}
/** Claims not yet settled: view id -> when claimed. `claims.n` is bumped by each, so the shell settles them. */
const pending = new Map<string, number>()
const claims = $state({ n: 0 })

/** The view `id` arrived (or was asked for again): it shows next, wherever the shell is by then. */
export function claimRightView(id: string): void {
  pending.set(id, Date.now())
  claims.n++
}

/** The person picked `id` in `place` (the switcher). */
export function chooseRightView(place: string, id: string): void {
  const was = history[place] ?? []
  if (was[was.length - 1] === id) return
  const list = was.filter((v) => v !== id)
  // the place moves to the end (the most recent), so the oldest go first when trimmed
  delete history[place]
  history[place] = [...list, id]
  save()
}

/** Apply waiting claims whose view is available in `place` now (an older claim is dropped). */
export function settleRightViews(place: string, available: readonly string[]): void {
  void claims.n // the shell's effect runs again for each claim
  untrack(() => settle(place, available))
}

function settle(place: string, available: readonly string[]) {
  const now = Date.now()
  for (const [id, at] of [...pending]) {
    if (now - at > CLAIM_MS) pending.delete(id)
    else if (available.includes(id)) {
      pending.delete(id)
      chooseRightView(place, id)
    }
  }
}

/** The view showing in `place`: the latest choice there still available, else the first available by `rank`. */
export function shownRightView(place: string, available: readonly string[], rank: readonly string[]): string | null {
  const list = history[place] ?? []
  for (let i = list.length - 1; i >= 0; i--) if (available.includes(list[i])) return list[i]
  return [...available].sort((a, b) => rankOf(rank, a) - rankOf(rank, b))[0] ?? null
}

const rankOf = (rank: readonly string[], id: string) => {
  const i = rank.indexOf(id)
  return i < 0 ? rank.length : i
}

/** Signed out, and tests: no choices or claims in memory (what's saved stays the account's). */
export function forgetRightViews(): void {
  for (const k of Object.keys(history)) delete history[k]
  pending.clear()
  owner = null
  rightHidden.views = []
}

onAppReset(forgetRightViews)
// a Ready for another account than the one in memory reads that account's choices
onAppReady((r) => {
  if (owner === r.me.id) return
  owner = r.me.id
  load(r.me.id)
})
