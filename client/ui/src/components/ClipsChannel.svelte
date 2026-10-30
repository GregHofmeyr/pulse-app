<script lang="ts">
  // A clips channel: the header (filters, view toggle, Share a clip), the feed for the view and filter, and
  // the replies panel of one clip, the editor (from Share a clip or a dropped file) and your clips being sent.
  import { tick, untrack } from 'svelte'
  import { getCurrentWebview } from '@tauri-apps/api/webview'
  import ClipEditor from './ClipEditor.svelte'
  import ClipFeed from './ClipFeed.svelte'
  import ClipsHeader from './ClipsHeader.svelte'
  import RepliesPanel from './RepliesPanel.svelte'
  import { clipView, type ClipFilter } from '../lib/clips'
  import { InlineError } from '../lib/inlineError.svelte'
  import { app, onAppEvent } from '../lib/store.svelte'
  import { api, errorText, type ClipSource } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'

  let { channelId, serverId }: { channelId: string; serverId: string } = $props()

  const FACETS_DEBOUNCE_MS = 1000

  let filter = $state<ClipFilter>({})
  const view = $derived(clipView(app.state, channelId))

  // --- the chips: loaded on open (and after a reconnect clears them), then a second after clip changes ---
  let facetsLoading = false
  let facetsFailed = false
  let facetsSeq = 0 // only the newest request's answer counts (an on-open load and a debounced one can overlap)
  async function loadFacets() {
    const seq = ++facetsSeq
    facetsLoading = true
    try {
      const f = await api.clipFacets(channelId)
      if (seq !== facetsSeq) return
      facetsFailed = false
      app.state = { ...app.state, clipFacets: { ...app.state.clipFacets, [channelId]: f } }
    } catch {
      if (seq === facetsSeq) facetsFailed = true // the chips stay as they were; the next clip change asks again
    } finally {
      if (seq === facetsSeq) facetsLoading = false
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

  // --- sharing: Share a clip or a dropped file opens the editor (one at a time) ---
  const WINDOWS_ONLY = 'Sharing clips is Windows-only for now.'
  const UNREADABLE = "Pulse couldn't read that video."
  const ONE_AT_A_TIME = 'One clip at a time.'
  let source = $state<ClipSource | null>(null)
  let opening = false
  let alive = true
  // Leaving with the editor open: the editor only lets its source go on close or send, so do it here.
  $effect(() => () => {
    alive = false
    const s = untrack(() => source)
    if (s) void api.closeClipSource(s.id).catch(() => {})
  })
  const notice = new InlineError()
  $effect(() => notice.stop)

  /** Whether this install can send; says why not when it can't. */
  async function canSend(): Promise<boolean> {
    try {
      const s = await api.clipSupport()
      if (s.can_send) return true
      notice.show(s.reason || WINDOWS_ONLY)
    } catch (e) {
      notice.show(errorText(e))
    }
    return false
  }
  /** Opens a file in the editor (after `canSend`); a file the core can't read says so. */
  async function open(path: string): Promise<boolean> {
    try {
      const s = await api.openClip(path)
      if (!alive) {
        void api.closeClipSource(s.id).catch(() => {})
        return false
      }
      source = s
      return true
    } catch (e) {
      notice.show(typeof e === 'string' && e ? e : UNREADABLE) // the core's own words, else the plain one
      return false
    }
  }
  async function share() {
    if (opening || source) return
    opening = true
    try {
      if (!(await canSend())) return
      const path = await api.pickClipFile()
      if (path && alive) await open(path)
    } catch (e) {
      notice.show(errorText(e))
    } finally {
      opening = false
    }
  }
  async function dropped(paths: string[]) {
    if (!paths.length) return
    if (opening || source) return notice.show(ONE_AT_A_TIME) // one is open (or opening) already
    opening = true
    try {
      if (!(await canSend())) return
      if ((await open(paths[0])) && paths.length > 1) notice.show(ONE_AT_A_TIME)
    } finally {
      opening = false
    }
  }

  // Files dragged over the channel: it lights up, and a drop opens the first one. Tauri reports every file drag over the
  // window (its position in physical pixels), so only one whose point is on this channel counts: not the sidebar, and
  // not a modal on top (settings, a clip's Edit). One on our own share editor still counts, so it says one at a time.
  let dragging = $state(false)
  let editorHost = $state<HTMLElement>()
  function onChannel(pos: { x: number; y: number }): boolean {
    const scale = window.devicePixelRatio || 1
    const el = document.elementFromPoint(pos.x / scale, pos.y / scale)
    if (!el) return false
    if (editorHost?.contains(el)) return true
    if (!chan?.contains(el)) return false
    // any other modal open (each has its backdrop over the whole window) takes the drop away from the channel
    return ![...document.querySelectorAll('[aria-modal="true"]')].some((m) => !editorHost?.contains(m))
  }
  $effect(() => {
    let off: (() => void) | null = null
    let gone = false
    try {
      void getCurrentWebview()
        .onDragDropEvent((e) => {
          const p = e.payload
          if (p.type === 'enter' || p.type === 'over') dragging = !source && onChannel(p.position)
          else if (p.type === 'leave') dragging = false
          else if (p.type === 'drop') {
            dragging = false
            if (onChannel(p.position)) void dropped(p.paths)
          }
        })
        .then((f) => (gone ? f() : (off = f)), () => {})
    } catch {
      // not in the app's webview (tests without it): no dropping
    }
    return () => {
      gone = true
      off?.()
      dragging = false
    }
  })

  // Your clips being sent here (the core's order: oldest first). A posted one's card goes once the clip is in a feed.
  const shownClips = $derived(new Set(Object.values(app.state.clipFeeds).flatMap((f) => (f.channelId === channelId ? f.items.map((m) => m.id) : []))))
  const pending = $derived(
    Object.values(app.state.clipJobs).filter((j) => j.channel_id === channelId && !(j.stage === 'done' && j.message_id && shownClips.has(j.message_id))),
  )
</script>

<div class="chan" class:with-replies={!!repliesRoot} bind:this={chan}>
  <ClipsHeader {channelId} {serverId} {filter} onFilter={(f) => (filter = f)} {view} onShare={share} />
  <ClipFeed {channelId} {filter} layout={view} onOpenReplies={openReplies} {unsentReplies} {pending} />
  {#if repliesRoot}
    {#key repliesRoot.id}
      <RepliesPanel root={repliesRoot} {serverId} onClose={closeReplies} />
    {/key}
  {/if}
  {#if dragging}<div class="drop-zone" aria-hidden="true"><span>Drop a video to share it</span></div>{/if}
  {#if notice.text}<p class="notice fade-in" role="status">{notice.text}</p>{/if}
</div>
<div class="editor-host" bind:this={editorHost}>
  {#if source}
    {#key source.id}
      <ClipEditor {source} {channelId} {serverId} onClose={() => (source = null)} />
    {/key}
  {/if}
</div>

<style>
  .chan { flex: 1; display: flex; flex-direction: column; min-height: 0; }
  /* the replies panel sits on the right (RepliesPanel, absolutely placed); the channel keeps the rest */
  .chan { position: relative; --replies-w: min(380px, 45%); }
  .chan.with-replies { padding-right: var(--replies-w); }
  .editor-host { display: contents; }
  .drop-zone { position: absolute; inset: 8px; z-index: 4; display: grid; place-items: center; border: 2px dashed var(--accent); border-radius: 14px; background: color-mix(in srgb, var(--bg-1) 80%, transparent); pointer-events: none; }
  .drop-zone span { padding: 8px 16px; border-radius: 10px; background: var(--bg-2); color: var(--text); font-size: 15px; font-weight: 600; }
  .notice { position: absolute; left: 50%; bottom: 72px; transform: translateX(-50%); z-index: 5; margin: 0; padding: 8px 14px; border-radius: 10px; background: var(--bg-3); color: var(--text); font-size: 13px; box-shadow: 0 6px 20px rgba(0, 0, 0, .3); max-width: 80%; }
</style>
