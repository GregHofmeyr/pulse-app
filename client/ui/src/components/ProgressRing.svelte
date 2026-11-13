<script lang="ts">
  // How far a download is (#205), as a ring round its button's icon: the upload bar's look (accent on a quiet
  // track, the same easing), bent into a circle. `fraction` null: the size isn't known, so a short arc turns
  // (still, with reduced motion). Decorative: its button's label says the percent.
  let { fraction, size = 24 }: { fraction: number | null; size?: number } = $props()

  const R = 10
  const C = 2 * Math.PI * R
  const offset = $derived(fraction === null ? C * 0.75 : C * (1 - Math.min(1, Math.max(0, fraction))))
</script>

<svg class="ring" class:spin={fraction === null} width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
  <circle class="track" cx="12" cy="12" r={R} />
  <circle class="fill" cx="12" cy="12" r={R} stroke-dasharray={C} stroke-dashoffset={offset} />
</svg>

<style>
  .ring { position: absolute; inset: 0; margin: auto; transform: rotate(-90deg); pointer-events: none; }
  circle { fill: none; stroke-width: 2.5; }
  .track { stroke: var(--ring-track, var(--bg-3)); }
  .fill { stroke: var(--accent); stroke-linecap: round; transition: stroke-dashoffset var(--dur-slow) var(--ease); }
  .spin { animation: ring-spin 1s linear infinite; }
  @keyframes ring-spin { from { transform: rotate(-90deg); } to { transform: rotate(270deg); } }
  @media (prefers-reduced-motion: reduce) {
    .spin { animation: none; }
  }
</style>
