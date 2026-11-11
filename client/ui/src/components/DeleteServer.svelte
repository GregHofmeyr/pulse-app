<script lang="ts">
  // Server settings → Delete server (#184), shown to the server's owner only. A danger zone: typing the
  // server's name, as it is, unlocks the button. It goes to the owner's Recently deleted (Settings) for 7
  // days, where they can restore it whole; everyone else loses it from their rail at once.
  import { api, errorText } from '../lib/tauri'
  import type { Server } from '../lib/protocol/Server'

  let { server, onDeleted }: { server: Server; onDeleted: () => void } = $props()

  let typed = $state('')
  let busy = $state(false)
  let error = $state('')
  const matches = $derived(typed.trim() === server.name)

  async function go(e: SubmitEvent) {
    e.preventDefault()
    if (!matches || busy) return
    busy = true
    error = ''
    try {
      await api.deleteServer(server.id, typed.trim())
      onDeleted()
    } catch (err) {
      error = errorText(err)
    } finally {
      busy = false
    }
  }
</script>

<section class="settings-pane">
  <h2>Delete server</h2>
  <form class="danger" onsubmit={go}>
    <p>
      Deleting <strong>{server.name}</strong> takes it away from everyone at once: its channels, messages, voice,
      clips, soundboard and emoji. Anyone in its voice channels is disconnected. It stays in your Recently deleted
      (Settings) for 7 days, where you can restore it with everything in it; after that it's gone for good.
    </p>
    <label>
      <span>Type <strong>{server.name}</strong> to confirm</span>
      <input class="input" type="text" bind:value={typed} autocomplete="off" spellcheck="false" aria-label="Server name" disabled={busy} />
    </label>
    {#if error}<p class="err" role="alert">{error}</p>{/if}
    <div class="actions">
      <button type="submit" class="btn btn-danger" disabled={!matches || busy}>Delete server</button>
    </div>
  </form>
</section>

<style>
  .danger { display: flex; flex-direction: column; gap: 12px; padding: 16px; border: 1px solid var(--danger-text); border-radius: var(--radius-md); }
  p { margin: 0; font-size: 14px; color: var(--text-2); overflow-wrap: anywhere; }
  label { display: flex; flex-direction: column; gap: 6px; font-size: 13px; color: var(--text-2); }
  label strong, p strong { color: var(--text); overflow-wrap: anywhere; }
  .err { font-size: 13px; color: var(--danger-text); }
  .actions { display: flex; justify-content: flex-end; }
</style>
