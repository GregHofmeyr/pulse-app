<script lang="ts">
  // Asks before a server channel is deleted. A dialog, not the floating question a message's delete
  // has: it's bigger, and it goes for everyone. It moves to the server's Recently deleted, from where
  // it can be restored. Cancel has the focus first.
  import { modal } from '../lib/actions'
  import { api, errorText } from '../lib/tauri'
  import type { Channel } from '../lib/protocol/Channel'

  let { channel, onClose }: { channel: Channel; onClose: () => void } = $props()

  const label = $derived(channel.kind === 'voice' ? channel.name : `#${channel.name}`)
  const kicks = $derived(channel.kind === 'voice' ? 'Anyone in it is disconnected. ' : '')
  let busy = $state(false)
  let error = $state('')

  async function go() {
    if (busy) return
    busy = true
    error = ''
    try {
      await api.deleteChannel(channel.id)
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
  <div class="modal pop-in" role="dialog" aria-modal="true" aria-label="Delete {label}" tabindex="-1" use:modal={cancel}>
    <h1 class="modal-title">Delete {label}</h1>
    <p>Delete {label}? {kicks}It moves to Recently deleted for 7 days, where the server owner or you can restore it.</p>
    {#if error}<p class="err" role="alert">{error}</p>{/if}
    <div class="actions">
      <button type="button" class="btn btn-lg btn-secondary" disabled={busy} onclick={cancel}>Cancel</button>
      <button type="button" class="btn btn-lg btn-danger" disabled={busy} onclick={go}>Delete channel</button>
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 30; }
  .modal { width: min(400px, 100%); }
  p { margin: 0; font-size: 14px; color: var(--text-2); overflow-wrap: anywhere; }
  .err { font-size: 13px; color: var(--danger-text); }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
</style>
