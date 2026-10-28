<script lang="ts">
  // A Quote of the Year result (a system message carrying `qoty_card`, spec §5.3): the winner card
  // (big green quote mark, the quote, who said it, the year, the votes and who cast them, how it
  // ended), or the plain no-vote line. "Reopen voting" while the vote can still be reopened. A
  // reopen deletes the card on the server: a deleted one shows nothing.
  import { untrack } from 'svelte'
  import PixelSpill from './PixelSpill.svelte'
  import QotyStart from './QotyStart.svelte'
  import QuoteImage from './QuoteImage.svelte'
  import VoterStack from './VoterStack.svelte'
  import { motion } from '../lib/animate.svelte'
  import { canReopen, plural, snapshotSpeakers, speakersOf } from '../lib/quotes'
  import { api } from '../lib/tauri'
  import { nameOfIn } from '../lib/selectors'
  import { app, clock } from '../lib/store.svelte'
  import type { Message } from '../lib/protocol/Message'
  import { deletedWinnerContent } from '../lib/state'

  let { message, serverId, quote }: {
    message: Message
    serverId: string
    /** The winning quote, when the channel has it loaded (the card sits in that quote's channel);
     *  otherwise the card fetches it. */
    quote: Message | null
  } = $props()

  const card = $derived(message.qoty_card!)
  const nameOf = $derived(nameOfIn(() => app.state, serverId))
  const time = $derived(new Date(message.created_at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }))
  const reopenable = $derived(canReopen(app.state.qoty[serverId], card.event_id, Date.parse(clock.nowIso)))
  // The winner is older than its card, so a channel's latest page usually doesn't hold it: fetch it
  // (a window around it; only the quote is kept, the channel's history is left alone).
  let fetched = $state<Message | null>(null)
  /** The fetch found the winner gone: the server no longer shows it (deleted). The server 404s an
   *  `around` whose target isn't a visible main-list message, and `listMessagesAround` resolves that
   *  404 as `null` (client/src-tauri/src/api.rs `messages_around`), so `page` is null here; any
   *  other failure rejects and the fallback text stays. A soft-deleted winner comes back in the
   *  window with `deleted` set instead. */
  let missing = $state(false)
  $effect(() => {
    const id = card.message_id
    if (quote || !id || message.deleted) return
    let gone = false
    api.listMessagesAround(message.channel_id, id).then(
      (page) => {
        if (gone) return
        fetched = page?.find((m) => m.id === id) ?? null
        missing = !fetched
      },
      () => {}, // the fallback text stays
    )
    return () => {
      gone = true
    }
  })
  // The loaded (live) message whenever the channel has it; the fetched one is a snapshot, only for until then.
  const shown = $derived(quote ?? fetched)
  // A deleted winner keeps its win (spec §3.1) but not its words. The card's own text says so as
  // soon as the quote's delete arrives, before any re-fetch of a copy that still has the words.
  const deleted = $derived(message.content === deletedWinnerContent(card.year) || !!shown?.deleted || (!shown && missing))
  const lines = $derived(shown?.quote && !deleted ? shown.quote.lines : null)
  /** While the quote loads (or if it can't be): the text the server wrote ("Quote of the Year 2026: “…” — sam"). */
  const fallback = $derived(message.content.replace(`Quote of the Year ${card.year}: `, ''))
  // Who said it when the vote ended (spec §3.2: a later edit or link never moves a win); a card
  // from before that was kept names today's speakers.
  const speakers = $derived(
    card.speakers.length ? snapshotSpeakers(app.state, serverId, card.speakers) : shown && lines ? speakersOf(app.state, serverId, shown) : '',
  )
  // A card that just arrived animates in once (not every time the channel is opened).
  const reveal = untrack(() => Date.now() - Date.parse(message.created_at) < 60_000 && !motion.reducedMotion)
  let reopening = $state(false)
</script>

{#if !message.deleted}
  {#if card.message_id === null}
    <div class="none" data-message-id={message.id}>
      {message.content} <span class="time">{time}</span>
      {#if reopenable}<button type="button" class="link" onclick={() => (reopening = true)}>Reopen voting</button>{/if}
    </div>
  {:else}
    <div class="qoty-card" class:reveal data-message-id={message.id}>
      <PixelSpill seed={card.event_id} cols={9} rows={100} down />
      <div class="qoty-mark big" aria-hidden="true">“</div>
      {#if lines}
        <div class="q">
          {#each lines as line (line.id)}
            {#if line.media}
              <QuoteImage media={line.media} caption={line.caption} />
              {#if line.caption}<p class="caption">{line.caption}</p>{/if}
            {:else}
              <p>“{line.text}”</p>
            {/if}
          {/each}
        </div>
        <div class="who">{speakers} · <span>Quote of the Year {card.year}</span></div>
      {:else if deleted}
        <div class="q"><p class="gone">This quote was deleted.</p></div>
        <div class="who">{speakers ? `${speakers} · ` : ''}<span>Quote of the Year {card.year}</span></div>
      {:else}
        <div class="q"><p>{fallback}</p></div>
        <div class="who"><span>Quote of the Year {card.year}</span></div>
      {/if}
      <div class="tally">
        <small>{plural(card.votes, 'vote', 'votes')} · {card.reason === 'early' ? 'ended early' : 'ended by time'}</small>
        <VoterStack userIds={card.voters} {nameOf} />
      </div>
      {#if reopenable}<button type="button" class="qoty-btn reopen" onclick={() => (reopening = true)}>Reopen voting</button>{/if}
    </div>
  {/if}
  {#if reopening}<QotyStart {serverId} reopen={card.event_id} onClose={() => (reopening = false)} />{/if}
{/if}

<style>
  .none { padding: 6px 18px 6px 30px; font-size: 13px; color: var(--text-3); }
  .time { font-size: 11px; color: var(--text-3); }
  .link { margin-left: 6px; padding: 0; border: 0; background: none; color: var(--quote-green); font-size: 13px; font-weight: 600; }
  .link:hover { text-decoration: underline; }
  .qoty-card { position: relative; overflow: hidden; margin: 8px 18px; padding: 96px 16px 18px; border-radius: 14px; background: var(--quote-surface); border: 1px solid color-mix(in srgb, var(--quote-green) 35%, transparent); display: flex; flex-direction: column; align-items: center; gap: 6px; text-align: center; }
  .qoty-card :global(.pixel-spill) { position: absolute; left: 0; top: 0; width: 100%; height: 90px; }
  .big, .q, .who, .tally, .reopen { position: relative; } /* over the spill */
  .big { font-size: 56px; line-height: .5; color: var(--quote-green); text-shadow: 0 3px 0 var(--quote-green-deep); }
  .q { font-size: 18px; line-height: 1.4; overflow-wrap: anywhere; text-shadow: 0 1px 0 var(--quote-surface); }
  .q p { margin: 0; }
  .q .caption { font-size: 13px; color: var(--text-2); }
  .q .gone { font-size: 15px; font-style: italic; color: var(--text-3); }
  .q :global(.pic) { margin: 0 auto; }
  .who { font-size: 14px; font-weight: 600; color: var(--quote-green); }
  .who span { color: var(--text); }
  .tally { display: flex; align-items: center; gap: 8px; }
  .tally small { color: var(--text-3); font-size: 12px; }
  .tally :global(.stack .av) { box-shadow: 0 0 0 2px var(--quote-surface); }
  .reopen { margin-top: 4px; }

  /* a card that just arrived: the spill pours down, the mark drops in (never under reduced motion) */
  .reveal :global(.pixel-spill) { animation: qoty-pour .7s steps(7, end); }
  .reveal .big { animation: qoty-drop .6s steps(6, end); }
  .reveal .q, .reveal .who { animation: qoty-in .6s steps(4, end); }
  @keyframes qoty-pour { from { clip-path: inset(0 0 100% 0); } to { clip-path: inset(0 0 0 0); } }
  @keyframes qoty-drop {
    0% { transform: translateY(-40px); opacity: 0; }
    60% { transform: translateY(6px); opacity: 1; }
    100% { transform: translateY(0); }
  }
  @keyframes qoty-in { from { opacity: 0; } to { opacity: 1; } }
</style>
