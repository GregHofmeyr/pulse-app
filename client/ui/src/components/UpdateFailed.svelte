<script lang="ts">
  import type { FailedUpdate } from '../lib/tauri'
  import { safeHref } from '../lib/safeHref'

  // #460: the last install didn't finish (Pulse found itself still on the old version). It won't download it again by
  // itself for a while ("Restart to update" still can); this says so, with the installer to get by hand.
  let { failed, onClose }: { failed: FailedUpdate; onClose: () => void } = $props()
</script>

<div class="update-failed" role="alert">
  <p>Couldn't update to {failed.version}.
    {#if failed.downloadUrl}<a href={safeHref(failed.downloadUrl)} target="_blank" rel="noreferrer">Download it</a> and run it to update.{/if}
  </p>
  <button type="button" class="btn btn-secondary" onclick={onClose}>Dismiss</button>
</div>

<style>
  .update-failed {
    position: fixed; left: 50%; bottom: 24px; transform: translateX(-50%); z-index: 150; max-width: min(520px, calc(100vw - 32px));
    display: flex; align-items: center; gap: 14px; padding: 12px 14px; border-radius: var(--radius-md);
    background: var(--bg-2); border: 1px solid var(--line); box-shadow: var(--shadow-2); color: var(--text);
  }
  p { margin: 0; font-size: 13px; line-height: 1.45; }
  a { color: var(--accent-text); }
</style>
