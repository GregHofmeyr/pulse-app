<script lang="ts">
  import { diag, diagUi } from '../lib/stream/diag.svelte'
  import { inboundTrouble, layerLine, noteLabel } from '../lib/stream/diagText'

  let { kind }: { kind: 'out' | 'in' } = $props()
  const out = $derived(diag.out)
  const inn = $derived(diag.in)
  const num = (v: number | null, unit = '') => (v == null ? '-' : `${Math.round(v)}${unit}`)
</script>

{#if diagUi.on && ((kind === 'out' && out) || (kind === 'in' && inn))}
  <div class="diag" aria-label="Stream diagnostics">
    {#if kind === 'out' && out}
      <div>capture {num(out.captureFps, 'fps')}</div>
      {#each out.layers as l (l.rid ?? `${l.w}x${l.h}`)}
        <div>{layerLine(l)}</div>
      {/each}
      {@const full = out.layers.reduce((a, b) => (b.w > a.w ? b : a), out.layers[0])}
      {#if full}
        <div>{full.encoder || '-'}{full.powerEfficient == null ? '' : full.powerEfficient ? ' (hw)' : ' (sw)'} qlr {full.qlr}</div>
      {/if}
      <div>rtt {num(out.rttMs, 'ms')}</div>
      <div class:calm={out.note === 'ok' || out.note === 'still'}>{noteLabel(out.note)}</div>
    {:else if kind === 'in' && inn}
      <div>{inn.w}×{inn.h} {Math.round(inn.fps)}fps {Math.round(inn.kbps)}kbps</div>
      <div>{inn.decoder || '-'}{inn.powerEfficient == null ? '' : inn.powerEfficient ? ' (hw)' : ' (sw)'}</div>
      {#if inn.stillLikely}
        <div class="calm">Still picture (normal)</div>
      {:else}
        <div class:bad={inboundTrouble(inn)}>freezes {inn.freezes} ({Math.round(inn.freezeMs)}ms)</div>
      {/if}
      <div>lost {inn.lost} jitter {num(inn.jitterMsPerFrame, 'ms')}</div>
      <div>connect {num(diag.connectMs, 'ms')} first frame {num(diag.firstFrameMs, 'ms')}</div>
    {/if}
  </div>
{/if}

<style>
  .diag { position: absolute; top: 8px; left: 8px; z-index: 2; padding: 6px 8px; border-radius: 6px; background: rgba(0, 0, 0, .65); color: #e6e6e6; font: 11px/1.4 ui-monospace, monospace; pointer-events: none; white-space: nowrap; }
  .calm { color: var(--text-3, #aaa); }
  .bad { color: var(--warn, #f0b429); }
</style>
