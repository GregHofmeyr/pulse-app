<script lang="ts">
  // Add a sound (pick a file, trim it to 6 s or less on the waveform, name it, preview, save) or edit
  // one (name, emoji and boost only: the audio is fixed once uploaded). The server has the last word
  // on the name and emoji, and what it says is shown inline.
  import { onMount, untrack } from 'svelte'
  import { modal } from '../lib/actions'
  import type { Sound } from '../lib/protocol/Sound'
  import { previewPercent } from '../lib/soundboard'
  import { soundboard } from '../lib/soundboard.svelte'
  import { MAX_SPAN_MS, STEP_MS, initialSpan, moveEnd, moveStart, msAt, spanLabel, type Span } from '../lib/soundTrim'
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
  let emoji = $state(editing?.emoji ?? '')
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
      path = p
      clip = c
      span = initialSpan(c.duration_ms)
    } catch (e) {
      error = errorText(e)
    }
  }

  async function preview() {
    if (!path || busy) return
    error = ''
    try {
      await soundboardCmd.preview(path, span.start, span.end, boost, previewPercent(soundboard.volume_pct), loadAudioConfig().output ?? null)
    } catch (e) {
      error = errorText(e)
    }
  }

  async function save() {
    if (busy || !ready) return
    busy = true
    error = ''
    try {
      const e = emoji.trim()
      if (editing) {
        await soundboardCmd.update(editing.id, { name: trimmed, emoji: e || null, volume: boost })
      } else if (mode.kind === 'add' && path) {
        const media = await soundboardCmd.upload(path, span.start, span.end)
        await soundboardCmd.create(mode.serverId, trimmed, e || null, media.hash, boost)
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

  // Waveform: 200 bars in a 200x60 box; the handles are positioned in the same units.
  const W = 200
  const H = 60
  const x = (ms: number) => (clip ? (ms / clip.duration_ms) * W : 0)

  let dragging: 'start' | 'end' | null = null
  function msFromPointer(e: PointerEvent) {
    const r = svg!.getBoundingClientRect()
    return msAt(r.width ? (e.clientX - r.left) / r.width : 0, clip!.duration_ms)
  }
  function down(which: 'start' | 'end', e: PointerEvent) {
    dragging = which
    ;(e.currentTarget as Element).setPointerCapture?.(e.pointerId)
  }
  function move(which: 'start' | 'end', e: PointerEvent) {
    if (dragging !== which || !clip) return
    const ms = msFromPointer(e)
    span = which === 'start' ? moveStart(span, ms, clip.duration_ms) : moveEnd(span, ms, clip.duration_ms)
  }
  const up = () => (dragging = null)
  function keys(which: 'start' | 'end', e: KeyboardEvent) {
    if (!clip || (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight')) return
    e.preventDefault()
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
            <p class="hint">Length {spanLabel(span)} (up to {MAX_SPAN_MS / 1000} s). Drag the handles, or use the arrow keys.</p>
          {/if}
        {/if}
        <input class="input" aria-label="Name" placeholder="Name" bind:value={name} disabled={busy} />
        {#if nameProblem}<p class="err" role="alert">{nameProblem}</p>{/if}
        <input class="input" aria-label="Emoji" placeholder="Emoji (optional)" bind:value={emoji} disabled={busy} />
        <label class="boost">
          <span class="settings-label">Boost {boost}%</span>
          <input type="range" min="10" max="200" step="5" aria-label="Boost" bind:value={boost} disabled={busy} />
        </label>
        {#if error}<p class="err" role="alert">{error}</p>{/if}
        <div class="actions">
          {#if clip}<button type="button" class="btn btn-lg btn-secondary prev" disabled={busy} onclick={preview}>Preview</button>{/if}
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
  .handle:focus-visible .grip { stroke: var(--text); stroke-width: 1; vector-effect: non-scaling-stroke; }
  .boost { display: flex; flex-direction: column; gap: 4px; }
  input[type='range'] { accent-color: var(--accent); }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
  .prev { margin-right: auto; }
</style>
