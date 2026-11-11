<script lang="ts">
  // Settings → Recently deleted (#184): the servers you deleted, kept for 7 days with everything in them, as a
  // server's deleted channels are in its Server settings (#112). Only you, their owner, see them; Restore brings
  // one back for everyone. Once its 7 days are up it's being deleted for good and can't be restored.
  import { untrack } from 'svelte'
  import { app, clock } from '../lib/store.svelte'
  import { windowFocus } from '../lib/windowFocus.svelte'
  import { api, errorText } from '../lib/tauri'
  import Loading from './Loading.svelte'
  import type { DeletedServer } from '../lib/protocol/DeletedServer'

  let items = $state<DeletedServer[] | null>(null)
  let error = $state('')
  // The servers on the rail: a delete or restore (here or on another device) changes them, and the list reloads.
  const live = $derived(app.state.servers.map((s) => s.id).sort().join())
  // A purge sends no event (the server is off everyone's rail already, #287), so it's asked again: on each tick of
  // the clock while one listed is due (its 7 days are up, or it's being deleted for good), and when the window comes
  // back into focus (Settings left open in the background).
  let again = $state(0)
  const due = $derived(!!items?.some((d) => !d.can_restore || Date.parse(d.purge_at) <= Date.parse(clock.nowIso)))
  $effect(() => {
    void clock.nowIso
    untrack(() => due && again++)
  })
  let wasFocused = untrack(() => windowFocus.focused)
  $effect(() => {
    const f = windowFocus.focused
    if (f && !wasFocused) untrack(() => again++)
    wasFocused = f
  })
  $effect(() => {
    void live
    void again
    error = ''
    let stale = false
    api.listDeletedServers().then(
      (list) => { if (!stale) items = list },
      (e) => { if (!stale) error = errorText(e) },
    )
    return () => { stale = true }
  })

  const when = (iso: string) => new Date(iso).toLocaleDateString()
  function left(d: DeletedServer): string {
    const days = Math.ceil((Date.parse(d.purge_at) - Date.parse(clock.nowIso)) / 86_400_000)
    return !d.can_restore || days <= 0 ? 'Being deleted for good' : days === 1 ? '1 day left' : `${days} days left`
  }

  let busy = $state(false)
  async function restore(d: DeletedServer) {
    if (busy) return
    busy = true
    error = ''
    try {
      await api.restoreServer(d.server.id)
      items = (items ?? []).filter((x) => x.server.id !== d.server.id)
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
</script>

<section class="settings-pane">
  <h2>Recently deleted</h2>
  <p class="hint">Servers you delete stay here for 7 days, with everything in them, then go for good. Only you can restore them.</p>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if !items}
    {#if !error}<Loading />{/if}
  {:else if items.length === 0}
    <p class="empty">No deleted servers.</p>
  {:else}
    <ul class="settings-list">
      {#each items as d (d.server.id)}
        <li class="settings-list-item">
          <div class="info">
            <span class="name">{d.server.name}</span>
            <small>Deleted {when(d.deleted_at)} · {left(d)}</small>
          </div>
          {#if d.can_restore}
            <button class="btn btn-secondary" aria-label="Restore {d.server.name}" disabled={busy} onclick={() => restore(d)}>Restore</button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .hint { margin: -12px 0 0; color: var(--text-3); font-size: 12px; }
  .error { margin: 0; color: var(--danger-text); font-size: 13px; }
  .empty { margin: 0; color: var(--text-3); }
  /* the list is the shared .settings-list (app.css) */
  .info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .name { font-weight: 600; overflow-wrap: anywhere; }
  small { color: var(--text-3); font-size: 12px; }
</style>
