<script lang="ts">
  // An F1 card (#386), a system line in a server's F1 channel: the race weekend card (F1WeekendCard), the "lights out
  // in 1 hour" line, the podium and results card (the top 10, the fastest lap, the standings after the round), or the
  // predictions' scores with the season's table. With "F1 race weekends" off (Settings › Chat, this device), each
  // folds to one line that opens on a click.
  import F1WeekendCard from './F1WeekendCard.svelte'
  import { driverById, driverShort, flagOf, ordinal, pointsText, sessionWhen } from '../lib/f1'
  import { f1Prefs } from '../lib/f1Prefs.svelte'
  import { jumpTo } from '../lib/jump'
  import type { F1Card } from '../lib/protocol/F1Card'
  import type { Message } from '../lib/protocol/Message'

  let { message, nameOf, time }: { message: Message; nameOf: (userId: string | null) => string; time: string } = $props()
  const uid = $props.id()
  const card = $derived(message.f1 as F1Card)
  let opened = $state(false)
  // a pick's driver by name (the podium's, else the weekend's field)
  const nameIn = (id: string): string => (card.kind === 'scores' ? (driverById([...card.podium, ...card.drivers], id)?.name ?? id) : id)
  const folded = $derived(!f1Prefs.on && !opened)

  const summary = $derived.by(() => {
    switch (card.kind) {
      case 'weekend':
        return `🏁 Race weekend: ${card.weekend.race_name}`
      case 'lights_out':
        return `🚦 Lights out in 1 hour: ${card.race_name}`
      case 'results':
        return `🏆 ${card.race_name}: ${card.results.slice(0, 3).map((r) => driverShort(r.driver)).join(', ')}`
      case 'scores':
        return `🔮 Podium predictions, ${card.race_name}`
    }
  })
</script>

{#if folded}
  <div class="system folded" data-message-id={message.id}>{summary} · <button type="button" class="link-btn" onclick={() => (opened = true)}>Show</button> <span class="time">{time}</span></div>
{:else if card.kind === 'weekend'}
  <F1WeekendCard {message} {card} {nameOf} {time} />
{:else if card.kind === 'lights_out'}
  <div class="system line" data-message-id={message.id}>🚦 Lights out in 1 hour: {card.race_name} ({sessionWhen(card.race_at)})
    {#if card.weekend_message_id}{@const wid = card.weekend_message_id} · <button type="button" class="link-btn" onclick={() => void jumpTo(message.channel_id, wid)}>Your picks</button>{/if}
    <span class="time">{time}</span></div>
{:else if card.kind === 'results'}
  {@const podium = card.results.slice(0, 3)}
  <section class="f1" data-message-id={message.id} aria-labelledby="{uid}-t">
    <header>
      <span class="flag" aria-hidden="true">{flagOf(card.country)}</span>
      <p class="title" id="{uid}-t">{card.race_name} <span class="kind">Results</span></p>
      <span class="time">{time}</span>
    </header>
    <ol class="podium" aria-label="Podium">
      {#each [1, 0, 2] as i (i)}
        {@const r = podium[i]}
        {#if r}
          <li class="step p{r.position}">
            <span class="code">{driverShort(r.driver)}</span>
            <span class="name">{r.driver.name}</span>
            {#if r.driver.team}<span class="team">{r.driver.team}</span>{/if}
            <span class="block">{ordinal(r.position)}</span>
          </li>
        {/if}
      {/each}
    </ol>
    {#if card.results.length > 3}
      <table class="rows">
        <tbody>
          {#each card.results.slice(3) as r (r.position)}
            <tr><td class="pos">{r.position}</td><td class="who">{r.driver.name}{#if r.driver.team}<span class="team"> {r.driver.team}</span>{/if}</td><td class="gap">{r.time}</td><td class="pts">{r.points}</td></tr>
          {/each}
        </tbody>
      </table>
    {/if}
    {#if card.fastest_lap}
      {@const f = card.fastest_lap}
      <p class="fast">⏱ Fastest lap: <strong>{f.driver.name}</strong> {f.time}{f.lap ? ` (lap ${f.lap})` : ''}</p>
    {/if}
    {#if card.standings.length}
      <details class="standings">
        <summary>Drivers' standings after round {card.round}</summary>
        <table class="rows">
          <tbody>
            {#each card.standings as s (s.driver.id)}
              <tr><td class="pos">{s.position}</td><td class="who">{s.driver.name}{#if s.driver.team}<span class="team"> {s.driver.team}</span>{/if}</td><td class="gap">{s.wins ? `${s.wins} ${s.wins === 1 ? 'win' : 'wins'}` : ''}</td><td class="pts">{s.points}</td></tr>
            {/each}
          </tbody>
        </table>
      </details>
    {/if}
  </section>
{:else if card.kind === 'scores'}
  <section class="f1" data-message-id={message.id} aria-labelledby="{uid}-t">
    <header>
      <span class="flag" aria-hidden="true">🔮</span>
      <p class="title" id="{uid}-t">Podium predictions <span class="kind">{card.race_name}</span></p>
      <span class="time">{time}</span>
    </header>
    <p class="real">The podium: {card.podium.map((d, i) => `${i + 1}. ${d.name}`).join(' · ')}</p>
    <ul class="scores">
      {#each card.scores as sc (sc.user_id)}
        <li>
          <span class="name">{nameOf(sc.user_id)}</span>
          <span class="their">{#each sc.picks as id, i (i)}<span class="code" class:hit={card.podium[i]?.id === id} class:near={card.podium[i]?.id !== id && card.podium.some((d) => d.id === id)} data-tip={nameIn(id)}>{driverShort(driverById([...card.podium, ...card.drivers], id), id)}</span>{/each}</span>
          <span class="pts">{pointsText(sc.points)}</span>
        </li>
      {/each}
    </ul>
    {#if card.season_table.length}
      <p class="sub">Season {card.season}</p>
      <ol class="season">
        {#each card.season_table as row, i (row.user_id)}
          <li><span class="rank">{i + 1}</span><span class="name">{nameOf(row.user_id)}</span><span class="races">{row.races} {row.races === 1 ? 'race' : 'races'}</span><span class="pts">{pointsText(row.points)}</span></li>
        {/each}
      </ol>
    {/if}
    {#if card.weekend_message_id}
      {@const wid = card.weekend_message_id}
      <p class="foot"><button type="button" class="link-btn" onclick={() => void jumpTo(message.channel_id, wid)}>See everyone's picks</button></p>
    {/if}
  </section>
{/if}

<style>
  .system { padding: 6px 18px 6px 30px; font-size: 13px; color: var(--text-3); }
  .time { color: var(--text-3); font-size: 12px; margin-left: 4px; }
  .system .link-btn { color: var(--text-2); }
  .system .link-btn:hover { color: var(--text); }
  .f1 { margin: 6px 18px 6px 30px; max-width: 480px; padding: 12px; border-radius: var(--radius-lg); background: var(--bg-2); border: 1px solid var(--line); color: var(--text-body); font-size: 14px; display: flex; flex-direction: column; gap: 10px; }
  header { display: flex; align-items: center; gap: 10px; }
  .flag { font-size: 24px; line-height: 1; flex: none; }
  .title { flex: 1; min-width: 0; margin: 0; font-weight: 700; font-size: 15px; color: var(--text); overflow-wrap: anywhere; }
  .kind { font-weight: 400; color: var(--text-2); margin-left: 4px; }
  /* the podium: P2, P1, P3 on steps, the winner in gold */
  .podium { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(3, 1fr); align-items: end; gap: 6px; }
  .step { display: flex; flex-direction: column; align-items: center; gap: 2px; text-align: center; min-width: 0; }
  .step .code { font-weight: 800; font-size: 16px; color: var(--text); }
  .step .name { font-size: 12px; color: var(--text); max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .step .team { font-size: 11px; color: var(--text-3); max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .block { align-self: stretch; margin-top: 4px; display: grid; place-items: center; border-radius: var(--radius-sm) var(--radius-sm) 0 0; background: var(--bg-3); border: 1px solid var(--line); font-weight: 700; font-size: 12px; color: var(--text-2); }
  .p1 .block { height: 52px; background: var(--first-gold-soft); border-color: var(--first-gold); color: var(--first-gold); }
  .p2 .block { height: 38px; }
  .p3 .block { height: 28px; }
  .rows { width: 100%; border-collapse: collapse; font-size: 13px; }
  .rows td { padding: 2px 4px; color: var(--text-2); vertical-align: baseline; }
  .rows .pos { width: 24px; color: var(--text-3); font-variant-numeric: tabular-nums; }
  .rows .who { color: var(--text); }
  .rows .team { color: var(--text-3); font-size: 12px; }
  .rows .gap, .rows .pts { text-align: right; white-space: nowrap; font-variant-numeric: tabular-nums; }
  .rows .pts { width: 36px; color: var(--text); font-weight: 600; }
  .fast { margin: 0; font-size: 13px; color: var(--text-2); }
  .fast strong { color: var(--text); }
  .standings summary { cursor: pointer; font-size: 13px; font-weight: 600; color: var(--text-2); }
  .standings[open] summary { margin-bottom: 4px; }
  .real { margin: 0; font-size: 13px; color: var(--text-2); }
  .scores, .season { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .scores li, .season li { display: flex; align-items: center; gap: 8px; font-size: 13px; }
  .name { color: var(--text); font-weight: 600; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .their { display: inline-flex; gap: 4px; margin-left: auto; }
  .code { font-size: 12px; font-weight: 700; padding: 1px 6px; border-radius: var(--radius-sm); background: var(--bg-3); color: var(--text-3); }
  .scores .code.hit { background: var(--ok-soft); color: var(--ok-text); }
  .scores .code.near { background: var(--warn-soft); color: var(--warn-text); }
  .pts { flex: none; min-width: 44px; text-align: right; font-weight: 700; color: var(--text); font-variant-numeric: tabular-nums; }
  .sub { margin: 4px 0 0; font-size: 12px; font-weight: 700; color: var(--text-3); text-transform: uppercase; }
  .rank { width: 16px; color: var(--text-3); font-variant-numeric: tabular-nums; }
  .races { margin-left: auto; color: var(--text-3); font-size: 12px; }
  .foot { margin: 0; font-size: 12px; }
</style>
