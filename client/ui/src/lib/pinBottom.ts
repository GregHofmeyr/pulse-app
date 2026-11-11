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
 */
export function pinToBottom(list: HTMLElement, opts: { on: () => boolean; threshold?: number }) {
  const threshold = opts.threshold ?? 40
  // A fresh list opens at the bottom (and nothing is read now: no synchronous layout on a channel switch, #173).
  let pinned = true
  let ownTop: number | null = null
  const mark = () => {
    ownTop = null
    pinned = list.scrollHeight - list.scrollTop - list.clientHeight < threshold
  }
  const onScroll = () => {
    if (ownTop !== null && list.scrollTop === ownTop) return void (ownTop = null) // our own move's echo
    mark()
  }
  list.addEventListener('scroll', onScroll, { passive: true })
  if (typeof ResizeObserver !== 'function') return { mark, destroy: () => list.removeEventListener('scroll', onScroll) }
  const ro = new ResizeObserver(() => {
    if (!pinned || !opts.on() || list.scrollHeight - list.scrollTop - list.clientHeight <= 0) return
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
    destroy() {
      list.removeEventListener('scroll', onScroll)
      ro.disconnect()
      mo.disconnect()
    },
  }
}
