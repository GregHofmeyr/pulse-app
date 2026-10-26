<script lang="ts">
  import { untrack } from 'svelte'
  import StreamDiagOverlay from './StreamDiagOverlay.svelte'
  import Select from './Select.svelte'
  import { app } from '../lib/store.svelte'
  import { displayName, liveIn } from '../lib/selectors'
  import { clampVolume, volumeLabel } from '../lib/voiceui'
  import { loadStreamVolumes } from '../lib/stream/prefs'
  import { findWatch, streamApi } from '../lib/stream/stream.svelte'
  import { hostVideo } from '../lib/stream/videoHost'
  import { viewing } from '../lib/stream/viewer.svelte'

  /** The current request's watch call has finished (started, failed or been superseded). */
  let settled = $state(false)
  let volumes = $state(loadStreamVolumes())
  let quality = $state<string | null>('auto')

  const req = $derived(viewing.req)
  const watch = $derived(req ? findWatch(req.channelId, req.streamer) : undefined)
  const name = $derived(req ? displayName(app.state, app.state.channels[req.channelId]?.server_id ?? null, req.streamer) : '')
  const volume = $derived(req ? (volumes[req.streamer] ?? 100) : 100)
  const reconnecting = $derived(!!req && liveIn(app.state, req.channelId)[req.streamer]?.status === 'reconnecting')

  // Start (or switch) watching whenever the request changes; this viewer is the full view.
  $effect(() => {
    const r = req
    if (!r) return
    settled = false
    untrack(() => {
      void streamApi.watch(r.channelId, r.streamer).finally(() => {
        if (viewing.req === r) settled = true
      })
      streamApi.setView(r.channelId, r.streamer, 'full')
      quality = findWatch(r.channelId, r.streamer)?.quality ?? 'auto'
    })
  })

  // The stream is gone (ended, failed, or voice changed): close the viewer. The notice stays.
  $effect(() => {
    if (settled && !watch) viewing.req = null
  })

  function setVolume(pct: number) {
    if (!req) return
    volumes = { ...volumes, [req.streamer]: clampVolume(pct) }
    streamApi.setStreamVolume(req.streamer, pct)
  }

  function close() {
    const r = req
    viewing.req = null
    if (r) void streamApi.unwatch(r.channelId, r.streamer)
  }
</script>

<div class="viewer">
  <div class="bar">
    <strong>{name}</strong>
    <span class="spacer"></span>
    <div class="ctl">
      <span>Quality</span>
      <Select bind:value={quality} label="Quality" options={[{ value: 'auto', label: 'Auto' }, { value: 'saver', label: 'Data saver' }]}
        onchange={() => req && streamApi.setQuality(req.channelId, req.streamer, quality === 'saver' ? 'saver' : 'auto')} />
    </div>
    <label class="ctl" for="stream-vol">Volume <strong>{volumeLabel(volume)}</strong></label>
    <input id="stream-vol" type="range" min="0" max="200" step="5" value={volume} oninput={(e) => setVolume(+e.currentTarget.value)} />
    <button type="button" class="close" onclick={close}>Close</button>
  </div>
  <div class="stage">
    {#if watch}<div class="host" {@attach hostVideo(watch.channelId, watch.streamer)}></div>{/if}
    <StreamDiagOverlay kind="in" />
    {#if reconnecting}<div class="overlay" role="status">Reconnecting…</div>{/if}
  </div>
</div>

<style>
  .viewer { flex: 1; display: flex; flex-direction: column; min-height: 0; min-width: 0; background: #000; }
  .bar { flex-shrink: 0; height: 52px; padding: 0 16px; display: flex; align-items: center; gap: 14px; background: var(--bg-1); border-bottom: 1px solid var(--bg-3); }
  .spacer { flex: 1; }
  .ctl { display: flex; align-items: center; gap: 8px; font-size: 13px; color: var(--text-2); }
  input[type='range'] { width: 120px; accent-color: var(--accent); }
  .close { height: 32px; padding: 0 14px; border: 0; border-radius: 10px; background: var(--danger); color: #fff; font-weight: 700; }
  .stage { position: relative; flex: 1; min-height: 0; }
  .host { position: absolute; inset: 0; }
  .host :global(video) { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; background: #000; }
  .overlay { position: absolute; inset: 0; display: grid; place-items: center; background: rgba(0, 0, 0, .5); color: #fff; font-size: 18px; font-weight: 600; }
</style>
