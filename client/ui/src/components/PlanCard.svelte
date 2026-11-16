<script lang="ts">
  // A plan in the chat (#387), in its message's body: a date badge, the title and when it is in your own time, the
  // place (with the forecast for its day there, once it's within reach), the note, then Going / Maybe / Can't with
  // their counts (your answer filled with the accent; click it again to take it back) and who picked each, and Add to
  // calendar. Answers from everyone land live (PlanChanged). A cancelled plan says so and takes no answers. Its
  // creator edits or cancels it from the message's menu.
  import Icon from './Icon.svelte'
  import VoterStack from './VoterStack.svelte'
  import { RSVPS, answered, forecastLine, inForecastWindow, myRsvp, nextRsvp, planStarted, planWhen, weatherOf } from '../lib/plan'
  import { forecast, loadForecast, planPrefs, rsvpPlan, saveIcs } from '../lib/plans.svelte'
  import { app, clock } from '../lib/store.svelte'
  import { serverNow } from '../lib/serverClock'
  import { errorText } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'
  import type { Plan } from '../lib/protocol/Plan'
  import type { Rsvp } from '../lib/protocol/Rsvp'

  let { message, nameOf }: { message: Message; nameOf: (userId: string | null) => string } = $props()
  const uid = $props.id()

  const plan = $derived(message.plan as Plan)
  // the clock moves every 30 s: "Today" and "started" follow it
  const now = $derived((void clock.nowIso, serverNow()))
  const me = $derived(app.state.me?.id ?? '')
  const mine = $derived(myRsvp(plan, me))
  const cancelled = $derived(!!plan.cancelled_at)
  const started = $derived(planStarted(plan, now))
  const when = $derived(planWhen(plan.starts_at, now))
  const badge = $derived.by(() => {
    const t = Date.parse(plan.starts_at)
    return { month: new Intl.DateTimeFormat(undefined, { month: 'short' }).format(t), day: new Date(t).getDate() }
  })
  const wantForecast = $derived(planPrefs.on && inForecastWindow(plan, now))
  const fc = $derived(wantForecast ? forecast.byId[message.id] : undefined)
  $effect(() => {
    if (wantForecast) loadForecast(message.id)
  })

  let busy = $state(false)
  let error = $state('')
  async function answer(r: Rsvp) {
    if (busy || cancelled) return
    busy = true
    error = ''
    try {
      await rsvpPlan(message, nextRsvp(mine, r))
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
  async function addToCalendar() {
    error = ''
    try {
      await saveIcs(message)
    } catch (e) {
      error = errorText(e)
    }
  }
</script>

<div class="plan" class:cancelled role="group" aria-labelledby="{uid}-t">
  <div class="head">
    <span class="badge" aria-hidden="true"><span class="month">{badge.month}</span><span class="day">{badge.day}</span></span>
    <div class="what">
      <p class="title" id="{uid}-t">{plan.title}</p>
      <p class="when"><time datetime={plan.starts_at}>{when}</time>{#if cancelled} · <span class="state">Cancelled</span>{:else if started} · <span class="state">Started</span>{/if}</p>
    </div>
  </div>
  {#if plan.place}
    <p class="place"><Icon name="place" size={14} /> <span>{plan.place}</span></p>
  {/if}
  {#if fc}
    <p class="forecast" data-tip="Forecast for {fc.place} from Open-Meteo"><span aria-hidden="true">{weatherOf(fc.code).emoji}</span> {forecastLine(fc)}</p>
  {/if}
  {#if plan.note}<p class="note">{plan.note}</p>{/if}
  <div class="rsvps" role="group" aria-label="Your answer">
    {#each RSVPS as r (r.value)}
      {@const who = answered(plan, r.value)}
      <button type="button" class="rsvp" class:on={mine === r.value} aria-pressed={mine === r.value} disabled={cancelled || busy} onclick={() => void answer(r.value)}>
        <span>{r.label}</span><span class="count">{who.length}</span>
      </button>
    {/each}
  </div>
  {#if plan.going.length || plan.maybe.length || plan.cant.length}
    <ul class="who">
      {#each RSVPS as r (r.value)}
        {@const who = answered(plan, r.value)}
        {#if who.length}<li><span class="who-label">{r.label}</span><VoterStack userIds={who} {nameOf} max={6} /></li>{/if}
      {/each}
    </ul>
  {/if}
  <p class="foot">
    {#if !cancelled}<button type="button" class="add" onclick={() => void addToCalendar()}><Icon name="calendar" size={14} /> Add to calendar</button>{/if}
    {#if plan.edited_at}<span class="edited">(edited)</span>{/if}
  </p>
  {#if error}<p class="err" role="alert">{error}</p>{/if}
</div>

<style>
  /* a card like the poll card: bg-2 on the chat, a line round it */
  .plan { max-width: 440px; margin-top: 4px; padding: 12px; background: var(--bg-2); border: 1px solid var(--line); border-radius: var(--radius-lg); display: flex; flex-direction: column; gap: 8px; }
  .head { display: flex; align-items: center; gap: 12px; }
  /* a calendar page: the month on the accent, the day under it */
  .badge { flex-shrink: 0; width: 44px; display: flex; flex-direction: column; align-items: center; overflow: hidden; border-radius: var(--radius-md); background: var(--bg-3); border: 1px solid var(--line); }
  .month { align-self: stretch; text-align: center; padding: 1px 0; font-size: 10px; font-weight: 700; text-transform: uppercase; background: var(--accent); color: var(--on-accent); }
  .day { padding: 2px 0 3px; font-size: 18px; font-weight: 700; color: var(--text); font-variant-numeric: tabular-nums; }
  .what { min-width: 0; }
  .title { margin: 0; font-weight: 700; font-size: 15px; color: var(--text); overflow-wrap: anywhere; }
  .when { margin: 2px 0 0; font-size: 13px; color: var(--text-2); }
  .state { font-weight: 600; color: var(--text-3); }
  .cancelled .title { text-decoration: line-through; color: var(--text-3); }
  .cancelled .state { color: var(--danger-text); }
  .place, .forecast { margin: 0; display: flex; align-items: center; gap: 6px; font-size: 13px; color: var(--text-2); overflow-wrap: anywhere; }
  .forecast { color: var(--text-3); }
  .note { margin: 0; font-size: 13px; color: var(--text); white-space: pre-wrap; overflow-wrap: anywhere; }
  .rsvps { display: flex; gap: 6px; flex-wrap: wrap; }
  .rsvp { flex: 1; min-width: 84px; min-height: 32px; display: inline-flex; align-items: center; justify-content: center; gap: 8px; padding: 0 10px; border: 1px solid var(--line); border-radius: var(--radius-md); background: var(--bg-3); color: var(--text); font: inherit; font-size: 13px; font-weight: 600; cursor: pointer; transition: background var(--dur-fast) var(--ease), border-color var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease); }
  .rsvp:not(:disabled):not(.on):hover { background: var(--bg-4); }
  /* your answer: the live action, in the accent */
  .rsvp.on { background: var(--accent); border-color: var(--focus-ring); color: var(--on-accent); }
  .rsvp:disabled { cursor: default; }
  .cancelled .rsvp { opacity: .6; }
  .count { font-variant-numeric: tabular-nums; font-weight: 700; }
  .who { list-style: none; margin: 0; padding: 0; display: flex; flex-wrap: wrap; gap: 4px 14px; }
  .who li { display: inline-flex; align-items: center; gap: 6px; }
  .who-label { font-size: 12px; color: var(--text-3); }
  .foot { margin: 0; display: flex; align-items: center; gap: 8px; font-size: 12px; color: var(--text-3); }
  .foot:empty { display: none; }
  .add { display: inline-flex; align-items: center; gap: 6px; min-height: 26px; padding: 0 6px; margin-left: -6px; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--accent-text); font: inherit; font-size: 12px; font-weight: 600; cursor: pointer; }
  .add:hover { background: var(--bg-3); }
  .edited { color: var(--text-3); }
  .err { margin: 0; font-size: 12px; color: var(--danger-text); }
</style>
