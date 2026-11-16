<script lang="ts">
  // The F1 countdown banner (#386), atop a server's F1 channel: the next race weekend (flag, race, circuit), the
  // countdown to lights out, the next session in your own time (every session on a click), and the season's prediction
  // leaders. The server reads Jolpica-F1 and caches it; this asks the server every few minutes at most. Hidden with
  // "F1 race weekends" off, outside the F1 channel, and between seasons.
  import { SESSION_LABELS, countdown, flagOf, placeLine, pointsText, sessionWhen } from '../lib/f1'
  import { f1Prefs, f1Status, loadF1Status } from '../lib/f1.svelte'
  import { clock } from '../lib/store.svelte'
  import { serverNow } from '../lib/serverClock'

  let { serverId, channelId, nameOf }: { serverId: string; channelId: string; nameOf: (userId: string | null) => string } = $props()
  const uid = $props.id()

  // asked when the chat opens, and again once it's stale (the clock moves every 30 s)
  $effect(() => {
    void clock.nowIso
    if (f1Prefs.on) void loadF1Status(serverId)
  })
  const st = $derived(f1Status.byServer[serverId])
  const w = $derived(f1Prefs.on && st?.channel_id === channelId ? st.next : null)
  const now = $derived((void clock.nowIso, serverNow()))
  const left = $derived(w ? countdown(Date.parse(w.race_at), now) : null)
  const upcoming = $derived(w?.sessions.find((s) => Date.parse(s.starts_at) > now) ?? null)
  const leaders = $derived((st?.season_table ?? []).slice(0, 3))
  let open = $state(false)
</script>

{#if w && left}
  <div class="f1-banner" role="region" aria-labelledby="{uid}-t">
    <button type="button" class="row" aria-expanded={open} onclick={() => (open = !open)}>
      <span class="flag" aria-hidden="true">{flagOf(w.country)}</span>
      <span class="what">
        <span class="title" id="{uid}-t">{w.race_name}</span>
        <span class="where">{placeLine(w)}</span>
      </span>
      <span class="count"><span class="label">Lights out in</span> <strong>{left}</strong></span>
    </button>
    {#if upcoming && !open}<p class="next">Next: {SESSION_LABELS[upcoming.kind]} · <time datetime={upcoming.starts_at}>{sessionWhen(upcoming.starts_at)}</time></p>{/if}
    {#if open}
      <ul class="sessions">
        {#each w.sessions as s (s.kind)}
          <li class:past={Date.parse(s.starts_at) <= now} class:race={s.kind === 'race'}><span>{SESSION_LABELS[s.kind]}</span><time datetime={s.starts_at}>{sessionWhen(s.starts_at)}</time></li>
        {/each}
      </ul>
    {/if}
    {#if leaders.length}
      <p class="leaders">Predictions: {#each leaders as row, i (row.user_id)}{i ? ' · ' : ''}<span class="who">{nameOf(row.user_id)}</span> {pointsText(row.points)}{/each}</p>
    {/if}
  </div>
{/if}

<style>
  .f1-banner { margin: 8px 16px 0; padding: 8px 12px; border: 1px solid var(--line); border-radius: var(--radius-lg); background: var(--bg-2); display: flex; flex-direction: column; gap: 4px; }
  .row { display: flex; align-items: center; gap: 10px; width: 100%; padding: 0; border: 0; background: none; color: inherit; font: inherit; text-align: left; cursor: pointer; }
  .flag { font-size: 22px; line-height: 1; flex: none; }
  .what { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .title { font-weight: 700; font-size: 14px; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .where { font-size: 12px; color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .count { flex: none; font-size: 12px; color: var(--text-2); white-space: nowrap; }
  .count strong { font-size: 15px; color: var(--accent-text); font-variant-numeric: tabular-nums; }
  .next, .leaders { margin: 0; font-size: 12px; color: var(--text-3); }
  .next time { color: var(--text-2); }
  .leaders .who { color: var(--text-2); font-weight: 600; }
  .sessions { list-style: none; margin: 4px 0 0; padding: 0; display: flex; flex-direction: column; gap: 2px; font-size: 12px; }
  .sessions li { display: flex; justify-content: space-between; gap: 12px; color: var(--text-2); }
  .sessions time { color: var(--text); font-variant-numeric: tabular-nums; }
  .sessions .past, .sessions .past time { color: var(--text-3); }
  .sessions .race { font-weight: 700; }
  @media (max-width: 520px) { .count .label { display: none; } }
</style>
