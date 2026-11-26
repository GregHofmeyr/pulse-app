<script lang="ts">
  // A clips channel: the header (view toggle, Share a clip, the panel toggle), the feed for the view and filter, the
  // filters panel on the right (games and people, #663; it steps aside while a clip's replies are open), and
  // the replies panel of one clip, the editor (from Share a clip or a dropped file) and your clips being sent.
  import { tick, untrack } from 'svelte'
  import { getCurrentWebview } from '@tauri-apps/api/webview'
  import ClipEditor from './ClipEditor.svelte'
  import { exitFade } from '../lib/exit'
  import ClipFeed from './ClipFeed.svelte'
  import ClipsHeader from './ClipsHeader.svelte'
  import ClipsPanel from './ClipsPanel.svelte'
  import { clipPanelOpen, setClipPanelOpen } from '../lib/clipPrefs'
  import RepliesPanel from './RepliesPanel.svelte'
  import { clipView, type ClipFilter } from '../lib/clips'
  import { InlineError } from '../lib/inlineError.svelte'
  import { app, onAppEvent, ui } from '../lib/store.svelte'
  import { pendingFresh, rememberClipsChannel } from '../lib/clipDrop.svelte'
  import { api, errorText, type ClipSource } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'

  let { channelId, serverId }: { channelId: string; serverId: string } = $props()

  const FACETS_DEBOUNCE_MS = 1000

  let filter = $state<ClipFilter>({})
  const view = $derived(clipView(app.state, channelId))

  // --- the filters panel (#663): open the first time, then as you left it in this channel (this device) ---
  // A clip's replies take the right side while open, so the panel steps aside (not a choice to close it); the header
  // is told what's on screen. Its toggle then closes the replies and shows the panel, keeping the focus on itself
  // (`panelShown`, `togglePanel` below the replies).
  let panel = $state(untrack(() => clipPanelOpen(channelId)))
  const panelId = $derived(`clips-panel-${channelId}`)

  // --- the panel's counts: loaded on open (and after a reconnect clears them), then a second after clip changes ---
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
  /** Closes the replies panel; `refocus` puts the focus back on what opened it (else it stays where it is). */
  async function closeReplies(refocus = true) {
    const id = repliesOpen?.id
    const opener = repliesOpener
    repliesOpen = null
    repliesOpener = null
    if (!refocus) return
    await tick()
    const back = (opener?.isConnected ? opener : null) ?? chan?.querySelector<HTMLElement>(`[data-clip-id="${id}"] button`)
    back?.focus()
  }
  // --- the panel beside the replies (see the panel's note above) ---
  const panelShown = $derived(panel && !repliesRoot)
  function togglePanel() {
    if (repliesRoot) {
      void closeReplies(false)
      if (!panel) setClipPanelOpen(channelId, (panel = true))
      return
    }
    panel = !panel
    setClipPanelOpen(channelId, panel)
  }

  // A search result asked for one of this channel's clips: its panel shows the clip and its replies.
  $effect(() => {
    const r = ui.openClip
    if (!r || r.channelId !== channelId) return
    untrack(() => {
      ui.openClip = null
      openReplies(r.clip)
    })
  })
  $effect(() => () => {
    untrack(() => {
      if (ui.openClip?.channelId === channelId) ui.openClip = null
      if (ui.jumpClip?.channelId === channelId) ui.jumpClip = null
    })
  })
  // A search result (or inbox item) asked to go to one of this channel's clips (#120): the whole feed scrolls to it
  // and flashes it. One the feed can't reach (deleted since, or very far back) opens its panel instead.
  let jumpTarget = $state<{ clip: Message } | null>(null)
  $effect(() => {
    const r = ui.jumpClip
    if (!r || r.channelId !== channelId) return
    untrack(() => {
      ui.jumpClip = null
      filter = {} // a filter could hide it
      jumpTarget = { clip: r.clip }
    })
  })
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
      rememberClipsChannel(channelId) // an offered recording (#382) goes here next
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

  // An offered recording's Share (#382) asked for the editor here: as a dropped file (one at a time, Windows only).
  $effect(() => {
    const r = ui.shareClip
    if (!r || r.channelId !== channelId) return
    untrack(() => {
      ui.shareClip = null
      // one that waited too long (the channel never opened when asked, #394) isn't opened now
      if (pendingFresh(r.at)) void dropped([r.path])
    })
  })

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
  <ClipsHeader {channelId} {serverId} {filter} onFilter={(f) => (filter = f)} {view} onShare={share} onOpenPinned={openReplies}
    panel={panelShown} onPanel={togglePanel} {panelId} />
  <div class="body">
    <div class="main">
      <ClipFeed {channelId} {filter} layout={view} onOpenReplies={openReplies} {unsentReplies} {pending}
        jumpTo={jumpTarget} onJumpMissed={openReplies} />
    </div>
    {#if panelShown}
      <ClipsPanel {serverId} id={panelId} facets={app.state.clipFacets[channelId]} {filter} onFilter={(f) => (filter = f)} />
    {/if}
  </div>
  {#if repliesRoot}
    {#key repliesRoot.id}
      <RepliesPanel root={repliesRoot} {serverId} onClose={() => closeReplies()} />
    {/key}
  {/if}
  {#if dragging}<div class="drop-zone" aria-hidden="true"><span>Drop a video to share it</span></div>{/if}
  {#if notice.text}<p class="toast notice fade-in" role="status" out:exitFade>{notice.text}</p>{/if}
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
  /* the feed and the filters panel side by side; the feed gives way (the panel keeps its width) */
  .body { flex: 1; display: flex; min-height: 0; }
  .main { flex: 1; min-width: 0; display: flex; flex-direction: column; min-height: 0; }
  .editor-host { display: contents; }
  .drop-zone { position: absolute; inset: 8px; z-index: 4; display: grid; place-items: center; border: 2px dashed var(--focus-ring); border-radius: 14px; background: var(--veil); pointer-events: none; }
  .drop-zone span { padding: 8px 16px; border-radius: var(--radius-md); background: var(--bg-2); color: var(--text); font-size: 15px; font-weight: 600; }
  /* the shared .toast look; centred by margins, not a transform (WebKitGTK blurs transformed text) */
  .notice { position: absolute; left: 0; right: 0; bottom: 72px; z-index: 5; width: max-content; max-width: 80%; margin: 0 auto; }
</style>
