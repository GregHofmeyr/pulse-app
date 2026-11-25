<script lang="ts">
  // A builds channel (#617): the header, then the builds of the filter as a gallery of cards or a list of rows (both
  // read one paged feed in `buildFeeds`, newest first; older pages load at the end). Opening a build shows its page in
  // place of all that (#618); Back brings the browse view back as it was, filter and scroll.
  import { tick, untrack } from 'svelte'
  import BuildCard from './BuildCard.svelte'
  import BuildPage from './BuildPage.svelte'
  import BuildQuickAdd from './BuildQuickAdd.svelte'
  import BuildRow from './BuildRow.svelte'
  import BuildsHeader from './BuildsHeader.svelte'
  import BuildsPanel from './BuildsPanel.svelte'
  import { buildFacetsOf, buildFeedFailed, buildFeedKey, buildFilter, loadBuildPage, openBuildFeed, setBuildFacets, type BuildFeed, type BuildFilter } from '../lib/builds'
  import { classifyPaste, type BuildPaste, type QuickPrefill } from '../lib/buildPaste'
  import { panelOpen, setPanelOpen, setView, viewOf, type BuildView } from '../lib/buildPrefs'
  import { nameOfIn } from '../lib/selectors'
  import { tracksUnread } from '../lib/state'
  import { windowFocus } from '../lib/windowFocus.svelte'
  import { app, onAppEvent, ui } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'

  let { channelId, serverId }: { channelId: string; serverId: string } = $props()

  let game = $state<string | undefined>(undefined)
  let archived = $state(false)
  let q = $state('')
  const filter = $derived<BuildFilter>(buildFilter({ game, archived, q }))
  const key = $derived(buildFeedKey(channelId, filter))
  const feed = $derived(app.state.buildFeeds[key])
  const items = $derived(feed?.items ?? [])
  let view = $state<BuildView>(untrack(() => viewOf(channelId)))
  const nameOf = $derived(nameOfIn(() => app.state, serverId))

  const games = $derived(app.state.games[serverId] ?? [])
  const title = $derived(game ? (games.find((g) => g.id === game)?.name ?? 'Game') : 'All games')
  const facets = $derived(buildFacetsOf(app.state, channelId, archived))
  const count = $derived(
    filter.q ? (feed?.start ? items.length : null) : facets ? (game ? (facets.games.find((g) => g.game_id === game)?.count ?? 0) : facets.total) : null,
  )

  let panel = $state(untrack(() => panelOpen(channelId)))
  function togglePanel() {
    panel = !panel
    setPanelOpen(channelId, panel)
  }

  function pickView(v: BuildView) {
    view = v
    setView(channelId, v)
  }

  // --- counts: loaded on open (and after a reconnect clears them), then a second after a build changes ---
  const FACETS_DEBOUNCE_MS = 1000
  let facetsSeq = 0
  let facetsFailed = $state(false)
  async function loadFacets(withArchived: boolean) {
    const seq = ++facetsSeq
    try {
      const f = await api.buildFacets(channelId, withArchived)
      if (seq !== facetsSeq || !f) return
      facetsFailed = false
      app.state = setBuildFacets(app.state, channelId, withArchived, f)
    } catch {
      if (seq === facetsSeq) facetsFailed = true // the counts stay as they were; the next build change asks again
    }
  }
  $effect(() => {
    if (!facets && !facetsFailed) untrack(() => void loadFacets(archived))
  })
  let facetsTimer: ReturnType<typeof setTimeout> | undefined
  $effect(() =>
    onAppEvent((e) => {
      const ours =
        e.t === 'MessageCreated' || e.t === 'MessageUpdated' ? e.d.message.channel_id === channelId && !!e.d.message.build
        : e.t === 'MessageDeleted' ? e.d.channel_id === channelId && !e.d.thread_root_id
        : e.t === 'BuildsArchived' ? e.d.channel_id === channelId
        : false
      if (!ours) return
      clearTimeout(facetsTimer)
      facetsTimer = setTimeout(() => {
        void loadFacets(archived)
        // a search's matches are fetched again, not guessed from the event
        if (filter.q) refetch()
      }, FACETS_DEBOUNCE_MS)
    }),
  )
  $effect(() => () => clearTimeout(facetsTimer))

  // --- paging: one fetch per feed at a time (its `loading` flag), so builds deleted meanwhile stay gone ---
  let failedKey = $state<string | null>(null)
  let error = $state('')
  const epochs = new Map<string, number>()
  const unfetched = (f: BuildFeed | undefined) => !f || (!f.items.length && !f.start && !f.loading)

  async function fetchPage(older: boolean): Promise<boolean> {
    const k = key
    const f = filter
    const held = app.state.buildFeeds[k]
    if (held?.loading) return false
    if (older && (!held || held.start || !held.items.length)) return false
    if (unfetched(held)) epochs.set(k, (epochs.get(k) ?? 0) + 1)
    const epoch = epochs.get(k)
    const before = older ? held!.items[held!.items.length - 1].id : undefined
    app.state = openBuildFeed(app.state, channelId, f)
    try {
      const page = (await api.listBuilds(channelId, f, before)) ?? []
      if (epochs.get(k) !== epoch) return false
      app.state = loadBuildPage(app.state, k, page)
      if (failedKey === k) failedKey = null
      return true
    } catch (e) {
      if (epochs.get(k) !== epoch) return false
      app.state = buildFeedFailed(app.state, k)
      failedKey = k
      error = errorText(e)
      return false
    }
  }
  /** Throws the current feed away and fetches it fresh. */
  function refetch() {
    const k = key
    if (!(k in app.state.buildFeeds)) return
    epochs.set(k, (epochs.get(k) ?? 0) + 1)
    const { [k]: _gone, ...rest } = app.state.buildFeeds
    app.state = { ...app.state, buildFeeds: rest }
  }

  $effect(() => {
    if (!unfetched(feed) || failedKey === key) return
    untrack(() => void fetchPage(false))
  })

  let scroller: HTMLDivElement | undefined = $state()
  let sentinel: HTMLDivElement | undefined = $state()
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
    untrack(() => void fetchPage(true))
  })

  // --- read: once the newest build is on screen with the window focused (the whole channel's live feed only) ---
  let newestOnScreen = $state(false)
  $effect(() => {
    void view
    const id = items[0]?.id
    newestOnScreen = false
    if (!id || !scroller || typeof IntersectionObserver === 'undefined') return
    const el = scroller.querySelector(`[data-build-id="${id}"]`)
    if (!el) return
    const io = new IntersectionObserver((es) => { for (const e of es) newestOnScreen = e.isIntersecting }, { root: scroller })
    io.observe(el)
    return () => io.disconnect()
  })
  $effect(() => {
    const newest = items[0]?.id
    const read = app.state.reads[channelId]?.last_read_message_id ?? null
    const whole = !filter.game && !filter.q
    if (!newest || !newestOnScreen || !windowFocus.focused || !whole || (read !== null && newest <= read)) return
    if (!untrack(() => tracksUnread(app.state, channelId))) return
    const t = setTimeout(() => void api.markRead(channelId, newest).catch(() => {}), 300)
    return () => clearTimeout(t)
  })

  // --- the build page: kept on a feed's current copy, else the store's held one (ThreadChat holds it) ---
  let open = $state<Message | null>(null)
  const openNow = $derived.by(() => {
    const o = open
    if (!o) return null
    for (const f of Object.values(app.state.buildFeeds)) {
      if (f.channelId !== channelId) continue
      const m = f.items.find((x) => x.id === o.id)
      if (m) return m
    }
    return app.state.openRoot?.id === o.id ? app.state.openRoot : o
  })
  let scrollBack = 0
  function openPage(m: Message) {
    scrollBack = scroller?.scrollTop ?? 0
    open = m
  }
  async function closePage() {
    const id = open?.id
    open = null
    await tick()
    if (scroller) scroller.scrollTop = scrollBack
    scroller?.querySelector<HTMLElement>(`[data-build-id="${id}"] button.open`)?.focus({ preventScroll: true })
  }
  // Asked to open one of this channel's builds (an inbox item, a search result, a notification): the one held in a
  // feed, else fetched.
  let openSeq = 0
  $effect(() => {
    const r = ui.openBuild
    if (!r || r.channelId !== channelId) return
    untrack(() => {
      ui.openBuild = null
      const seq = ++openSeq
      const held = Object.values(app.state.buildFeeds).flatMap((f) => (f.channelId === channelId ? f.items : [])).find((m) => m.id === r.messageId)
      if (held) return openPage(held)
      api.getBuild(r.messageId).then(
        (m) => { if (seq === openSeq && m) openPage(m) },
        (e) => { if (seq === openSeq) ui.notice = `Couldn't open that build: ${errorText(e)}` },
      )
    })
  })
  $effect(() => () => {
    untrack(() => {
      if (ui.openBuild?.channelId === channelId) ui.openBuild = null
    })
  })

  // --- adding (#618): "+ Add build", or a paste anywhere in the browse view that isn't into a field, opens quick add ---
  let quick = $state<{ initial: BuildPaste | null } | null>(null)
  let quickAdd: BuildQuickAdd | undefined = $state()
  /** What the full form opens with (Add details). */
  let form = $state<QuickPrefill | null>(null)
  function openQuick(p: BuildPaste | null = null) {
    if (quick && quickAdd) {
      if (p) void quickAdd.take(p)
      else quickAdd.focus()
      return
    }
    quick = { initial: p }
  }
  const FIELD = 'input, textarea, select, [contenteditable="true"], [contenteditable="plaintext-only"]'
  function browsePaste(e: ClipboardEvent) {
    if (e.defaultPrevented || (e.target instanceof Element && e.target.closest(FIELD))) return
    const p = e.clipboardData ? classifyPaste(e.clipboardData) : null
    if (!p) return
    e.preventDefault()
    openQuick(p)
  }
  function details(p: QuickPrefill) {
    quick = null
    form = p
  }

  const empty = $derived(!!feed?.start && items.length === 0)
  const filtered = $derived(!!filter.game || !!filter.q)
</script>

<div class="chan" data-testid="builds-channel">
  {#if openNow}
    <BuildPage message={openNow} {serverId} onBack={() => void closePage()} />
  {:else}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="browse" onpaste={browsePaste}>
      <BuildsHeader {title} {count} {q} onQ={(v) => (q = v)} {view} onView={pickView} {panel} onPanel={togglePanel} onAdd={() => openQuick()} />
      {#if quick}
        <BuildQuickAdd bind:this={quickAdd} {channelId} {serverId} initial={quick.initial} onDetails={details} onClose={() => (quick = null)} />
      {/if}
      <div class="body">
        <div class="feed" class:list={view === 'list'} bind:this={scroller}>
          {#if view === 'gallery'}
            <div class="grid">
              {#each items as m (m.id)}<BuildCard message={m} {serverId} {nameOf} onOpen={openPage} />{/each}
            </div>
          {:else}
            <div class="rows">
              {#each items as m (m.id)}<BuildRow message={m} {serverId} {nameOf} onOpen={openPage} />{/each}
            </div>
          {/if}
          <div class="sentinel" bind:this={sentinel}></div>
          {#if empty}
            <p class="empty">{filtered ? 'No builds match.' : 'No builds yet. Save your first with + Add build: a title, a link, a screenshot or a few notes is enough.'}</p>
          {/if}
          {#if failedKey === key}
            <p class="error" role="alert">{error} <button type="button" class="link-btn" onclick={() => (failedKey = null)}>Try again</button></p>
          {/if}
        </div>
        {#if panel}
          <BuildsPanel {channelId} {serverId} {facets} {game} onGame={(g) => (game = g)} {archived} onArchived={(on) => (archived = on)} />
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .chan { flex: 1; display: flex; flex-direction: column; min-height: 0; position: relative; }
  .browse { flex: 1; display: flex; flex-direction: column; min-height: 0; }
  .body { flex: 1; display: flex; min-height: 0; }
  .feed { flex: 1; min-width: 0; overflow-y: auto; display: flex; flex-direction: column; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 14px; padding: 16px 18px 8px; align-items: start; }
  .rows { display: flex; flex-direction: column; }
  .sentinel { height: 1px; flex-shrink: 0; }
  .empty { margin: 0; padding: 24px 18px; color: var(--text-3); font-size: 13px; }
  .error { margin: 8px 18px; color: var(--danger-text); font-size: 13px; }
</style>
