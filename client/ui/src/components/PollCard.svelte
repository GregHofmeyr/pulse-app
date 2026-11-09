<script lang="ts">
  // A poll in the chat (#157), in its message's body: the question, the options as rows to vote on, how it's set up
  // and the time left, and the total. Results (a bar, the count and the percent, and who voted unless it's
  // anonymous) show once you may see them: after you vote, as its creator, or once it closes. Click a row to vote;
  // single choice moves your vote (or, on yours, takes it back), multiple choice toggles each. Closed, it shows the
  // final results with the winner(s) emphasised. The bars grow only when motion is allowed.
  import Icon from './Icon.svelte'
  import VoterStack from './VoterStack.svelte'
  import { motion } from '../lib/animate.svelte'
  import { nextChoices, percent, pollOver, pollStatus, totalLabel, winners } from '../lib/poll'
  import { votePoll } from '../lib/polls.svelte'
  import { clock } from '../lib/store.svelte'
  import { serverNow } from '../lib/serverClock'
  import { errorText } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'
  import type { Poll } from '../lib/protocol/Poll'

  let { message, nameOf }: { message: Message; nameOf: (userId: string | null) => string } = $props()
  const uid = $props.id()

  const poll = $derived(message.poll as Poll)
  // the clock moves every 30 s: the time left (and the end) follow it, before the server's sweep closes it
  const now = $derived((void clock.nowIso, serverNow()))
  const over = $derived(pollOver(poll, now))
  const won = $derived(over ? winners(poll) : [])
  const results = $derived(poll.results)

  let busy = $state(false)
  let error = $state('')
  async function vote(i: number) {
    if (busy || over) return
    busy = true
    error = ''
    try {
      await votePoll(message, nextChoices(poll.mine, i, poll.multi))
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
</script>

<div class="poll" class:animated={!motion.reducedMotion} class:over role="group" aria-labelledby="{uid}-q">
  <p class="q" id="{uid}-q">{poll.question}</p>
  <p class="status">{pollStatus(poll, now)}</p>
  <ul class="options">
    {#each poll.options as option, i (i)}
      {@const mine = poll.mine.includes(i)}
      {@const t = results?.[i]}
      <li class="opt" class:mine class:won={won.includes(i)}>
        {#if t}<span class="fill" style:transform="scaleX({percent(t.votes, poll.voters) / 100})"></span>{/if}
        <button type="button" class="choose" aria-pressed={mine} disabled={over || busy} onclick={() => void vote(i)}>
          <span class="mark" class:multi={poll.multi} aria-hidden="true">{#if mine}<Icon name="check" size={14} />{/if}</span>
          <span class="label">{option}</span>
        </button>
        {#if t}
          <span class="tally">
            {#if t.voters.length}<VoterStack userIds={t.voters} {nameOf} max={3} />{/if}
            <span class="count">{t.votes}</span><span class="pct">{percent(t.votes, poll.voters)}%</span>
          </span>
        {/if}
      </li>
    {/each}
  </ul>
  <p class="foot">
    <span>{totalLabel(poll)}</span>
    {#if over}· <span class="final">Final results</span>
    {:else if !results}· <span>Vote to see results</span>{/if}
  </p>
  {#if error}<p class="err" role="alert">{error}</p>{/if}
</div>

<style>
  /* a card like the link previews and the spin card: bg-2 on the chat, a line round it */
  .poll { max-width: 440px; margin-top: 4px; padding: 12px; background: var(--bg-2); border: 1px solid var(--line); border-radius: var(--radius-lg); }
  .q { margin: 0; font-weight: 700; font-size: 15px; color: var(--text); overflow-wrap: anywhere; }
  .status { margin: 2px 0 10px; font-size: 12px; color: var(--text-3); }
  .options { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .opt { position: relative; overflow: hidden; display: flex; align-items: center; gap: 8px; min-height: 36px; padding-right: 10px; border-radius: var(--radius-md); background: var(--bg-3); border: 1px solid transparent; }
  .opt.mine { border-color: var(--focus-ring); }
  .fill { position: absolute; inset: 0; background: var(--accent-soft); pointer-events: none; transform-origin: left; }
  .choose, .tally { position: relative; }
  .choose { flex: 1; min-width: 0; display: flex; align-items: center; gap: 8px; align-self: stretch; padding: 6px 0 6px 10px; border: 0; background: transparent; color: var(--text); font: inherit; font-size: 14px; text-align: left; cursor: pointer; }
  .choose:disabled { cursor: default; }
  .poll:not(.over) .opt:not(.mine):hover { background: var(--bg-4); }
  /* a round mark for one choice, a square one for multiple: filled with a tick on yours */
  /* border-box: 18 px less the 2 px border each side leaves exactly the 14 px tick, so it centres instead of spilling */
  .mark { width: 18px; height: 18px; flex-shrink: 0; display: grid; place-items: center; border: 2px solid var(--text-3); border-radius: 50%; color: var(--on-accent); }
  .mark.multi { border-radius: var(--radius-xs); }
  .mine .mark { border-color: var(--focus-ring); background: var(--accent); }
  .label { min-width: 0; overflow-wrap: anywhere; }
  .won .label { font-weight: 700; }
  .tally { display: flex; align-items: center; gap: 8px; flex-shrink: 0; font-size: 12px; color: var(--text-2); font-variant-numeric: tabular-nums; }
  .count { font-weight: 700; color: var(--text); }
  .pct { min-width: 34px; text-align: right; }
  .foot { margin: 8px 0 0; font-size: 12px; color: var(--text-3); }
  .final { color: var(--text-2); font-weight: 600; }
  .err { margin: 6px 0 0; font-size: 12px; color: var(--danger-text); }
  /* motion (off under reduced motion: no .animated) */
  .animated .fill { transition: transform var(--dur) var(--ease); }
</style>
