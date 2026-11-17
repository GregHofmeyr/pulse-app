<script lang="ts">
  // A sports card (#385), a system line in a server's Sports channel: one per match of a team members follow, edited
  // in place from match day (kick-off in your own time) through the live score to full time (with the scorers, for
  // football). Who follows either team is named under it. Each team's logo shows beside its name (#406, the server's
  // copy), and a card whose window passed with no live data says so rather than look stuck (#406).
  import type { SportsCard } from '../lib/protocol/SportsCard'
  import { NO_UPDATES, ballOf, joinNames, leagueName, logoUrl, phaseText, scorerText } from '../lib/sports'

  let { card, nameOf, time, messageId }: { card: SportsCard; nameOf: (userId: string) => string; time: string; messageId: string } = $props()
  const corner = $derived(phaseText(card, Date.now()))
  const scored = $derived(card.phase !== 'upcoming' && card.home.score != null && card.away.score != null)
  const homeScorers = $derived(card.scorers.filter((s) => s.home))
  const awayScorers = $derived(card.scorers.filter((s) => !s.home))
  const followers = $derived(card.followers.map(nameOf))
  const logos = $derived({ home: logoUrl(card.home.logo), away: logoUrl(card.away.logo) })
  // a logo that won't load just goes
  let failed = $state<{ home?: boolean; away?: boolean }>({})
  const winner = $derived(card.phase === 'final' && scored ? (card.home.score! > card.away.score! ? 'home' : card.away.score! > card.home.score! ? 'away' : null) : null)
</script>

<section class="sports" data-message-id={messageId} data-phase={card.phase} aria-label="{card.home.name} v {card.away.name}">
  <div class="head">
    <span aria-hidden="true">{ballOf(card.league)}</span> {leagueName(card.league)}
    <span class="corner" class:live={card.phase === 'live' && !card.no_updates} data-corner>{corner}</span>
    <span class="time">{time}</span>
  </div>
  <div class="sides">
    <div class="side" class:won={winner === 'home'} data-side="home">
      {#if logos.home && !failed.home}<img class="logo" src={logos.home} alt="" onerror={() => (failed.home = true)} />{/if}
      <span class="team">{card.home.name}</span>
      {#if scored}<span class="score">{card.home.score}</span>{/if}
    </div>
    <div class="side" class:won={winner === 'away'} data-side="away">
      {#if logos.away && !failed.away}<img class="logo" src={logos.away} alt="" onerror={() => (failed.away = true)} />{/if}
      <span class="team">{card.away.name}</span>
      {#if scored}<span class="score">{card.away.score}</span>{/if}
    </div>
  </div>
  {#if card.scorers.length}
    <div class="scorers" aria-label="Scorers">
      <span>{homeScorers.map(scorerText).join(', ')}</span>
      <span class="away">{awayScorers.map(scorerText).join(', ')}</span>
    </div>
  {/if}
  {#if card.no_updates}<p class="stale" data-no-updates>{NO_UPDATES}</p>{/if}
  {#if followers.length}<p class="followers">Followed by {joinNames(followers)}</p>{/if}
</section>

<style>
  .sports { margin: 6px 18px 6px 30px; max-width: 420px; padding: 10px 12px; border-radius: var(--radius-md); background: var(--bg-2); border: 1px solid var(--line); color: var(--text-body); font-size: 14px; }
  .head { display: flex; align-items: center; gap: 6px; color: var(--text-2); font-size: 13px; }
  .corner { margin-left: auto; font-size: 12px; font-weight: 700; color: var(--text-2); border-radius: 999px; padding: 1px 8px; background: var(--bg-3); }
  .corner.live { background: var(--accent); color: var(--on-accent); }
  .time { color: var(--text-3); font-size: 12px; }
  .sides { display: flex; flex-direction: column; gap: 4px; margin-top: 8px; }
  .side { display: flex; align-items: center; gap: 8px; }
  .team { flex: 1; min-width: 0; color: var(--text); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .score { flex: none; font-size: 18px; font-weight: 700; color: var(--text); font-variant-numeric: tabular-nums; }
  .side.won .score { color: var(--ok-text); }
  .scorers { display: flex; justify-content: space-between; gap: 12px; margin-top: 6px; color: var(--text-2); font-size: 12px; }
  .scorers .away { text-align: right; }
  .followers { margin: 6px 0 0; color: var(--text-3); font-size: 12px; }
  .logo { width: 20px; height: 20px; flex: none; object-fit: contain; }
  .stale { margin: 6px 0 0; color: var(--text-2); font-size: 12px; }
</style>
