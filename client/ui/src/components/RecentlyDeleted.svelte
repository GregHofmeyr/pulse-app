<script lang="ts">
  // Server settings → Recently deleted (#112): the server's deleted channels, kept for 7 days. The
  // server owner and whoever deleted one may restore it (back in its place, with its history); the
  // owner may delete one for good now. The server says who may do what (`can_restore`, `can_purge`).
  import { app, clock } from '../lib/store.svelte'
  import { displayName } from '../lib/selectors'
  import { api, errorText } from '../lib/tauri'
  import Loading from './Loading.svelte'
  import type { Server } from '../lib/protocol/Server'
  import type { Channel } from '../lib/protocol/Channel'
  import type { DeletedChannel } from '../lib/protocol/DeletedChannel'

  let { server }: { server: Server } = $props()

  let items = $state<DeletedChannel[] | null>(null)
  let error = $state('')
  // The server's live channels: a delete or restore elsewhere changes them, and the list reloads.
  const live = $derived(
    Object.values(app.state.channels)
      .filter((c) => c.server_id === server.id)
      .map((c) => c.id)
      .sort()
      .join(),
  )
  $effect(() => {
    const id = server.id
    void live
    error = ''
    let stale = false
    api.listDeletedChannels(id).then(
      (list) => { if (!stale) items = list },
      (e) => { if (!stale) error = errorText(e) },
    )
    return () => { stale = true }
  })

  const KIND: Record<string, string> = { text: 'Text', voice: 'Voice', quotes: 'Quotes', clips: 'Clips' }
  const label = (c: Channel) => (c.kind === 'voice' ? (c.name ?? '') : `#${c.name}`)
  const when = (iso: string) => new Date(iso).toLocaleDateString()
  function left(d: DeletedChannel): string {
    const days = Math.ceil((Date.parse(d.purge_at) - Date.parse(clock.nowIso)) / 86_400_000)
    return days <= 0 ? 'Deleted for good soon' : days === 1 ? '1 day left' : `${days} days left`
  }

  let confirming = $state<string | null>(null)
  let busy = $state(false)
  async function act(d: DeletedChannel, run: (id: string) => Promise<unknown>) {
    if (busy) return
    busy = true
    error = ''
    try {
      await run(d.channel.id)
      items = (items ?? []).filter((x) => x.channel.id !== d.channel.id)
      confirming = null
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
</script>

<section class="settings-pane">
  <h2>Recently deleted</h2>
  <p class="hint">Deleted channels stay here for 7 days, with everything in them, then go for good. The server owner, or whoever deleted one, can restore it.</p>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if !items}
    {#if !error}<Loading />{/if}
  {:else if items.length === 0}
    <p class="empty">No deleted channels.</p>
  {:else}
    <ul class="settings-list">
      {#each items as d (d.channel.id)}
        <li class="settings-list-item">
          <div class="info">
            <span class="name">{label(d.channel)}</span>
            <small>{KIND[d.channel.kind] ?? d.channel.kind} · Deleted by {d.deleted_by ? displayName(app.state, server.id, d.deleted_by) : 'someone'} · {when(d.deleted_at)}</small>
            <small class:held={!!d.held}>{d.held ?? left(d)}</small>
          </div>
          {#if confirming === d.channel.id}
            <span class="q">Delete {label(d.channel)} for good? Everything in it goes, and it can't be undone.</span>
            <button class="btn btn-secondary btn-danger-text" disabled={busy} onclick={() => act(d, api.purgeChannel)}>Delete for good</button>
            <button class="btn btn-secondary" disabled={busy} onclick={() => { confirming = null; error = '' }}>Cancel</button>
          {:else}
            {#if d.can_restore}
              <button class="btn btn-secondary" aria-label="Restore {label(d.channel)}" disabled={busy} onclick={() => act(d, api.restoreChannel)}>Restore</button>
            {/if}
            {#if d.can_purge}
              <button class="btn btn-secondary btn-danger-text" aria-label="Delete {label(d.channel)} now" disabled={busy} onclick={() => (confirming = d.channel.id)}>Delete now</button>
            {/if}
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
  small.held { color: var(--text-2); }
  .q { font-size: 13px; color: var(--text-2); max-width: 260px; }
</style>
