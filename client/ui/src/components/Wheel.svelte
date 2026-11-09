<script lang="ts">
  // The picker wheel itself (#158), shared by the spin card, the popover's preview and Settings: neon slices
  // (lib/wheelColours.ts) with thin gaps, labels along each slice, a pointer at 12 o'clock and a hub in the middle.
  // `rotation` is the wheel's clockwise turn in degrees; whoever spins it moves that. Slice i of n covers
  // [i·360/n, (i+1)·360/n) clockwise from 12 o'clock, as the server reads its angle. Once a spin lands, `result`
  // shows the result view over it (#271): the wheel stays, under a dark, centre-weighted fade, with the winner
  // written large across the hub.
  import { fitLabel, labelFit, resultFontSize } from '../lib/wheel'
  import { labelColour, sliceColour } from '../lib/wheelColours'

  let { options, rotation = 0, size = 220, landed = null, result = null, onSpin, spinLabel = 'Spin', spinDisabled = false, bare = false }: {
    options: string[]
    rotation?: number
    /** Its width and height in px. */
    size?: number
    /** Once a spin has landed: the winner, the other slices fade back. */
    landed?: number | null
    /** The result view (#271): the winner's text over a dark fade, in place of the hub. */
    result?: string | null
    /** Makes the hub a Spin button. */
    onSpin?: () => void
    spinLabel?: string
    spinDisabled?: boolean
    /** A thumbnail (Settings' list): the slices alone, no labels, pointer or hub. */
    bare?: boolean
  } = $props()

  const R = 100
  const n = $derived(options.length)
  const fit = $derived(labelFit(n))
  // a point on the rim, `deg` clockwise from 12 o'clock
  const at = (deg: number, r = R) => {
    const a = (deg * Math.PI) / 180
    return `${(r * Math.sin(a)).toFixed(3)} ${(-r * Math.cos(a)).toFixed(3)}`
  }
  const slices = $derived(
    options.map((label, i) => {
      const seg = 360 / n
      const from = i * seg
      return {
        label: fitLabel(label, fit.chars),
        full: label,
        // one option: the whole disc
        d: n === 1 ? `M0 ${-R}A${R} ${R} 0 1 1 0 ${R}A${R} ${R} 0 1 1 0 ${-R}Z` : `M0 0L${at(from)}A${R} ${R} 0 ${seg > 180 ? 1 : 0} 1 ${at(from + seg)}Z`,
        fill: sliceColour(i, n),
        ink: labelColour(i, n),
        // the label's frame: x runs out along the slice's middle
        turn: from + seg / 2 - 90,
      }
    }),
  )
</script>

<div class="wheel" style:width="{size}px" style:height="{size}px">
  <svg viewBox="-104 -104 208 208" aria-hidden="true">
    <circle class="rim" r="103" />
    {#if n === 0}
      <circle class="empty-disc" r={R} />
      {#if !bare}<text class="empty-text" y="62" text-anchor="middle">Add some options</text>{/if}
    {:else}
      <g transform="rotate({rotation})">
        {#each slices as s, i (i)}
          <g class="slice" class:dim={landed !== null && landed !== i}>
            <path d={s.d} fill={s.fill} />
            {#if !bare}<text transform="rotate({s.turn})" x={R - 7} y="0" text-anchor="end" dominant-baseline="central" fill={s.ink}
              font-size={fit.size}>{s.label}</text>{/if}
          </g>
        {/each}
      </g>
    {/if}
  </svg>
  {#if bare}
    <!-- a thumbnail: nothing over the slices -->
  {:else if result !== null}
    <!-- the app's fade-in: none under reduced motion, while live or unfocused (app.css) -->
    <div class="result-view fade-in"><span class="result-text" style:font-size="{resultFontSize(result, size)}px">{result}</span></div>
  {:else if onSpin}
    <button type="button" class="hub" onclick={onSpin} disabled={spinDisabled}>{spinLabel}</button>
  {:else}
    <span class="hub" aria-hidden="true"></span>
  {/if}
  {#if !bare}<span class="pointer" aria-hidden="true"></span>{/if}
</div>

<style>
  .wheel { position: relative; flex-shrink: 0; }
  svg { display: block; width: 100%; height: 100%; overflow: visible; }
  /* the rim and the thin gaps between slices are the surface behind the wheel */
  .rim { fill: var(--bg-0); }
  .slice path { stroke: var(--bg-0); stroke-width: 1.2; stroke-linejoin: round; }
  .slice text { font-weight: 700; pointer-events: none; }
  .slice { transition: opacity var(--dur-slow) var(--ease); }
  .slice.dim { opacity: .35; }
  .empty-disc { fill: var(--bg-3); }
  .empty-text { fill: var(--text-3); font-size: 10px; font-weight: 600; }
  /* the pointer: a triangle at 12 o'clock, over the rim */
  .pointer { position: absolute; top: -4px; left: 50%; width: 0; height: 0; transform: translateX(-50%); border-left: 9px solid transparent; border-right: 9px solid transparent; border-top: 16px solid var(--text); filter: drop-shadow(0 1px 2px var(--scrim)); }
  .hub { position: absolute; inset: 0; margin: auto; width: 24%; height: 24%; min-width: 28px; min-height: 28px; display: grid; place-items: center; padding: 0; border: 3px solid var(--bg-0); border-radius: 50%; background: var(--bg-1); color: var(--text); font: inherit; font-size: 12px; font-weight: 700; letter-spacing: .04em; text-transform: uppercase; box-shadow: var(--shadow-2); }
  button.hub { cursor: pointer; transition: color var(--dur-fast) var(--ease); }
  button.hub:hover:not(:disabled) { color: var(--accent-text); }
  button.hub:disabled { color: var(--text-3); cursor: default; }
  button.hub:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  /* the result view (#271): over the disc (the rim is 4 of the 208 viewBox units), darkest in the middle where the
     winner is written; the video chrome's dark glass and white, which read over the neon in every theme */
  .result-view { position: absolute; inset: calc(100% * 4 / 208); border-radius: 50%; display: grid; place-items: center; padding: 0 12%; pointer-events: none;
    background: radial-gradient(closest-side, var(--video-chrome-strong) 0%, var(--video-chrome-strong) 35%, var(--video-chrome) 85%, var(--video-chrome) 100%); }
  .result-text { max-width: 100%; color: var(--on-video); font-weight: 800; line-height: 1.1; text-align: center; overflow-wrap: anywhere;
    text-shadow: 0 1px 2px var(--video-chrome-strong), 0 2px 10px var(--video-chrome-strong);
    display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 3; line-clamp: 3; overflow: hidden; }
</style>
