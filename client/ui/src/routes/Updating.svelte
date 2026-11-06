<script lang="ts">
  import { updates } from '../lib/updates.svelte'

  // "Updating Pulse…" (#82): over whatever is showing (the boot screen, the shell, the forced-update screen), so a
  // failed install hands back exactly what was there. The app's own background, like the boot and waiting screens.
  const known = $derived(updates.progress && updates.progress.total ? updates.progress : null)
  const pct = $derived(known && known.total ? Math.min(100, Math.round((known.downloaded / known.total) * 100)) : 0)
</script>

<div class="updating" data-tauri-drag-region>
  <p role="status">Updating Pulse…</p>
  {#if known}
    <div class="bar" role="progressbar" aria-label="Downloading the update" aria-valuemin="0" aria-valuemax="100" aria-valuenow={pct}>
      <span class="fill" style:transform="scaleX({pct / 100})"></span>
    </div>
  {:else}
    <div class="bar indeterminate" role="progressbar" aria-label="Downloading the update"><span class="fill"></span></div>
  {/if}
</div>

<style>
  .updating { position: fixed; inset: 0; z-index: 200; display: grid; place-content: center; justify-items: center; gap: 16px; background: var(--bg-0); color: var(--text); text-align: center; padding: 24px; }
  p { margin: 0; font-size: 14px; }
  .bar { position: relative; width: 220px; height: 4px; border-radius: 2px; background: var(--bg-2); overflow: hidden; }
  /* grows by transform, never width (motionCss.test) */
  .fill { position: absolute; inset: 0; border-radius: 2px; background: var(--accent); transform-origin: left; transition: transform var(--dur-slow) var(--ease); }
  /* the size isn't known: a short piece slides across at its own speed, like a spinner */
  .indeterminate .fill { right: auto; width: 35%; animation: slide 1.2s ease-in-out infinite; }
  @keyframes slide { from { transform: translateX(-100%); } to { transform: translateX(290%); } }
</style>
