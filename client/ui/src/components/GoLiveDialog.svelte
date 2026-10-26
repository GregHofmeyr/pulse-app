<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import Icon from './Icon.svelte'
  import { StreamRoom, type Picked } from '../lib/stream/room'
  import { resolutionsFor, type Fps } from '../lib/stream/encodings'
  import { clampGoLive, loadGoLive, saveGoLive } from '../lib/stream/prefs'
  import { errorText } from '../lib/tauri'
  import { stream, streamApi } from '../lib/stream/stream.svelte'

  let { channelId, picked = $bindable(), onClose }: { channelId: string; picked: Picked; onClose: () => void } = $props()

  const FPS: Fps[] = [15, 30, 60]
  const initial = clampGoLive(loadGoLive(), picked.height)
  let res = $state(initial.res)
  let fps = $state(initial.fps)
  let audio = $state(initial.audio)
  let busy = $state(false)
  let pickError = $state('')
  let video = $state<HTMLVideoElement>()
  // The capture belongs to this dialog until Go Live hands it to the stream.
  let track = picked.track
  let handedOff = false
  let destroyed = false

  onDestroy(() => {
    destroyed = true
    if (!handedOff) track.stop()
  })

  const resolutions = $derived(resolutionsFor(picked.height))

  onMount(() => {
    stream.error = ''
  })

  $effect(() => {
    if (video) video.srcObject = new MediaStream([picked.track.mediaStreamTrack])
  })

  async function changeSource() {
    try {
      const next = await StreamRoom.pick()
      if (destroyed) return void next.track.stop()
      track.stop()
      picked = next
      track = next.track
      const c = clampGoLive({ res, fps, audio }, next.height)
      res = c.res
      fps = c.fps
      pickError = ''
    } catch (e) {
      // cancelled: keep the current source
      if ((e as Error)?.name !== 'NotAllowedError') pickError = errorText(e)
    }
  }

  async function goLive() {
    busy = true
    const settings = { res, fps, audio }
    saveGoLive(settings)
    handedOff = true
    try {
      await streamApi.goLive(channelId, track, picked.label, settings)
      onClose() // the track now belongs to the live stream
    } catch {
      // kept for Retry, unless the dialog is already gone (its onDestroy left the capture to us)
      if (destroyed) track.stop()
      else handedOff = false
    } finally {
      busy = false
    }
  }

  function cancel() {
    if (busy) return
    onClose() // onDestroy stops the capture
  }
</script>

<svelte:window onkeydown={(e) => { if (e.key === 'Escape') { e.preventDefault(); cancel() } }} />

<div class="overlay">
  <div class="backdrop" role="presentation" onclick={cancel}></div>
  <div class="modal" role="dialog" aria-label="Go Live">
    <div class="top">
      <h1>Go Live</h1>
      <button type="button" class="close" aria-label="Cancel" disabled={busy} onclick={cancel}><Icon name="x" /></button>
    </div>
    <video bind:this={video} muted autoplay playsinline></video>
    <div class="source">
      <span class="label" title={picked.label}>{picked.label}</span>
      <button type="button" class="ghost" disabled={busy} onclick={changeSource}>Change source</button>
    </div>

    <fieldset disabled={busy}>
      <legend>Resolution</legend>
      {#each resolutions as r}
        <label><input type="radio" name="res" value={r} bind:group={res} /> {r}p</label>
      {/each}
    </fieldset>
    <fieldset disabled={busy}>
      <legend>Frame rate</legend>
      {#each FPS as f}
        <label><input type="radio" name="fps" value={f} bind:group={fps} /> {f} fps</label>
      {/each}
    </fieldset>
    <label class="check"><input type="checkbox" bind:checked={audio} disabled={busy} /> Stream audio</label>

    {#if pickError || stream.error}<p class="err" role="alert">{pickError || stream.error}</p>{/if}
    <div class="actions">
      <button type="button" class="ghost" disabled={busy} onclick={cancel}>Cancel</button>
      <button type="button" class="primary" disabled={busy} onclick={goLive}>{busy ? 'Starting…' : stream.error ? 'Retry' : 'Go Live'}</button>
    </div>
  </div>
</div>

<style>
  /* flex-centred (not transform): WebKitGTK blurs transformed text */
  .overlay { position: fixed; inset: 0; z-index: 30; display: grid; place-items: center; padding: 16px; }
  .backdrop { position: absolute; inset: 0; background: rgba(10, 11, 13, .6); }
  .modal { position: relative; width: min(480px, 100%); max-height: calc(100vh - 32px); overflow-y: auto; padding: 22px; border-radius: 16px; background: var(--bg-2); border: 1px solid var(--line); display: flex; flex-direction: column; gap: 12px; }
  .top { display: flex; align-items: center; justify-content: space-between; }
  h1 { margin: 0; font-size: 18px; font-weight: 600; }
  .close { width: 32px; height: 32px; border-radius: 50%; border: 0; background: var(--bg-3); color: var(--text-2); display: grid; place-items: center; }
  video { width: 100%; aspect-ratio: 16 / 9; background: #000; border-radius: 10px; object-fit: contain; }
  .source { display: flex; align-items: center; gap: 10px; }
  .label { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-2); font-size: 13px; }
  fieldset { margin: 0; padding: 0; border: 0; display: flex; flex-wrap: wrap; align-items: center; gap: 6px 16px; }
  legend { float: left; width: 96px; padding: 0; color: var(--text-3); font-size: 12px; font-weight: 600; }
  label { display: flex; align-items: center; gap: 6px; font-size: 14px; }
  input { accent-color: var(--accent); }
  .err { margin: 0; font-size: 13px; color: var(--warn); }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
  .primary { height: 38px; padding: 0 18px; border: 0; border-radius: 10px; background: var(--ok); color: #0f2a1c; font-weight: 700; }
  .primary:disabled { opacity: .6; }
  .ghost { height: 36px; padding: 0 14px; border-radius: 10px; border: 1px solid var(--bg-4); background: transparent; color: var(--text-2); }
  .ghost:disabled { opacity: .6; }
</style>
