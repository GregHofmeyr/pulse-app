<script lang="ts">
  // A builds channel (#617): its header and its builds.
  import { app } from '../lib/store.svelte'

  let { channelId, serverId: _serverId }: { channelId: string; serverId: string } = $props()

  const name = $derived(app.state.channels[channelId]?.name ?? '')
</script>

<div class="chan" data-testid="builds-channel">
  <header class="head"><h2>{name}</h2></header>
  <p class="empty">No builds yet.</p>
</div>

<style>
  .chan { flex: 1; display: flex; flex-direction: column; min-height: 0; position: relative; }
  .head { display: flex; align-items: center; gap: 8px; padding: 12px 16px; border-bottom: 1px solid var(--border); }
  .head h2 { margin: 0; font-size: 16px; }
  .empty { color: var(--text-muted); padding: 24px 16px; }
</style>
