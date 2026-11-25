// The right panel's views (#638): search results, a voice channel's chat, the pinned build, the members list, and
// whatever comes next all share one column. More than one available shows a switcher (RightViewTabs) at its top.
// Which one shows: the person's last choice in this place (a server, or a DM / group) that's still available, else the
// one before it (so closing a view goes back), else the first available by the shell's rank. A view that arrives (a
// search run, the voice chat opened, a build pinned) claims the column: `claimRightView` from where it opens, applied
// by the shell's `settleRightViews` in the place it then shows. Nothing here knows any view by name.

import { untrack } from 'svelte'
import { onAppReset } from './store.svelte'

/** How long a claim waits for its view to become available (the chat opens a tick after its jump asks for it). */
const CLAIM_MS = 1000

/** place -> the views chosen there, oldest first (each once; the last is the current choice). */
const history = $state<Record<string, string[]>>({})
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
  const list = (history[place] ?? []).filter((v) => v !== id)
  history[place] = [...list, id]
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

/** Signed out, and tests: no choices, no claims. */
export function forgetRightViews(): void {
  for (const k of Object.keys(history)) delete history[k]
  pending.clear()
}

onAppReset(forgetRightViews)
