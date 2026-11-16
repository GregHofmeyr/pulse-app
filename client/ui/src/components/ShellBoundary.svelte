<script lang="ts">
  // The Shell's safety net (#132): an error while showing it (a render or an effect) leaves this screen, not a window
  // whose effects quietly stopped. The error goes to the log (lib/uiErrors.ts); Reload mounts the Shell afresh.
  import type { Snippet } from 'svelte'
  import { logBoundaryError } from '../lib/uiErrors'

  let { children }: { children: Snippet } = $props()
</script>

<svelte:boundary onerror={(e) => logBoundaryError('shell', e)}>
  {@render children()}
  {#snippet failed(_error, reset)}
    <div class="crashed" role="alert">
      <h1>Something went wrong</h1>
      <p>Pulse hit an error showing this screen.</p>
      <button type="button" class="btn btn-primary" onclick={reset}>Reload</button>
    </div>
  {/snippet}
</svelte:boundary>

<style>
  .crashed { height: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; padding: 24px; background: var(--bg-0); color: var(--text); text-align: center; }
  h1 { margin: 0; font-size: 20px; font-weight: 700; }
  p { margin: 0; color: var(--text-2); font-size: 14px; }
</style>
