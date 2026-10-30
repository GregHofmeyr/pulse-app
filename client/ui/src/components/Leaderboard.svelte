<script lang="ts">
  // The quotes leaderboard (spec §4.3): a popover under the header's trophy button. Each quote
  // entry opens that quote in its channel (the caller jumps to it).
  import VoterStack from './VoterStack.svelte'
  import { clickOutside } from '../lib/actions'
  import { plural, quotePreview, score, snapshotSpeakers } from '../lib/quotes'
  import { nameOfIn } from '../lib/selectors'
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { LOADING_DELAY_MS } from '../lib/loading'
  import type { Leaderboard } from '../lib/protocol/Leaderboard'
  import type { Message } from '../lib/protocol/Message'
  import type { QuoteSpeaker } from '../lib/protocol/QuoteSpeaker'
  import type { SpeakerCount } from '../lib/protocol/SpeakerCount'

  let { serverId, onOpen, onClose }: {
    serverId: string
    /** An entry was picked: show that quote. */
    onOpen: (message: Message) => void
    onClose: () => void
  } = $props()

  let board = $state<Leaderboard | null>(null)
  let error = $state('')
  // "Loading…" only once the board is slow to come, so a quick one doesn't flash it
  let slow = $state(false)
  $effect(() => {
    const timer = setTimeout(() => (slow = true), LOADING_DELAY_MS)
    return () => clearTimeout(timer)
  })
  $effect(() => {
    const id = serverId
    api.leaderboard(id).then(
      (b) => {
        if (id === serverId) board = b
      },
      (e) => (error = errorText(e)),
    )
  })

  const nameOf = $derived(nameOfIn(() => app.state, serverId))
  const preview = (m: Message, speakers?: QuoteSpeaker[]) => quotePreview(app.state, serverId, m, speakers)
  const who = (c: SpeakerCount) => (c.user_id ? nameOf(c.user_id) : (c.name ?? ''))
  const empty = $derived(!!board && !board.winners.length && !board.top_year.length && !board.top_all_time.length
    && !board.most_quoted.length && !board.most_awarded.length)
</script>

{#snippet entry(m: Message, note: string, speakers?: QuoteSpeaker[])}
  {@const p = preview(m, speakers)}
  <button type="button" class="entry" onclick={() => onOpen(m)}>
    <span class="said">{p.said}</span>
    {#if p.by}<span class="by">{p.by}</span>{/if}
    <span class="note">{note}</span>
  </button>
{/snippet}

{#snippet quotes(title: string, list: Message[])}
  <section>
    <h3>{title}</h3>
    {#each list as m (m.id)}
      {@render entry(m, plural(score(m), 'reaction', 'reactions'))}
    {:else}
      <p class="none">Nothing yet</p>
    {/each}
  </section>
{/snippet}

{#snippet people(title: string, list: SpeakerCount[], one: string, many: string)}
  <section>
    <h3>{title}</h3>
    {#each list as c, i (`${c.user_id ?? ''}:${c.name ?? ''}:${i}`)}
      <div class="person"><span class="name">{who(c)}</span><span class="note">{plural(c.count, one, many)}</span></div>
    {:else}
      <p class="none">Nothing yet</p>
    {/each}
  </section>
{/snippet}

<div class="leaderboard menu-surface" role="dialog" aria-label="Leaderboard" use:clickOutside={onClose}>
  <h2>Leaderboard</h2>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {:else if !board}
    {#if slow}<p class="none">Loading…</p>{/if}
  {:else if empty}
    <p class="none">No quotes yet. Post one and get the board going.</p>
  {:else}
    <section>
      <h3>Quote of the Year</h3>
      {#each board.winners as w (`${w.year}:${w.message.id}`)}
        <div class="win">
          <span class="year">{w.year}</span>
          <!-- who said it when the vote ended (a later edit or link never moves a win) -->
          {#if w.message.deleted}
            <!-- the win stays (spec §3.1), the words don't; nothing to jump to -->
            {@const by = snapshotSpeakers(app.state, serverId, w.speakers)}
            <div class="entry gone">
              <span class="said">This quote was deleted.</span>
              {#if by}<span class="by">{by}</span>{/if}
              <span class="note">{plural(w.votes, 'vote', 'votes')}</span>
            </div>
          {:else}
            {@render entry(w.message, plural(w.votes, 'vote', 'votes'), w.speakers)}
          {/if}
          <VoterStack userIds={w.voters} {nameOf} />
        </div>
      {:else}
        <p class="none">No winners yet</p>
      {/each}
    </section>
    {@render quotes('Most reacted: this year', board.top_year)}
    {@render quotes('Most reacted: all time', board.top_all_time)}
    {@render people('Most quoted', board.most_quoted, 'quote', 'quotes')}
    {@render people('Most quote awards', board.most_awarded, 'award', 'awards')}
  {/if}
</div>

<style>
  .leaderboard { position: absolute; top: calc(100% + 6px); right: 12px; z-index: 30; width: min(400px, calc(100vw - 32px)); max-height: 70vh; overflow-y: auto; padding: 14px; color: var(--text); }
  h2 { margin: 0 0 8px; font-size: 15px; }
  section + section { margin-top: 12px; }
  h3 { margin: 0 0 4px; font-size: 11px; font-weight: 700; letter-spacing: .06em; text-transform: uppercase; color: var(--quote-green); }
  .entry { width: 100%; display: flex; align-items: baseline; gap: 8px; padding: 6px 8px; border: 0; border-radius: 8px; background: transparent; color: var(--text); font: inherit; font-size: 13px; text-align: left; min-width: 0; }
  .entry:hover, .entry:focus-visible { background: var(--bg-4); }
  .gone:hover { background: transparent; }
  .gone .said { color: var(--text-3); font-style: italic; }
  .said { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .by { flex-shrink: 0; color: var(--text-2); }
  .by::before { content: '— '; color: var(--text-3); }
  .note { flex-shrink: 0; color: var(--text-3); font-size: 12px; }
  .win { display: flex; align-items: center; gap: 6px; }
  .win .entry { flex: 1; }
  .year { flex-shrink: 0; padding: 1px 7px; border-radius: 10px; background: var(--quote-green); color: var(--on-quote-green); font-size: 11px; font-weight: 700; }
  .person { display: flex; justify-content: space-between; gap: 8px; padding: 4px 8px; font-size: 13px; }
  .none { margin: 2px 8px; color: var(--text-3); font-size: 13px; }
  .error { margin: 2px 8px; color: var(--danger); font-size: 13px; }
</style>
