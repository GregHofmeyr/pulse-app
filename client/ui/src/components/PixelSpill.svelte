<script lang="ts">
  // The Quote of the Year pixel spill (lib/pixelSpill.ts), one SVG path per green, crisp edges.
  // `down`: solid along the top, spilling downwards (else solid on the left, spilling right).
  // Size and place it from the parent (`.pixel-spill`); cells stay square (`slice`).
  import { pixelSpill } from '../lib/pixelSpill'

  const SHADES = ['var(--quote-green)', 'var(--quote-green-mid)', 'var(--quote-green-deep)']

  let { seed, cols, rows, down = false }: { seed: string; cols: number; rows: number; down?: boolean } = $props()

  /** `cols` cells along the spill, `rows` across it. */
  const paths = $derived.by(() => {
    const d = ['', '', '']
    for (const p of pixelSpill(seed, cols, rows)) {
      const [x, y] = down ? [p.y, p.x] : [p.x, p.y]
      d[p.shade] += `M${x} ${y}h1v1h-1z`
    }
    return d
  })
</script>

<svg class="pixel-spill" aria-hidden="true" shape-rendering="crispEdges" viewBox={down ? `0 0 ${rows} ${cols}` : `0 0 ${cols} ${rows}`}
  preserveAspectRatio={down ? 'xMidYMin slice' : 'xMinYMid slice'}>
  {#each paths as d, shade (shade)}<path {d} style:fill={SHADES[shade]} />{/each}
</svg>

<style>
  .pixel-spill { display: block; image-rendering: pixelated; pointer-events: none; }
</style>
