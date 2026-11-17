<script lang="ts">
  // Server settings › F1 (#386): where the F1 race weekend cards go. The owner picks one of the server's text channels,
  // or none (the default: no cards, no countdown, and the server never asks Jolpica-F1); everyone else sees the choice.
  // With a channel, the owner also picks when podium predictions close (#403): lights out (the default) or qualifying.
  import { app } from '../lib/store.svelte'
  import { putF1Status } from '../lib/f1.svelte'
  import { errorText, f1Cmd } from '../lib/tauri'
  import type { Server } from '../lib/protocol/Server'
  import type { F1PicksClose } from '../lib/protocol/F1PicksClose'

  let { server }: { server: Server } = $props()
  const owner = $derived(!!server.owner_id && server.owner_id === app.state.me?.id)
  const textChannels = $derived(
    Object.values(app.state.channels)
      .filter((c) => c.server_id === server.id && c.kind === 'text')
      .sort((a, b) => a.position - b.position),
  )
  let current = $state<string | null>(null)
  let picksClose = $state<F1PicksClose>('lights_out')
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
        picksClose = r.picks_close ?? 'lights_out'
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
      picksClose = st.picks_close ?? 'lights_out'
    } catch (e) {
      error = errorText(e)
    } finally {
      saving = false
    }
  }
  async function pickClose(value: F1PicksClose) {
    if (!current) return
    saving = true
    error = ''
    try {
      const st = await f1Cmd.setChannel(server.id, current, value)
      putF1Status(server.id, st)
      picksClose = st.picks_close ?? 'lights_out'
    } catch (e) {
      error = errorText(e)
    } finally {
      saving = false
    }
  }
  const CLOSE_LABELS: Record<F1PicksClose, string> = { lights_out: 'At lights out', qualifying: 'When qualifying starts' }
  const nameOf = (id: string | null) => (id ? (app.state.channels[id]?.name ?? 'a channel') : null)
</script>

<div class="settings-field">
  <h3 class="settings-section-title">F1 channel</h3>
  <p class="hint">Race weekends go here: a countdown to lights out, a card when the weekend starts with every session in each person's own time and podium predictions, a "lights out in 1 hour" line, a sprint's results on sprint weekends, then the podium, results, drivers' and constructors' standings and everyone's prediction scores.</p>
  {#if owner}
    <label class="pick">Channel
      <select class="input input-sm" aria-label="F1 channel" disabled={!loaded || saving} value={current ?? ''} onchange={(e) => void pick(e.currentTarget.value)}>
        <option value="">None (no F1 cards)</option>
        {#each textChannels as c (c.id)}<option value={c.id}>#{c.name}</option>{/each}
      </select>
    </label>
    {#if current}
      <label class="pick">Predictions close
        <select class="input input-sm" aria-label="Predictions close" disabled={!loaded || saving} value={picksClose} onchange={(e) => void pickClose(e.currentTarget.value as F1PicksClose)}>
          {#each Object.entries(CLOSE_LABELS) as [value, label] (value)}<option {value}>{label}</option>{/each}
        </select>
      </label>
    {/if}
  {:else if loaded}
    <p class="now">{current ? `F1 cards go to #${nameOf(current)}. Predictions close ${picksClose === 'qualifying' ? 'when qualifying starts' : 'at lights out'}.` : 'No channel: no F1 cards are posted.'} Only the server's owner can change it.</p>
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
