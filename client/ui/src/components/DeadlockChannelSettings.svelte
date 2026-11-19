<script lang="ts">
  // Server settings › Deadlock (#390): where members' Deadlock cards go and which kinds. The owner picks one of the
  // server's text channels, or none (the default: nothing posts), and switches post-match cards, the weekly rank
  // movers and patch notes (all on); everyone else sees the choice.
  import Select from './Select.svelte'
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import type { Server } from '../lib/protocol/Server'
  import type { DeadlockChannel } from '../lib/protocol/DeadlockChannel'

  let { server }: { server: Server } = $props()
  const owner = $derived(!!server.owner_id && server.owner_id === app.state.me?.id)
  const textChannels = $derived(
    Object.values(app.state.channels)
      .filter((c) => c.server_id === server.id && c.kind === 'text')
      .sort((a, b) => a.position - b.position),
  )
  let current = $state<DeadlockChannel | null>(null)
  let saving = $state(false)
  let error = $state('')
  $effect(() => {
    const id = server.id
    let gone = false
    current = null
    api.deadlockChannel(id).then(
      (r) => !gone && (current = r),
      (e) => !gone && (error = errorText(e)),
    )
    return () => (gone = true)
  })
  type Kind = 'match_cards' | 'rank_movers' | 'patch_notes'
  const kinds: { key: Kind; label: string; help: string }[] = [
    { key: 'match_cards', label: 'Post-match cards', help: "Hero, K/D/A, win or loss, length and rank change after members' matches; friends in the same match share a card." },
    { key: 'rank_movers', label: 'Weekly rank movers', help: 'Every Monday morning: whose rank went up or down that week.' },
    { key: 'patch_notes', label: 'Patch notes', help: 'Each Deadlock update, with the heroes it touches.' },
  ]
  async function save(change: { channel_id?: string | null } & Partial<Record<Kind, boolean>>) {
    if (!current) return
    saving = true
    error = ''
    try {
      const channel_id = 'channel_id' in change ? (change.channel_id ?? null) : current.channel_id
      current = await api.setDeadlockChannel(server.id, { ...change, channel_id })
    } catch (e) {
      error = errorText(e)
    } finally {
      saving = false
    }
  }
  const nameOf = (id: string | null) => (id ? (app.state.channels[id]?.name ?? 'a channel') : null)
</script>

<div class="settings-field">
  <h3 class="settings-section-title">Deadlock channel</h3>
  <p class="hint">Members who linked Steam get Deadlock cards here, from deadlock-api.com. Each member chooses what of theirs is shared.</p>
  {#if owner}
    <label class="pick">Channel
      <Select size="sm" label="Deadlock channel" disabled={!current || saving} value={current?.channel_id ?? ''}
        options={[{ value: '', label: 'None (no cards)' }, ...textChannels.map((c) => ({ value: c.id, label: `#${c.name}` }))]} onchange={(v) => void save({ channel_id: v || null })} />
    </label>
    {#if current?.channel_id}
      {#each kinds as k (k.key)}
        <label class="row">
          <input type="checkbox" role="switch" class="switch" data-kind={k.key} checked={current[k.key]} disabled={saving}
            onchange={(e) => void save({ [k.key]: e.currentTarget.checked })} />
          <span>{k.label}</span>
        </label>
        <small class="sub">{k.help}</small>
      {/each}
    {/if}
  {:else if current}
    <p class="now">{current.channel_id ? `Cards go to #${nameOf(current.channel_id)}.` : 'No channel: no cards are posted.'} Only the server's owner can change it.</p>
  {/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
  p { margin: 0; color: var(--text-2); font-size: 13px; }
  .pick, .row { display: flex; align-items: center; gap: 8px; font-weight: 600; }
  /* the dropdown (#455): wide enough for a channel name, never past the pane */
  .pick :global(.select) { flex: 1; min-width: 180px; max-width: 320px; }
  .sub { margin-left: 24px; color: var(--text-3); }
  .now { color: var(--text); }
  .error { color: var(--danger-text); }
</style>
