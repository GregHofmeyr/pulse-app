<script lang="ts">
  // Server settings › Sports (#385): where the match cards for teams members follow go. The owner picks one of the
  // server's text channels, or none (the default: no cards); everyone else sees the choice.
  import { app } from '../lib/store.svelte'
  import { errorText, sportsCmd } from '../lib/tauri'
  import type { Server } from '../lib/protocol/Server'

  let { server }: { server: Server } = $props()
  const owner = $derived(!!server.owner_id && server.owner_id === app.state.me?.id)
  const textChannels = $derived(
    Object.values(app.state.channels)
      .filter((c) => c.server_id === server.id && c.kind === 'text')
      .sort((a, b) => a.position - b.position),
  )
  let current = $state<string | null>(null)
  let loaded = $state(false)
  let saving = $state(false)
  let error = $state('')
  $effect(() => {
    const id = server.id
    let gone = false
    loaded = false
    sportsCmd.channel(id).then(
      (r) => {
        if (gone) return
        current = r.channel_id
        loaded = true
      },
      (e) => !gone && (error = errorText(e)),
    )
    return () => (gone = true)
  })
  async function pick(value: string) {
    saving = true
    error = ''
    try {
      current = (await sportsCmd.setChannel(server.id, value || null)).channel_id
    } catch (e) {
      error = errorText(e)
    } finally {
      saving = false
    }
  }
  const nameOf = (id: string | null) => (id ? (app.state.channels[id]?.name ?? 'a channel') : null)
</script>

<div class="settings-field">
  <h3 class="settings-section-title">Sports channel</h3>
  <p class="hint">On match day a card goes here for each match of a team a member follows (Settings › Sports): kick-off in everyone's own time, then the score as it changes, edited in place, and full time.</p>
  {#if owner}
    <label class="pick">Channel
      <select class="input input-sm" aria-label="Sports channel" disabled={!loaded || saving} value={current ?? ''} onchange={(e) => void pick(e.currentTarget.value)}>
        <option value="">None (no cards)</option>
        {#each textChannels as c (c.id)}<option value={c.id}>#{c.name}</option>{/each}
      </select>
    </label>
  {:else if loaded}
    <p class="now">{current ? `Cards go to #${nameOf(current)}.` : 'No channel: no cards are posted.'} Only the server's owner can change it.</p>
  {/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
  p { margin: 0; color: var(--text-2); font-size: 13px; }
  .pick { display: flex; align-items: center; gap: 8px; font-weight: 600; }
  .now { color: var(--text); }
  .error { color: var(--danger-text); }
</style>
