<script lang="ts">
  // Server settings → Transfer ownership (#418), its owner's only. A danger zone: pick another member, then the shell's
  // dialog asks for the server's name before handing it over (the same confirm as Make owner on a person's menu).
  // Once it's theirs you're a member like anyone else, and may leave.
  import Select from './Select.svelte'
  import { app, ui } from '../lib/store.svelte'
  import { displayName } from '../lib/selectors'
  import type { Server } from '../lib/protocol/Server'

  let { server }: { server: Server } = $props()

  const options = $derived(
    (app.state.members[server.id] ?? [])
      .filter((m) => m.user.id !== app.state.me?.id)
      .map((m) => ({ value: m.user.id, label: displayName(app.state, server.id, m.user.id) }))
      .sort((a, b) => a.label.localeCompare(b.label)),
  )
  let picked = $state<string | null>(null)
  // a pick that left the server meanwhile is no pick
  const chosen = $derived(options.some((o) => o.value === picked) ? picked : null)
</script>

<section class="settings-pane">
  <h2>Transfer ownership</h2>
  <div class="danger">
    <p>
      Hand <strong>{server.name}</strong> to someone else in it. They become its owner, with everything only the owner
      can do (removing people, archiving channels, deleting the server), and you become a member like anyone else, free
      to leave. Only they can hand it back.
    </p>
    {#if options.length}
      <Select bind:value={picked} label="New owner" options={[{ value: null, label: 'Choose someone' }, ...options]} />
      <div class="actions">
        <button type="button" class="btn btn-danger" disabled={!chosen}
          onclick={() => { if (chosen) ui.serverAction = { kind: 'transfer', serverId: server.id, userId: chosen } }}>Transfer ownership</button>
      </div>
    {:else}
      <p>There's nobody else in it to hand it to yet.</p>
    {/if}
  </div>
</section>

<style>
  .danger { display: flex; flex-direction: column; gap: 12px; padding: 16px; border: 1px solid var(--danger-text); border-radius: var(--radius-md); }
  p { margin: 0; font-size: 14px; color: var(--text-2); overflow-wrap: anywhere; }
  p strong { color: var(--text); overflow-wrap: anywhere; }
  .actions { display: flex; justify-content: flex-end; }
</style>
