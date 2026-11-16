<script lang="ts">
  // A member's Deadlock section on their profile card (#390): rank badge, last match, most-played heroes, their games
  // with you and their best duo in this server. Read when the card opens; nothing shows when there's nothing (not
  // linked, nothing shared, deadlock-api unreachable, an older server). Pictures are the server's copies.
  import { app } from '../lib/store.svelte'
  import { api } from '../lib/tauri'
  import { displayName } from '../lib/selectors'
  import { thumbUrl } from '../lib/achievements'
  import { kdaText, rankText, winRateText } from '../lib/deadlock'
  import type { DeadlockProfile } from '../lib/protocol/DeadlockProfile'

  let { userId, serverId }: { userId: string; serverId: string } = $props()
  let got = $state<DeadlockProfile | null>(null)
  $effect(() => {
    const [s, u] = [serverId, userId]
    let gone = false
    got = null
    api.deadlockProfile(s, u).then(
      (p) => !gone && (got = p),
      () => {},
    )
    return () => (gone = true)
  })
  const any = $derived(!!got && (!!got.rank || !!got.last_match || got.heroes.length > 0 || !!got.with_you || !!got.best_duo))
  const share = (wins: number, games: number) => (games > 0 ? `${Math.round((wins / games) * 100)} %` : '')
</script>

{#if got && any}
  <div class="deadlock" data-deadlock-panel>
    <div class="title">Deadlock</div>
    {#if got.rank}
      {@const badge = thumbUrl(got.rank.image)}
      <div class="line rank" data-deadlock-rank>{#if badge}<img class="badge" src={badge} alt="" />{/if}<span>{rankText(got.rank)}</span></div>
    {/if}
    {#if got.last_match}
      {@const m = got.last_match}
      {@const icon = thumbUrl(m.hero_icon)}
      <div class="line" data-deadlock-last>
        {#if icon}<img class="icon" src={icon} alt="" />{/if}
        <span>Last: {m.hero} · {kdaText(m.kills, m.deaths, m.assists)} · <span class:won={m.won} class="res">{m.won ? 'Win' : 'Loss'}</span></span>
      </div>
    {/if}
    {#if got.heroes.length}
      <div class="heroes" data-deadlock-heroes>
        {#each got.heroes as h (h.hero)}
          {@const icon = thumbUrl(h.icon)}
          <span class="chip" data-tip="{h.matches} games · {share(h.wins, h.matches)} won">{#if icon}<img src={icon} alt="" />{/if}{h.hero}</span>
        {/each}
      </div>
    {/if}
    {#if got.with_you}
      <div class="line" data-deadlock-with-you><span class="k">With you</span><span>{winRateText(got.with_you.wins, got.with_you.matches)}</span></div>
    {/if}
    {#if got.best_duo}
      <div class="line" data-deadlock-duo><span class="k">Best duo</span><span>{displayName(app.state, serverId, got.best_duo.user_id)} · {winRateText(got.best_duo.wins, got.best_duo.matches)}</span></div>
    {/if}
  </div>
{/if}

<style>
  .deadlock { border-radius: var(--radius-md); padding: 10px 12px; background: var(--card-panel); font-size: 13px; display: flex; flex-direction: column; gap: 6px; }
  .title { font-size: 11px; font-weight: 700; letter-spacing: .06em; text-transform: uppercase; color: var(--card-text-2); }
  .line { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .line .k { color: var(--card-text-2); flex: none; }
  .line .k + span { margin-left: auto; text-align: right; }
  .badge { width: 28px; height: 28px; object-fit: contain; }
  .rank { font-weight: 600; }
  .icon { width: 20px; height: 20px; border-radius: var(--radius-xs); object-fit: cover; }
  .res { color: var(--card-danger); }
  .res.won { color: var(--card-ok); }
  .heroes { display: flex; flex-wrap: wrap; gap: 6px; }
  .chip { display: inline-flex; align-items: center; gap: 4px; padding: 2px 8px 2px 2px; border-radius: 999px; background: var(--card-hover); }
  .chip img { width: 18px; height: 18px; border-radius: 50%; object-fit: cover; }
</style>
