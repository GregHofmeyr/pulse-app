<script lang="ts">
  import { untrack } from 'svelte'
  import StreamDiagOverlay from './StreamDiagOverlay.svelte'
  import Select from './Select.svelte'
  import { app } from '../lib/store.svelte'
  import { displayName, liveIn } from '../lib/selectors'
  import { clampVolume, volumeLabel } from '../lib/voiceui'
  import { loadStreamVolumes, saveStreamVolumes } from '../lib/stream/prefs'
  import { stream, streamApi } from '../lib/stream/stream.svelte'
  import { viewing } from '../lib/stream/viewer.svelte'
  import { streamCmd } from '../lib/tauri'

  let video = $state<HTMLVideoElement>()
  /** The current request's watch call has finished (started, failed or been superseded). */
  let settled = $state(false)
  let volumes = $state(loadStreamVolumes())
  let quality = $state<string | null>('auto')

  const req = $derived(viewing.req)
  const name = $derived(req ? displayName(app.state, app.state.channels[req.channelId]?.server_id ?? null, req.streamer) : '')
  const volume = $derived(req ? (volumes[req.streamer] ?? 100) : 100)
  const reconnecting = $derived(!!req && liveIn(app.state, req.channelId)[req.streamer]?.status === 'reconnecting')

  // Start (or switch) watching whenever the request or the element changes.
  $effect(() => {
    const r = req
    const el = video
    if (!r || !el) return
    settled = false
    quality = 'auto'
    untrack(() => {
      void streamApi.watch(r.channelId, r.streamer, el).finally(() => {
        if (viewing.req === r) settled = true
      })
    })
  })

  // The stream is gone (ended, failed, or voice changed): close the viewer. The notice stays.
  $effect(() => {
    if (settled && !stream.watching) viewing.req = null
  })

  // Remembered stream volume, applied once the watch is up.
  $effect(() => {
    if (!settled || !stream.watching) return
    const { streamer } = stream.watching
    const pct = untrack(() => volumes[streamer] ?? 100)
    void streamCmd.setVolume(streamer, pct).catch(() => {})
  })

  function setVolume(pct: number) {
    if (!req) return
    const v = clampVolume(pct)
    volumes = { ...volumes, [req.streamer]: v }
    saveStreamVolumes(volumes)
    void streamCmd.setVolume(req.streamer, v).catch(() => {})
  }

  function close() {
    viewing.req = null
    void streamApi.unwatch()
  }
</script>

<div class="viewer">
  <div class="bar">
    <strong>{name}</strong>
    <span class="spacer"></span>
    <div class="ctl">
      <span>Quality</span>
      <Select bind:value={quality} label="Quality" options={[{ value: 'auto', label: 'Auto' }, { value: 'saver', label: 'Data saver' }]}
        onchange={() => streamApi.setQuality(quality === 'saver' ? 'saver' : 'auto')} />
    </div>
    <label class="ctl" for="stream-vol">Volume <strong>{volumeLabel(volume)}</strong></label>
    <input id="stream-vol" type="range" min="0" max="200" step="5" value={volume} oninput={(e) => setVolume(+e.currentTarget.value)} />
    <button type="button" class="close" onclick={close}>Close</button>
  </div>
  <div class="stage">
    <video bind:this={video} autoplay playsinline></video>
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
  video { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; background: #000; }
  .overlay { position: absolute; inset: 0; display: grid; place-items: center; background: rgba(0, 0, 0, .5); color: #fff; font-size: 18px; font-weight: 600; }
</style>
