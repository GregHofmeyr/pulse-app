<script lang="ts">
  // A Deadlock card (#390), a system line in a server's Deadlock channel: a match (one row per linked friend in it:
  // hero art, K/D/A, win or loss, rank change), the weekly rank movers, or a patch (the start of its notes, the heroes
  // it names, a link). Pictures are the server's copies: no third party is contacted.
  import type { DeadlockCard } from '../lib/protocol/DeadlockCard'
  import { thumbUrl } from '../lib/achievements'
  import { deltaText, durationText, kdaText, rankText, stepsText } from '../lib/deadlock'

  let { card, nameOf, time, messageId }: { card: DeadlockCard; nameOf: (userId: string) => string; time: string; messageId: string } = $props()
</script>

{#if card.kind === 'match'}
  <section class="dl match" data-message-id={messageId} aria-label="Deadlock match">
    <div class="head"><span aria-hidden="true">⚔️</span> Deadlock match · {durationText(card.duration_s)} <span class="time">{time}</span></div>
    <ul>
      {#each card.players as p (p.user_id)}
        {@const art = thumbUrl(p.hero_art)}
        {@const badge = thumbUrl(p.rank?.image)}
        <li class:won={p.won} data-result={p.won ? 'win' : 'loss'}>
          {#if art}<img class="hero" src={art} alt="" />{:else}<span class="hero none" aria-hidden="true">⚔️</span>{/if}
          <span class="what">
            <span><strong class="who">{nameOf(p.user_id)}</strong> as <span class="hero-name">{p.hero}</span></span>
            <span class="kda">{kdaText(p.kills, p.deaths, p.assists)}</span>
          </span>
          {#if p.rank}
            <span class="rank" data-tip={rankText(p.rank)}>
              {#if badge}<img src={badge} alt="" />{/if}
              <span>{rankText(p.rank)}{#if p.rank_delta != null}{' '}<span class="delta">{deltaText(p.rank_delta)}</span>{/if}</span>
            </span>
          {/if}
          <span class="result">{p.won ? 'Win' : 'Loss'}</span>
        </li>
      {/each}
    </ul>
  </section>
{:else if card.kind === 'rank_movers'}
  <section class="dl movers" data-message-id={messageId} aria-label="Deadlock rank movers">
    <div class="head"><span aria-hidden="true">📈</span> Rank movers this week <span class="time">{time}</span></div>
    <ul>
      {#each card.movers as m (m.user_id)}
        {@const badge = thumbUrl(m.to.image)}
        <li data-up={m.steps > 0}>
          {#if badge}<img class="badge" src={badge} alt="" />{:else}<span class="badge none" aria-hidden="true"></span>{/if}
          <span class="what"><strong class="who">{nameOf(m.user_id)}</strong> <span class="move">{rankText(m.from)} → {rankText(m.to)}</span></span>
          <span class="steps" class:up={m.steps > 0}>{stepsText(m.steps)}</span>
        </li>
      {/each}
    </ul>
  </section>
{:else if card.kind === 'patch'}
  <section class="dl patch" data-message-id={messageId} aria-label="Deadlock update: {card.title}">
    <div class="head"><span aria-hidden="true">📜</span> Deadlock update <span class="time">{time}</span></div>
    <a class="title" href={card.link} target="_blank" rel="noopener noreferrer">{card.title}</a>
    {#if card.summary}<p class="summary">{card.summary}</p>{/if}
    {#if card.heroes.length}
      <div class="heroes">
        {#each card.heroes as h (h.name)}
          {@const icon = thumbUrl(h.icon)}
          <span class="hero-chip" data-tip={h.name}>{#if icon}<img src={icon} alt="" />{/if}<span>{h.name}</span></span>
        {/each}
      </div>
    {/if}
  </section>
{/if}

<style>
  .dl { margin: 6px 18px 6px 30px; max-width: 520px; padding: 10px 12px; border-radius: var(--radius-md); background: var(--bg-2); border: 1px solid var(--line); color: var(--text-body); font-size: 14px; }
  .head { color: var(--text-2); font-size: 13px; }
  .time { color: var(--text-3); font-size: 12px; margin-left: 4px; }
  ul { list-style: none; margin: 8px 0 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  li { display: flex; align-items: center; gap: 10px; }
  .hero { width: 44px; height: 44px; border-radius: var(--radius-sm); flex: none; object-fit: cover; display: block; }
  .hero.none, .badge.none { display: grid; place-items: center; background: var(--bg-3); }
  .what { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .who { color: var(--text); }
  .hero-name { color: var(--text); font-weight: 600; }
  .kda { color: var(--text-2); font-size: 13px; font-variant-numeric: tabular-nums; }
  .rank { display: flex; align-items: center; gap: 4px; color: var(--text-2); font-size: 12px; white-space: nowrap; }
  .rank img { width: 24px; height: 24px; object-fit: contain; }
  .delta { color: var(--text-3); }
  .result { flex: none; font-size: 12px; font-weight: 700; border-radius: 999px; padding: 1px 8px; color: var(--danger-text); background: var(--danger-soft); }
  li.won .result { color: var(--ok-text); background: var(--ok-soft); }
  .badge { width: 32px; height: 32px; flex: none; object-fit: contain; border-radius: var(--radius-sm); }
  .move { color: var(--text-2); font-size: 13px; }
  .steps { flex: none; font-size: 12px; font-weight: 700; color: var(--danger-text); }
  .steps.up { color: var(--ok-text); }
  .title { display: block; margin-top: 6px; color: var(--text); font-weight: 700; text-decoration: none; }
  .title:hover { text-decoration: underline; }
  .summary { margin: 4px 0 0; color: var(--text-2); font-size: 13px; line-height: 1.4; }
  .heroes { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 8px; }
  .hero-chip { display: inline-flex; align-items: center; gap: 4px; padding: 2px 8px 2px 2px; border-radius: 999px; background: var(--bg-3); color: var(--text-2); font-size: 12px; }
  .hero-chip img { width: 20px; height: 20px; border-radius: 50%; object-fit: cover; }
</style>
