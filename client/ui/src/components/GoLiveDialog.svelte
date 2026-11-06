<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import Icon from './Icon.svelte'
  import { modal } from '../lib/actions'
  import { StreamRoom, type Picked } from '../lib/stream/room'
  import { resolutionsFor, type Fps } from '../lib/stream/encodings'
  import { clampGoLive, loadGoLive, saveGoLive } from '../lib/stream/prefs'
  import { errorText, streamCmd } from '../lib/tauri'
  import { stream, streamApi } from '../lib/stream/stream.svelte'
  import { CHOICE_LABEL, defaultChoice, soundChoices, UNIDENTIFIED_TIP, type Sound, type SoundChoice, type SoundProbe } from '../lib/stream/sound'

  let { channelId, picked = $bindable(), onClose }: { channelId: string; picked: Picked; onClose: () => void } = $props()

  const FPS: Fps[] = [15, 30, 60]
  /** How often the live "is it playing" hint is asked for while choosing. */
  const PROBE_EVERY_MS = 1500
  const initial = clampGoLive(loadGoLive(), picked.height)
  let res = $state(initial.res)
  let fps = $state(initial.fps)
  let allowGhosts = $state(initial.allowGhosts)
  /** The source last chosen by hand for a window, remembered for the next share. */
  let preferred: Sound = initial.sound
  let sound = $state<SoundChoice>(defaultChoice(initial, picked.surface, null))
  let probe = $state<SoundProbe | null>(null)
  const choices = $derived(soundChoices(picked.surface))
  // greyed out only when the app can't be told apart; never because it is silent right now
  const appUnknown = $derived(probe?.identified === false)

  $effect(() => {
    if (picked.surface !== 'window') return
    const window = { hwnd: picked.hwnd ?? null, title: picked.label }
    probe = null
    let gone = false
    const ask = async () => {
      const p = await streamCmd.audioProbe(window).catch(() => null)
      if (gone || !p) return
      probe = p
      if (!p.identified && sound === 'app') sound = 'everything'
    }
    void ask()
    const t = setInterval(ask, PROBE_EVERY_MS)
    return () => {
      gone = true
      clearInterval(t)
    }
  })

  function choose(c: SoundChoice) {
    if (c !== 'off' && picked.surface === 'window') preferred = c
  }
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
      const c = clampGoLive({ res, fps, audio: sound !== 'off', allowGhosts, sound: preferred }, next.height)
      res = c.res
      fps = c.fps
      sound = defaultChoice(c, next.surface, null)
      pickError = ''
    } catch (e) {
      // cancelled: keep the current source
      if ((e as Error)?.name !== 'NotAllowedError') pickError = errorText(e)
    }
  }

  async function goLive() {
    busy = true
    const settings = { res, fps, audio: sound !== 'off', allowGhosts, sound: sound === 'off' ? preferred : sound }
    // a screen share's Everything is not a choice against This app next time
    saveGoLive({ ...settings, sound: preferred })
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

<div class="overlay modal-overlay fade-in">
  <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={cancel}></div>
  <div class="modal pop-in" role="dialog" aria-modal="true" aria-label="Go Live" tabindex="-1" use:modal={cancel}>
    <div class="top">
      <h1 class="modal-title">Go Live</h1>
      <button type="button" class="icon-btn round modal-close" aria-label="Cancel" disabled={busy} onclick={cancel}><Icon name="x" size={16} /></button>
    </div>
    <video bind:this={video} muted autoplay playsinline></video>
    <div class="source">
      <span class="label" data-tip={picked.label}>{picked.label}</span>
      <button type="button" class="btn btn-secondary" disabled={busy} onclick={changeSource}>Change source</button>
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
    <fieldset disabled={busy}>
      <legend>Sound</legend>
      {#each choices as c}
        {#if c === 'app'}
          <label class:unknown={appUnknown} data-tip={appUnknown ? UNIDENTIFIED_TIP : undefined}>
            <input type="radio" name="sound" value={c} bind:group={sound} disabled={appUnknown} onchange={() => choose(c)} /> {CHOICE_LABEL[c]}
            {#if probe?.identified}<span class="hint">{probe.playing ? 'Playing sound' : 'Not playing sound right now'}</span>{/if}
          </label>
        {:else}
          <label><input type="radio" name="sound" value={c} bind:group={sound} onchange={() => choose(c)} /> {CHOICE_LABEL[c]}</label>
        {/if}
      {/each}
    </fieldset>
    <label class="check" data-tip="People in the server who aren't in this call can watch without showing up; you see how many (👻)"><input type="checkbox" bind:checked={allowGhosts} disabled={busy} /> Allow ghost viewers</label>

    {#if pickError || stream.error}<p class="err" role="alert">{pickError || stream.error}</p>{/if}
    <div class="actions">
      <button type="button" class="btn btn-lg btn-secondary" disabled={busy} onclick={cancel}>Cancel</button>
      <button type="button" class="btn btn-lg btn-primary" disabled={busy} onclick={goLive}>{busy ? 'Starting…' : stream.error ? 'Retry' : 'Go Live'}</button>
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 30; }
  .modal { width: min(480px, 100%); }
  .top { display: flex; align-items: center; justify-content: space-between; }
  video { width: 100%; aspect-ratio: 16 / 9; background: #000; border-radius: var(--radius-md); object-fit: contain; }
  .source { display: flex; align-items: center; gap: 10px; }
  .label { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-2); font-size: 13px; }
  fieldset { margin: 0; padding: 0; border: 0; display: flex; flex-wrap: wrap; align-items: center; gap: 6px 16px; }
  legend { float: left; width: 96px; padding: 0; color: var(--text-2); font-size: 12px; font-weight: 600; }
  label { display: flex; align-items: center; gap: 6px; font-size: 14px; }
  input { accent-color: var(--accent); }
  .unknown { opacity: .5; cursor: not-allowed; }
  .hint { color: var(--text-3); font-size: 12px; }
  .err { margin: 0; font-size: 13px; color: var(--danger-text); }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
</style>
