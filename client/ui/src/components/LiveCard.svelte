<script lang="ts">
  import { app } from '../lib/store.svelte'
  import { liveIn, displayName } from '../lib/selectors'
  import { loadHidePreview, saveHidePreview } from '../lib/stream/prefs'
  import { liveMediaTrack, stream, streamApi } from '../lib/stream/stream.svelte'

  let { channelId, serverId }: { channelId: string; serverId: string | null } = $props()

  const live = $derived(stream.live)
  const viewers = $derived(liveIn(app.state, channelId)[app.state.me?.id ?? '']?.viewers ?? [])
  let hidden = $state(loadHidePreview())
  let video = $state<HTMLVideoElement>()

  $effect(() => {
    if (!video) return
    const t = liveMediaTrack()
    video.srcObject = t ? new MediaStream([t]) : null
    return () => {
      video!.srcObject = null
    }
  })

  const toggleHidden = () => {
    hidden = !hidden
    saveHidePreview(hidden)
  }
</script>

{#if live}
  <section class="card" aria-label="You're live">
    <div class="row">
      <span class="dot"></span>
      <strong>You're live</strong>
      <span class="meta">{live.res}p{live.fps}</span>
      <span class="spacer"></span>
      <button type="button" class="ghost" onclick={toggleHidden}>{hidden ? 'Show preview' : 'Hide preview'}</button>
    </div>
    {#if !hidden}<video bind:this={video} muted autoplay playsinline></video>{/if}
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
  video { width: 100%; aspect-ratio: 16 / 9; background: #000; border-radius: 10px; object-fit: contain; }
  .viewers { margin: 0; font-size: 13px; color: var(--text-2); }
  .warn { margin: 0; font-size: 13px; color: var(--warn); }
  .ghost { height: 30px; padding: 0 12px; border-radius: 8px; border: 1px solid var(--bg-4); background: transparent; color: var(--text-2); font-size: 12px; }
  .stop { height: 30px; padding: 0 16px; border: 0; border-radius: 8px; background: var(--danger); color: #fff; font-weight: 600; }
</style>
