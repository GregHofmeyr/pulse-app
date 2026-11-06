<script lang="ts">
  import { tick, untrack } from 'svelte'
  import type { Attachment } from 'svelte/attachments'
  import Icon from './Icon.svelte'
  import { app, clock } from '../lib/store.svelte'
  import { chatBadge } from '../lib/conversations'
  import { myServerTips, selfVoice, voice, voiceApi } from '../lib/voice.svelte'
  import { callRoster, liveIn } from '../lib/selectors'
  import { gridTiles, nobodyStreaming } from '../lib/callgrid'
  import { stageLabel } from '../lib/voicestage'
  import { flipInverse, stageLayout, type Rect } from '../lib/stagelayout'
  import { motion, watchMotion } from '../lib/animate.svelte'
  import FullView from './FullView.svelte'
  import QotyStrip from './QotyStrip.svelte'
  import ParticipantMenu from './ParticipantMenu.svelte'
  import SoundboardPopover from './SoundboardPopover.svelte'
  import SoundDialog from './SoundDialog.svelte'
  import SoundTidy from './SoundTidy.svelte'
  import type { Sound } from '../lib/protocol/Sound'
  import StreamTile from './StreamTile.svelte'
  import UserTile from './UserTile.svelte'
  import { loadShowEveryone, saveShowEveryone } from '../lib/stream/prefs'
  import { menuPoint } from '../lib/participantMenu'
  import { goLiveUi, pickSource } from '../lib/stream/golive.svelte'
  import { isWindows } from '../lib/stream/platform'
  import { findWatch, stream } from '../lib/stream/stream.svelte'
  import { callView, openFull } from '../lib/stream/viewer.svelte'
  import { PANEL_ID } from '../lib/stream/fullPanels'
  import { CHAT_TOGGLE_ID, toggleVoiceChat, voiceChat } from '../lib/voiceChat.svelte'

  let { channelId, serverId, onMessage, onVoiceSettings }: {
    channelId: string
    serverId: string | null
    onMessage: (userId: string) => void
    onVoiceSettings: () => void
  } = $props()

  const channel = $derived(app.state.channels[channelId])
  // the chat's unreads and mentions, as on its row's chat icon, while the panel is closed (#230)
  const badge = $derived(channel && !voiceChat.open ? chatBadge(app.state, channel, clock.nowIso) : null)
  const inHere = $derived(voice.channelId === channelId)
  const tips = $derived(myServerTips())
  const meId = $derived(app.state.me?.id ?? '')
  const roster = $derived(callRoster(app.state, serverId, channelId, selfVoice()))
  let showEveryone = $state(true)
  // the choice is per signed-in user: load it when the user is known (and again if it changes)
  $effect(() => {
    showEveryone = loadShowEveryone(meId)
  })
  let menu = $state<{ userId: string; x: number; y: number; opener: HTMLElement } | null>(null)
  const myLive = $derived(stream.live?.channelId === channelId)
  const tiles = $derived(gridTiles({ roster, streams: liveIn(app.state, channelId), meId, showEveryone, myLive }))

  const tileKey = (t: (typeof tiles)[number]) => (t.kind === 'user' ? `u:${t.entry.userId}` : `s:${t.streamer}`)

  // The stage (#222): every tile, a person's or a stream's, sized and placed by stageLayout to fill it.
  let stageEl = $state<HTMLElement>()
  let innerEl = $state<HTMLElement>()
  let stageW = $state(0)
  let stageH = $state(0)
  $effect(() => {
    const el = stageEl
    if (!el) return
    const measure = () => {
      stageW = el.clientWidth
      stageH = el.clientHeight
    }
    measure()
    const ro = typeof ResizeObserver === 'function' ? new ResizeObserver(measure) : null
    ro?.observe(el)
    return () => ro?.disconnect()
  })
  // until it's measured the tiles stay hidden: they'd show at the smallest size for a frame, then jump
  const measured = $derived(stageW > 0 && stageH > 0)
  const layout = $derived(stageLayout({ count: tiles.length, width: stageW, height: stageH }))

  // FLIP when tiles join, leave or the stage resizes: each slot is put at its new place, drawn back where it was by a
  // transform, then let go to glide in on the stylesheet's transition (motion tokens). One measure before the change
  // and one after, transforms only in between: nothing re-lays out per frame.
  watchMotion()
  const slots = new Map<string, HTMLElement>()
  // not $state: the first layout on screen doesn't animate, and the tiles that arrive after it fade in
  let ready = false
  let before: Map<string, Rect> | null = null
  const animates = () => ready && !motion.reducedMotion && !motion.streaming
  // This reruns when a tile's entry changes (someone mutes): only a slot new to its key is an arrival. Keys that left
  // are pruned after each layout, below.
  const slot = (key: string): Attachment<HTMLElement> => (el) => {
    if (slots.get(key) === el) return
    slots.set(key, el)
    if (ready && animates()) el.classList.add('fade-in')
  }
  $effect.pre(() => {
    void layout
    untrack(() => {
      if (!animates()) {
        before = null
        return
      }
      before = new Map()
      for (const [k, el] of slots) {
        const r = el.getBoundingClientRect()
        before.set(k, { x: r.left, y: r.top, w: r.width, h: r.height })
      }
    })
  })
  $effect(() => {
    const { rects } = layout
    const keys = tiles.map(tileKey)
    untrack(() => {
      for (const [k, el] of slots) if (!keys.includes(k) || !el.isConnected) slots.delete(k)
      if (!measured || !innerEl) return
      const from = before
      before = null
      if (!ready) {
        ready = true
        return
      }
      if (!from) return
      const origin = innerEl.getBoundingClientRect()
      const moves: [HTMLElement, string][] = []
      keys.forEach((k, i) => {
        const el = slots.get(k)
        const was = from.get(k)
        const to = rects[i]
        if (!el || !was || !to) return
        const inv = flipInverse(was, { x: origin.left + to.x, y: origin.top + to.y, w: to.w, h: to.h })
        if (inv) moves.push([el, `translate(${inv.dx}px, ${inv.dy}px) scale(${inv.sx}, ${inv.sy})`])
      })
      if (!moves.length) return
      for (const [el, t] of moves) {
        el.classList.add('snap')
        el.style.transform = t
      }
      void innerEl.offsetWidth // the one reflow: the drawn-back place is where the glide starts
      for (const [el] of moves) {
        el.classList.remove('snap')
        el.style.transform = ''
      }
    })
  })

  function setShowEveryone(on: boolean) {
    showEveryone = on
    if (meId) saveShowEveryone(meId, on)
  }
  function openMenu(userId: string, e: MouseEvent | KeyboardEvent, anchor: HTMLElement) {
    // clicking the opener again closes its menu
    if (e.type === 'click' && menu?.opener === anchor) menu = null
    else menu = { userId, ...menuPoint(e, anchor), opener: anchor }
  }
  /** The watch in full view, when it's in this channel and still being watched. */
  const fullStreamer = $derived.by(() => {
    const f = callView.full
    return f && f.channelId === channelId && findWatch(f.channelId, f.streamer) ? f.streamer : null
  })

  // Leaving full view unmounts whatever had keyboard focus there: hand it to that stream's grid tile.
  let wasFull: string | null = null
  $effect(() => {
    const now = fullStreamer
    const left = wasFull
    wasFull = now
    if (!left || now) return
    void tick().then(() => {
      const at = document.activeElement
      if (at && at !== document.body) return // something else already has it
      document.querySelector<HTMLElement>(`[data-stream="${CSS.escape(left)}"] .host`)?.focus()
    })
  })

  let sbOpen = $state(false)
  let sbBtn = $state<HTMLButtonElement>()
  // The dialogs live outside the popover (its click-outside would otherwise close it under them),
  // so opening one closes the popover.
  let sbDialog = $state<{ kind: 'add'; serverId: string } | { kind: 'edit'; sound: Sound } | { kind: 'tidy'; serverId: string } | null>(null)
  const openDialog = (d: NonNullable<typeof sbDialog>) => {
    sbOpen = false
    sbDialog = d
  }
  // leaving the call takes the button away, and the popover with it
  $effect(() => {
    if (!inHere) {
    sbOpen = false
    sbDialog = null
  }
  })

  const inCall = $derived(inHere && voice.connection === 'connected')
  const canGoLive = $derived(inCall && !stream.live)
  const windows = isWindows()
</script>

<div class="panel">
  <div class="head">
    <Icon name="speaker" /> <span class="title">{channel?.name}</span>
    {#if inHere}
      <span class="status" class:warn={voice.stage.stage !== 'connected'}>
        {stageLabel(voice.stage)}
      </span>
    {/if}
    <button type="button" class="icon-btn eye" aria-pressed={showEveryone} aria-label="Show non-streaming participants" data-tip="Show non-streaming participants" onclick={() => setShowEveryone(!showEveryone)}>
      <Icon name={showEveryone ? 'eye' : 'eyeOff'} size={18} />
    </button>
    <!-- its chat (#226): the panel on the right; a click opens it with the message box focused (#202) -->
    <button type="button" id={CHAT_TOGGLE_ID} class="icon-btn chat" aria-pressed={voiceChat.open} aria-controls={PANEL_ID.right} aria-label="Chat{badge?.label ?? ''}" data-tip={voiceChat.open ? 'Hide chat' : 'Show chat'} onclick={() => toggleVoiceChat(true)}>
      <Icon name="message" size={18} />
      {#if badge?.mentions}<span class="pill chat-pill">{badge.mentions}</span>{:else if badge?.unread}<span class="chat-dot"></span>{/if}
    </button>
  </div>

  {#if voice.error}<p class="error" role="alert">{voice.error}</p>{/if}
  {#if voice.stalled && inHere}<p class="error" role="alert">Your audio device stopped responding. Check it's connected, then rejoin.</p>{/if}

  {#if fullStreamer}
    {#key fullStreamer}<FullView {channelId} streamer={fullStreamer} {serverId} {onMessage} {onVoiceSettings} />{/key}
  {:else}
    <div class="grid-wrap" bind:this={stageEl}>
      {#if nobodyStreaming(tiles, showEveryone)}
        <p class="empty">Nobody's streaming — <button type="button" class="link-btn" onclick={() => setShowEveryone(true)}>show everyone</button></p>
      {:else if !tiles.length}
        <p class="empty">Nobody's here yet.</p>
      {:else}
        <div class="stage" class:measuring={!measured} bind:this={innerEl} style:height="{layout.height}px">
          {#each tiles as t, i (tileKey(t))}
            {@const r = layout.rects[i]}
            <div class="slot" style:left="{r.x}px" style:top="{r.y}px" style:width="{r.w}px" style:height="{r.h}px" {@attach slot(tileKey(t))}>
              {#if t.kind === 'user'}
                <UserTile entry={t.entry} {serverId} height={r.h} onMenu={(e, a) => openMenu(t.entry.userId, e, a)} />
              {:else}
                <StreamTile {channelId} {serverId} streamer={t.streamer} mine={t.mine} onMenu={(e, a) => openMenu(t.streamer, e, a)} onFull={openFull} height={r.h} />
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}

  {#if serverId}<QotyStrip {serverId} />{/if}

  {#if goLiveUi.error && goLiveUi.channelId === channelId}<p class="error" role="alert">{goLiveUi.error}</p>{/if}

  <div class="bar-wrap">
    {#if inHere}
      <div class="controls">
        <!-- your own mute still toggles while server muted (#140); the tip says why the mic stays shut -->
        <button class="ctl" class:on={voice.controls.muted} class:server={!!tips.mute} data-tip={tips.mute ?? undefined} aria-pressed={voice.controls.muted} aria-label={(voice.controls.muted ? "Unmute" : "Mute") + (tips.mute ? ` (${tips.mute})` : "")} onclick={() => voiceApi.toggleMute()}>
          <Icon name={voice.controls.muted || tips.mute ? 'micOff' : 'mic'} size={20} />
        </button>
        <button class="ctl" class:on={voice.controls.deafened} class:server={!!tips.deafen} data-tip={tips.deafen ?? undefined} aria-pressed={voice.controls.deafened} aria-label={(voice.controls.deafened ? "Undeafen" : "Deafen") + (tips.deafen ? ` (${tips.deafen})` : "")} onclick={() => voiceApi.toggleDeafen()}>
          <Icon name={voice.controls.deafened || tips.deafen ? 'headphonesOff' : 'headphones'} size={20} />
        </button>
        <button bind:this={sbBtn} class="ctl" class:open={sbOpen} aria-label="Soundboard" aria-haspopup="dialog" aria-expanded={sbOpen} data-tip="Soundboard" data-menu-toggle onclick={() => (sbOpen = !sbOpen)}>
          <Icon name="soundboard" size={20} />
        </button>
        {#if canGoLive}
          <!-- aria-disabled, not disabled: a disabled button swallows the hover that shows the tooltip -->
          <button class="golive" class:off={!windows} aria-disabled={!windows} data-tip={windows ? undefined : "Streaming from Linux isn't supported yet"} onclick={() => pickSource(channelId)}>Go Live</button>
        {/if}
        <button class="leave" aria-label="Leave voice" onclick={() => voiceApi.leave()}><Icon name="hangup" size={22} /></button>
      </div>
    {:else}
      <button class="primary" onclick={() => voiceApi.join(channelId)}>Join voice</button>
    {/if}
  </div>
</div>

{#if sbOpen && sbBtn}
  <SoundboardPopover
    anchor={sbBtn}
    onClose={() => (sbOpen = false)}
    onAdd={(serverId) => openDialog({ kind: 'add', serverId })}
    onEdit={(sound) => openDialog({ kind: 'edit', sound })}
    onTidy={(serverId) => openDialog({ kind: 'tidy', serverId })}
  />
{/if}

{#if sbDialog?.kind === 'tidy'}
  <SoundTidy serverId={sbDialog.serverId} onClose={() => (sbDialog = null)} />
{:else if sbDialog}
  <SoundDialog mode={sbDialog} onClose={() => (sbDialog = null)} />
{/if}

{#if menu}
  {#key menu}<ParticipantMenu {channelId} {serverId} userId={menu.userId} x={menu.x} y={menu.y} opener={menu.opener} {onMessage} {onVoiceSettings} onClose={() => (menu = null)} />{/key}
{/if}

<style>
  .panel { flex: 1; display: flex; flex-direction: column; min-height: 0; }
  .head { height: 52px; flex-shrink: 0; padding: 0 18px; display: flex; align-items: center; gap: 10px; border-bottom: 1px solid var(--bg-3); color: var(--ok-text); }
  .title { color: var(--text); font-size: 15px; font-weight: 600; flex: 1; }
  .status { font-size: 12px; font-weight: 600; color: var(--ok-text); padding: 4px 10px; border-radius: 12px; background: var(--ok-soft); }
  .status.warn { color: var(--warn-text); background: var(--warn-soft); }
  .error { margin: 12px 18px 0; color: var(--danger-text); font-size: 13px; }
  .eye[aria-pressed='true'], .chat[aria-pressed='true'] { color: var(--text); }
  /* the chat's dot and count (#230): the row's chat icon's (ChannelList.svelte), ringed in the header's colour */
  .chat { position: relative; --ring: var(--bg-2); }
  .chat:hover { --ring: var(--bg-3); }
  .chat-dot { position: absolute; top: 4px; right: 4px; width: 7px; height: 7px; border-radius: 50%; background: var(--text); box-shadow: 0 0 0 2px var(--ring); }
  .pill { min-width: 16px; height: 16px; border-radius: var(--radius-sm); background: var(--danger); color: var(--on-danger); font-size: 10px; font-weight: 700; display: inline-block; box-sizing: border-box; text-align: center; line-height: normal; text-box: trim-both cap alphabetic; padding: calc((16px - 1cap) / 2) 4px; }
  .chat-pill { position: absolute; top: -2px; right: -4px; box-shadow: 0 0 0 2px var(--ring); }
  /* the stage (#222): stageLayout places each slot (its 20px padding and 14px gap are the grid's were); a crowd too
     big for it scrolls */
  .grid-wrap { flex: 1; min-height: 0; overflow-x: hidden; overflow-y: auto; }
  .stage { position: relative; }
  .stage.measuring { visibility: hidden; }
  .slot { position: absolute; transform-origin: 0 0; transition: transform var(--dur-slow) var(--ease); }
  /* set from script for the FLIP start, so global (Svelte would drop it as unused) */
  .slot:global(.snap) { transition: none; }
  .slot > :global(.tile) { width: 100%; height: 100%; }
  .empty { margin: 20px; color: var(--text-3); }
  .bar-wrap { height: 84px; flex-shrink: 0; display: grid; place-items: center; }
  .controls { padding: 8px; background: var(--bg-inset); border-radius: 16px; display: flex; gap: 8px; }
  .ctl { width: 48px; height: 48px; border: 0; border-radius: 14px; background: var(--bg-3); color: var(--text); display: grid; place-items: center; }
  .ctl:hover { background: var(--bg-4); }
  .ctl.open { background: var(--bg-4); }
  .ctl.on { background: var(--danger); color: var(--on-danger); }
  /* server muted / deafened (#140): filled, with a ring, whatever your own toggle says */
  .ctl.server { background: var(--danger); color: var(--on-danger); box-shadow: 0 0 0 2px var(--ring), 0 0 0 4px var(--danger); }
  .leave { width: 64px; height: 48px; border: 0; border-radius: 14px; background: var(--danger); color: var(--on-danger); display: grid; place-items: center; }
  .golive { height: 48px; padding: 0 16px; border: 0; border-radius: 14px; background: var(--accent); color: var(--on-accent); font-weight: 600; }
  .golive.off { opacity: .5; cursor: not-allowed; }
  .ctl.on:hover, .leave:hover, .golive:not(.off):hover { filter: brightness(1.1); }
  .primary { height: 44px; padding: 0 22px; border: 0; border-radius: 12px; background: var(--accent); color: var(--on-accent); font-weight: 600; }
  .primary:hover { filter: brightness(1.08); }
</style>
