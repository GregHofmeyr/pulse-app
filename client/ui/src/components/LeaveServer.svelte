<script lang="ts">
  // Server settings → Leave server (#371). A danger zone like Delete server's: the button acts, as the pane already
  // says what happens. Its owner can't leave while it's theirs, so they're pointed at Transfer ownership (#418).
  import { api, errorText } from '../lib/tauri'
  import type { Server } from '../lib/protocol/Server'

  let { server, owner, onLeft }: { server: Server; owner: boolean; onLeft: () => void } = $props()

  let busy = $state(false)
  let error = $state('')

  async function go() {
    if (busy) return
    busy = true
    error = ''
    try {
      await api.leaveServer(server.id)
      onLeft()
    } catch (err) {
      error = errorText(err)
    } finally {
      busy = false
    }
  }
</script>

<section class="settings-pane">
  <h2>Leave server</h2>
  <div class="danger">
    {#if owner}
      <p>
        You own <strong>{server.name}</strong>, so you can't leave it: hand it to someone else first (Transfer
        ownership), then leave. Or delete it (Delete server).
      </p>
    {:else}
      <p>
        Leaving <strong>{server.name}</strong> disconnects you from its voice channels, and its unread counts and
        mentions go from your inbox. Your messages stay. It stays in your server list, where you can join it again.
      </p>
      {#if error}<p class="err" role="alert">{error}</p>{/if}
      <div class="actions">
        <button type="button" class="btn btn-danger" disabled={busy} onclick={go}>Leave server</button>
      </div>
    {/if}
  </div>
</section>

<style>
  .danger { display: flex; flex-direction: column; gap: 12px; padding: 16px; border: 1px solid var(--danger-text); border-radius: var(--radius-md); }
  p { margin: 0; font-size: 14px; color: var(--text-2); overflow-wrap: anywhere; }
  p strong { color: var(--text); overflow-wrap: anywhere; }
  .err { font-size: 13px; color: var(--danger-text); }
  .actions { display: flex; justify-content: flex-end; }
</style>
