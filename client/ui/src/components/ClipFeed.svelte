<script lang="ts">
  // A clips channel's feed for one filter, as a list (newest at the bottom, like messages) or a wall (newest
  // top-left). Both read one paged feed in `clipFeeds` (newest first); older pages load at the far end.
  import { tick, untrack } from 'svelte'
  import ClipCard from './ClipCard.svelte'
  import ClipItem from './ClipItem.svelte'
  import PendingClip from './PendingClip.svelte'
  import { clipFilter, feedKey, isNewClip, type ClipFilter } from '../lib/clips'
  import { feedFailed, loadFeedPage, openFeed, tracksUnread } from '../lib/state'
  import { nameOfIn } from '../lib/selectors'
  import { app } from '../lib/store.svelte'
  import { api, errorText, type ClipJob } from '../lib/tauri'
  import { windowFocus } from '../lib/windowFocus.svelte'
  import type { Message } from '../lib/protocol/Message'

  let { channelId, filter, layout, onOpenReplies, unsentReplies = {}, pending = [] }: {
    channelId: string
    filter: ClipFilter
    layout: 'list' | 'wall'
    /** "View N replies" (and an unsent-reply marker) on a clip. */
    onOpenReplies?: (m: Message) => void
    /** Your unsent replies per clip id (shown while its panel is closed). */
    unsentReplies?: Record<string, number>
    /** Your clips being sent here, oldest first: at the newest end (the list's bottom, the wall's top-left). */
    pending?: ClipJob[]
  } = $props()

  const serverId = $derived(app.state.channels[channelId]?.server_id ?? '')
  const flt = $derived(clipFilter(filter.game, filter.author))
  const key = $derived(feedKey(channelId, flt))
  const feed = $derived(app.state.clipFeeds[key])
  const items = $derived(feed?.items ?? []) // newest first
  const shown = $derived(layout === 'list' ? [...items].reverse() : items)
  const meId = $derived(app.state.me?.id ?? '')
  const nameOf = $derived(nameOfIn(() => app.state, serverId))
  const unfiltered = $derived(!flt.game && !flt.author)

  // --- new since you last looked: the read point as it was when the channel was opened ---
  const tracked = untrack(() => channelId in app.state.reads) // no read point (not a member): nothing is "new"
  const seenAt = untrack(() => app.state.reads[channelId]?.last_read_message_id ?? null)
  const isNew = (m: Message) => tracked && m.author_id !== meId && isNewClip(app.state, channelId, m.id, seenAt)
  const newLineAt = $derived(layout === 'list' ? (shown.find(isNew)?.id ?? null) : null)

  let scroller: HTMLDivElement | undefined = $state()
  let sentinel: HTMLDivElement | undefined = $state()

  // --- paging: one fetch per feed at a time (its `loading` flag), so clips deleted meanwhile stay gone ---
  let failedKey = $state<string | null>(null)
  let error = $state('')
  // A feed cleared by a reconnect starts again: a page still coming for the old one is dropped.
  const epochs = new Map<string, number>()
  // Coming back to a filter whose page failed tries again (the error belongs to the visit that saw it).
  let lastKey = untrack(() => key)
  $effect.pre(() => {
    if (key === lastKey) return
    lastKey = key
    failedKey = null
  })
  /** Not fetched yet: absent, or left empty by a failed first page (`feedFailed` keeps it). */
  const unfetched = (f: (typeof app.state.clipFeeds)[string] | undefined) => !f || (!f.items.length && !f.start && !f.loading)

  /** Fetches the newest page, or the one before the oldest held; resolves true once it is in. */
  async function fetchPage(older: boolean): Promise<boolean> {
    const k = key
    const f = flt
    const held = app.state.clipFeeds[k]
    if (held?.loading) return false
    if (older && (!held || held.start || !held.items.length)) return false
    if (unfetched(held)) epochs.set(k, (epochs.get(k) ?? 0) + 1)
    const epoch = epochs.get(k)
    const before = older ? held!.items[held!.items.length - 1].id : undefined
    app.state = openFeed(app.state, channelId, f)
    try {
      const page = await api.listClips(channelId, before, f.game, f.author)
      if (epochs.get(k) !== epoch) return false
      app.state = loadFeedPage(app.state, k, page)
      if (failedKey === k) failedKey = null
      return true
    } catch (e) {
      if (epochs.get(k) !== epoch) return false
      app.state = feedFailed(app.state, k)
      failedKey = k
      error = errorText(e)
      return false
    }
  }

  // The newest page whenever this feed isn't fetched (first look, another filter, after a reconnect, or "Try again").
  $effect(() => {
    if (!unfetched(feed) || failedKey === key) return
    untrack(() => void fetchPage(false))
  })

  // The far end (the list's top, the wall's bottom) in or near view pages back, one page at a time, for as
  // long as it stays there.
  let atEnd = $state(false)
  $effect(() => {
    const el = sentinel
    if (!el || !scroller || typeof IntersectionObserver === 'undefined') return
    const io = new IntersectionObserver((es) => { for (const e of es) atEnd = e.isIntersecting }, { root: scroller, rootMargin: '600px 0px' })
    io.observe(el)
    return () => io.disconnect()
  })
  $effect(() => {
    if (!atEnd || !feed || feed.loading || feed.start || failedKey === key) return
    untrack(() => void loadOlder())
  })
  async function loadOlder() {
    // The list grows upwards: keep what's on screen where it was.
    const list = layout === 'list' && scroller ? scroller : null
    const fromBottom = list ? list.scrollHeight - list.scrollTop : 0
    if (!(await fetchPage(true)) || !list) return
    await tick()
    list.scrollTop = list.scrollHeight - fromBottom
  }

  // --- where the list sits: at the bottom when it first shows, and following new clips while you're there ---
  let placed: string | null = null
  let nearBottom = true
  $effect.pre(() => {
    void items[0]?.id
    const el = untrack(() => scroller)
    if (el) nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 120
  })
  $effect(() => {
    const newest = items[0]?.id
    if (layout !== 'list' || !scroller || !newest) return
    const at = `${key}|list`
    if (placed !== at || nearBottom) scroller.scrollTop = scroller.scrollHeight
    placed = at
  })

  // --- a clip you just sent shows where you'll see it: the list's bottom, the wall's top ---
  let pendingSeen = new Set<string>(untrack(() => pending.map((j) => j.id)))
  $effect(() => {
    const ids = pending.map((j) => j.id)
    const fresh = ids.some((id) => !pendingSeen.has(id))
    pendingSeen = new Set(ids)
    if (!fresh || !scroller) return
    const el = scroller
    void tick().then(() => (el.scrollTop = layout === 'list' ? el.scrollHeight : 0))
  })

  // --- switching between list and wall keeps the clip you were looking at in view ---
  let lastLayout = untrack(() => layout)
  let keep: string | null = null
  function firstVisible(): string | null {
    if (!scroller) return null
    const box = scroller.getBoundingClientRect()
    const els = [...scroller.querySelectorAll<HTMLElement>('[data-clip-id]')]
    const rects = els.map((el) => el.getBoundingClientRect())
    const i = rects.findIndex((r) => r.top >= box.top && r.bottom <= box.bottom)
    const j = i >= 0 ? i : rects.findIndex((r) => r.bottom > box.top && r.top < box.bottom)
    return j >= 0 ? (els[j].dataset.clipId ?? null) : null
  }
  $effect.pre(() => {
    const l = layout
    if (l === lastLayout) return
    lastLayout = l
    keep = untrack(firstVisible)
  })
  $effect(() => {
    const l = layout
    const id = keep
    keep = null
    untrack(() => (playingId = null))
    if (!id || !scroller) return
    placed = `${untrack(() => key)}|${l}` // the kept clip, not the bottom
    scroller.querySelector<HTMLElement>(`[data-clip-id="${id}"]`)?.scrollIntoView({ block: 'start' })
  })

  // --- the wall's one player ---
  let playingId = $state<string | null>(null)

  // --- read: once the newest clip is on screen with the window focused (the whole channel's feed only) ---
  let newestOnScreen = $state(false)
  $effect(() => {
    void layout
    const id = items[0]?.id
    newestOnScreen = false
    if (!id || !scroller || typeof IntersectionObserver === 'undefined') return
    const el = scroller.querySelector(`[data-clip-id="${id}"]`)
    if (!el) return
    const io = new IntersectionObserver((es) => { for (const e of es) newestOnScreen = e.isIntersecting }, { root: scroller })
    io.observe(el)
    return () => io.disconnect()
  })
  $effect(() => {
    const newest = items[0]?.id
    const read = app.state.reads[channelId]?.last_read_message_id ?? null
    if (!newest || !newestOnScreen || !windowFocus.focused || !unfiltered || (read !== null && newest <= read)) return
    if (!untrack(() => tracksUnread(app.state, channelId))) return
    const t = setTimeout(() => void api.markRead(channelId, newest).catch(() => {}), 300)
    return () => clearTimeout(t)
  })
</script>

<div class="feed" class:list={layout === 'list'} class:wall={layout === 'wall'} bind:this={scroller}>
  {#if layout === 'list'}
    {#if feed?.start}<p class="start">{unfiltered ? 'This is the start of the clips.' : 'No older clips match.'}</p>{/if}
    <div class="sentinel" bind:this={sentinel}></div>
    <div class="items">
      {#each shown as m (m.id)}
        {#if m.id === newLineAt}<div class="newline" role="separator" aria-label="New clips">NEW</div>{/if}
        <ClipItem message={m} {serverId} {meId} {nameOf} unsentReplies={unsentReplies[m.id] ?? 0}
          onOpenReplies={onOpenReplies ? () => onOpenReplies(m) : undefined} />
      {/each}
      {#each pending as j (j.id)}<div class="pending-row"><PendingClip job={j} /></div>{/each}
    </div>
  {:else}
    <div class="grid">
      {#each [...pending].reverse() as j (j.id)}<PendingClip job={j} />{/each}
      {#each shown as m (m.id)}
        <ClipCard message={m} {serverId} {meId} {nameOf} isNew={isNew(m)} unsentReplies={unsentReplies[m.id] ?? 0}
          playing={playingId === m.id} onPlay={() => (playingId = m.id)}
          onOpenReplies={onOpenReplies ? () => onOpenReplies(m) : undefined} />
      {/each}
    </div>
    <div class="sentinel" bind:this={sentinel}></div>
  {/if}
  {#if feed?.start && items.length === 0 && pending.length === 0}<p class="empty">{unfiltered ? 'No clips yet. Share one!' : 'No clips match.'}</p>{/if}
  {#if failedKey === key}
    <p class="error" role="alert">{error} <button type="button" class="retry" onclick={() => (failedKey = null)}>Try again</button></p>
  {/if}
</div>

<style>
  .feed { flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; }
  .feed.list { padding: 12px 0 8px; }
  /* few clips sit at the bottom, like messages */
  .list .items { margin-top: auto; display: flex; flex-direction: column; }
  .sentinel { height: 1px; flex-shrink: 0; }
  .start, .empty { margin: 0; padding: 16px 18px; color: var(--text-3); font-size: 13px; }
  .newline { display: flex; align-items: center; gap: 8px; margin: 6px 18px; color: var(--danger); font-size: 11px; font-weight: 700; letter-spacing: .04em; }
  .newline::before, .newline::after { content: ''; flex: 1; height: 1px; background: var(--danger); opacity: .6; }
  .pending-row { padding: 6px 18px 6px 70px; max-width: 520px; }
  .grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 14px; padding: 16px 18px 8px; align-items: start; }
  @media (max-width: 899.98px) {
    .grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  }
  .error { margin: 8px 18px; color: var(--danger-text); font-size: 13px; }
  .retry { padding: 0; border: 0; background: none; color: var(--accent); font: inherit; text-decoration: underline; }
</style>
