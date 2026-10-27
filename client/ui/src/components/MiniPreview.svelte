<script lang="ts">
  import { untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import { app } from '../lib/store.svelte'
  import { displayName } from '../lib/selectors'
  import { loadMiniCorner, saveMiniCorner, type Corner } from '../lib/stream/prefs'
  import { findWatch, streamApi } from '../lib/stream/stream.svelte'
  import { hostVideo } from '../lib/stream/videoHost'
  import { callView } from '../lib/stream/viewer.svelte'

  // The shell keys this on the stream, so the props never change for one instance.
  let { channelId, streamer }: { channelId: string; streamer: string } = $props()

  /** A press that moves further than this is a drag, not a click. */
  const DRAG_PX = 5

  const watch = $derived(findWatch(channelId, streamer))
  const name = $derived(displayName(app.state, app.state.channels[channelId]?.server_id ?? null, streamer))
  let corner = $state<Corner>(loadMiniCorner())
  let box = $state<HTMLElement>()
  /** While pressed: where the press started, and which pointer. */
  let press: { x: number; y: number; id: number } | null = null
  let offset = $state<{ x: number; y: number } | null>(null)
  /** The mouse click that ends a drag is not a click on the preview (cleared once that click has had its turn). */
  let dragged = false

  // A preview takes the low layer, even when you left from full view (which comes back on return).
  $effect(() => {
    untrack(() => streamApi.setView(channelId, streamer, 'preview'))
  })

  function down(e: PointerEvent) {
    if (e.button !== 0 || (e.target as Element).closest('.x')) return
    // no capture yet: it would retarget a plain click's click event away from the open button
    press = { x: e.clientX, y: e.clientY, id: e.pointerId }
    dragged = false
  }
  function move(e: PointerEvent) {
    if (!press) return
    // the button came up somewhere we didn't hear (released off the box before a drag began)
    if (!offset && !(e.buttons & 1)) return abort()
    const dx = e.clientX - press.x
    const dy = e.clientY - press.y
    if (!offset) {
      if (Math.hypot(dx, dy) < DRAG_PX) return
      // a drag: moves and the release come here even off the box (or the window)
      try {
        box?.setPointerCapture?.(press.id)
      } catch {
        /* no such pointer (synthetic events) */
      }
    }
    offset = { x: dx, y: dy }
  }
  function up(e: PointerEvent) {
    if (!press) return
    const panel = box?.parentElement?.getBoundingClientRect()
    if (offset && panel) {
      // snap to the corner nearest where it was let go
      const v = e.clientY < panel.top + panel.height / 2 ? 't' : 'b'
      const h = e.clientX < panel.left + panel.width / 2 ? 'l' : 'r'
      corner = `${v}${h}` as Corner
      saveMiniCorner(corner)
      dragged = true
      // released off the open button, no click follows: don't swallow a later one
      setTimeout(() => (dragged = false))
    }
    press = null
    offset = null
  }
  // Fixed to the window, so no panel's overflow can clip it and it floats over the side panels when
  // dragged there; its corners still follow the area it belongs to (the call or chat panel), which
  // moves when the window resizes or full view hides the side panels.
  // Watching the area's size is enough: in the shell, everything that moves it (a side panel hiding
  // or showing) also resizes it, and a window resize doesn't move it (the header and gutters are fixed).
  let area = $state({ top: 0, left: 0, right: 0, bottom: 0 })
  function measure() {
    const r = box?.parentElement?.getBoundingClientRect()
    if (!r) return
    const next = { top: r.top, left: r.left, right: window.innerWidth - r.right, bottom: window.innerHeight - r.bottom }
    // untracked: the effect that first calls this must not re-run (and observe again) on its own write
    const now = untrack(() => area)
    if (next.top !== now.top || next.left !== now.left || next.right !== now.right || next.bottom !== now.bottom) area = next
  }
  $effect(() => {
    const parent = box?.parentElement
    if (!parent) return
    measure()
    const ro = typeof ResizeObserver === 'function' ? new ResizeObserver(measure) : null
    ro?.observe(parent)
    return () => ro?.disconnect()
  })

  /** The drag was taken away (cancelled, capture lost): back where it was, nothing saved. */
  function abort() {
    press = null
    offset = null
  }

  /** Back to the call view, in full view if that's where you left from (`callView.full` is kept). */
  function open(e: MouseEvent) {
    // detail 0: Enter/Space on the button, never the end of a drag
    if (dragged && e.detail !== 0) {
      dragged = false
      return
    }
    callView.goTo = channelId
  }
</script>

<!-- a plain box: dragging is a pointer shortcut; the controls are the open button and the ✕ -->
<div bind:this={box} class="mini-preview {corner}" class:dragging={!!offset} role="group" aria-label="{name}'s stream"
  style:translate={offset ? `${offset.x}px ${offset.y}px` : undefined}
  style:--area-top="{area.top}px" style:--area-left="{area.left}px" style:--area-right="{area.right}px" style:--area-bottom="{area.bottom}px"
  onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={abort} onlostpointercapture={abort}>
  {#if watch}<div class="host" {@attach hostVideo(channelId, streamer)}></div>{/if}
  <button type="button" class="open" aria-label="Back to {name}'s stream" onclick={open}></button>
  <span class="name">{name}</span>
  <button type="button" class="x" aria-label="Stop watching" onclick={() => void streamApi.unwatch(channelId, streamer)}>
    <Icon name="x" size={14} />
  </button>
</div>

<style>
  /* above panel content (z 0-2), below every popup, menu and dialog (z 5+) */
  .mini-preview { position: fixed; z-index: 4; width: 320px; aspect-ratio: 16 / 9; border-radius: 12px; overflow: hidden; background: #000; box-shadow: 0 12px 32px rgba(8, 9, 12, .5); cursor: pointer; touch-action: none; user-select: none; }
  /* bottom corners sit above a text channel's composer; toasts (z 40) stay on top */
  .tl { top: calc(var(--area-top) + 68px); left: calc(var(--area-left) + 16px); }
  .tr { top: calc(var(--area-top) + 68px); right: calc(var(--area-right) + 16px); }
  .bl { bottom: calc(var(--area-bottom) + 96px); left: calc(var(--area-left) + 16px); }
  .br { bottom: calc(var(--area-bottom) + 96px); right: calc(var(--area-right) + 16px); }
  .dragging { cursor: grabbing; opacity: .9; }
  .host { position: absolute; inset: 0; pointer-events: none; }
  .open { position: absolute; inset: 0; padding: 0; border: 0; background: transparent; cursor: pointer; }
  .open:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .host :global(video) { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; background: #000; }
  .name { position: absolute; left: 8px; bottom: 6px; padding: 1px 7px; border-radius: 6px; background: rgba(0, 0, 0, .55); color: #fff; font-size: 12px; font-weight: 600; pointer-events: none; }
  .x { position: absolute; top: 6px; right: 6px; width: 26px; height: 26px; border: 0; border-radius: 50%; background: rgba(0, 0, 0, .6); color: #fff; display: grid; place-items: center; opacity: 0; transition: opacity .12s; }
  .mini-preview:hover .x, .mini-preview:focus-within .x { opacity: 1; }
  .dragging .open { cursor: grabbing; }
</style>
