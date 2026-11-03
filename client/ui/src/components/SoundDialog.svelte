<script lang="ts">
  // Add a sound (pick a file, trim it to 6 s or less on the waveform, name it, preview, save) or edit
  // one (name, emoji and boost only: the audio is fixed once uploaded). The server has the last word
  // on the name and emoji, and what it says is shown inline. The emoji comes from the chat's picker
  // (a Unicode emoji or a server's custom one); play/pause plays the source between the handles.
  import { onDestroy, onMount, untrack } from 'svelte'
  import EmojiPopover from './EmojiPopover.svelte'
  import SoundEmoji from './SoundEmoji.svelte'
  import Icon from './Icon.svelte'
  import type { Std } from '../lib/emoji'
  import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
  import { modal } from '../lib/actions'
  import type { Sound } from '../lib/protocol/Sound'
  import { previewPercents } from '../lib/soundboard'
  import { soundboard } from '../lib/soundboard.svelte'
  import { mixer } from '../lib/mixer.svelte'
  import { MAX_SPAN_MS, STEP_MS, initialSpan, moveEnd, moveSpan, moveStart, msAt, playFrom, spanLabel, type Span } from '../lib/soundTrim'
  import { errorText, soundboardCmd, type SoundPreview } from '../lib/tauri'
  import { loadAudioConfig } from '../lib/voiceui'

  type Mode = { kind: 'add'; serverId: string } | { kind: 'edit'; sound: Sound }
  let { mode, onClose }: { mode: Mode; onClose: () => void } = $props()

  const editing = untrack(() => (mode.kind === 'edit' ? mode.sound : null))
  const title = editing ? 'Edit sound' : 'Add a sound'

  let support = $state<{ can_send: boolean; reason: string | null } | null>(editing ? { can_send: true, reason: null } : null)
  let path = $state<string | null>(null)
  let clip = $state<SoundPreview | null>(null)
  let span = $state<Span>({ start: 0, end: 0 })
  let name = $state(editing?.name ?? '')
  let emoji = $state<string | null>(editing?.emoji ?? null)
  let emojiId = $state<string | null>(editing?.emoji_id ?? null)
  let picking = $state(false)
  let emojiBtn = $state<HTMLButtonElement>()
  let boost = $state(editing?.volume ?? 100)
  let busy = $state(false)
  let error = $state('')
  let svg = $state<SVGSVGElement>()

  onMount(() => {
    if (!editing) soundboardCmd.support().then((s) => (support = s), (e) => (support = { can_send: false, reason: errorText(e) }))
  })

  const chars = (s: string) => [...s].length
  const trimmed = $derived(name.trim())
  const nameProblem = $derived(trimmed && chars(trimmed) > 32 ? 'Names can be up to 32 characters.' : null)
  const ready = $derived(!!trimmed && !nameProblem && (editing !== null || (path !== null && clip !== null)))

  async function choose() {
    error = ''
    try {
      const p = await soundboardCmd.pickFile()
      if (!p) return
      const c = await soundboardCmd.open(p)
      stopPlaying()
      path = p
      clip = c
      span = initialSpan(c.duration_ms)
    } catch (e) {
      error = errorText(e)
    }
  }

  async function preview() {
    if (!path || busy) return
    stopPlaying()
    error = ''
    try {
      const [calls, level] = previewPercents(mixer.calls.master, soundboard.volume_pct)
      await soundboardCmd.preview(path, span.start, span.end, boost, calls, level, loadAudioConfig().output ?? null)
    } catch (e) {
      error = errorText(e)
    }
  }

  async function save() {
    if (busy || !ready) return
    busy = true
    error = ''
    try {
      if (editing) {
        await soundboardCmd.update(editing.id, { name: trimmed, emoji, emoji_id: emojiId, volume: boost })
      } else if (mode.kind === 'add' && path) {
        stopPlaying()
        const media = await soundboardCmd.upload(path, span.start, span.end)
        await soundboardCmd.create(mode.serverId, trimmed, emoji, emojiId, media.hash, boost)
      }
      onClose()
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
  const cancel = () => {
    if (!busy) onClose()
  }

  function pickEmoji(e: Std | CustomEmoji) {
    if ('u' in e) [emoji, emojiId] = [e.u, null]
    else [emoji, emojiId] = [null, e.id]
    picking = false
  }

  // Play/pause: the source from the playhead (or the start) to the end handle, only here. The core
  // answers once the audio is queued; the playhead runs from then on.
  let playing = $state<{ from: number; startedAt: number; len: number } | null>(null)
  let pausedAt = $state<number | null>(null)
  let now = $state(0)
  let endTimer: ReturnType<typeof setTimeout> | undefined
  let frame = 0
  const position = () => (playing ? Math.min(playing.from + (performance.now() - playing.startedAt), playing.from + playing.len) : pausedAt)
  const playhead = $derived.by(() => {
    void now
    return playing ? Math.min(playing.from + (now - playing.startedAt), playing.from + playing.len) : pausedAt
  })
  function animate() {
    now = performance.now()
    frame = requestAnimationFrame(animate)
  }
  function halt() {
    playing = null
    clearTimeout(endTimer)
    cancelAnimationFrame(frame)
  }
  async function play() {
    if (!path || !clip || busy) return
    const from = playFrom(span, pausedAt)
    error = ''
    try {
      const [calls, level] = previewPercents(mixer.calls.master, soundboard.volume_pct)
      const len = await soundboardCmd.trimPlay(path, from, span.end, calls, level, loadAudioConfig().output ?? null)
      halt()
      const startedAt = performance.now()
      playing = { from, startedAt, len }
      now = startedAt
      pausedAt = from
      endTimer = setTimeout(() => {
        halt()
        pausedAt = null // played to the end handle: next time from the start
      }, len)
      frame = requestAnimationFrame(animate)
    } catch (e) {
      error = errorText(e)
    }
  }
  function pause() {
    if (!playing) return
    pausedAt = Math.round(position() ?? span.start)
    halt()
    soundboardCmd.trimStop().catch(() => {})
  }
  /** Moving the span (or leaving) ends playback and forgets the playhead. */
  function stopPlaying() {
    if (playing) soundboardCmd.trimStop().catch(() => {})
    halt()
    pausedAt = null
  }
  onDestroy(stopPlaying)

  // Waveform: 200 bars in a 200x60 box; the handles are positioned in the same units.
  const W = 200
  const H = 60
  const x = (ms: number) => (clip ? (ms / clip.duration_ms) * W : 0)

  let dragging: 'start' | 'end' | 'span' | null = null
  /** Where a selection drag was grabbed, and the span then. */
  let grab: { ms: number; span: Span } | null = null
  function msFromPointer(e: PointerEvent) {
    const r = svg!.getBoundingClientRect()
    return msAt(r.width ? (e.clientX - r.left) / r.width : 0, clip!.duration_ms)
  }
  function down(which: 'start' | 'end' | 'span', e: PointerEvent) {
    if (!clip) return
    stopPlaying()
    dragging = which
    grab = which === 'span' ? { ms: msFromPointer(e), span } : null
    ;(e.currentTarget as Element).setPointerCapture?.(e.pointerId)
  }
  function move(which: 'start' | 'end' | 'span', e: PointerEvent) {
    if (dragging !== which || !clip) return
    const ms = msFromPointer(e)
    if (which === 'span') span = grab ? moveSpan(grab.span, ms - grab.ms, clip.duration_ms) : span
    else span = which === 'start' ? moveStart(span, ms, clip.duration_ms) : moveEnd(span, ms, clip.duration_ms)
  }
  const up = () => {
    dragging = null
    grab = null
  }
  function keys(which: 'start' | 'end', e: KeyboardEvent) {
    if (!clip || (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight')) return
    e.preventDefault()
    stopPlaying()
    const d = e.key === 'ArrowRight' ? STEP_MS : -STEP_MS
    span = which === 'start' ? moveStart(span, span.start + d, clip.duration_ms) : moveEnd(span, span.end + d, clip.duration_ms)
  }
</script>

<div class="overlay modal-overlay fade-in">
  <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={cancel}></div>
  <div class="modal pop-in" role="dialog" aria-modal="true" aria-label={title} tabindex="-1" use:modal={cancel}>
    <h1 class="modal-title">{title}</h1>
    {#if !support}
      <p class="hint">Checking…</p>
    {:else if !support.can_send}
      <p class="err" role="alert">{support.reason ?? 'Sounds can’t be added on this install.'}</p>
      <div class="actions"><button type="button" class="btn btn-lg btn-secondary" onclick={cancel}>Close</button></div>
    {:else}
      <form onsubmit={(e) => { e.preventDefault(); void save() }}>
        {#if !editing}
          <div class="file">
            <button type="button" class="btn btn-secondary" disabled={busy} onclick={choose}>{clip ? 'Choose another file' : 'Choose a file'}</button>
            {#if path}<span class="path">{path.split(/[\\/]/).pop()}</span>{/if}
          </div>
          {#if clip}
            <svg class="wave" bind:this={svg} viewBox="0 0 {W} {H}" preserveAspectRatio="none" role="group" aria-label="Waveform">
              {#each clip.peaks as p, i (i)}
                {@const h = Math.max(2, p * (H - 6))}
                {@const inSpan = x(span.start) <= i + 0.5 && i + 0.5 <= x(span.end)}
                <rect class="bar" class:on={inSpan} x={i} y={(H - h) / 2} width="0.7" height={h} />
              {/each}
              <!-- the selection itself: drag it to move the whole span (keyboard users move the handles) -->
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <rect class="sel" x={x(span.start)} y="0" width={Math.max(0, x(span.end) - x(span.start))} height={H}
                onpointerdown={(e) => down('span', e)} onpointermove={(e) => move('span', e)} onpointerup={up} onpointercancel={up} />
              {#if playhead !== null}
                <line class="playhead" data-testid="playhead" x1={x(playhead)} x2={x(playhead)} y1="0" y2={H} />
              {/if}
              {#each ['start', 'end'] as const as which (which)}
                <g
                  class="handle" role="slider" tabindex="0" aria-label={which === 'start' ? 'Start' : 'End'}
                  aria-orientation="horizontal" aria-valuemin="0" aria-valuemax={clip.duration_ms} aria-valuenow={span[which]}
                  aria-valuetext="{(span[which] / 1000).toFixed(1)} s"
                  onpointerdown={(e) => down(which, e)} onpointermove={(e) => move(which, e)} onpointerup={up} onpointercancel={up}
                  onkeydown={(e) => keys(which, e)}
                >
                  <line x1={x(span[which])} x2={x(span[which])} y1="0" y2={H} />
                  <rect class="grip" x={x(span[which]) - 3} y={H / 2 - 8} width="6" height="16" rx="2" />
                </g>
              {/each}
            </svg>
            <p class="hint">Length {spanLabel(span)} (up to {MAX_SPAN_MS / 1000} s). Drag the handles or the selection, or use the arrow keys.</p>
          {/if}
        {/if}
        <input class="input" aria-label="Name" placeholder="Name" bind:value={name} disabled={busy} />
        {#if nameProblem}<p class="err" role="alert">{nameProblem}</p>{/if}
        <div class="emoji-row">
          <button type="button" class="btn btn-secondary emoji-btn" aria-label="Choose emoji" title="Choose emoji" data-menu-toggle
            bind:this={emojiBtn} disabled={busy} onclick={() => (picking = !picking)}>
            <SoundEmoji {emoji} {emojiId} size={20} />
          </button>
          <span class="hint">{emoji || emojiId ? 'Emoji' : 'Emoji (optional)'}</span>
          {#if emoji || emojiId}
            <button type="button" class="link-btn" disabled={busy} onclick={() => ([emoji, emojiId] = [null, null])}>Remove emoji</button>
          {/if}
        </div>
        {#if picking && emojiBtn}
          <EmojiPopover anchor={emojiBtn} onPick={pickEmoji} onClose={() => (picking = false)} />
        {/if}
        <label class="boost">
          <span class="settings-label">Boost {boost}%</span>
          <input type="range" min="10" max="200" step="5" aria-label="Boost" bind:value={boost} disabled={busy} />
        </label>
        {#if error}<p class="err" role="alert">{error}</p>{/if}
        <div class="actions">
          {#if clip}
            <div class="prev">
              {#if playing}
                <button type="button" class="btn btn-lg btn-secondary icon" aria-label="Pause" title="Pause" onclick={pause}><Icon name="pause" size={16} /></button>
              {:else}
                <button type="button" class="btn btn-lg btn-secondary icon" aria-label="Play selection" title="Play selection" disabled={busy} onclick={play}><Icon name="play" size={16} /></button>
              {/if}
              <button type="button" class="btn btn-lg btn-secondary" disabled={busy} onclick={preview}>Preview</button>
            </div>
          {/if}
          <button type="button" class="btn btn-lg btn-secondary" disabled={busy} onclick={cancel}>Cancel</button>
          <button type="submit" class="btn btn-lg btn-primary" disabled={busy || !ready}>{busy ? 'Saving…' : 'Save'}</button>
        </div>
      </form>
    {/if}
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 30; }
  .modal { width: min(460px, 100%); }
  form { display: flex; flex-direction: column; gap: 12px; }
  p { margin: 0; font-size: 14px; color: var(--text-2); overflow-wrap: anywhere; }
  .hint { font-size: 13px; color: var(--text-3); }
  .err { font-size: 13px; color: var(--danger-text); }
  .file { display: flex; align-items: center; gap: 10px; min-width: 0; }
  .path { font-size: 13px; color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .wave { width: 100%; height: 72px; background: var(--bg-inset); border-radius: 10px; touch-action: none; }
  .bar { fill: var(--text-3); }
  .bar.on { fill: var(--accent); }
  .handle { cursor: ew-resize; outline: none; }
  .handle line { stroke: var(--accent-text); stroke-width: 1; vector-effect: non-scaling-stroke; }
  .grip { fill: var(--accent); }
  .sel { fill: var(--accent); fill-opacity: 0.12; cursor: grab; }
  .playhead { stroke: var(--text); stroke-width: 1.5; vector-effect: non-scaling-stroke; pointer-events: none; }
  .emoji-row { display: flex; align-items: center; gap: 10px; }
  .emoji-btn { width: 44px; height: 36px; padding: 0; display: grid; place-items: center; }
  .prev { display: flex; gap: 8px; }
  .icon { width: 44px; padding: 0; display: grid; place-items: center; }
  .handle:focus-visible .grip { stroke: var(--text); stroke-width: 1; vector-effect: non-scaling-stroke; }
  .boost { display: flex; flex-direction: column; gap: 4px; }
  input[type='range'] { accent-color: var(--accent); }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
  .prev { margin-right: auto; }
</style>
