<script lang="ts">
  import { untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import StreamDiagOverlay from './StreamDiagOverlay.svelte'
  import Select from './Select.svelte'
  import ParticipantMenu from './ParticipantMenu.svelte'
  import UserTile from './UserTile.svelte'
  import { menuPoint } from '../lib/participantMenu'
  import { app } from '../lib/store.svelte'
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

  function stop() {
    // first: closing full view unmounts this, and the props go with it
    void streamApi.unwatch(channelId, streamer)
    closeFull()
  }

  /** Esc goes back to the grid, unless it's meant for a text field, or something open over this view. */
  const TYPING = 'textarea, select, [contenteditable="true"], [contenteditable="plaintext-only"], [role="menu"], [role="listbox"], input:not([type="range"], [type="checkbox"], [type="radio"], [type="button"])'
  const OVER = '[role="dialog"], [aria-modal="true"], [role="menu"], [role="listbox"]'
  /** A dialog, popover or menu that's actually showing (one left open in a panel full view hid isn't). */
  const overlayShown = () =>
    [...document.querySelectorAll<HTMLElement>(OVER)].some((el) => !el.closest('[hidden]') && (el.checkVisibility?.() ?? true))
  function key(e: KeyboardEvent) {
    if (e.key !== 'Escape' || e.defaultPrevented) return
    if (document.activeElement?.closest(TYPING)) return
    // a dialog, popover or menu over this view takes Esc first (its own listener may run after this one)
    if (overlayShown()) return
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
    if (!p?.armed || justActivated() || e.detail > 1 || Math.hypot(e.clientX - p.x, e.clientY - p.y) > DRAG_PX) return
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

<div class="full-view">
  <div class="bar">
    {#if panels}
      <button type="button" class="panel-toggle" id={PANEL_TOGGLE_ID.left} aria-label={panels.leftName} aria-controls={PANEL_ID.left} aria-pressed={panels.left}
        data-tip="{panels.left ? 'Hide' : 'Show'} {panels.leftName.toLowerCase()}" onclick={() => panels.toggle('left')}><Icon name="panelLeft" size={18} /></button>
    {/if}
    <!-- between the corner toggles; when the panels leave too little room it wraps, the toggles stay put -->
    <div class="controls">
      <div class="lead">
        <button type="button" class="back" onclick={closeFull}><Icon name="chevronLeft" size={16} /> Back</button>
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
        <button type="button" class="stop" onclick={stop}>Stop watching</button>
      </div>
    </div>
    {#if panels?.hasRight}
      <button type="button" class="panel-toggle" id={PANEL_TOGGLE_ID.right} aria-label="Members" aria-controls={PANEL_ID.right} aria-pressed={panels.right}
        data-tip="{panels.right ? 'Hide' : 'Show'} members" onclick={() => panels.toggle('right')}><Icon name="panelRight" size={18} /></button>
    {/if}
  </div>
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
      <div class="slot"><UserTile {entry} compact onMenu={(e, a) => (menu = e.type === 'click' && menu?.opener === a ? null : { userId: entry.userId, ...menuPoint(e, a), opener: a })} /></div>
    {/each}
  </div>
</div>
{#if menu}
  {#key menu}<ParticipantMenu {channelId} {serverId} userId={menu.userId} x={menu.x} y={menu.y} opener={menu.opener} {onMessage} {onVoiceSettings} onClose={() => (menu = null)} />{/key}
{/if}

<style>
  .full-view { flex: 1; display: flex; flex-direction: column; min-height: 0; min-width: 0; background: #000; }
  .bar { flex-shrink: 0; min-height: 52px; padding: 4px 10px 5px; display: flex; align-items: flex-start; gap: 14px; background: var(--bg-1); border-bottom: 1px solid var(--bg-3); }
  /* the side panel toggles, one in each top corner; pressed while its panel shows */
  .panel-toggle { width: 32px; height: 32px; margin-top: 5px; flex-shrink: 0; padding: 0; border: 0; border-radius: 10px; background: transparent; color: var(--text-2); display: grid; place-items: center; }
  .panel-toggle:hover { background: var(--bg-3); color: var(--text); }
  .panel-toggle[aria-pressed='true'] { background: color-mix(in srgb, var(--accent) 18%, transparent); color: var(--accent); }
  .panel-toggle[aria-pressed='true']:hover { background: color-mix(in srgb, var(--accent) 28%, transparent); }
  .back { flex-shrink: 0; white-space: nowrap; height: 32px; padding: 0 12px 0 8px; border: 0; border-radius: 10px; background: var(--bg-3); color: var(--text); font-weight: 600; display: flex; align-items: center; gap: 4px; }
  /* rows of 42px (the quality select's height): 52px in all on one row, as before, with the corner toggles centred on the first */
  .controls { flex: 1; min-width: 0; display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px 14px; }
  .lead, .trail { display: flex; align-items: center; gap: 14px; min-width: 0; min-height: 42px; }
  .trail { flex-wrap: wrap; justify-content: flex-end; row-gap: 8px; margin-left: auto; }
  .name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ctl { display: flex; align-items: center; gap: 8px; font-size: 13px; color: var(--text-2); white-space: nowrap; }
  input[type='range'] { width: 120px; accent-color: var(--accent); }
  .stop { flex-shrink: 0; white-space: nowrap; height: 32px; padding: 0 14px; border: 0; border-radius: 10px; background: var(--danger); color: #fff; font-weight: 700; }
  .stage { position: relative; flex: 1; min-height: 0; }
  .host { position: absolute; inset: 0; }
  .unfocus { position: absolute; inset: 0; cursor: zoom-out; }
  .host :global(video) { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; background: #000; }
  .overlay { position: absolute; inset: 0; display: grid; place-items: center; background: rgba(0, 0, 0, .5); color: #fff; font-size: 18px; font-weight: 600; pointer-events: none; }
  .strip { flex-shrink: 0; padding: 8px 12px; display: flex; gap: 8px; overflow-x: auto; background: var(--bg-1); border-top: 1px solid var(--bg-3); }
  .slot { flex: 0 0 120px; }
</style>
