<script lang="ts">
  // Server settings › Sports (#385): where the match cards for teams members follow go. The owner picks one of the
  // server's text channels, or none (the default: no cards); everyone else sees the choice. With a channel, the owner
  // can also limit which leagues post there (#406; every league by default, and at least one stays on).
  import { app } from '../lib/store.svelte'
  import { errorText, sportsCmd } from '../lib/tauri'
  import { LEAGUE_NAMES, leagueName } from '../lib/sports'
  import type { Server } from '../lib/protocol/Server'
  import type { SportsLeague } from '../lib/protocol/SportsLeague'

  let { server }: { server: Server } = $props()
  const owner = $derived(!!server.owner_id && server.owner_id === app.state.me?.id)
  const textChannels = $derived(
    Object.values(app.state.channels)
      .filter((c) => c.server_id === server.id && c.kind === 'text')
      .sort((a, b) => a.position - b.position),
  )
  const ALL = Object.keys(LEAGUE_NAMES) as SportsLeague[]
  let current = $state<string | null>(null)
  /** The leagues that post here; null = every league. */
  let leagues = $state<SportsLeague[] | null>(null)
  const on = $derived(leagues ?? ALL)
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
        leagues = r.leagues ?? null
        loaded = true
      },
      (e) => !gone && (error = errorText(e)),
    )
    return () => (gone = true)
  })
  async function save(channelId: string | null, pick?: SportsLeague[]) {
    saving = true
    error = ''
    try {
      const r = await sportsCmd.setChannel(server.id, channelId, pick)
      current = r.channel_id
      leagues = r.leagues ?? null
    } catch (e) {
      error = errorText(e)
    } finally {
      saving = false
    }
  }
  function toggle(l: SportsLeague, checked: boolean) {
    const next = checked ? ALL.filter((x) => x === l || on.includes(x)) : on.filter((x) => x !== l)
    if (next.length) void save(current, next)
  }
  const nameOf = (id: string | null) => (id ? (app.state.channels[id]?.name ?? 'a channel') : null)
</script>

<div class="settings-field">
  <h3 class="settings-section-title">Sports channel</h3>
  <p class="hint">On match day a card goes here for each match of a team a member follows (Settings › Sports): kick-off in everyone's own time, then the score as it changes, edited in place, and full time.</p>
  {#if owner}
    <label class="pick">Channel
      <select class="input input-sm" aria-label="Sports channel" disabled={!loaded || saving} value={current ?? ''} onchange={(e) => void save(e.currentTarget.value || null)}>
        <option value="">None (no cards)</option>
        {#each textChannels as c (c.id)}<option value={c.id}>#{c.name}</option>{/each}
      </select>
    </label>
    {#if current}
      <p class="sub-title">Leagues that post here</p>
      {#each ALL as l (l)}
        <!-- the last league on can't be turned off: no channel is how to have none -->
        <label class="row">
          <input type="checkbox" role="switch" class="switch" data-league={l} checked={on.includes(l)} disabled={saving || (on.length === 1 && on.includes(l))}
            onchange={(e) => toggle(l, e.currentTarget.checked)} />
          <span>{leagueName(l)}</span>
        </label>
      {/each}
    {/if}
  {:else if loaded}
    <p class="now">{current ? `Cards go to #${nameOf(current)}.${leagues ? ` Only ${leagues.map(leagueName).join(', ')}.` : ''}` : 'No channel: no cards are posted.'} Only the server's owner can change it.</p>
  {/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
  p { margin: 0; color: var(--text-2); font-size: 13px; }
  .pick, .row { display: flex; align-items: center; gap: 8px; font-weight: 600; }
  .sub-title { margin-top: 4px; font-weight: 600; color: var(--text); }
  .now { color: var(--text); }
  .error { color: var(--danger-text); }
</style>
