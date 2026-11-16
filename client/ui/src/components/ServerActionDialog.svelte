<script lang="ts">
  // Asks before a server membership change (#371): you leaving a server (its tab's menu, Server settings), or its owner
  // removing someone (their person menu in that server). A dialog like a channel's delete, with Cancel focused first:
  // it acts for good. Their messages stay either way, and the server stays listed, so Join server brings them back.
  import { modal } from '../lib/actions'
  import { api, errorText } from '../lib/tauri'
  import { app } from '../lib/store.svelte'
  import { displayName } from '../lib/selectors'

  type Action = { kind: 'leave'; serverId: string } | { kind: 'remove'; serverId: string; userId: string }
  let { action, onClose }: { action: Action; onClose: () => void } = $props()

  const server = $derived(app.state.servers.find((s) => s.id === action.serverId)?.name ?? 'this server')
  const person = $derived(action.kind === 'remove' ? displayName(app.state, action.serverId, action.userId) : '')
  const title = $derived(action.kind === 'leave' ? `Leave ${server}` : `Remove ${person}`)
  let busy = $state(false)
  let error = $state('')

  async function go() {
    if (busy) return
    busy = true
    error = ''
    try {
      if (action.kind === 'leave') await api.leaveServer(action.serverId)
      else await api.removeServerMember(action.serverId, action.userId)
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
    {#if action.kind === 'leave'}
      <p>Leave <strong>{server}</strong>? If you're in one of its voice channels, you're disconnected. Your messages stay, and you can join it again from the server list.</p>
    {:else}
      <p>Remove <strong>{person}</strong> from <strong>{server}</strong>? They're disconnected from its voice channels and streams. Their messages stay, and they can join again.</p>
    {/if}
    {#if error}<p class="err" role="alert">{error}</p>{/if}
    <div class="actions">
      <button type="button" class="btn btn-lg btn-secondary" disabled={busy} onclick={cancel}>Cancel</button>
      <button type="button" class="btn btn-lg btn-danger" disabled={busy} onclick={go}>{action.kind === 'leave' ? 'Leave server' : 'Remove'}</button>
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 30; }
  .modal { width: min(400px, 100%); }
  p { margin: 0; font-size: 14px; color: var(--text-2); overflow-wrap: anywhere; }
  p strong { color: var(--text); }
  .err { font-size: 13px; color: var(--danger-text); }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
</style>
