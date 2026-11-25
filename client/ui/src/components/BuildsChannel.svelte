<script lang="ts">
  // A builds channel (#617): the header, then the builds of the filter as a gallery of cards or a list of rows (both
  // read one paged feed in `buildFeeds`, newest first; older pages load at the end). Opening a build shows its page in
  // place of all that (#618); Back brings the browse view back as it was, filter and scroll.
  import { tick, untrack } from 'svelte'
  import BuildCard from './BuildCard.svelte'
  import BuildForm from './BuildForm.svelte'
  import BuildMenu from './BuildMenu.svelte'
  import BuildsSelectBar from './BuildsSelectBar.svelte'
  import DeleteBuildsDialog from './DeleteBuildsDialog.svelte'
  import Icon from './Icon.svelte'
  import BuildPage from './BuildPage.svelte'
  import BuildQuickAdd from './BuildQuickAdd.svelte'
  import BuildRow from './BuildRow.svelte'
  import BuildsHeader from './BuildsHeader.svelte'
  import BuildsPanel from './BuildsPanel.svelte'
  import { buildFacetsOf, buildFeedFailed, dropBuildFeed, buildFeedKey, buildFilter, loadBuildPage, openBuildFeed, refreshedBuild, setBuildFacets, settleBuild, unsureBuilds, type BuildFeed, type BuildFilter } from '../lib/builds'
  import { barActions as shownActions, bulkPartial, bulkPlan, bulkSummary, runChunks, rangeSelected, toggleSelected, type BulkAction, type BuildMenuItem, type SelectBarAction } from '../lib/buildSelect'
  import { buildRights, isFavourite } from '../lib/builds'
  import { menuPoint } from '../lib/participantMenu'
  import type { BuildsBatchResult } from '../lib/protocol/BuildsBatchResult'
  import { classifyPaste, type BuildPaste, type QuickPrefill } from '../lib/buildPaste'
  import { panelOpen, setPanelOpen, setView, viewOf, type BuildView } from '../lib/buildPrefs'
  import { nameOfIn } from '../lib/selectors'
  import { tracksUnread } from '../lib/state'
  import { windowFocus } from '../lib/windowFocus.svelte'
  import { app, onAppEvent, onAppReady, ui } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import type { BuildFacets } from '../lib/protocol/BuildFacets'
  import type { Message } from '../lib/protocol/Message'

  let { channelId, serverId }: { channelId: string; serverId: string } = $props()

  let game = $state<string | undefined>(undefined)
  let archived = $state(false)
  let q = $state('')
  const filter = $derived<BuildFilter>(buildFilter({ game, archived, q }))
  const key = $derived(buildFeedKey(channelId, filter))
  const feed = $derived(app.state.buildFeeds[key])
  const items = $derived(feed?.items ?? [])
  // The Favourites section's own feed (#617): your starred builds of the filter in view, all of them (paged through
  // to the end), not only the ones on the browse feed's loaded pages.
  const favFilter = $derived<BuildFilter>(buildFilter({ ...filter, favourite: true }))
  const favKey = $derived(buildFeedKey(channelId, favFilter))
  const favFeed = $derived(app.state.buildFeeds[favKey])
  let view = $state<BuildView>(untrack(() => viewOf(channelId)))
  const nameOf = $derived(nameOfIn(() => app.state, serverId))

  const games = $derived(app.state.games[serverId] ?? [])
  const title = $derived(game ? (games.find((g) => g.id === game)?.name ?? 'Game') : 'All games')
  const stored = $derived(buildFacetsOf(app.state, channelId, archived))
  /** The counts last fetched here, with and without archived builds: shown while fresh ones are fetched. */
  let held = $state<{ live?: BuildFacets; all?: BuildFacets }>({})
  const facets = $derived(stored ?? (archived ? held.all : held.live))
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

  // --- counts: fetched whenever they're missing: on open, when Show archived flips, after a reconnect, and a second
  // after a build changes (any build event drops a channel's counts, open or not: `buildsOnEvent`). The last ones
  // fetched stay on screen meanwhile. ---
  const FACETS_DEBOUNCE_MS = 1000
  let facetsSeq = 0
  /** The fetch for the counts in view failed: not asked again until a build changes, the variant flips or a Ready. */
  let facetsFailed = $state(false)
  /** Bumped by a build event so the counts are asked for again (also when they were already being fetched). */
  let facetsTick = $state(0)
  let facetsSoon = false
  async function loadFacets(withArchived: boolean) {
    const seq = ++facetsSeq
    try {
      const f = await api.buildFacets(channelId, withArchived)
      if (seq !== facetsSeq || !f) return
      facetsFailed = false
      held = withArchived ? { ...held, all: f } : { ...held, live: f }
      app.state = setBuildFacets(app.state, channelId, withArchived, f)
    } catch {
      if (seq === facetsSeq) facetsFailed = true // the counts stay as they were
    }
  }
  $effect(() => {
    void facetsTick
    if (stored || facetsFailed) return
    const a = archived
    const soon = facetsSoon
    facetsSoon = false
    if (!soon) return untrack(() => void loadFacets(a))
    const t = setTimeout(() => void loadFacets(a), FACETS_DEBOUNCE_MS)
    return () => clearTimeout(t)
  })
  function pickArchived(on: boolean) {
    archived = on
    facetsFailed = false
  }
  $effect(() =>
    onAppReady(() => {
      facetsFailed = false
    }),
  )
  let searchTimer: ReturnType<typeof setTimeout> | undefined
  $effect(() =>
    onAppEvent((e) => {
      const ours =
        e.t === 'MessageCreated' || e.t === 'MessageUpdated' ? e.d.message.channel_id === channelId && !!e.d.message.build && !e.d.message.thread_root_id
        : e.t === 'MessageDeleted' ? e.d.channel_id === channelId && !e.d.thread_root_id
        : e.t === 'BuildsArchived' ? e.d.channel_id === channelId
        : false
      // a star changes no counts; a search's Favourites are fetched again (its words can't be matched here)
      if (e.t === 'BuildFavouriteChanged' && e.d.channel_id === channelId && filter.q) {
        clearTimeout(searchTimer)
        searchTimer = setTimeout(() => { if (filter.q) refetch() }, FACETS_DEBOUNCE_MS)
        return
      }
      if (!ours) return
      // a fetch already under way may predate the change: its answer is ignored and the counts asked for a second later
      facetsSeq++
      facetsSoon = true
      facetsFailed = false
      facetsTick++
      // a search's matches are fetched again, not guessed from the event
      clearTimeout(searchTimer)
      searchTimer = setTimeout(() => {
        if (filter.q) refetch()
      }, FACETS_DEBOUNCE_MS)
    }),
  )
  $effect(() => () => clearTimeout(searchTimer))

  // --- paging: one fetch per feed at a time (its `loading` flag), so builds deleted meanwhile stay gone ---
  let failedKey = $state<string | null>(null)
  let error = $state('')
  const epochs = new Map<string, number>()
  const unfetched = (f: BuildFeed | undefined) => !f || (!f.items.length && !f.start && !f.loading)

  async function fetchPage(older: boolean, fav = false): Promise<boolean> {
    const k = fav ? favKey : key
    const f = fav ? favFilter : filter
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
  /** Throws the current feeds (the browse feed and its Favourites) away and fetches them fresh. */
  function refetch() {
    for (const k of [key, favKey]) {
      if (!(k in app.state.buildFeeds)) continue
      epochs.set(k, (epochs.get(k) ?? 0) + 1)
      app.state = dropBuildFeed(app.state, k)
    }
  }

  // A search's feed is thrown away once its words change or the channel closes: one per query would pile up, and
  // events never keep it current (coming back to the same words fetches them again).
  $effect(() => {
    const ks = [key, favKey]
    if (!filter.q) return
    return () =>
      untrack(() => {
        for (const k of ks) {
          epochs.set(k, (epochs.get(k) ?? 0) + 1)
          app.state = dropBuildFeed(app.state, k)
        }
      })
  })

  $effect(() => {
    if (!unfetched(feed) || failedKey === key) return
    untrack(() => void fetchPage(false))
  })
  // the Favourites feed: its first page, then every older one in turn (it's shown whole)
  $effect(() => {
    const f = favFeed
    if (failedKey === favKey || f?.loading) return
    if (unfetched(f)) return untrack(() => void fetchPage(false, true))
    if (f && !f.start && f.items.length) untrack(() => void fetchPage(true, true))
  })
  // Builds an event left without your star or seen point (a restore no copy of yours was held for, a star from
  // another session): fetched as yours, once each.
  const fetching = new Set<string>()
  $effect(() => {
    const ids = unsureBuilds(app.state, channelId).filter((id) => !fetching.has(id))
    untrack(() => {
      for (const id of ids) {
        fetching.add(id)
        api.getBuild(id).then(
          (m) => { app.state = m ? refreshedBuild(app.state, m) : settleBuild(app.state, channelId, id) },
          () => { app.state = settleBuild(app.state, channelId, id) },
        ).finally(() => fetching.delete(id))
      }
    })
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
  // Deleted while open: keep the deleted copy, so the page says so (the held one is let go once its comments close,
  // and the copy it was opened with would show the build again).
  $effect(() => {
    const r = app.state.openRoot
    if (r?.deleted && open && r.id === open.id && !open.deleted) untrack(() => (open = r))
  })
  let scrollBack = 0
  function openPage(m: Message) {
    if (!open) scrollBack = scroller?.scrollTop ?? 0 // a build opened from a build page keeps the browse view's
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

  // --- right-click menu and multi-select (#617) ---
  let menu = $state<{ message: Message; x: number; y: number; opener: HTMLElement } | null>(null)
  function openMenu(m: Message, e: MouseEvent | KeyboardEvent, opener: HTMLElement) {
    menu = { message: m, ...menuPoint(e, opener), opener }
  }
  /** The build the full form edits (Edit in its menu). */
  let editing = $state<Message | null>(null)
  /** Delete…, asked first: the builds it deletes, how many of the selection it skips, and how. */
  let asking = $state<{ count: number; skipped: number; go: () => Promise<void> } | null>(null)
  const report = (what: string) => (e: unknown) => { ui.notice = `Couldn't ${what}: ${errorText(e)}` }
  function pickMenu(it: BuildMenuItem, m: Message) {
    if (it === 'open') return openPage(m)
    if (it === 'edit') return void (editing = m)
    if (it === 'select') return startSelecting(m)
    if (it === 'favourite' || it === 'unfavourite') return toggleFavourite(m, it === 'favourite')
    if (it === 'archive' || it === 'restore') return void api.setBuildArchived(m.id, it === 'archive').catch(report(`${it} that build`))
    asking = { count: 1, skipped: 0, go: () => api.deleteBuild(m.id) }
  }

  /** Your own star (#617); the BuildFavouriteChanged that follows moves it between the sections. */
  function toggleFavourite(m: Message, on = !isFavourite(m)) {
    api.setBuildFavourite(m.id, on).catch(report(on ? 'add that build to your favourites' : 'remove that build from your favourites'))
  }
  // Favourites first (#617): your starred builds of the filter in view (so its game, Show archived and words), from their
  // own feed, in their own section at the top; then the browse feed's others. Never both: a build in the section is
  // left out of the rest (one starred but not in the section yet stays in the rest until it is).
  const favs = $derived((favFeed?.items ?? []).filter(isFavourite))
  const favIds = $derived(new Set(favs.map((m) => m.id)))
  const rest = $derived(favs.length ? items.filter((m) => !favIds.has(m.id)) : items)
  /** The builds in the order shown (a Shift+click's range runs along it). */
  const shown = $derived(favs.length ? [...favs, ...rest] : items)

  let selecting = $state(false)
  /** Selected build ids, in the order picked; only those still shown count. */
  let picked = $state<string[]>([])
  /** Where a Shift+click's range starts: the build last clicked. */
  let anchor: string | null = null
  const selected = $derived(shown.filter((m) => picked.includes(m.id)))
  // A build that leaves the view (archived, deleted, filtered out) leaves the selection, so it isn't still selected
  // if it comes back.
  $effect(() => {
    const here = new Set(shown.map((m) => m.id))
    untrack(() => {
      if (picked.some((id) => !here.has(id))) picked = picked.filter((id) => here.has(id))
      if (anchor !== null && !here.has(anchor)) anchor = null
    })
  })
  function startSelecting(m: Message) {
    selecting = true
    picked = [m.id]
    anchor = m.id
  }
  function stopSelecting() {
    selecting = false
    picked = []
    anchor = null
  }
  function select(m: Message, shift: boolean) {
    if (!selecting) return startSelecting(m)
    picked = shift ? rangeSelected(picked, shown.map((x) => x.id), anchor, m.id) : toggleSelected(picked, m.id)
    anchor = m.id
  }
  // another filter, or the build page: selecting ends
  $effect(() => {
    void key
    if (openNow) return untrack(stopSelecting)
    return () => untrack(stopSelecting)
  })
  function browseKey(e: KeyboardEvent) {
    if (e.key !== 'Escape' || !selecting || e.defaultPrevented || menu || asking) return
    e.preventDefault()
    stopSelecting()
  }

  // The bar's actions. Mixed rights: an action applies to the selected builds you may change and skips the rest, saying
  // how many (the bar says "N not yours" beforehand); it's disabled when it applies to none you may change.
  const RUN: Record<BulkAction['key'], (ids: string[]) => Promise<BuildsBatchResult>> = {
    favourite: (ids) => api.favouriteManyBuilds(channelId, ids, true),
    unfavourite: (ids) => api.favouriteManyBuilds(channelId, ids, false),
    archive: (ids) => api.archiveManyBuilds(channelId, ids, true),
    restore: (ids) => api.archiveManyBuilds(channelId, ids, false),
    delete: (ids) => api.deleteManyBuilds(channelId, ids),
  }
  const ICON: Record<BulkAction['key'], SelectBarAction['icon']> = { favourite: 'star', unfavourite: 'star', archive: 'inbox', restore: 'inbox', delete: 'trash' }
  let busy = $state(false)
  /** Sends `ids` in batches of 100. All done: says what was done and what was skipped, and why (`notYours` counted
   *  here, the rest by the server). A batch after the first failing: says what the earlier ones did, and leaves the
   *  builds not done selected. The first failing throws (nothing was done). */
  async function runBulk(a: BulkAction, ids: string[], notYours: number) {
    const r = await runChunks(ids, RUN[a.key])
    if (r.failed && r.done === 0 && r.failed.rest.length === ids.length) throw r.failed.error
    if (r.failed) {
      ui.notice = bulkPartial(a, r.done, errorText(r.failed.error))
      const rest = new Set(r.failed.rest)
      picked = picked.filter((id) => rest.has(id))
      return
    }
    ui.notice = bulkSummary(a, r.done, notYours, r.skipped)
    stopSelecting()
  }
  function bulk(a: BulkAction) {
    const { ids, skipped } = bulkPlan(app.state, serverId, selected, a)
    if (!ids.length) return
    if (a.confirm) {
      asking = { count: ids.length, skipped, go: () => runBulk(a, ids, skipped) }
      return
    }
    busy = true
    runBulk(a, ids, skipped).catch(report(`${a.label.toLowerCase()} them`)).finally(() => (busy = false))
  }
  const barActions = $derived<SelectBarAction[]>(
    shownActions(selected).map((a) => ({ key: a.key, label: a.label, icon: ICON[a.key], danger: a.confirm, disabled: bulkPlan(app.state, serverId, selected, a).ids.length === 0, run: () => bulk(a) })),
  )
  const notYours = $derived(selected.filter((m) => { const r = buildRights(app.state, serverId, m); return !r.archive && !r.delete }).length)

  const empty = $derived(!!feed?.start && items.length === 0)
  const filtered = $derived(!!filter.game || !!filter.q)
</script>

{#snippet list(ms: Message[])}
  {#if view === 'gallery'}
    <div class="grid">
      {#each ms as m (m.id)}<BuildCard message={m} {serverId} {nameOf} onOpen={openPage} {selecting} selected={picked.includes(m.id)} onSelect={select} onMenu={openMenu} onFavourite={toggleFavourite} />{/each}
    </div>
  {:else}
    <div class="rows">
      {#each ms as m (m.id)}<BuildRow message={m} {serverId} {nameOf} onOpen={openPage} {selecting} selected={picked.includes(m.id)} onSelect={select} onMenu={openMenu} onFavourite={toggleFavourite} />{/each}
    </div>
  {/if}
{/snippet}

<div class="chan" data-testid="builds-channel">
  {#if openNow}
    <BuildPage message={openNow} {serverId} onBack={() => void closePage()} />
  {:else}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="browse" tabindex="-1" onpaste={browsePaste} onkeydown={browseKey}>
      <div class="head">
        <!-- under the selection bar: out of Tab's reach while it's up -->
        <div class="head-under" inert={selecting}>
          <BuildsHeader {title} {count} {q} onQ={(v) => (q = v)} {view} onView={pickView} {panel} onPanel={togglePanel} onAdd={() => openQuick()} />
        </div>
        {#if selecting}
          <BuildsSelectBar count={selected.length} {notYours} actions={barActions} {busy} onClear={() => { picked = []; anchor = null }} onCancel={stopSelecting} />
        {/if}
      </div>
      {#if quick}
        <BuildQuickAdd bind:this={quickAdd} {channelId} {serverId} initial={quick.initial} onDetails={details} onClose={() => (quick = null)} />
      {/if}
      <div class="body">
        <div class="feed" class:list={view === 'list'} bind:this={scroller}>
          <!-- the empty state takes the grid's or the rows' place, never sits under an empty (padded) grid,
               so it stays put when the view switches -->
          {#if empty}
            <p class="empty">{filtered ? 'No builds match.' : 'No builds yet. Save your first with + Add build: a title, a link, a screenshot or a few notes is enough.'}</p>
          {:else}
            {#if favs.length}
              <section class="section" data-section="favourites" aria-labelledby="builds-favs-{channelId}">
                <h3 class="section-head" id="builds-favs-{channelId}"><Icon name="star" size={14} /><span class="section-name">Favourites</span></h3>
                {@render list(favs)}
              </section>
              <div class="sep" role="separator"></div>
            {/if}
            <div class="section" data-section="rest">{@render list(rest)}</div>
          {/if}
          <div class="sentinel" bind:this={sentinel}></div>
          {#if failedKey === key || failedKey === favKey}
            <p class="error" role="alert">{error} <button type="button" class="link-btn" onclick={() => (failedKey = null)}>Try again</button></p>
          {/if}
        </div>
        {#if panel}
          <BuildsPanel {channelId} {serverId} {facets} {game} onGame={(g) => (game = g)} {archived} onArchived={pickArchived} />
        {/if}
      </div>
    </div>
  {/if}
</div>

{#if form}
  <BuildForm {channelId} {serverId} prefill={form} onClose={() => (form = null)} />
{/if}
{#if editing}
  <BuildForm {channelId} {serverId} build={editing} onClose={() => (editing = null)} />
{/if}
{#each menu ? [menu] : [] as mm (mm)}
  <BuildMenu message={mm.message} {serverId} x={mm.x} y={mm.y} opener={mm.opener} onPick={pickMenu} onClose={() => (menu = null)} />
{/each}
{#if asking}
  <DeleteBuildsDialog count={asking.count} skipped={asking.skipped} onConfirm={asking.go} onClose={() => (asking = null)} />
{/if}

<style>
  .chan { flex: 1; display: flex; flex-direction: column; min-height: 0; position: relative; }
  /* focusable (-1) so a click on its blank space puts the focus here, where a paste is heard */
  .browse { flex: 1; display: flex; flex-direction: column; min-height: 0; outline: none; }
  /* the selection bar lies over the header, in its box */
  .head { position: relative; flex-shrink: 0; }
  .body { flex: 1; display: flex; min-height: 0; }
  .feed { flex: 1; min-width: 0; overflow-y: auto; display: flex; flex-direction: column; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 14px; padding: 16px 18px 8px; align-items: start; }
  .rows { display: flex; flex-direction: column; }
  /* the Favourites section's head and the line under it: one line that never spills */
  .section-head { display: flex; align-items: center; gap: 6px; min-width: 0; margin: 0; padding: 14px 18px 0; font-size: 11px; font-weight: 700; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .section-head :global(svg) { flex-shrink: 0; color: var(--first-gold); fill: currentColor; }
  .section-name { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .sep { flex-shrink: 0; height: 1px; margin: 8px 18px; background: var(--bg-3); }
  .sentinel { height: 1px; flex-shrink: 0; }
  .empty { margin: 0; padding: 24px 18px; color: var(--text-3); font-size: 13px; }
  .error { margin: 8px 18px; color: var(--danger-text); font-size: 13px; }
</style>
