<script lang="ts">
  import { tick, untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import ColumnHandle from './ColumnHandle.svelte'
  import TextChannel from './TextChannel.svelte'
  import StreamDiagOverlay from './StreamDiagOverlay.svelte'
  import Select from './Select.svelte'
  import ParticipantMenu from './ParticipantMenu.svelte'
  import UserTile from './UserTile.svelte'
  import { overlayShown } from '../lib/escape'
  import { menuPoint } from '../lib/participantMenu'
  import { app, clock } from '../lib/store.svelte'
  import { chatBadge } from '../lib/conversations'
  import { fitStrip, STRIP } from '../lib/columns'
  import { columns, resetColumn, setColumn } from '../lib/columns.svelte'
  import { exitFade } from '../lib/exit'
  import { api } from '../lib/tauri'
  import { setVoiceChat, STRIP_ID, STRIP_TOGGLE_ID, toggleVoiceChat, voiceChat } from '../lib/voiceChat.svelte'
  import { callRoster, displayName, liveIn } from '../lib/selectors'
  import { selfVoice } from '../lib/voice.svelte'
  import { volumeLabel } from '../lib/voiceui'
  import { findWatch, stream, streamApi } from '../lib/stream/stream.svelte'
  import { hostVideo } from '../lib/stream/videoHost'
  import { callView, closeFull } from '../lib/stream/viewer.svelte'
  import { getFullPanels, PANEL_ID, PANEL_TOGGLE_ID } from '../lib/stream/fullPanels'
  import { watchWindowFocus, windowFocus } from '../lib/windowFocus.svelte'

  // The parent keys this on the stream, so the props never change for one instance.
  let { channelId, streamer, serverId, onMessage, onVoiceSettings }: {
    channelId: string
    streamer: string
    serverId: string | null
    onMessage: (userId: string) => void
    onVoiceSettings: () => void
  } = $props()

  /** The shell's side panels (hidden while this is on screen): a toggle for each in the bar's top corners. */
  const panels = getFullPanels()

  const watch = $derived(findWatch(channelId, streamer))
  const name = $derived(displayName(app.state, serverId, streamer))
  const reconnecting = $derived(liveIn(app.state, channelId)[streamer]?.status === 'reconnecting')
  const roster = $derived(callRoster(app.state, serverId, channelId, selfVoice()))
  const volume = $derived(stream.volumes[streamer] ?? 100)
  let quality = $state<string | null>(untrack(() => findWatch(channelId, streamer)?.quality ?? 'auto'))

  // Shown here: the full layer (under Auto) and the diagnostics, also when coming back from the
  // mini-preview. Leaving sets the preview again: Back (`closeFull`) or the mini-preview, not a teardown
  // here (state read in a teardown is the old value).
  $effect(() => {
    untrack(() => streamApi.setView(channelId, streamer, 'full'))
  })
  // on screen: the shell hides its side panels for it (and shows them again when this goes)
  $effect(() => {
    untrack(() => callView.fullViews++)
    return () => {
      untrack(() => callView.fullViews--)
    }
  })

  // The channel's chat as a strip beside the stream (#63): the voice chat panel's own view and its open/closed (the
  // shell keeps it out of its right column meanwhile), with its own width. Inside this view, so it stays in full screen.
  const channel = $derived(app.state.channels[channelId])
  const chatShown = $derived(!!channel && voiceChat.open)
  // unreads and mentions while it's closed, as on the stage header's toggle (#230)
  const badge = $derived(channel && !voiceChat.open ? chatBadge(app.state, channel, clock.nowIso) : null)
  let body = $state<HTMLElement>()
  let bodyWidth = $state(0)
  $effect(() => {
    if (!body || typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver((entries) => (bodyWidth = entries[entries.length - 1]?.contentRect.width ?? 0))
    ro.observe(body)
    return () => ro.disconnect()
  })
  const strip = $derived(fitStrip(columns.strip, bodyWidth))
  let stripEl = $state<HTMLElement>()
  /** The strip's close button: it goes, and the focus it had goes to the toggle. */
  function closeChat() {
    const at = document.activeElement
    const inside = !at || at === document.body || !!stripEl?.contains(at)
    setVoiceChat(false)
    if (inside) void tick().then(() => document.getElementById(STRIP_TOGGLE_ID)?.focus())
  }

  // Full screen (#63): the window goes full screen (as a clip's player does) and this view covers the whole app, so
  // dialogs, menus, popovers and toasts still show over it. Leaving (the button, Esc, or this view going) puts the
  // window back as it was.
  let screen = $state(false)
  let screenOn = false // for the teardown, which sees state as it was
  let windowWas: Promise<boolean> = Promise.resolve(false)
  function setScreen(on: boolean) {
    if (on === screenOn) return
    screen = screenOn = on
    if (on) windowWas = Promise.resolve(api.setFullscreen(true)).then((was) => was === true, () => false)
    else void windowWas.then((was) => api.setFullscreen(was)).catch(() => {})
  }
  $effect(() => () => setScreen(false))

  function stop() {
    // first: closing full view unmounts this, and the props go with it
    void streamApi.unwatch(channelId, streamer)
    closeFull()
  }

  /** Esc goes back to the grid, unless it's meant for a text field, or something open over this view. */
  const TYPING = 'textarea, select, [contenteditable="true"], [contenteditable="plaintext-only"], [role="menu"], [role="listbox"], input:not([type="range"], [type="checkbox"], [type="radio"], [type="button"])'
  function key(e: KeyboardEvent) {
    if (e.key !== 'Escape' || e.defaultPrevented) return
    if (document.activeElement?.closest(TYPING)) return
    // a dialog, popover or menu over this view takes Esc first (its own listener may run after this one)
    if (overlayShown()) return
    // full screen first, as a clip's player leaves it; then back to the grid
    if (screenOn) {
      e.preventDefault()
      setScreen(false)
      return
    }
    closeFull()
  }
  let menu = $state<{ userId: string; x: number; y: number; opener: HTMLElement } | null>(null)

  // Clicking the stream itself goes back to every tile, but only a plain, deliberate click: not the
  // second of a double-click, a drag, a press that closed a menu or list, or the click that brought
  // the window to the front (the webview passes that one on).
  const DRAG_PX = 5
  const ACTIVATION_MS = 300
  watchWindowFocus()
  // Two signals of the window coming to the front: the app's focus (from the OS, which can arrive
  // after the press, across processes) and the page's own focus event (in the page, before it).
  let activatedAt = 0
  let wasFocused = untrack(() => windowFocus.focused)
  $effect(() => {
    const f = windowFocus.focused
    if (f && !wasFocused) activatedAt = performance.now()
    wasFocused = f
  })
  const justActivated = () => performance.now() - activatedAt <= ACTIVATION_MS
  let press: { x: number; y: number; armed: boolean } | null = null
  function pressStream(e: PointerEvent) {
    const armed = e.button === 0 && !overlayShown() && document.hasFocus() && !justActivated()
    press = { x: e.clientX, y: e.clientY, armed }
  }
  function clickStream(e: MouseEvent) {
    const p = press
    press = null
    // checked again at the click: an activation noticed after the press still counts
    // not in full screen: a stray click there would throw you out of it and the view at once
    if (!p?.armed || screenOn || justActivated() || e.detail > 1 || Math.hypot(e.clientX - p.x, e.clientY - p.y) > DRAG_PX) return
    closeFull()
  }
  let stage = $state<HTMLElement>()
  /** Right-click on the stream: the streamer's menu, as on their grid tile. */
  function streamMenu(e: MouseEvent) {
    e.preventDefault()
    if (stage) menu = { userId: streamer, ...menuPoint(e, stage), opener: stage }
  }
</script>

<svelte:window onkeydown={key} onfocus={() => (activatedAt = performance.now())} />

<div class="full-view" class:screen>
  <div class="bar">
    <!-- the shell's side panels are under full screen: no toggles for them there -->
    {#if panels && !screen}
      <button type="button" class="panel-toggle" id={PANEL_TOGGLE_ID.left} aria-label={panels.leftName} aria-controls={PANEL_ID.left} aria-pressed={panels.left}
        data-tip="{panels.left ? 'Hide' : 'Show'} {panels.leftName.toLowerCase()}" onclick={() => panels.toggle('left')}><Icon name="panelLeft" size={18} /></button>
    {/if}
    <!-- between the corner toggles; when the panels leave too little room it wraps, the toggles stay put -->
    <div class="controls">
      <div class="lead">
        <button type="button" class="btn btn-quiet back" onclick={closeFull}><Icon name="chevronLeft" size={16} /> Back</button>
        <strong class="name">{name}</strong>
      </div>
      <div class="trail">
        <div class="ctl">
          <span>Quality</span>
          <Select bind:value={quality} label="Quality" options={[{ value: 'auto', label: 'Auto' }, { value: 'saver', label: 'Data saver' }]}
            onchange={() => streamApi.setQuality(channelId, streamer, quality === 'saver' ? 'saver' : 'auto')} />
        </div>
        <label class="ctl">
          Volume <strong>{volumeLabel(volume)}</strong>
          <input type="range" min="0" max="200" step="5" value={volume} aria-label="Stream volume" oninput={(e) => streamApi.setStreamVolume(streamer, +e.currentTarget.value)} />
        </label>
        <div class="tools">
          <!-- the channel's chat beside the stream (#63); a click opens it with the message box focused (#202) -->
          <button type="button" id={STRIP_TOGGLE_ID} class="tool chat" aria-pressed={voiceChat.open} aria-controls={STRIP_ID} aria-label="Chat{badge?.label ?? ''}"
            data-tip={voiceChat.open ? 'Hide chat' : 'Show chat'} onclick={() => toggleVoiceChat(true)}>
            <Icon name="message" size={18} />
            {#if badge?.mentions}<span class="pill chat-pill">{badge.mentions}</span>{:else if badge?.unread}<span class="chat-dot"></span>{/if}
          </button>
          <button type="button" class="tool" aria-label={screen ? 'Exit full screen' : 'Full screen'} data-tip={screen ? 'Exit full screen' : 'Full screen'}
            onclick={() => setScreen(!screen)}><Icon name="fullscreen" size={18} /></button>
        </div>
        <button type="button" class="btn btn-danger stop" onclick={stop}>Stop watching</button>
      </div>
    </div>
    {#if panels?.hasRight && !screen}
      {@const name = panels.rightName ?? 'Members'}
      <button type="button" class="panel-toggle" id={PANEL_TOGGLE_ID.right} aria-label={name} aria-controls={PANEL_ID.right} aria-pressed={panels.right}
        data-tip="{panels.right ? 'Hide' : 'Show'} {name.toLowerCase()}" onclick={() => panels.toggle('right')}><Icon name="panelRight" size={18} /></button>
    {/if}
  </div>
  <div class="body" bind:this={body}>
  <div class="watch">
  <!-- focusable from script only: the stream's menu hands focus back here when it closes -->
  <div class="stage" bind:this={stage} tabindex="-1">
    {#if watch}<div class="host" {@attach hostVideo(channelId, streamer)}></div>{/if}
    <!-- the focused stream itself: a mouse shortcut back to every tile (Back and Esc are the keyboard's) -->
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="unfocus" onpointerdown={pressStream} onclick={clickStream} oncontextmenu={streamMenu}></div>
    <StreamDiagOverlay kind="in" />
    {#if reconnecting}<div class="overlay" role="status">Reconnecting…</div>{/if}
  </div>
  <div class="strip">
    {#each roster as entry (entry.userId)}
      <div class="slot"><UserTile {entry} {serverId} compact onMenu={(e, a) => (menu = e.type === 'click' && menu?.opener === a ? null : { userId: entry.userId, ...menuPoint(e, a), opener: a })} /></div>
    {/each}
  </div>
  </div>
  {#if chatShown}
    <!-- the columns' handle, centred on the strip's edge -->
    <div class="grip"><ColumnHandle side="right" width={strip.width} min={STRIP.min} max={strip.max} controls={STRIP_ID} label="Resize the chat"
      onResize={(w, done) => setColumn('strip', w, done)} onReset={() => resetColumn('strip')} /></div>
  {/if}
  {#if chatShown}
    <!-- fades in and out as the stage's chat panel does (not under reduced motion, while live or unfocused); leaving,
         it lies over the stream's edge so the stream takes the room back at once -->
    <aside id={STRIP_ID} class="side-chat fade-in" aria-label="Chat" style:width="{strip.width}px" bind:this={stripEl}
      onanimationend={(e) => e.currentTarget.classList.remove('fade-in')} out:exitFade onoutrostart={(e) => e.currentTarget.classList.add('leaving')}>
      <TextChannel {channelId} {serverId} panel={{ onClose: closeChat, shown: true }} />
    </aside>
  {/if}
  </div>
</div>
{#if menu}
  {#key menu}<ParticipantMenu {channelId} {serverId} userId={menu.userId} x={menu.x} y={menu.y} opener={menu.opener} {onMessage} {onVoiceSettings} onClose={() => (menu = null)} />{/key}
{/if}

<style>
  .full-view { flex: 1; display: flex; flex-direction: column; min-height: 0; min-width: 0; background: #000; }
  /* full screen (#63): the whole window, over the app's own panels (z 0-19) and under its dialogs (20 and up), menus,
     popovers, toasts and tooltips, so they still show */
  .full-view.screen { position: fixed; inset: 0; z-index: 19; }
  .bar { flex-shrink: 0; min-height: 52px; padding: 4px 10px 5px; display: flex; align-items: flex-start; gap: 14px; background: var(--bg-1); border-bottom: 1px solid var(--bg-3); }
  /* the side panel toggles, one in each top corner; pressed while its panel shows */
  /* the chat and full screen buttons (#63) look the same, in the controls */
  .panel-toggle, .tool { width: 32px; height: 32px; flex-shrink: 0; padding: 0; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text-2); display: grid; place-items: center; }
  .panel-toggle { margin-top: 5px; }
  .panel-toggle:hover, .tool:hover { background: var(--bg-3); color: var(--text); }
  .panel-toggle[aria-pressed='true'], .tool[aria-pressed='true'] { background: var(--accent-mid); color: var(--accent-text); }
  .panel-toggle[aria-pressed='true']:hover, .tool[aria-pressed='true']:hover { background: var(--accent-strong); }
  .tools { display: flex; align-items: center; gap: 4px; flex-shrink: 0; }
  /* the chat's dot and count (#230), as on the stage header's toggle, ringed in the bar's colour */
  .chat { position: relative; --ring: var(--bg-1); }
  .chat:hover { --ring: var(--bg-3); }
  .chat-dot { position: absolute; top: 4px; right: 4px; width: 7px; height: 7px; border-radius: 50%; background: var(--text); box-shadow: 0 0 0 2px var(--ring); }
  .pill { min-width: 16px; height: 16px; border-radius: var(--radius-sm); background: var(--danger); color: var(--on-danger); font-size: 10px; font-weight: 700; display: inline-block; box-sizing: border-box; text-align: center; line-height: normal; text-box: trim-both cap alphabetic; padding: calc((16px - 1cap) / 2) 4px; }
  .chat-pill { position: absolute; top: -2px; right: -4px; box-shadow: 0 0 0 2px var(--ring); }
  .back { flex-shrink: 0; white-space: nowrap; padding-left: 8px; gap: 4px; }
  /* rows of 42px (the quality select's height): 52px in all on one row, as before, with the corner toggles centred on the first */
  .controls { flex: 1; min-width: 0; display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px 14px; }
  .lead, .trail { display: flex; align-items: center; gap: 14px; min-width: 0; min-height: 42px; }
  .trail { flex-wrap: wrap; justify-content: flex-end; row-gap: 8px; margin-left: auto; }
  .name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ctl { display: flex; align-items: center; gap: 8px; font-size: 13px; color: var(--text-2); white-space: nowrap; }
  input[type='range'] { width: 120px; accent-color: var(--accent); }
  .stop { flex-shrink: 0; white-space: nowrap; }
  /* under the bar: the stream with its people below, and the chat strip (#63) beside both */
  .body { position: relative; flex: 1; min-height: 0; display: flex; }
  .watch { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  /* a zero-width slot: the handle's 8px (and its -8px margins) sit centred on the strip's edge */
  .grip { flex: 0 0 0; display: flex; justify-content: center; }
  .side-chat { flex-shrink: 0; display: flex; flex-direction: column; min-height: 0; background: var(--bg-1); border-left: 1px solid var(--bg-3); overflow: hidden; }
  .side-chat:global(.leaving) { position: absolute; top: 0; right: 0; bottom: 0; z-index: 1; pointer-events: none; }
  .stage { position: relative; flex: 1; min-height: 0; }
  .host { position: absolute; inset: 0; }
  .unfocus { position: absolute; inset: 0; cursor: zoom-out; }
  .screen .unfocus { cursor: default; }
  .host :global(video) { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; background: #000; }
  .overlay { position: absolute; inset: 0; display: grid; place-items: center; background: rgba(0, 0, 0, .5); color: #fff; font-size: 18px; font-weight: 600; pointer-events: none; }
  .strip { flex-shrink: 0; padding: 8px 12px; display: flex; gap: 8px; overflow-x: auto; background: var(--bg-1); border-top: 1px solid var(--bg-3); }
  .slot { flex: 0 0 120px; }
</style>
