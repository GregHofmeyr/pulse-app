<script lang="ts">
  // The clip editor: a large modal over a local video file. Cut it into pieces, drop what you don't want, pick a
  // quality, and Trim & send hands the kept spans to the core (which encodes and uploads in the background).
  // The preview plays the LOCAL file through the asset protocol (not ClipPlayer, which plays posted clips); on Linux,
  // whose webview won't play video over asset://, from the core's loopback address instead (`api.videoUrl`).
  import { onDestroy, onMount } from 'svelte'
  import { convertFileSrc } from '@tauri-apps/api/core'
  import { listen } from '@tauri-apps/api/event'
  import { modal } from '../lib/actions'
  import { cutAt, dragEdge, dropSelected, endDrag, kept, keptMs, keptTimeAt, MAX_KEPT_MS, newCut, nextPlayable, pieceCount, select, sendBlock, undo } from '../lib/clipCut'
  import { DEFAULT_PRESET, estimateBytes, PRESETS, type PresetId } from '../lib/clipPresets'
  import { formatBytes, formatDuration } from '../lib/clips'
  import { api, errorText, type ClipSource } from '../lib/tauri'
  import ClipTimeline from './ClipTimeline.svelte'
  import GamePicker from './GamePicker.svelte'
  import Icon from './Icon.svelte'

  let { source, channelId, serverId, onClose }: { source: ClipSource; channelId: string; serverId: string; onClose: () => void } = $props()

  const SKIP_MS = 5000
  const MAX_CAPTION = 300
  const RESOLUTION: Record<PresetId, string> = { light: '720p60', sharp: '1080p60' }
  const OVER_COPY = 'Clips can be up to 1:30. Drop a bit more.'
  const EMPTY_COPY = 'Nothing kept. Bring a piece back.'
  const UNREADABLE = "Pulse couldn't read that video."

  // svelte-ignore state_referenced_locally
  let cut = $state(newCut(source.duration_ms))
  let time = $state(0) // the playhead, source ms
  let playing = $state(false)
  let el: HTMLVideoElement | undefined = $state()
  let src = $state<string | null>(null)
  let preparing = $state(false)
  let percent = $state(0)
  let problem = $state('')
  let film = $state<{ path: string; tiles: number } | null>(null)
  let caption = $state('')
  // svelte-ignore state_referenced_locally
  let gameId = $state<string | null>(null)
  let preset = $state<PresetId>(DEFAULT_PRESET)
  let sending = $state(false)
  let sendError = $state('')
  let triedPrepared = false
  let destroyed = false
  let frameCb: number | null = null

  const fps = $derived(source.fps > 0 ? source.fps : 30)
  const block = $derived(sendBlock(cut))
  const total = $derived(keptMs(cut))
  const pieces = $derived(pieceCount(cut))
  const why = $derived(block === 'over' ? OVER_COPY : block === 'empty' ? EMPTY_COPY : '')
  const fpsText = $derived(`${Math.round(source.fps * 100) / 100} fps`)

  /** Where the <video> loads a local file from: the core's loopback address (Linux), else the asset protocol. */
  async function playable(path: string): Promise<string> {
    const url = await api.videoUrl(source.id, path).catch(() => null)
    return url ?? convertFileSrc(path)
  }

  /** Plays a prepared copy (remux / re-encode) instead of the original. */
  async function prepare() {
    triedPrepared = true
    preparing = true
    percent = 0
    problem = ''
    src = null
    try {
      const path = await api.preparePreview(source.id)
      const url = await playable(path)
      if (!destroyed) src = url
    } catch (e) {
      if (!destroyed) problem = errorText(e) || UNREADABLE
    } finally {
      if (!destroyed) preparing = false
    }
  }

  onMount(() => {
    if (source.preview === 'direct') void playable(source.path).then((url) => { if (!destroyed && !triedPrepared) src = url })
    else void prepare()
    api.clipFilmstrip(source.id).then((f) => { if (!destroyed) film = f ?? null }, () => {}) // the timeline works without it
    const off = listen<{ source: string; percent: number }>('clip://preview', (e) => {
      if (e.payload.source === source.id) percent = Math.round(e.payload.percent)
    })
    return () => void off.then((f) => f())
  })
  onDestroy(() => {
    destroyed = true
    stopFrames()
  })

  function onVideoError() {
    if (!triedPrepared) void prepare()
    else {
      src = null
      problem = UNREADABLE
    }
  }

  // Playback ----------------------------------------------------------------

  /** Follows the video; while playing, hops over dropped pieces and stops after the last kept one. */
  function tick() {
    if (!el) return
    const t = Math.round(el.currentTime * 1000)
    time = t
    if (el.paused) return
    const n = nextPlayable(cut, t)
    if (n === null) el.pause()
    else if (n > t) {
      el.currentTime = n / 1000
      time = n
    }
  }

  function stopFrames() {
    if (frameCb !== null && el && 'cancelVideoFrameCallback' in el) el.cancelVideoFrameCallback(frameCb)
    frameCb = null
  }

  /** One rVFC chain at a time: it follows the playing video frame by frame. */
  function watchFrames() {
    stopFrames()
    if (!el || !('requestVideoFrameCallback' in el)) return // timeupdate does the job
    frameCb = el.requestVideoFrameCallback(() => {
      frameCb = null
      tick()
      if (el && !el.paused) watchFrames()
    })
  }

  function seek(ms: number) {
    const t = Math.min(Math.max(Math.round(ms), 0), source.duration_ms)
    if (el) el.currentTime = t / 1000
    time = t
  }

  function togglePlay() {
    if (!el) return
    if (!el.paused) return el.pause()
    const t = Math.round(el.currentTime * 1000)
    // From a dropped stretch (or the end) play from the next kept start; with nothing ahead, from the first.
    const n = nextPlayable(cut, t) ?? nextPlayable(cut, 0)
    if (n === null) return
    if (n !== t) seek(n)
    void el.play()?.catch(() => {})
  }

  function step(ms: number) {
    el?.pause()
    seek((el ? el.currentTime * 1000 : time) + ms)
  }

  // Cutting -----------------------------------------------------------------

  const cutHere = () => (cut = cutAt(cut, time))
  const dropPiece = () => (cut = dropSelected(cut))
  const undoStep = () => (cut = undo(cut))

  // Leaving -----------------------------------------------------------------

  async function release() {
    try {
      await api.closeClipSource(source.id)
    } catch {
      // the core clears stale sources at the next session start
    }
  }

  async function close() {
    if (sending) return
    await release()
    onClose()
  }

  async function send() {
    if (block || sending) return
    sending = true
    sendError = ''
    try {
      await api.sendClip({ source: source.id, channelId, spans: kept(cut), preset, caption: caption.trim(), gameId })
    } catch (e) {
      sendError = errorText(e)
      sending = false
      return
    }
    await release()
    sending = false
    onClose()
  }

  function onKey(e: KeyboardEvent) {
    const t = e.target as HTMLElement | null
    // Not while typing, and not Space/Enter on a focused control (that presses it).
    if (t && (t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName))) return
    if (t?.tagName === 'BUTTON' && (e.key === ' ' || e.key === 'Enter')) return
    if (t?.getAttribute('role') === 'slider' && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) return // a timeline edge takes its own arrows
    if (e.ctrlKey || e.metaKey) {
      if (e.key.toLowerCase() === 'z' && !e.shiftKey && !e.altKey) {
        e.preventDefault()
        if (!e.repeat) undoStep() // a held key is one press
      }
      return
    }
    if (e.altKey) return
    const handled = (fn: () => void) => { e.preventDefault(); fn() }
    switch (e.key) {
      case ' ': return handled(togglePlay)
      case 'ArrowLeft': return handled(() => step(-SKIP_MS))
      case 'ArrowRight': return handled(() => step(SKIP_MS))
      case ',': return handled(() => step(-1000 / fps))
      case '.': return handled(() => step(1000 / fps))
      case 'c': case 'C': return handled(() => { if (!e.repeat) cutHere() })
      case 'Delete': return handled(() => { if (!e.repeat) dropPiece() })
    }
  }

  const sizeText = (p: PresetId) => block === 'empty' ? '—' : `~${Math.max(1, Math.round(estimateBytes(p, total) / 1e6))} MB`
</script>

<svelte:window onkeydown={onKey} />

<div class="overlay">
  <div class="backdrop" role="presentation" data-backdrop onclick={close}></div>
  <div class="modal" role="dialog" aria-modal="true" aria-label="Share a clip" tabindex="-1" use:modal={close}>
    <header>
      <div class="file">
        <strong title={source.file_name}>{source.file_name}</strong>
        <span>{formatDuration(source.duration_ms)} · {source.width}×{source.height} · {fpsText} · {formatBytes(source.size)}</span>
      </div>
      <button type="button" class="close" aria-label="Close" disabled={sending} onclick={close}><Icon name="x" /></button>
    </header>

    <div class="body">
      <section class="stage">
        <div class="screen">
          {#if src}
            <!-- svelte-ignore a11y_media_has_caption -->
            <video
              bind:this={el}
              {src}
              preload="auto"
              playsinline
              onclick={togglePlay}
              ontimeupdate={tick}
              onseeked={tick}
              onplay={() => { playing = true; watchFrames() }}
              onpause={() => { playing = false; stopFrames(); tick() }}
              onended={() => (playing = false)}
              onerror={onVideoError}
            ></video>
          {:else if preparing}
            <p class="status">Preparing preview… {percent}%</p>
          {/if}
          {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
        </div>

        <div class="controls">
          <button type="button" class="ico" aria-label={playing ? 'Pause' : 'Play'} data-tip="Play / pause (Space)" onclick={togglePlay}><Icon name={playing ? 'pause' : 'play'} /></button>
          <button type="button" class="ico" aria-label="Back 5 seconds" data-tip="Back 5 s (←)" onclick={() => step(-SKIP_MS)}><Icon name="back5" /></button>
          <button type="button" class="ico" aria-label="Forward 5 seconds" data-tip="Forward 5 s (→)" onclick={() => step(SKIP_MS)}><Icon name="fwd5" /></button>
          <button type="button" class="ico" aria-label="Previous frame" data-tip="Previous frame (,)" onclick={() => step(-1000 / fps)}><Icon name="frameBack" /></button>
          <button type="button" class="ico" aria-label="Next frame" data-tip="Next frame (.)" onclick={() => step(1000 / fps)}><Icon name="frameFwd" /></button>
          <span class="sep"></span>
          <button type="button" class="act" data-tip="Cut at the playhead (C)" onclick={cutHere}><Icon name="scissors" size={16} /> Cut here</button>
          <button type="button" class="act" data-tip="Drop the selected piece (Delete)" disabled={cut.selected === null} onclick={dropPiece}><Icon name="trash" size={16} /> Drop piece</button>
          <button type="button" class="act" data-tip="Undo (Ctrl+Z)" disabled={cut.past.length === 0} onclick={undoStep}><Icon name="undo" size={16} /> Undo</button>
          <span class="grow"></span>
          <span class="readout" class:over={block === 'over'} data-readout>
            {formatDuration(keptTimeAt(cut, time))} / {formatDuration(total)} kept · {pieces} {pieces === 1 ? 'piece' : 'pieces'}
          </span>
        </div>

        <ClipTimeline
          {cut}
          duration={source.duration_ms}
          {time}
          filmstrip={film}
          onSeek={seek}
          onSelect={(i) => (cut = select(cut, i))}
          onDragEdge={(edge, ms) => (cut = dragEdge(cut, edge, ms))}
          onDragEnd={() => (cut = endDrag(cut))}
          frameMs={1000 / fps}
        />
      </section>

      <aside class="side">
        <label class="field">
          <span>Caption</span>
          <textarea rows="3" maxlength={MAX_CAPTION} placeholder="Say something about it (optional)" bind:value={caption}></textarea>
          <small>{caption.length} / {MAX_CAPTION}</small>
        </label>

        <div class="field">
          <span>Game</span>
          <GamePicker {serverId} value={gameId} guess={source.game_guess} onChange={(id) => (gameId = id)} />
        </div>

        <fieldset class="field presets">
          <legend>Quality</legend>
          {#each Object.keys(PRESETS) as id (id)}
            {@const p = id as PresetId}
            <label class="preset" class:on={preset === p} data-preset={p}>
              <input type="radio" name="preset" value={p} bind:group={preset} />
              {PRESETS[p].label} · {RESOLUTION[p]} · {sizeText(p)}
            </label>
          {/each}
        </fieldset>

        <div class="send">
          {#if why}<p class="why" class:over={block === 'over'}>{why}</p>{/if}
          {#if sendError}<p class="why over" role="alert">{sendError}</p>{/if}
          <button type="button" class="primary" disabled={block !== null || sending} onclick={send}>{sending ? 'Starting…' : 'Trim & send'}</button>
        </div>
      </aside>
    </div>
  </div>
</div>

<style>
  /* flex-centred (not transform): WebKitGTK blurs transformed text */
  .overlay { position: fixed; inset: 0; z-index: 30; display: grid; place-items: center; padding: 16px; }
  .backdrop { position: absolute; inset: 0; background: rgba(10, 11, 13, 0.7); }
  .modal { position: relative; width: min(1180px, 100%); max-height: calc(100vh - 32px); overflow-y: auto; border-radius: 16px; background: var(--bg-2); border: 1px solid var(--line); display: flex; flex-direction: column; outline: none; }
  header { display: flex; align-items: center; gap: 12px; padding: 14px 18px; border-bottom: 1px solid var(--line); }
  .file { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .file strong { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 15px; }
  .file span { font-size: 12px; color: var(--text-3); font-variant-numeric: tabular-nums; }
  .close { width: 32px; height: 32px; border-radius: 50%; border: 0; background: var(--bg-3); color: var(--text-2); display: grid; place-items: center; cursor: pointer; }
  .close:hover { background: var(--bg-4); color: var(--text); }
  .body { display: grid; grid-template-columns: minmax(0, 1fr) 300px; gap: 18px; padding: 16px 18px 18px; }
  .stage { display: flex; flex-direction: column; gap: 10px; min-width: 0; }
  .screen { position: relative; background: #000; border-radius: 10px; aspect-ratio: 16 / 9; max-height: 52vh; width: 100%; display: grid; place-items: center; overflow: hidden; }
  video { width: 100%; height: 100%; object-fit: contain; display: block; }
  .status, .problem { margin: 0; color: var(--text-2); font-size: 14px; }
  .problem { position: absolute; inset: auto 12px 12px; text-align: center; color: var(--warn); }
  .controls { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .ico { width: 32px; height: 32px; border-radius: 8px; border: 0; background: var(--bg-3); color: var(--text); display: grid; place-items: center; cursor: pointer; }
  .act { height: 32px; padding: 0 10px; border-radius: 8px; border: 1px solid var(--bg-4); background: transparent; color: var(--text); display: inline-flex; align-items: center; gap: 6px; cursor: pointer; font: inherit; font-size: 13px; }
  .act:disabled { opacity: 0.45; cursor: default; }
  .sep { width: 1px; height: 20px; background: var(--line); margin: 0 4px; }
  .grow { flex: 1; }
  .readout { font-size: 13px; font-variant-numeric: tabular-nums; color: var(--text-2); }
  .readout.over { color: var(--danger); font-weight: 600; }
  .side { display: flex; flex-direction: column; gap: 14px; }
  .field { display: flex; flex-direction: column; gap: 6px; margin: 0; padding: 0; border: 0; min-width: 0; }
  .field > span, legend { font-size: 12px; font-weight: 600; color: var(--text-3); padding: 0; }
  textarea { resize: none; padding: 8px 10px; border-radius: 8px; border: 1px solid var(--bg-4); background: var(--bg-1); color: var(--text); font: inherit; }
  small { align-self: flex-end; color: var(--text-3); font-size: 11px; }
  .preset { display: flex; align-items: center; gap: 8px; padding: 8px 10px; border-radius: 8px; border: 1px solid var(--bg-4); font-size: 14px; cursor: pointer; font-variant-numeric: tabular-nums; }
  .preset.on { border-color: var(--accent); }
  input[type='radio'] { accent-color: var(--accent); }
  .send { margin-top: auto; display: flex; flex-direction: column; gap: 8px; }
  .why { margin: 0; font-size: 13px; color: var(--text-2); }
  .why.over { color: var(--danger); }
  .primary { height: 40px; border: 0; border-radius: 10px; background: var(--ok); color: var(--on-ok); font-weight: 700; cursor: pointer; }
  .primary:disabled { opacity: 0.5; cursor: default; }
  @media (max-width: 860px) { .body { grid-template-columns: minmax(0, 1fr); } }
</style>
