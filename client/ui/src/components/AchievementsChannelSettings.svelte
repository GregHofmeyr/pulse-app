<script lang="ts">
  // Server settings › Achievements (#379): where members' Steam achievement cards go. The owner picks one of the
  // server's text channels, or none (the default: no cards); everyone else sees the choice.
  import Select from './Select.svelte'
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
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
    api.achievementsChannel(id).then(
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
      current = (await api.setAchievementsChannel(server.id, value || null)).channel_id
    } catch (e) {
      error = errorText(e)
    } finally {
      saving = false
    }
  }
  const nameOf = (id: string | null) => (id ? (app.state.channels[id]?.name ?? 'a channel') : null)
</script>

<div class="settings-field">
  <h3 class="settings-section-title">Achievements channel</h3>
  <p class="hint">When a member who linked Steam unlocks achievements while playing, or finishes a game 100 %, a card goes here. Each member chooses whether to share theirs.</p>
  {#if owner}
    <label class="pick">Channel
      <Select size="sm" label="Achievements channel" disabled={!loaded || saving} value={current ?? ''}
        options={[{ value: '', label: 'None (no cards)' }, ...textChannels.map((c) => ({ value: c.id, label: `#${c.name}` }))]} onchange={(v) => void pick(v ?? '')} />
    </label>
  {:else if loaded}
    <p class="now">{current ? `Cards go to #${nameOf(current)}.` : 'No channel: no cards are posted.'} Only the server's owner can change it.</p>
  {/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
  p { margin: 0; color: var(--text-2); font-size: 13px; }
  .pick { display: flex; align-items: center; gap: 8px; font-weight: 600; }
  /* the dropdown (#455): wide enough for a channel name, never past the pane */
  .pick :global(.select) { flex: 1; min-width: 180px; max-width: 320px; }
  .now { color: var(--text); }
  .error { color: var(--danger-text); }
</style>
