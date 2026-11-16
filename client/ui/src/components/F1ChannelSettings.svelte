<script lang="ts">
  // Server settings › F1 (#386): where the F1 race weekend cards go. The owner picks one of the server's text channels,
  // or none (the default: no cards, no countdown, and the server never asks Jolpica-F1); everyone else sees the choice.
  import { app } from '../lib/store.svelte'
  import { putF1Status } from '../lib/f1.svelte'
  import { errorText, f1Cmd } from '../lib/tauri'
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
    f1Cmd.status(id).then(
      (r) => {
        if (gone) return
        putF1Status(id, r)
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
      const st = await f1Cmd.setChannel(server.id, value || null)
      putF1Status(server.id, st)
      current = st.channel_id
    } catch (e) {
      error = errorText(e)
    } finally {
      saving = false
    }
  }
  const nameOf = (id: string | null) => (id ? (app.state.channels[id]?.name ?? 'a channel') : null)
</script>

<div class="settings-field">
  <h3 class="settings-section-title">F1 channel</h3>
  <p class="hint">Race weekends go here: a countdown to lights out, a card when the weekend starts with every session in each person's own time and podium predictions, a "lights out in 1 hour" line, then the podium, results, standings and everyone's prediction scores.</p>
  {#if owner}
    <label class="pick">Channel
      <select class="input input-sm" aria-label="F1 channel" disabled={!loaded || saving} value={current ?? ''} onchange={(e) => void pick(e.currentTarget.value)}>
        <option value="">None (no F1 cards)</option>
        {#each textChannels as c (c.id)}<option value={c.id}>#{c.name}</option>{/each}
      </select>
    </label>
  {:else if loaded}
    <p class="now">{current ? `F1 cards go to #${nameOf(current)}.` : 'No channel: no F1 cards are posted.'} Only the server's owner can change it.</p>
  {/if}
  <p class="hint">Each member can fold the cards away in Settings › Chat › F1 race weekends.</p>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
  p { margin: 0; color: var(--text-2); font-size: 13px; }
  .pick { display: flex; align-items: center; gap: 8px; font-weight: 600; }
  .now { color: var(--text); }
  .error { color: var(--danger-text); }
</style>
