/**
 * Keeps a scrolled list at its bottom while what's in it changes height after it's drawn (#292): a link's preview
 * card unfurling a moment after its message, an image loading, reactions, a poll or spin card growing, the list
 * itself getting shorter. WebKitGTK has no CSS scroll anchoring, so it's done by hand: a ResizeObserver on the list
 * and on each of its children, and after a change, back to the bottom if the view was pinned there.
 *
 * Pinned is decided where the view moves: each scroll event (the user's, or code's: a jump to a message, older ones
 * drawn above with the view kept still) measures it, and `mark` measures it after a change made on purpose (a page
 * added below). A scroll event is dispatched before resize observers run in a frame, so a jump made in the same task
 * as some growth is seen first. The scroll event of its own move back to the bottom comes a frame later, maybe after
 * more growth, so that one is passed over rather than measured. Without ResizeObserver it does nothing.
 *
 * Scrolling up never pins (#613, `followsBottom`): what the view does as it leaves the bottom (a Jump button showing
 * makes the list taller) can bring the bottom back within the threshold, and re-pinning on that pulled the view down
 * again, a frame at a time, so a chat couldn't be scrolled up. `onScroll` hands the caller the same answer, measured
 * once per scroll event, so the view's "at the bottom" and this never disagree.
 */
export function pinToBottom(list: HTMLElement, opts: { on: () => boolean; threshold?: number; onScroll?: (pinned: boolean) => void }) {
  const threshold = opts.threshold ?? 40
  // A fresh list opens at the bottom (and nothing is read now: no synchronous layout on a channel switch, #173).
  let pinned = true
  let ownTop: number | null = null
  let lastTop: number | null = null
  const below = () => list.scrollHeight - list.scrollTop - list.clientHeight
  const mark = () => {
    ownTop = null
    lastTop = list.scrollTop
    pinned = below() < threshold
  }
  const onScroll = () => {
    const top = list.scrollTop
    if (ownTop !== null && top === ownTop) ownTop = null // our own move's echo: still pinned
    else pinned = followsBottom(pinned, lastTop === null ? null : top - lastTop, below(), threshold)
    lastTop = top
    opts.onScroll?.(pinned)
  }
  list.addEventListener('scroll', onScroll, { passive: true })
  const pinnedNow = () => pinned
  if (typeof ResizeObserver !== 'function') return { mark, pinned: pinnedNow, destroy: () => list.removeEventListener('scroll', onScroll) }
  const ro = new ResizeObserver(() => {
    if (!pinned || !opts.on() || below() <= 0) return
    list.scrollTop = list.scrollHeight
    ownTop = list.scrollTop
  })
  ro.observe(list)
  for (const c of list.children) ro.observe(c)
  const mo = new MutationObserver((records) => {
    for (const r of records) {
      for (const n of r.addedNodes) if (n instanceof Element) ro.observe(n)
      for (const n of r.removedNodes) if (n instanceof Element) ro.unobserve(n)
    }
  })
  mo.observe(list, { childList: true })
  return {
    /** Measures now whether the view is at the bottom: call it after adding below the view on purpose. */
    mark,
    /** Whether the view follows the bottom (as of the last scroll event, or `mark`). */
    pinned: pinnedNow,
    destroy() {
      list.removeEventListener('scroll', onScroll)
      ro.disconnect()
      mo.disconnect()
    },
  }
}

/** Whether a scrolled list follows its bottom after a scroll event (#613): `was` the answer before, `moved` how far the
 *  view moved since the last one (null: the first), `below` how far it is from the bottom now. At the very bottom it
 *  does (the list may have got shorter under it). Moving up it never starts to: it stays only if it was and is still
 *  within `threshold`, whatever a layout change did to `below` meanwhile. Moving down it does once within `threshold`.
 *  Not moving, as it was. */
export function followsBottom(was: boolean, moved: number | null, below: number, threshold: number): boolean {
  if (below <= 1) return true // sub-pixel slack: scrollTop is fractional at a display scale
  if (moved === null) return below < threshold
  if (moved < 0) return was && below < threshold
  if (moved > 0) return below < threshold
  return was
}
