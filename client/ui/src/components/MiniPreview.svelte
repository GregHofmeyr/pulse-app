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
  /** While pressed: where the press started, and how far it has moved. */
  let press: { x: number; y: number } | null = null
  let offset = $state<{ x: number; y: number } | null>(null)
  /** The click that ends a drag is not a click on the preview. */
  let dragged = false

  // A preview takes the low layer, even when you left from full view (which comes back on return).
  $effect(() => {
    untrack(() => streamApi.setView(channelId, streamer, 'preview'))
  })

  function down(e: PointerEvent) {
    if (e.button !== 0 || (e.target as Element).closest('.x')) return
    press = { x: e.clientX, y: e.clientY }
    dragged = false
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
  }
  function move(e: PointerEvent) {
    if (!press) return
    const dx = e.clientX - press.x
    const dy = e.clientY - press.y
    if (!offset && Math.hypot(dx, dy) < DRAG_PX) return
    offset = { x: dx, y: dy }
  }
  function up(e: PointerEvent) {
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', up)
    const area = box?.parentElement?.getBoundingClientRect()
    if (offset && area) {
      // snap to the corner nearest where it was let go
      const v = e.clientY < area.top + area.height / 2 ? 't' : 'b'
      const h = e.clientX < area.left + area.width / 2 ? 'l' : 'r'
      corner = `${v}${h}` as Corner
      saveMiniCorner(corner)
      dragged = true
    }
    press = null
    offset = null
  }
  // dropped mid-drag (the watch ended): stop listening
  $effect(() => () => {
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', up)
  })

  /** Back to the call view, in full view if that's where you left from (`callView.full` is kept). */
  function open() {
    if (dragged) {
      dragged = false
      return
    }
    callView.goTo = channelId
  }
  function key(e: KeyboardEvent) {
    if (e.target !== e.currentTarget || (e.key !== 'Enter' && e.key !== ' ')) return
    e.preventDefault()
    open()
  }
</script>

<div bind:this={box} class="mini-preview {corner}" class:dragging={!!offset} role="button" tabindex="0" aria-label="Back to {name}'s stream"
  style:translate={offset ? `${offset.x}px ${offset.y}px` : undefined}
  onpointerdown={down} onclick={open} onkeydown={key}>
  {#if watch}<div class="host" {@attach hostVideo(channelId, streamer)}></div>{/if}
  <span class="name">{name}</span>
  <button type="button" class="x" aria-label="Stop watching" onclick={(e) => { e.stopPropagation(); void streamApi.unwatch(channelId, streamer) }}>
    <Icon name="x" size={14} />
  </button>
</div>

<style>
  .mini-preview { position: absolute; z-index: 20; width: 320px; aspect-ratio: 16 / 9; border-radius: 12px; overflow: hidden; background: #000; box-shadow: 0 12px 32px rgba(8, 9, 12, .5); cursor: pointer; touch-action: none; user-select: none; }
  .tl { top: 16px; left: 16px; }
  .tr { top: 16px; right: 16px; }
  .bl { bottom: 16px; left: 16px; }
  .br { bottom: 16px; right: 16px; }
  .dragging { cursor: grabbing; opacity: .9; }
  .host { position: absolute; inset: 0; pointer-events: none; }
  .host :global(video) { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; background: #000; }
  .name { position: absolute; left: 8px; bottom: 6px; padding: 1px 7px; border-radius: 6px; background: rgba(0, 0, 0, .55); color: #fff; font-size: 12px; font-weight: 600; pointer-events: none; }
  .x { position: absolute; top: 6px; right: 6px; width: 26px; height: 26px; border: 0; border-radius: 50%; background: rgba(0, 0, 0, .6); color: #fff; display: grid; place-items: center; opacity: 0; transition: opacity .12s; }
  .mini-preview:hover .x, .mini-preview:focus-within .x { opacity: 1; }
</style>
