<script lang="ts">
  import { diagUi, setDiagOverlay } from '../lib/stream/diag.svelte'
</script>

<h2>Advanced</h2>
<label class="row">
  <input type="checkbox" checked={diagUi.on} onchange={(e) => setDiagOverlay(e.currentTarget.checked)} />
  <span>Stream diagnostics</span>
</label>
<p class="hint">Shows live numbers on streams you share or watch. They are always written to the log for bug reports.</p>

<style>
  h2 { margin: 0 0 16px; font-size: 20px; }
  .row { display: flex; align-items: center; gap: 10px; font-weight: 600; }
  .hint { margin: 6px 0 0 26px; font-size: 13px; color: var(--text-3); }
</style>
