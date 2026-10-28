<script lang="ts">
  // A clips channel: the header (filters, view toggle, Share a clip), the feed for the view and filter, and
  // the replies panel of one clip. (Task 20 adds the editor and the pending cards here.)
  import { tick } from 'svelte'
  import ClipFeed from './ClipFeed.svelte'
  import ClipsHeader from './ClipsHeader.svelte'
  import RepliesPanel from './RepliesPanel.svelte'
  import { clipView, type ClipFilter } from '../lib/clips'
  import { app, onAppEvent } from '../lib/store.svelte'
  import { api } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'

  let { channelId, serverId }: { channelId: string; serverId: string } = $props()

  const FACETS_DEBOUNCE_MS = 1000

  let filter = $state<ClipFilter>({})
  const view = $derived(clipView(app.state, channelId))

  // --- the chips: loaded on open (and after a reconnect clears them), then a second after clip changes ---
  let facetsLoading = false
  let facetsFailed = false
  async function loadFacets() {
    facetsLoading = true
    try {
      const f = await api.clipFacets(channelId)
      facetsFailed = false
      app.state = { ...app.state, clipFacets: { ...app.state.clipFacets, [channelId]: f } }
    } catch {
      facetsFailed = true // the chips stay as they were; the next clip change asks again
    } finally {
      facetsLoading = false
    }
  }
  $effect(() => {
    if (!app.state.clipFacets[channelId] && !facetsLoading && !facetsFailed) void loadFacets()
  })
  let facetsTimer: ReturnType<typeof setTimeout> | undefined
  $effect(() =>
    onAppEvent((e) => {
      const ours =
        (e.t === 'MessageCreated' || e.t === 'MessageUpdated') ? e.d.message.channel_id === channelId && !!e.d.message.clip
        : e.t === 'MessageDeleted' ? e.d.channel_id === channelId && !e.d.thread_root_id
        : false
      if (!ours) return
      clearTimeout(facetsTimer)
      facetsTimer = setTimeout(() => void loadFacets(), FACETS_DEBOUNCE_MS)
    }),
  )
  $effect(() => () => clearTimeout(facetsTimer))

  // --- the replies panel (one at a time), kept on the feeds' current copy or the store's held one ---
  let repliesOpen = $state<Message | null>(null)
  const repliesRoot = $derived.by(() => {
    const open = repliesOpen
    if (!open) return null
    for (const feed of Object.values(app.state.clipFeeds)) {
      if (feed.channelId !== channelId) continue
      const m = feed.items.find((x) => x.id === open.id)
      if (m) return m
    }
    return app.state.openRoot?.id === open.id ? app.state.openRoot : open
  })
  let repliesOpener: HTMLElement | null = null
  let chan: HTMLDivElement
  function openReplies(m: Message) {
    const at = document.activeElement
    repliesOpener = at instanceof HTMLElement && chan?.contains(at) ? at : null
    repliesOpen = m
  }
  async function closeReplies() {
    const id = repliesOpen?.id
    const opener = repliesOpener
    repliesOpen = null
    repliesOpener = null
    await tick()
    const back = (opener?.isConnected ? opener : null) ?? chan?.querySelector<HTMLElement>(`[data-clip-id="${id}"] button`)
    back?.focus()
  }
  // A deleted clip's thread is gone with it.
  $effect(() => {
    if (repliesRoot?.deleted) void closeReplies()
  })
  // Unsent replies wait in their clip's panel; while it's closed, the clip says it has some.
  const unsentReplies = $derived(
    (app.state.pending[channelId] ?? []).reduce<Record<string, number>>((acc, p) => {
      if (p.thread_root_id && p.status === 'failed' && p.thread_root_id !== repliesOpen?.id) acc[p.thread_root_id] = (acc[p.thread_root_id] ?? 0) + 1
      return acc
    }, {}),
  )

  // "Share a clip": the editor opens here (Task 20).
  function share() {}
</script>

<div class="chan" class:with-replies={!!repliesRoot} bind:this={chan}>
  <ClipsHeader {channelId} {serverId} {filter} onFilter={(f) => (filter = f)} {view} onShare={share} />
  <ClipFeed {channelId} {filter} layout={view} onOpenReplies={openReplies} {unsentReplies} />
  {#if repliesRoot}
    {#key repliesRoot.id}
      <RepliesPanel root={repliesRoot} {serverId} onClose={closeReplies} />
    {/key}
  {/if}
</div>

<style>
  .chan { flex: 1; display: flex; flex-direction: column; min-height: 0; }
  /* the replies panel sits on the right (RepliesPanel, absolutely placed); the channel keeps the rest */
  .chan { position: relative; --replies-w: min(380px, 45%); }
  .chan.with-replies { padding-right: var(--replies-w); }
</style>
