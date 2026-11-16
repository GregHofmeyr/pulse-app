<script lang="ts">
  // A Steam achievement card (#379), a system line in a server's Achievements channel: someone's new unlocks in one
  // game (icon, name, description, how rare), or the bigger 100 % card with the game's art. Pictures are the server's
  // copies: Valve is never contacted.
  import type { AchievementCard } from '../lib/protocol/AchievementCard'
  import { percentText, progressShare, progressText, rarityOf, thumbUrl } from '../lib/achievements'

  let { card, name, time, messageId }: { card: AchievementCard; name: string; time: string; messageId: string } = $props()
  const art = $derived(thumbUrl(card.art))
  const share = $derived(progressShare(card.unlocked, card.total))
</script>

{#if card.complete}
  <section class="ach complete" data-message-id={messageId} aria-label="{name} completed {card.game}">
    {#if art}<img class="art" src={art} alt="" />{/if}
    <div class="body">
      <span class="big" aria-hidden="true">💯</span>
      <div class="text">
        <strong class="headline">100 % complete</strong>
        <span><span class="who">{name}</span> got every achievement in <span class="game">{card.game}</span></span>
        <span class="meta">{card.total} of {card.total} <span class="time">{time}</span></span>
      </div>
    </div>
  </section>
{:else}
  <section class="ach" data-message-id={messageId} aria-label="{name} unlocked achievements in {card.game}">
    <div class="head"><span aria-hidden="true">🏆</span> <span class="who">{name}</span> unlocked in <span class="game">{card.game}</span> <span class="time">{time}</span></div>
    <ul>
      {#each card.achievements as a, i (i)}
        {@const icon = thumbUrl(a.icon)}
        {@const rarity = rarityOf(a.global_percent)}
        <li>
          {#if icon}<img class="icon" src={icon} alt="" />{:else}<span class="icon none" aria-hidden="true">🏆</span>{/if}
          <span class="what">
            <strong>{a.name}</strong>
            {#if a.description}<span class="desc">{a.description}</span>{/if}
          </span>
          {#if a.global_percent != null}
            <span class="rarity" data-rarity={rarity} style:--r="var(--rarity-{rarity})">{percentText(a.global_percent)}</span>
          {/if}
        </li>
      {/each}
    </ul>
    {#if card.more > 0}<p class="more">and {card.more} more</p>{/if}
    {#if card.total > 0}
      <div class="progress" aria-label="{progressText(card.unlocked, card.total)}">
        <span class="bar"><span class="fill" style:transform="scaleX({share})"></span></span>
        <span class="count">{progressText(card.unlocked, card.total)}</span>
      </div>
    {/if}
  </section>
{/if}

<style>
  .ach { margin: 6px 18px 6px 30px; max-width: 480px; padding: 10px 12px; border-radius: var(--radius-md); background: var(--bg-2); border: 1px solid var(--line); color: var(--text-body); font-size: 14px; }
  .head { color: var(--text-2); font-size: 13px; }
  .who, .game { color: var(--text); font-weight: 600; }
  .time { color: var(--text-3); font-size: 12px; margin-left: 4px; }
  ul { list-style: none; margin: 8px 0 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  li { display: flex; align-items: center; gap: 10px; }
  .icon { width: 40px; height: 40px; border-radius: var(--radius-sm); flex: none; object-fit: cover; display: block; }
  .icon.none { display: grid; place-items: center; background: var(--bg-3); font-size: 20px; }
  .what { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .what strong { color: var(--text); }
  .desc { color: var(--text-2); font-size: 13px; overflow: hidden; text-overflow: ellipsis; }
  .rarity { flex: none; font-size: 12px; font-weight: 600; color: var(--r, var(--text-2)); border: 1px solid var(--r, var(--line)); border-radius: 999px; padding: 1px 8px; white-space: nowrap; }
  .more { margin: 6px 0 0; color: var(--text-2); font-size: 13px; }
  .progress { margin-top: 10px; display: flex; align-items: center; gap: 8px; }
  .bar { flex: 1; height: 6px; border-radius: 3px; background: var(--bg-4); overflow: hidden; }
  .fill { display: block; height: 100%; background: var(--accent); transform-origin: left center; }
  .count { color: var(--text-2); font-size: 12px; white-space: nowrap; }
  /* the 100 % moment: bigger, gold, the game's art behind it */
  .complete { position: relative; overflow: hidden; padding: 0; border: 2px solid var(--first-gold); background: var(--bg-2); }
  .art { display: block; width: 100%; aspect-ratio: 460 / 215; object-fit: cover; }
  .complete .body { display: flex; align-items: center; gap: 12px; padding: 12px 14px; background: var(--first-gold-soft); }
  .big { font-size: 36px; line-height: 1; }
  .text { display: flex; flex-direction: column; gap: 2px; }
  .headline { font-size: 18px; color: var(--first-gold); }
  .meta { color: var(--text-2); font-size: 13px; }
</style>
