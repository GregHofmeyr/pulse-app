<script lang="ts">
  // The editor's timeline: the filmstrip stretched over the whole length, time ticks, the pieces (dropped ones hatched
  // red, the selected one outlined), draggable inner edges and the playhead. Times are source milliseconds.
  import { convertFileSrc } from '@tauri-apps/api/core'
  import { formatDuration } from '../lib/clips'
  import type { Cut } from '../lib/clipCut'

  let {
    cut,
    duration,
    time,
    filmstrip,
    onSeek,
    onSelect,
    onDragEdge,
    onDragEnd,
    frameMs = 1000 / 30,
  }: {
    cut: Cut
    duration: number
    time: number
    filmstrip: { path: string; tiles: number } | null
    onSeek: (ms: number) => void
    onSelect: (index: number) => void
    onDragEdge: (edge: number, ms: number) => void
    onDragEnd: () => void
    /** One frame, the arrow-key nudge of an edge (Shift: one second). */
    frameMs?: number
  } = $props()

  const MAX_TICKS = 150
  let track: HTMLElement | undefined = $state()

  const pct = (ms: number) => (duration > 0 ? (ms / duration) * 100 : 0)

  /** Every 5 s up to two minutes, every 10 s beyond; doubled again for very long files so they stay readable. */
  const ticks = $derived.by(() => {
    let step = duration <= 120_000 ? 5000 : 10_000
    while (duration / step > MAX_TICKS) step *= 2
    const out: number[] = []
    for (let t = step; t < duration; t += step) out.push(t)
    return out
  })

  /** The source time under a pointer on the track. */
  function timeAt(clientX: number): number | null {
    const r = track?.getBoundingClientRect()
    if (!r || r.width <= 0) return null
    return Math.min(Math.max(((clientX - r.left) / r.width) * duration, 0), duration)
  }

  function click(e: MouseEvent, i: number) {
    const p = cut.pieces[i]
    onSelect(i)
    onSeek(timeAt(e.clientX) ?? p.start)
  }

  let dragging: number | null = null
  function down(e: PointerEvent, edge: number) {
    e.preventDefault()
    e.stopPropagation()
    dragging = edge
    ;(e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId)
  }
  function move(e: PointerEvent) {
    if (dragging === null) return
    const t = timeAt(e.clientX)
    if (t !== null) onDragEdge(dragging, Math.round(t))
  }
  function nudge(e: KeyboardEvent, edge: number) {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return
    e.preventDefault()
    e.stopPropagation()
    const by = (e.shiftKey ? 1000 : frameMs) * (e.key === 'ArrowLeft' ? -1 : 1)
    onDragEdge(edge, Math.round(cut.pieces[edge].start + by))
    onDragEnd() // each nudge is its own undo step
  }
  function up(e: PointerEvent) {
    if (dragging === null) return
    ;(e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId)
    dragging = null
    onDragEnd()
  }
</script>

<div class="timeline">
  <div class="ticks" aria-hidden="true">
    {#each ticks as t (t)}
      <span style:left="{pct(t)}%">{formatDuration(t)}</span>
    {/each}
  </div>
  <div
    class="track"
    data-track
    bind:this={track}
    style:background-image={filmstrip ? `url("${convertFileSrc(filmstrip.path)}")` : undefined}
  >
    {#each cut.pieces as p, i (i)}
      <button
        type="button"
        class="piece"
        class:dropped={p.dropped}
        class:selected={cut.selected === i}
        data-piece={i}
        data-end={p.end}
        data-dropped={p.dropped ? 'true' : undefined}
        style:left="{pct(p.start)}%"
        style:width="{pct(p.end - p.start)}%"
        aria-label="{p.dropped ? 'Dropped piece' : 'Piece'} {formatDuration(p.start)} to {formatDuration(p.end)}"
        aria-pressed={cut.selected === i}
        onclick={(e) => click(e, i)}
      ></button>
    {/each}
    {#each cut.pieces as p, i (i)}
      {#if i > 0}
        <div
          class="edge"
          role="slider"
          tabindex="0"
          aria-orientation="horizontal"
          aria-label="Start of piece {i + 1}"
          aria-valuemin={cut.pieces[i - 1].start}
          aria-valuemax={p.end}
          aria-valuenow={p.start}
          aria-valuetext={formatDuration(p.start)}
          onkeydown={(e) => nudge(e, i)}
          data-edge={i}
          style:left="{pct(p.start)}%"
          onpointerdown={(e) => down(e, i)}
          onpointermove={move}
          onpointerup={up}
          onpointercancel={up}
        ></div>
      {/if}
    {/each}
    <div class="head" style:left="{pct(time)}%"></div>
  </div>
</div>

<style>
  .timeline { padding: 0 8px; }
  .ticks { position: relative; height: 16px; font-size: 10px; color: var(--text-3); font-variant-numeric: tabular-nums; }
  .ticks span { position: absolute; top: 0; padding-top: 3px; border-left: 1px solid var(--line); padding-left: 3px; }
  .track { position: relative; height: 72px; border-radius: 6px; background-color: var(--bg-0); background-size: 100% 100%; background-repeat: no-repeat; overflow: hidden; touch-action: none; }
  .piece { position: absolute; top: 0; bottom: 0; padding: 0; border: 0; border-right: 1px solid rgba(0, 0, 0, 0.5); background: transparent; cursor: pointer; }
  .piece.dropped { background: repeating-linear-gradient(45deg, rgba(217, 58, 66, 0.55) 0 6px, rgba(0, 0, 0, 0.6) 6px 12px); }
  .piece.selected { outline: 2px solid var(--accent); outline-offset: -2px; z-index: 1; }
  .edge { position: absolute; top: 0; bottom: 0; width: 12px; margin-left: -6px; z-index: 2; cursor: ew-resize; }
  .edge::after { content: ''; position: absolute; left: 4px; top: 0; bottom: 0; width: 4px; border-radius: 2px; background: var(--text); opacity: 0.8; }
  .head { position: absolute; top: 0; bottom: 0; width: 2px; margin-left: -1px; background: var(--danger); pointer-events: none; z-index: 3; }
</style>
