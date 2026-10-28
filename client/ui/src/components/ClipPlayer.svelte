<script lang="ts">
  // One clip: a lazy thumbnail until played, then the only mounted <video>. The link is asked for on
  // every play and never stored (it expires in an hour).
  import { onMount } from 'svelte'
  import type { Message } from '../lib/protocol/Message'
  import { api } from '../lib/tauri'
  import { mediaUrl } from '../lib/emoji'
  import { formatDuration } from '../lib/clips'
  import { claim, playback, release, setMuted, setVolume } from '../lib/clipPlayback.svelte'
  import Icon from './Icon.svelte'

  let { message, autoplay = false, compact = false }: { message: Message; autoplay?: boolean; compact?: boolean } = $props()

  const SKIP_S = 5
  const RENEW_AFTER_MS = 50 * 60_000
  const NETWORK_NO_SOURCE = 3

  let box: HTMLElement | undefined = $state()
  let el: HTMLVideoElement | undefined = $state()
  let src = $state<string | null>(null)
  let playing = $state(false)
  let cur = $state(0) // seconds
  let dur = $state<number | null>(null)
  let failed = $state(false)
  let fullscreen = $state(false)
  let fetchedAt = 0
  let renewed = false
  let resumeAt: number | null = null
  let gen = 0 // a newer play supersedes a slower earlier one

  const total = $derived(dur ?? (message.clip?.duration_ms ?? 0) / 1000)

  function stop() {
    gen++
    el?.pause()
    src = null
    playing = false
  }

  /** Asks for a link and points the video at it; plays from `from` seconds. */
  async function load(from: number, isRenewal: boolean) {
    const mine = ++gen
    failed = false
    try {
      const { url } = await api.clipUrl(message.id)
      if (mine !== gen) return
      fetchedAt = Date.now()
      renewed = isRenewal
      resumeAt = from
      claim(message.id, stop)
      src = url
    } catch {
      if (mine !== gen) return
      failed = true
      release(message.id)
    }
  }

  function play() {
    void load(el ? cur : 0, false)
  }

  function pause() {
    el?.pause()
  }

  const toggle = () => (playing ? pause() : play())

  function onMeta() {
    if (!el) return
    dur = Number.isFinite(el.duration) ? el.duration : dur
    if (resumeAt !== null) {
      el.currentTime = resumeAt
      resumeAt = null
    }
    void el.play()
  }

  function onError() {
    if (!el) return
    const expired = el.networkState === NETWORK_NO_SOURCE && Date.now() - fetchedAt > RENEW_AFTER_MS
    if (expired && !renewed) void load(cur, true)
    else {
      failed = true
      playing = false
    }
  }

  function seek(t: number) {
    if (!el) return
    const max = total > 0 ? total : t
    el.currentTime = cur = Math.min(max, Math.max(0, t))
  }

  function onKey(e: KeyboardEvent) {
    const onControl = e.target !== box && e.target instanceof HTMLElement && e.target.closest('button, input')
    if (onControl) return
    if (e.key === ' ') toggle()
    else if (e.key === 'ArrowRight' && el) seek(el.currentTime + SKIP_S)
    else if (e.key === 'ArrowLeft' && el) seek(el.currentTime - SKIP_S)
    else return
    e.preventDefault()
  }

  function toggleFullscreen() {
    if (document.fullscreenElement === box) void document.exitFullscreen()
    else void box?.requestFullscreen()
  }

  // Only this player's own change counts; the window follows the element.
  function onFullscreenChange() {
    const now = document.fullscreenElement === box
    if (now === fullscreen) return
    fullscreen = now
    void api.setFullscreen(now).catch(() => {})
  }

  $effect(() => {
    if (!el) return
    el.volume = playback.volume
    el.muted = playback.muted
  })

  onMount(() => {
    if (autoplay) play()
    return () => {
      release(message.id)
      if (fullscreen) void api.setFullscreen(false).catch(() => {})
    }
  })
</script>

<svelte:document onfullscreenchange={onFullscreenChange} />

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="clip" class:compact role="group" aria-label="Clip player" tabindex="0" bind:this={box} onkeydown={onKey}>
  <div class="screen" style:aspect-ratio="{message.clip?.width ?? 16} / {message.clip?.height ?? 9}">
    {#if src}
      <!-- svelte-ignore a11y_media_has_caption -->
      <video
        bind:this={el}
        {src}
        preload="metadata"
        playsinline
        onloadedmetadata={onMeta}
        ontimeupdate={() => (cur = el?.currentTime ?? 0)}
        ondurationchange={() => (dur = el && Number.isFinite(el.duration) ? el.duration : dur)}
        onplay={() => (playing = true)}
        onpause={() => (playing = false)}
        onended={() => (playing = false)}
        onerror={onError}
        onclick={toggle}
      ></video>
    {:else if message.clip}
      <img src={mediaUrl(message.clip.thumb_hash)} alt="" loading="lazy" decoding="async" />
    {/if}
    {#if failed}
      <p class="fail" role="alert">Couldn't play this clip. Try again.</p>
    {/if}
    {#if !playing && !failed}
      <button class="big" aria-label="Play" onclick={play}><Icon name="play" size={28} /></button>
    {/if}
  </div>
  <div class="bar">
    <button class="ico" aria-label={playing ? 'Pause' : 'Play'} onclick={toggle}><Icon name={playing ? 'pause' : 'play'} /></button>
    <input class="seek" type="range" aria-label="Seek" min="0" max={total || 1} step="0.1" value={cur} disabled={!el} oninput={(e) => seek(Number(e.currentTarget.value))} />
    <span class="time">{formatDuration(cur * 1000)} / {formatDuration(total * 1000)}</span>
    {#if !compact}
      <button class="ico" aria-label={playback.muted ? 'Unmute' : 'Mute'} onclick={() => setMuted(!playback.muted)}><Icon name={playback.muted || playback.volume === 0 ? 'speakerOff' : 'speaker'} /></button>
      <input class="vol" type="range" aria-label="Volume" min="0" max="1" step="0.05" value={playback.volume} oninput={(e) => { setVolume(Number(e.currentTarget.value)); if (playback.muted) setMuted(false) }} />
    {/if}
    <button class="ico" aria-label={fullscreen ? 'Exit full screen' : 'Full screen'} onclick={toggleFullscreen}><Icon name="fullscreen" /></button>
  </div>
</div>

<style>
  .clip { display: flex; flex-direction: column; background: #000; border-radius: 8px; overflow: hidden; outline-offset: 2px; max-width: 100%; }
  .clip:fullscreen { border-radius: 0; justify-content: center; }
  .screen { position: relative; background: #000; max-height: 70vh; width: 100%; }
  .clip:fullscreen .screen { max-height: none; flex: 1; aspect-ratio: auto !important; }
  video, img { width: 100%; height: 100%; object-fit: contain; display: block; }
  .big { position: absolute; inset: 0; margin: auto; width: 56px; height: 56px; border-radius: 50%; border: 0; background: rgba(0, 0, 0, 0.6); color: #fff; display: grid; place-items: center; cursor: pointer; }
  .big:hover { background: rgba(0, 0, 0, 0.8); }
  .fail { position: absolute; inset: auto 0 8px; margin: 0; text-align: center; color: #fff; font-size: 13px; }
  .bar { display: flex; align-items: center; gap: 8px; padding: 6px 8px; background: rgba(0, 0, 0, 0.85); color: #fff; }
  .ico { background: none; border: 0; color: inherit; cursor: pointer; padding: 2px; display: grid; place-items: center; }
  .seek { flex: 1; min-width: 40px; accent-color: var(--accent, #6b8afd); }
  .vol { width: 70px; accent-color: var(--accent, #6b8afd); }
  .time { font-size: 12px; font-variant-numeric: tabular-nums; white-space: nowrap; }
  .compact .bar { padding: 4px 6px; gap: 4px; }
</style>
