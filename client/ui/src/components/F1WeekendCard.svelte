<script lang="ts">
  // The "race weekend" card (#386), a system line in a server's F1 channel: the race, its flag and circuit, the
  // countdown to lights out, every session in your own time, and podium predictions. Until lights out you pick P1, P2
  // and P3 from the field (or let the dice pick), and change or clear your picks; everyone's picks show below, with
  // their points once the race is scored. Picks from everyone land live (F1PredictionsChanged).
  import Avatar from './Avatar.svelte'
  import { SESSION_LABELS, countdown, driverById, driverShort, flagOf, picksOpen, picksReady, placeLine, pointsText, predictionOf, randomPodium, sessionWhen } from '../lib/f1'
  import { predictPodium } from '../lib/f1.svelte'
  import { app, clock } from '../lib/store.svelte'
  import { serverNow } from '../lib/serverClock'
  import { errorText } from '../lib/tauri'
  import type { F1WeekendCard } from '../lib/protocol/F1WeekendCard'
  import type { Message } from '../lib/protocol/Message'

  let { message, card, nameOf, time }: { message: Message; card: F1WeekendCard; nameOf: (userId: string | null) => string; time: string } = $props()
  const uid = $props.id()

  const w = $derived(card.weekend)
  // the clock moves every 30 s: the countdown and "closed" follow it
  const now = $derived((void clock.nowIso, serverNow()))
  const left = $derived(countdown(Date.parse(w.race_at), now))
  const open = $derived(picksOpen(w, now) && card.drivers.length >= 3)
  const me = $derived(app.state.me?.id ?? '')
  const mine = $derived(predictionOf(card.predictions, me))

  // the picks being made: yours as saved, until you change one
  let draft = $state<(string | null)[] | null>(null)
  const picks = $derived(draft ?? mine?.picks ?? [null, null, null])
  const changed = $derived(!!draft && draft.join() !== (mine?.picks ?? []).join())
  let busy = $state(false)
  let error = $state('')

  function setPick(i: number, id: string) {
    const next = [...picks]
    next[i] = id || null
    draft = next
  }
  function dice() {
    draft = randomPodium(card.drivers)
  }
  async function save(value: string[] | null) {
    if (busy) return
    busy = true
    error = ''
    try {
      await predictPodium(message, value)
      draft = null
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
  const label = (id: string) => driverShort(driverById(card.drivers, id), id)
  const fullName = (id: string) => driverById(card.drivers, id)?.name ?? id
</script>

<section class="f1 weekend" data-message-id={message.id} aria-labelledby="{uid}-t">
  <header>
    <span class="flag" aria-hidden="true">{flagOf(w.country)}</span>
    <div class="what">
      <p class="title" id="{uid}-t">{w.race_name}</p>
      <p class="where">{placeLine(w)}</p>
    </div>
    <span class="round">Round {w.round} <span class="time">{time}</span></span>
  </header>
  <p class="count" class:done={!left}>{#if left}Lights out in <strong>{left}</strong>{:else}Lights out{/if}</p>
  <ul class="sessions">
    {#each w.sessions as s (s.kind)}
      <li class:race={s.kind === 'race'} class:past={Date.parse(s.starts_at) <= now}><span>{SESSION_LABELS[s.kind]}</span><time datetime={s.starts_at}>{sessionWhen(s.starts_at)}</time></li>
    {/each}
  </ul>
  {#if card.drivers.length >= 3}
    <div class="predict" role="group" aria-label="Podium predictions">
      <p class="sub">Podium predictions <span class="hint">{open ? '3 points for the exact place, 1 for the podium. Closes at lights out.' : 'Closed at lights out.'}</span></p>
      {#if open}
        <div class="picks">
          {#each [0, 1, 2] as i (i)}
            <label class="pick"><span class="pos">P{i + 1}</span>
              <select class="input input-sm" aria-label="P{i + 1}" disabled={busy} value={picks[i] ?? ''} onchange={(e) => setPick(i, e.currentTarget.value)}>
                <option value="">Pick a driver</option>
                {#each card.drivers as d (d.id)}<option value={d.id} disabled={picks.includes(d.id) && picks[i] !== d.id}>{d.name}{d.team ? ` (${d.team})` : ''}</option>{/each}
              </select>
            </label>
          {/each}
        </div>
        <div class="actions">
          <button type="button" class="dice" data-tip="Pick three at random" aria-label="Pick three at random" disabled={busy} onclick={dice}>🎲</button>
          <button type="button" class="save" disabled={busy || !picksReady(picks) || (!changed && !!mine)} onclick={() => picksReady(picks) && void save(picks)}>{mine ? 'Update picks' : 'Save picks'}</button>
          {#if mine}<button type="button" class="clear" disabled={busy} onclick={() => void save(null)}>Clear mine</button>{/if}
        </div>
      {/if}
      {#if card.predictions.length}
        <ul class="who">
          {#each card.predictions as p (p.user_id)}
            <li class:me={p.user_id === me}>
              <Avatar id={p.user_id} name={nameOf(p.user_id)} size={18} />
              <span class="name">{nameOf(p.user_id)}</span>
              <span class="their" aria-label={p.picks.map((id, i) => `P${i + 1} ${fullName(id)}`).join(', ')}>{#each p.picks as id, i (i)}<span class="code" data-tip={fullName(id)}>{label(id)}</span>{/each}</span>
              {#if p.points != null}<span class="pts">{pointsText(p.points)}</span>{/if}
            </li>
          {/each}
        </ul>
      {:else if !open}
        <p class="none">Nobody predicted this one.</p>
      {/if}
      {#if error}<p class="err" role="alert">{error}</p>{/if}
    </div>
  {/if}
</section>

<style>
  .f1 { margin: 6px 18px 6px 30px; max-width: 480px; padding: 12px; border-radius: var(--radius-lg); background: var(--bg-2); border: 1px solid var(--line); color: var(--text-body); font-size: 14px; display: flex; flex-direction: column; gap: 10px; }
  header { display: flex; align-items: center; gap: 10px; }
  .flag { font-size: 28px; line-height: 1; flex: none; }
  .what { flex: 1; min-width: 0; }
  .title { margin: 0; font-weight: 700; font-size: 15px; color: var(--text); overflow-wrap: anywhere; }
  .where { margin: 2px 0 0; font-size: 13px; color: var(--text-2); overflow-wrap: anywhere; }
  .round { flex: none; align-self: flex-start; font-size: 12px; color: var(--text-3); white-space: nowrap; }
  .time { margin-left: 4px; }
  .count { margin: 0; font-size: 13px; color: var(--text-2); }
  .count strong { color: var(--accent-text); font-variant-numeric: tabular-nums; }
  .count.done { color: var(--text-3); }
  .sessions { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; font-size: 13px; }
  .sessions li { display: flex; justify-content: space-between; gap: 12px; color: var(--text-2); }
  .sessions time { font-variant-numeric: tabular-nums; color: var(--text); }
  .sessions .past, .sessions .past time { color: var(--text-3); }
  .sessions .race { font-weight: 700; }
  .predict { display: flex; flex-direction: column; gap: 8px; padding-top: 10px; border-top: 1px solid var(--line-soft); }
  .sub { margin: 0; font-weight: 700; font-size: 13px; color: var(--text); }
  .hint { display: block; font-weight: 400; font-size: 12px; color: var(--text-3); }
  .picks { display: flex; flex-direction: column; gap: 6px; }
  .pick { display: flex; align-items: center; gap: 8px; }
  .pick select { flex: 1; min-width: 0; }
  .pos { width: 24px; flex: none; font-weight: 700; font-size: 12px; color: var(--text-2); }
  .actions { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .actions button { min-height: 30px; padding: 0 12px; border-radius: var(--radius-md); font: inherit; font-size: 13px; font-weight: 600; cursor: pointer; border: 1px solid var(--line); background: var(--bg-3); color: var(--text); transition: background var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease); }
  .actions button:not(:disabled):hover { background: var(--bg-4); }
  .actions button:disabled { cursor: default; opacity: .6; }
  .dice { font-size: 15px; padding: 0 8px; }
  /* saving your picks: the live action, in the accent */
  .actions .save { background: var(--accent); border-color: var(--focus-ring); color: var(--on-accent); }
  .actions .save:not(:disabled):hover { background: var(--accent-strong); }
  .clear { color: var(--text-2); }
  .who { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .who li { display: flex; align-items: center; gap: 8px; font-size: 13px; }
  .who .name { color: var(--text); font-weight: 600; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .who .me .name { color: var(--accent-text); }
  .their { display: inline-flex; gap: 4px; margin-left: auto; }
  .code { font-variant-numeric: tabular-nums; font-size: 12px; font-weight: 700; padding: 1px 6px; border-radius: var(--radius-sm); background: var(--bg-3); color: var(--text-2); }
  .pts { flex: none; min-width: 44px; text-align: right; font-weight: 700; color: var(--text); font-variant-numeric: tabular-nums; }
  .none { margin: 0; font-size: 13px; color: var(--text-3); }
  .err { margin: 0; font-size: 12px; color: var(--danger-text); }
</style>
