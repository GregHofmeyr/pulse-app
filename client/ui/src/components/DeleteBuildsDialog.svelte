<script lang="ts">
  // Asks before builds are deleted from the browse view (#617): one from its right-click menu, or the selection's from
  // the selection bar, with the count, and how many of the selection are skipped as not yours. Deleting is for good.
  // Cancel has the focus first.
  import { modal } from '../lib/actions'
  import { errorText } from '../lib/tauri'

  let { count, skipped = 0, onConfirm, onClose }: {
    count: number
    /** Selected builds that aren't yours to delete: left as they are. */
    skipped?: number
    /** Deletes them; the dialog closes once it's done, and says why if it failed. */
    onConfirm: () => Promise<void>
    onClose: () => void
  } = $props()

  const one = $derived(count === 1)
  const title = $derived(one ? 'Delete build' : `Delete ${count} builds`)
  let busy = $state(false)
  let error = $state('')
  async function go() {
    if (busy) return
    busy = true
    error = ''
    try {
      await onConfirm()
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
</script>

<div class="overlay modal-overlay fade-in">
  <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={cancel}></div>
  <div class="modal pop-in" role="dialog" aria-modal="true" aria-label={title} tabindex="-1" use:modal={cancel}>
    <h1 class="modal-title">{title}</h1>
    <p>{one ? 'Delete this build? Its screenshots and comments go too.' : `Delete ${count} builds? Their screenshots and comments go too.`}
      {#if skipped}{skipped === 1 ? '1 selected build isn’t yours and is left as it is.' : `${skipped} selected builds aren’t yours and are left as they are.`}{/if}</p>
    {#if error}<p class="err" role="alert">{error}</p>{/if}
    <div class="actions">
      <button type="button" class="btn btn-lg btn-secondary" disabled={busy} onclick={cancel}>Cancel</button>
      <button type="button" class="btn btn-lg btn-danger" disabled={busy} onclick={go}>{one ? 'Delete' : `Delete ${count} builds`}</button>
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 30; }
  .modal { width: min(400px, 100%); }
  p { margin: 0; font-size: 14px; color: var(--text-2); overflow-wrap: anywhere; }
  .err { font-size: 13px; color: var(--danger-text); }
  .actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 8px; }
</style>
