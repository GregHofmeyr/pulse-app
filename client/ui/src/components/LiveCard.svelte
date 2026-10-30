<script lang="ts">
  import StreamDiagOverlay from './StreamDiagOverlay.svelte'
  import { app } from '../lib/store.svelte'
  import { liveIn, displayName } from '../lib/selectors'
  import { loadHidePreview, saveHidePreview } from '../lib/stream/prefs'
  import { liveMediaTrack, stream, streamApi } from '../lib/stream/stream.svelte'

  /** `tile`: fills a 16:9 grid tile, the preview behind a compact overlay of the controls. */
  let { channelId, serverId, tile = false }: { channelId: string; serverId: string | null; tile?: boolean } = $props()

  const live = $derived(stream.live)
  const viewers = $derived(liveIn(app.state, channelId)[app.state.me?.id ?? '']?.viewers ?? [])
  let hidden = $state(loadHidePreview())
  let video = $state<HTMLVideoElement>()

  $effect(() => {
    // the element this run set up: by teardown the binding may already be cleared (the preview went with Stop)
    const v = video
    if (!v) return
    const t = liveMediaTrack()
    v.srcObject = t ? new MediaStream([t]) : null
    return () => {
      v.srcObject = null
    }
  })

  const toggleHidden = () => {
    hidden = !hidden
    saveHidePreview(hidden)
  }
</script>

{#if live}
  <section class="card" class:tile aria-label="You're live">
    <div class="row">
      <span class="dot"></span>
      <strong>You're live</strong>
      <span class="meta">{live.res}p{live.fps}</span>
      <span class="spacer"></span>
      <button type="button" class="ghost" onclick={toggleHidden}>{hidden ? 'Show preview' : 'Hide preview'}</button>
    </div>
    {#if !hidden}
      <div class="preview">
        <video bind:this={video} muted autoplay playsinline></video>
        <StreamDiagOverlay kind="out" />
      </div>
    {/if}
    <p class="viewers">
      {#if viewers.length}Watching: {viewers.map((u) => displayName(app.state, serverId, u)).join(', ')}{:else}Nobody is watching yet{/if}
    </p>
    {#if live.encoderWarning}<p class="warn" role="status">Your GPU encoder isn't being used, so this will cost game FPS</p>{/if}
    <div class="row">
      {#if live.audio}
        <button type="button" class="ghost" aria-pressed={live.audioMuted} onclick={() => streamApi.setAudioMuted(!live.audioMuted)}>
          {live.audioMuted ? 'Unmute stream audio' : 'Mute stream audio'}
        </button>
      {/if}
      <span class="spacer"></span>
      <button type="button" class="stop" onclick={() => streamApi.stop()}>Stop</button>
    </div>
  </section>
{/if}

<style>
  .card { margin: 0 20px 8px; padding: 12px; border-radius: 14px; background: var(--bg-1); border: 1px solid var(--bg-3); display: flex; flex-direction: column; gap: 8px; max-width: 420px; align-self: center; width: calc(100% - 40px); }
  .row { display: flex; align-items: center; gap: 10px; }
  .spacer { flex: 1; }
  .dot { width: 10px; height: 10px; border-radius: 50%; background: var(--danger); }
  .meta { color: var(--text-3); font-size: 12px; }
  .preview { position: relative; }
  video { display: block; width: 100%; aspect-ratio: 16 / 9; background: #000; border-radius: 10px; object-fit: contain; }
  .viewers { margin: 0; font-size: 13px; color: var(--text-2); }
  .warn { margin: 0; font-size: 13px; color: var(--warn); }
  .ghost { height: 30px; padding: 0 12px; border-radius: 8px; border: 1px solid var(--bg-4); background: transparent; color: var(--text-2); font-size: 12px; }
  .ghost:hover:not(:disabled) { background: var(--bg-3); color: var(--text); }
  .stop { height: 30px; padding: 0 16px; border: 0; border-radius: 8px; background: var(--danger); color: var(--on-danger); font-weight: 600; }
  /* the tile variant: the preview fills the tile; the rows float over it as top and bottom bars */
  .card.tile { position: absolute; inset: 0; margin: 0; padding: 0; max-width: none; width: auto; gap: 0; border: 0; border-radius: 0; background: #000; align-self: stretch; }
  .tile .preview { position: absolute; inset: 0; }
  .tile video { height: 100%; border-radius: 0; }
  .tile .row, .tile .viewers, .tile .warn { position: relative; z-index: 1; padding: 6px 10px; background: rgba(0, 0, 0, .55); color: #fff; }
  .tile .row:first-child { background: linear-gradient(rgba(0, 0, 0, .7), transparent); }
  .tile .viewers { margin-top: auto; font-size: 12px; padding-bottom: 0; background: rgba(0, 0, 0, .55); }
  .tile .warn { font-size: 12px; }
  .tile .row:last-child { background: rgba(0, 0, 0, .55); }
  .tile .ghost { color: #fff; border-color: rgba(255, 255, 255, .4); height: 26px; }
  .tile .meta { color: rgba(255, 255, 255, .8); }
</style>
