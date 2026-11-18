<script lang="ts">
  // Asks before a server membership change (#371): you leaving a server (its tab's menu, Server settings), or its owner
  // removing or banning someone (#80) (their person menu in that server). A dialog like a channel's delete, with Cancel focused first:
  // it acts for good. Their messages stay either way; servers are invite-only (#80), so only a new invite brings them back.
  // Handing the server over (#418, Server settings or Make owner on a person's menu) can't be taken back by the one
  // who did it, so typing the server's name, as it is, unlocks its button (as Delete server does).
  import { modal } from '../lib/actions'
  import { api, errorText } from '../lib/tauri'
  import { app, type ServerAction } from '../lib/store.svelte'
  import { displayName } from '../lib/selectors'

  let { action, onClose }: { action: ServerAction; onClose: () => void } = $props()

  const server = $derived(app.state.servers.find((s) => s.id === action.serverId)?.name ?? 'this server')
  const person = $derived(action.kind === 'leave' ? '' : displayName(app.state, action.serverId, action.userId))
  const title = $derived(action.kind === 'leave' ? `Leave ${server}` : action.kind === 'remove' ? `Remove ${person}` : action.kind === 'ban' ? `Ban ${person}` : `Make ${person} the owner`)
  const confirm = $derived(action.kind === 'leave' ? 'Leave server' : action.kind === 'remove' ? 'Remove' : action.kind === 'ban' ? 'Ban' : 'Transfer ownership')
  let typed = $state('')
  const unlocked = $derived(action.kind !== 'transfer' || typed.trim() === server)
  let busy = $state(false)
  let error = $state('')

  async function go() {
    if (busy || !unlocked) return
    busy = true
    error = ''
    try {
      if (action.kind === 'leave') await api.leaveServer(action.serverId)
      else if (action.kind === 'remove') await api.removeServerMember(action.serverId, action.userId)
      else if (action.kind === 'ban') await api.setServerBan(action.serverId, action.userId, true)
      else await api.transferServer(action.serverId, action.userId)
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
      <p>Leave <strong>{server}</strong>? If you're in one of its voice channels, you're disconnected. Your messages stay, and you'll need an invite to come back.</p>
    {:else if action.kind === 'remove'}
      <p>Remove <strong>{person}</strong> from <strong>{server}</strong>? They're disconnected from its voice channels and streams. Their messages stay, and they'll need a new invite to come back.</p>
    {:else if action.kind === 'ban'}
      <p>Ban <strong>{person}</strong> from <strong>{server}</strong>? They're removed and can't join again until you unban them (Server settings, Bans). Their messages stay.</p>
    {:else}
      <p>Hand <strong>{server}</strong> to <strong>{person}</strong>? They become its owner, with everything only the owner can do, and you become a member like anyone else. Only they can hand it back.</p>
      <label>
        <span>Type <strong>{server}</strong> to confirm</span>
        <input class="input" type="text" bind:value={typed} autocomplete="off" spellcheck="false" aria-label="Server name" disabled={busy}
          onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); void go() } }} />
      </label>
    {/if}
    {#if error}<p class="err" role="alert">{error}</p>{/if}
    <div class="actions">
      <button type="button" class="btn btn-lg btn-secondary" disabled={busy} onclick={cancel}>Cancel</button>
      <button type="button" class="btn btn-lg btn-danger" disabled={busy || !unlocked} onclick={go}>{confirm}</button>
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 30; }
  .modal { width: min(400px, 100%); }
  p { margin: 0; font-size: 14px; color: var(--text-2); overflow-wrap: anywhere; }
  p strong, label strong { color: var(--text); overflow-wrap: anywhere; }
  label { display: flex; flex-direction: column; gap: 6px; font-size: 13px; color: var(--text-2); }
  .err { font-size: 13px; color: var(--danger-text); }
  .actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 8px; }
</style>
