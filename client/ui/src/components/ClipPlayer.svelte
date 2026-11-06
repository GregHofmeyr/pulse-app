<script lang="ts">
  // One clip: a lazy thumbnail until played, then the only mounted <video>. The link is asked for on
  // every play and never stored (it expires in an hour).
  import { onMount } from 'svelte'
  import type { Message } from '../lib/protocol/Message'
  import { api } from '../lib/tauri'
  import { mediaUrl } from '../lib/emoji'
  import { formatDuration } from '../lib/clips'
  import { claim, elementVolume, playback, release, setMuted, setVolume } from '../lib/clipPlayback.svelte'
  import ClipDownload from './ClipDownload.svelte'
  import Icon from './Icon.svelte'

  let { message, autoplay = false, compact = false, onDownload }: {
    message: Message
    autoplay?: boolean
    compact?: boolean
    /** Shows the download icon at the top right of the video (#204). */
    onDownload?: () => void
  } = $props()

  const SKIP_S = 5
  const VOL_STEP = 5 // percent, for the arrow keys on the compact player's speaker (#208)

  let box: HTMLElement | undefined = $state()
  let el: HTMLVideoElement | undefined = $state()
  let src = $state<string | null>(null)
  let playing = $state(false)
  let loading = $state(false)
  let cur = $state(0) // seconds
  let dur = $state<number | null>(null)
  let failed = $state(false)
  let fullscreen = $state(false)
  let renewed = false // the one renewal is spent until the video plays again
  let resumeAt: number | null = null
  let startPlaying = true
  let gen = 0 // a newer play (or a pause, stop or unmount) supersedes a slower fetch
  let token: symbol | null = null
  let windowWas: Promise<boolean> = Promise.resolve(false)

  const total = $derived(dur ?? (message.clip?.duration_ms ?? 0) / 1000)

  function stop() {
    gen++
    loading = false
    el?.pause()
    src = null
    playing = false
  }

  /** Asks for a link and points the video at it; resumes at `from` seconds, playing or not. */
  async function load(from: number, isRenewal: boolean, andPlay: boolean) {
    const mine = ++gen
    failed = false
    loading = true
    try {
      const { url } = await api.clipUrl(message.id)
      if (mine !== gen) return
      loading = false
      renewed = isRenewal
      resumeAt = from
      startPlaying = andPlay
      token = claim(message.id, stop, token ?? undefined)
      src = url
    } catch {
      if (mine !== gen) return
      loading = false
      failed = true
      if (token) release(token)
    }
  }

  function play() {
    void load(total > 0 && cur >= total - 0.5 ? 0 : cur, false, true)
  }

  function pause() {
    if (loading) {
      gen++ // Pause cancels a Play still waiting for its link
      loading = false
    }
    el?.pause()
  }

  const toggle = () => (playing || loading ? pause() : play())

  function onMeta() {
    if (!el) return
    dur = Number.isFinite(el.duration) ? el.duration : dur
    if (resumeAt !== null) {
      el.currentTime = resumeAt
      resumeAt = null
    }
    if (startPlaying) el.play()?.catch(() => {})
  }

  // Any media error on a loaded clip may be a link that expired: ask once more, same spot, same
  // play/pause state. A renewed link that fails too is a real failure.
  function onError() {
    if (!el) return
    if (!renewed) void load(el.currentTime || cur, true, playing)
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

  /** The shared Clips volume; moving it while muted unmutes. */
  function changeVolume(pct: number) {
    setVolume(Math.min(100, Math.max(0, pct)))
    if (playback.muted) setMuted(false)
  }

  // The compact speaker's own keys: its pop-up slider is vertical, so up/right is louder and down/left quieter.
  function onSpeakerKey(e: KeyboardEvent) {
    const step = e.key === 'ArrowUp' || e.key === 'ArrowRight' ? VOL_STEP : e.key === 'ArrowDown' || e.key === 'ArrowLeft' ? -VOL_STEP : 0
    if (!step) return
    e.preventDefault()
    changeVolume(playback.volume + step)
  }

  function toggleFullscreen() {
    if (document.fullscreenElement === box) void document.exitFullscreen()?.catch(() => {})
    else void box?.requestFullscreen()?.catch(() => {})
  }

  // Only this player's own change counts; the window follows the element, and goes back to how it
  // was before (not blindly to windowed) on the way out.
  function onFullscreenChange() {
    const now = document.fullscreenElement === box
    if (now === fullscreen) return
    fullscreen = now
    if (now) windowWas = Promise.resolve(api.setFullscreen(true)).then((was) => was === true, () => false)
    else void windowWas.then((was) => api.setFullscreen(was)).catch(() => {})
  }

  $effect(() => {
    if (!el) return
    el.volume = elementVolume(playback.volume)
    el.muted = playback.muted
  })

  onMount(() => {
    if (autoplay) play()
    return () => {
      gen++ // a fetch still in flight must not claim for a gone player
      if (token) release(token)
      if (fullscreen) void windowWas.then((was) => api.setFullscreen(was)).catch(() => {})
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
        oncanplay={() => (renewed = false)}
        onplaying={() => (renewed = false)}
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
    {#if !playing && !loading && !failed}
      <button class="big" aria-label="Play clip" onclick={play}><Icon name="play" size={28} /></button>
    {/if}
    {#if onDownload}<ClipDownload onclick={onDownload} />{/if}
  </div>
  <div class="bar">
    <button class="ico" aria-label={playing || loading ? 'Pause' : 'Play'} onclick={toggle}><Icon name={playing || loading ? 'pause' : 'play'} /></button>
    <input class="seek" type="range" aria-label="Seek" min="0" max={total || 1} step="0.1" value={cur} disabled={!el} oninput={(e) => seek(Number(e.currentTarget.value))} />
    <span class="time">{formatDuration(cur * 1000)} / {formatDuration(total * 1000)}</span>
    {#if compact}
      <!-- #208: the wall card has no room for the slider, so it pops up above the speaker on hover or focus -->
      <div class="vol-wrap">
        <button class="ico" aria-label={playback.muted ? 'Unmute' : 'Mute'} onclick={() => setMuted(!playback.muted)} onkeydown={onSpeakerKey}><Icon name={playback.muted || playback.volume === 0 ? 'speakerOff' : 'speaker'} /></button>
        <div class="vol-pop">
          <input type="range" aria-label="Volume" aria-orientation="vertical" aria-valuetext="{playback.volume}%" min="0" max="100" step="1" value={playback.volume} oninput={(e) => changeVolume(Number(e.currentTarget.value))} />
        </div>
      </div>
    {:else}
      <button class="ico" aria-label={playback.muted ? 'Unmute' : 'Mute'} onclick={() => setMuted(!playback.muted)}><Icon name={playback.muted || playback.volume === 0 ? 'speakerOff' : 'speaker'} /></button>
      <input class="vol" type="range" aria-label="Volume" aria-valuetext="{playback.volume}%" min="0" max="100" step="1" value={playback.volume} oninput={(e) => changeVolume(Number(e.currentTarget.value))} />
    {/if}
    <button class="ico" aria-label={fullscreen ? 'Exit full screen' : 'Full screen'} onclick={toggleFullscreen}><Icon name="fullscreen" /></button>
  </div>
</div>

<style>
  .clip { display: flex; flex-direction: column; background: #000; border-radius: var(--radius-sm); overflow: hidden; outline-offset: 2px; max-width: 100%; }
  .clip:fullscreen { border-radius: 0; justify-content: center; }
  .screen { position: relative; background: #000; max-height: 70vh; width: 100%; }
  /* min-height 0 lets the screen shrink below the clip's aspect height, so the bar stays on screen; the
     aspect ratio is left alone: WebKit doesn't recompute it once an override is lifted */
  .clip:fullscreen .screen { max-height: none; flex: 1; min-height: 0; }
  video, img { width: 100%; height: 100%; object-fit: contain; display: block; }
  .big { position: absolute; inset: 0; margin: auto; width: 56px; height: 56px; border-radius: 50%; border: 0; background: rgba(0, 0, 0, 0.6); color: #fff; display: grid; place-items: center; cursor: pointer; }
  .big:hover { background: rgba(0, 0, 0, 0.8); }
  .fail { position: absolute; inset: auto 0 8px; margin: 0; text-align: center; color: #fff; font-size: 13px; }
  .bar { display: flex; align-items: center; gap: 8px; padding: 6px 8px; background: rgba(0, 0, 0, 0.85); color: #fff; }
  /* over the video bar: brightens on hover (it had no hover at all) */
  .ico { background: none; border: 0; color: inherit; cursor: pointer; padding: 2px; display: grid; place-items: center; opacity: .85; }
  .ico:hover { opacity: 1; }
  .seek { flex: 1; min-width: 40px; accent-color: var(--accent); }
  .vol { width: 70px; accent-color: var(--accent); }
  .time { font-size: 12px; font-variant-numeric: tabular-nums; white-space: nowrap; }
  /* the download icon shows while the pointer is over the player; full screen is for watching */
  .clip:hover :global(.clip-dl) { opacity: 1; }
  .clip:fullscreen :global(.clip-dl) { display: none; }
  .compact .bar { padding: 4px 6px; gap: 4px; }
  /* the compact speaker's vertical volume (#208): the bar's own look, straight above the speaker with no gap so the
     hover carries from one to the other; short enough to stay clear of the download icon at the top right */
  .vol-wrap { position: relative; display: grid; }
  .vol-pop { position: absolute; z-index: 2; bottom: 100%; left: 50%; width: 28px; height: 96px; margin-left: -14px; border-radius: var(--radius-sm) var(--radius-sm) 0 0; background: rgba(0, 0, 0, 0.85); visibility: hidden; opacity: 0; transition: opacity var(--dur) var(--ease), visibility var(--dur) var(--ease); }
  .vol-wrap:hover .vol-pop, .vol-wrap:focus-within .vol-pop { visibility: visible; opacity: 1; }
  /* rotated rather than writing-mode, which older WebKitGTK builds don't apply to range inputs */
  .vol-pop input { position: absolute; left: 50%; top: 50%; width: 76px; margin: 0 0 0 -38px; transform: translateY(-50%) rotate(-90deg); accent-color: var(--accent); }
</style>
