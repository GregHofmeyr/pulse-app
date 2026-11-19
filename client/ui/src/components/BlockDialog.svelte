<script lang="ts">
  // Block someone (#474): what it does, and the per-block choice of how their messages show in servers you share
  // (a "Blocked message" line, or nothing at all). Cancel is focused first, as in the other confirm dialogs.
  import { modal } from '../lib/actions'
  import { api, errorText } from '../lib/tauri'
  import { app } from '../lib/store.svelte'
  import { displayName } from '../lib/selectors'
  import type { BlockMode } from '../lib/protocol/BlockMode'

  let { userId, onClose }: { userId: string; onClose: () => void } = $props()

  const name = $derived(displayName(app.state, null, userId))
  let mode = $state<BlockMode>('line')
  let busy = $state(false)
  let error = $state('')

  async function go() {
    if (busy) return
    busy = true
    error = ''
    try {
      await api.setBlock(userId, mode)
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
  <div class="modal pop-in" role="dialog" aria-modal="true" aria-label="Block {name}" tabindex="-1" use:modal={cancel} data-block-dialog>
    <h1 class="modal-title">Block {name}</h1>
    <p>Their DMs, calls, invites, pings and sounds won't reach you, and you won't hear them in voice. They'll see you offline, with an empty profile. They aren't told.</p>
    <fieldset>
      <legend>Their messages in servers you share</legend>
      <label class="choice">
        <input type="radio" name="block-mode" value="line" bind:group={mode} disabled={busy} />
        <span><strong>Show a 'Blocked message' line</strong><small>Their messages collapse into one line you can open.</small></span>
      </label>
      <label class="choice">
        <input type="radio" name="block-mode" value="hide" bind:group={mode} disabled={busy} />
        <span><strong>Hide completely</strong><small>No messages, member or voice row, reactions or typing from them.</small></span>
      </label>
    </fieldset>
    {#if error}<p class="err" role="alert">{error}</p>{/if}
    <div class="actions">
      <button type="button" class="btn btn-lg btn-secondary" disabled={busy} onclick={cancel}>Cancel</button>
      <button type="button" class="btn btn-lg btn-danger" disabled={busy} onclick={go}>Block</button>
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 30; }
  .modal { width: min(440px, 100%); }
  p { margin: 0; font-size: 14px; color: var(--text-2); overflow-wrap: anywhere; }
  fieldset { margin: 0; padding: 0; border: 0; display: flex; flex-direction: column; gap: 8px; }
  legend { margin-bottom: 8px; font-size: 12px; font-weight: 600; letter-spacing: .04em; color: var(--text-2); }
  .choice { display: flex; gap: 10px; align-items: flex-start; padding: 10px; border-radius: var(--radius-sm); background: var(--bg-2); cursor: pointer; }
  .choice span { display: flex; flex-direction: column; gap: 2px; font-size: 14px; color: var(--text); }
  .choice small { font-size: 12px; color: var(--text-3); }
  .err { font-size: 13px; color: var(--danger-text); }
  .actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 8px; }
</style>
