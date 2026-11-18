<script lang="ts" module>
  // each league's team list for this session: the server caches them too, so a league is asked for once
  const teamCache = new Map<string, SportsTeam[]>()
  /** Tests start clean. */
  export const clearTeamCache = () => teamCache.clear()
</script>

<script lang="ts">
  // Settings › Sports (#385): "Sports score pings" (on by default), the teams you follow, and a team search per league
  // (PSL, URC, Premier League). On match day a followed team's match gets a card in the Sports channel of your servers
  // that have one. When the server has no API-Sports key, sports aren't set up: nothing to follow.
  import Select from './Select.svelte'
  import { errorText, sportsCmd } from '../lib/tauri'
  import { leagueName, matchTeams, sameTeam } from '../lib/sports'
  import type { SportsLeague } from '../lib/protocol/SportsLeague'
  import type { SportsStatus } from '../lib/protocol/SportsStatus'
  import type { SportsTeam } from '../lib/protocol/SportsTeam'
  const uid = $props.id()

  let status = $state<SportsStatus | null>(null)
  let unavailable = $state(false)
  let busy = $state(false)
  let error = $state('')
  $effect(() => {
    let gone = false
    sportsCmd.status().then(
      (s) => !gone && (status = s),
      () => !gone && (unavailable = true),
    )
    return () => (gone = true)
  })

  let league = $state<SportsLeague>('psl')
  let query = $state('')
  let teams = $state<SportsTeam[] | null>(null)
  let loadingTeams = $state(false)
  $effect(() => {
    if (!status?.available) return
    const l = league
    const cached = teamCache.get(l)
    if (cached) {
      teams = cached
      return
    }
    let gone = false
    teams = null
    loadingTeams = true
    sportsCmd.teams(l).then(
      (r) => {
        teamCache.set(l, r.teams)
        if (!gone) teams = r.teams
      },
      (e) => !gone && (error = errorText(e)),
    ).finally(() => !gone && (loadingTeams = false))
    return () => (gone = true)
  })
  const found = $derived(teams ? matchTeams(teams, query).slice(0, 8) : [])
  const following = (t: SportsTeam) => !!status?.follows.some((f) => sameTeam(f, t))

  async function act(call: () => Promise<SportsStatus>) {
    busy = true
    error = ''
    try {
      status = await call()
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
</script>

<div class="settings-field sports">
  <h3 class="settings-section-title">Sports</h3>
  {#if unavailable}
    <p class="hint">Sports aren't available on this server.</p>
  {:else if status && !status.available}
    <p class="hint" data-not-set-up>Sports aren't set up on this server: its owner needs an API-Sports key.</p>
  {:else if status}
    <label class="row">
      <input type="checkbox" role="switch" class="switch" data-switch="sports-pings" checked={status.pings} disabled={busy} aria-describedby="{uid}-pings-help"
        onchange={(e) => void act(() => sportsCmd.setPings(e.currentTarget.checked))} />
      <span>Sports score pings</span>
    </label>
    <small id="{uid}-pings-help">On match day, a team you follow gets a card in the Sports channel of your servers that have one: kick-off, the score as it changes and full time, and you're named as following it. Off, your teams bring no cards.</small>
    {#if status.note}<p class="note" role="status">{status.note}</p>{/if}

    <h4 class="sub-title">Teams you follow</h4>
    {#if status.follows.length}
      <ul class="settings-list">
        {#each status.follows as t (`${t.league}:${t.team_id}`)}
          <li class="settings-list-item">
            <span class="who"><span class="nm">{t.name}</span><small>{leagueName(t.league)}</small></span>
            <button type="button" class="btn btn-secondary" disabled={busy} aria-label="Unfollow {t.name}" onclick={() => void act(() => sportsCmd.unfollow(t.league, t.team_id))}>Unfollow</button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="hint">None yet.</p>
    {/if}

    <h4 class="sub-title">Follow a team</h4>
    <div class="search">
      <Select size="sm" label="League" value={league} options={status.leagues.map((l) => ({ value: l, label: leagueName(l) }))} onchange={(v) => (league = v as SportsLeague)} />
      <input class="input input-sm" type="search" aria-label="Search teams" placeholder="Search teams" bind:value={query} maxlength="60" />
    </div>
    {#if loadingTeams}
      <p class="hint">Loading teams…</p>
    {:else if teams && !teams.length}
      <p class="hint">No teams for this league right now.</p>
    {:else if teams}
      <ul class="settings-list" aria-label="Teams">
        {#each found as t (t.team_id)}
          <li class="settings-list-item">
            <span class="who"><span class="nm">{t.name}</span></span>
            {#if following(t)}
              <span class="followed">Following</span>
            {:else}
              <button type="button" class="btn btn-primary" disabled={busy} aria-label="Follow {t.name}" onclick={() => void act(() => sportsCmd.follow(t.league, t.team_id))}>Follow</button>
            {/if}
          </li>
        {:else}
          <li class="hint">No team matches.</li>
        {/each}
      </ul>
    {/if}
  {/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
  p { margin: 0; color: var(--text-2); font-size: 13px; }
  .sports { display: flex; flex-direction: column; gap: 6px; }
  .row { display: flex; align-items: center; gap: 10px; font-weight: 600; }
  .note { color: var(--warn-text); }
  .sub-title { margin: 8px 0 0; font-size: 13px; font-weight: 600; }
  .search { display: flex; gap: 6px; flex-wrap: wrap; }
  .search input { flex: 1; min-width: 160px; }
  .search :global(.select) { min-width: 160px; }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .nm { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .who small { color: var(--text-3); }
  .followed { color: var(--text-3); font-size: 13px; }
  li.hint { list-style: none; color: var(--text-3); font-size: 13px; }
  .error { color: var(--danger-text); }
</style>
