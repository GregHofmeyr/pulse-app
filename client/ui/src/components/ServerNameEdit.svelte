<script lang="ts">
  // Renaming a server in place (#55): the name at the top of the channel list becomes a field, from its menu or from
  // the server tab's right-click › Rename (its owner's). Enter saves (and keeps the field if refused), Escape puts it
  // back, and leaving the field saves, as a channel's rename in its row does.
  import { api, errorText } from '../lib/tauri'
  import type { Server } from '../lib/protocol/Server'

  let { server, onDone }: {
    server: Server
    /** Back to the name; `fromKey`: focus goes back to it too (the field it had is gone). */
    onDone: (fromKey: boolean) => void
  } = $props()

  // svelte-ignore state_referenced_locally
  let value = $state(server.name)
  let error = $state('')
  let saving = false
  let done = false
  const focus = (el: HTMLInputElement) => {
    el.focus()
    el.select()
  }

  function finish(fromKey: boolean) {
    if (done) return
    done = true
    onDone(fromKey)
  }
  async function save(fromKey: boolean) {
    if (done || saving) return
    if (!fromKey && error) return finish(false)
    const name = value.trim()
    if (!name || name === server.name) return finish(fromKey)
    saving = true
    try {
      await api.renameServer(server.id, name)
      finish(fromKey)
    } catch (e) {
      error = errorText(e)
    } finally {
      saving = false
    }
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'Enter') void save(true)
    if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      finish(true)
    }
  }
</script>

<div class="rename">
  <input class="input input-sm" use:focus bind:value maxlength="64" aria-label="Rename {server.name}" onkeydown={key} onblur={() => void save(false)} />
  {#if error}<small class="err" role="alert">{error}</small>{/if}
</div>

<style>
  .rename { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; margin: 0 -8px; }
  .input { width: 100%; font-weight: 600; }
  .err { color: var(--danger-text); font-size: 12px; font-weight: 400; position: absolute; left: 16px; bottom: 2px; }
</style>
