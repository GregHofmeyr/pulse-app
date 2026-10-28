<script lang="ts">
  // The Quote of the Year voting window (spec §5.2): every option with its quote, score, live votes,
  // a bar and who voted; click to vote, again elsewhere to move it, on yours to take it back. Add a
  // quote by search, or end the vote (after asking). Renders nothing once the vote isn't live.
  import Icon from './Icon.svelte'
  import PixelSpill from './PixelSpill.svelte'
  import QuoteItem from './QuoteItem.svelte'
  import VoterStack from './VoterStack.svelte'
  import { tick } from 'svelte'
  import { modal } from '../lib/actions'
  import { motion } from '../lib/animate.svelte'
  import { leftLabel, liveQoty, myVote, plural, quotePreview, score, voteAction } from '../lib/quotes'
  import { nameOfIn } from '../lib/selectors'
  import { applyMyVote } from '../lib/state'
  import { app, clock } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'

  let { serverId, onClose }: { serverId: string; onClose: () => void } = $props()
  const uid = $props.id()

  const e = $derived(liveQoty(app.state, serverId, Date.parse(clock.nowIso)))
  const meId = $derived(app.state.me?.id ?? '')
  const nameOf = $derived(nameOfIn(() => app.state, serverId))
  const mine = $derived(e ? myVote(e, meId) : null)
  const total = $derived(e ? e.options.reduce((n, o) => n + o.votes, 0) : 0)

  let error = $state('')
  async function attempt(f: () => Promise<unknown>): Promise<boolean> {
    error = ''
    try {
      await f()
      return true
    } catch (err) {
      error = errorText(err)
      return false
    }
  }

  // --- voting: one call at a time (the next click needs the server's answer to know what it
  // means; the answer moves my vote in state at once, before the gateway's update); the option you
  // voted for gets a short game-like pop once the vote is in ---
  let popped = $state<string | null>(null)
  let voting = $state(false)
  async function vote(id: string) {
    if (!e || voting) return
    const eventId = e.id
    const action = voteAction(mine, id)
    voting = true
    const ok = await attempt(async () => {
      const answer = await (action === 'unvote' ? api.unvoteQoty(eventId) : api.voteQoty(eventId, id))
      app.state = applyMyVote(app.state, answer)
    })
    voting = false
    if (ok && action !== 'unvote' && !motion.reducedMotion) popped = id
  }

  // --- add a quote: search the server's quotes ---
  let adding = $state(false)
  let query = $state('')
  let results = $state<Message[]>([])
  let searched = $state(false)
  let searchTimer: ReturnType<typeof setTimeout> | undefined
  let searchSeq = 0
  $effect(() => () => clearTimeout(searchTimer))
  function search() {
    clearTimeout(searchTimer)
    const q = query.trim()
    const seq = ++searchSeq
    searched = false
    if (!q) return void (results = [])
    searchTimer = setTimeout(async () => {
      try {
        const found = await api.searchQuotes(serverId, q)
        if (seq !== searchSeq) return // a newer search took over
        results = found
        searched = true
      } catch (err) {
        if (seq === searchSeq) error = errorText(err)
      }
    }, 250)
  }
  // Quotes already in the vote, deleted ones and earlier winners (the server refuses them) aren't offered.
  const offered = $derived(results.filter((m) => !m.deleted && !m.quote?.wins.length && !e?.options.some((o) => o.message.id === m.id)))
  let addButton = $state<HTMLButtonElement>()
  async function add(m: Message) {
    if (!e) return
    const eventId = e.id
    if (await attempt(() => api.addQotyOption(eventId, m.id))) {
      adding = false
      query = ''
      results = []
      // the search box is gone: focus goes back to where adding started
      await tick()
      addButton?.focus()
    }
  }

  // --- ending: asks first; the question takes focus (Cancel) and gives it back when withdrawn ---
  let confirming = $state(false)
  let ending = $state(false)
  let endButton = $state<HTMLButtonElement>()
  let cancelButton = $state<HTMLButtonElement>()
  async function ask() {
    confirming = true
    await tick()
    cancelButton?.focus()
  }
  async function unask() {
    confirming = false
    await tick()
    endButton?.focus()
  }
  async function end() {
    if (!e || ending) return
    const eventId = e.id
    ending = true
    if (await attempt(() => api.endQoty(eventId))) confirming = false
    ending = false
  }

  // Escape steps back out of the end question before it closes the window.
  function escape() {
    if (confirming) void unask()
    else onClose()
  }
</script>

{#if e}
  <div class="overlay">
    <div class="backdrop" role="presentation" data-backdrop onclick={onClose}></div>
    <div class="modal qoty-voting" class:animated={!motion.reducedMotion} role="dialog" aria-modal="true" aria-label="Quote of the Year {e.year}"
      tabindex="-1" use:modal={escape}>
      <div class="spill"><PixelSpill seed={e.id} cols={5} rows={88} down /></div>
      <div class="top">
        <span class="qoty-mark mark" aria-hidden="true">“</span>
        <h1>Quote of the Year {e.year}</h1>
        <span class="qoty-live live">LIVE</span>
        <span class="grow"></span>
        <button type="button" class="close" aria-label="Close" onclick={onClose}><Icon name="x" /></button>
      </div>
      <!-- only the count is live: the time left ticks every minute -->
      <p class="status">{leftLabel(e.ends_at, Date.parse(clock.nowIso))} · <span aria-live="polite">{e.voters} of {e.members} have voted</span></p>

      <div class="options">
        {#each e.options as o (o.message.id)}
          {@const id = o.message.id}
          {@const isMine = id === mine}
          {@const p = quotePreview(app.state, serverId, o.message)}
          <div class="opt" class:mine={isMine} class:popped={popped === id} data-option={id}
            onanimationend={(ev) => { if (ev.animationName.endsWith('qoty-pop')) popped = null }}>
            <div class="fill" style:width="{total ? (o.votes / total) * 100 : 0}%"></div>
            <div class="said">
              <QuoteItem message={o.message} {serverId} {meId} {nameOf} compact />
              <div class="meta">
                <span>{plural(score(o.message), 'reaction', 'reactions')}</span>
                {#if o.added_by}<span class="tag">added by {nameOf(o.added_by)}</span>{/if}
              </div>
            </div>
            <div class="tally">
              <span class="count" aria-hidden="true">{o.votes}</span><span class="sr">{plural(o.votes, 'vote', 'votes')}</span>
              <VoterStack userIds={o.voters} {nameOf} />
              <button type="button" class="vote" class:on={isMine} aria-pressed={isMine}
                aria-label="{isMine ? 'Your vote:' : 'Vote for'} {p.said} — {p.by}" disabled={voting} onclick={() => void vote(id)}>{isMine ? 'Your vote' : 'Vote'}</button>
            </div>
          </div>
        {:else}
          <p class="none">No quotes in this vote yet. Add one below.</p>
        {/each}
      </div>

      {#if adding}
        <div class="add">
          <!-- svelte-ignore a11y_autofocus -->
          <input aria-label="Search quotes" placeholder="Search this server's quotes" autofocus bind:value={query} oninput={search} />
          <div class="results">
            {#each offered as m (m.id)}
              {@const p = quotePreview(app.state, serverId, m)}
              <button type="button" onclick={() => void add(m)}><span class="r-said">{p.said}</span> <span class="r-by">{p.by}</span></button>
            {:else}
              {#if searched}<p class="none">No other quotes match.</p>{/if}
            {/each}
          </div>
        </div>
      {/if}

      {#if error}<p class="err" role="alert">{error}</p>{/if}

      {#if confirming}
        <div class="confirm" role="group" aria-label="End voting" aria-describedby="{uid}-end-q">
          <p id="{uid}-end-q">End voting now? Everyone's votes are final.</p>
          <button type="button" class="ghost" bind:this={cancelButton} aria-describedby="{uid}-end-q" disabled={ending} onclick={() => void unask()}>Cancel</button>
          <button type="button" class="qoty-btn danger" aria-describedby="{uid}-end-q" disabled={ending} onclick={end}>End voting</button>
        </div>
      {:else}
        <div class="foot">
          <button type="button" class="qoty-btn" bind:this={addButton} aria-expanded={adding} onclick={() => (adding = !adding)}>Add a quote</button>
          <span class="grow"></span>
          <button type="button" class="qoty-btn" bind:this={endButton} onclick={() => void ask()}>End voting…</button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  /* flex-centred (not transform): WebKitGTK blurs transformed text */
  .overlay { position: fixed; inset: 0; z-index: 30; display: grid; place-items: center; padding: 16px; }
  .backdrop { position: absolute; inset: 0; background: rgba(10, 11, 13, .6); }
  .modal { position: relative; width: min(620px, 100%); max-height: calc(100vh - 32px); overflow-y: auto; padding: 46px 20px 18px; border-radius: 16px; background: var(--bg-1); border: 1px solid color-mix(in srgb, var(--quote-green) 35%, transparent); display: flex; flex-direction: column; gap: 12px; outline: none; }
  /* 88 cells across ≈ 7 px each at full width, 5 down: the solid run and its thinning both show */
  .spill { position: absolute; left: 0; right: 0; top: 0; height: 35px; overflow: hidden; border-radius: 16px 16px 0 0; }
  .spill :global(.pixel-spill) { width: 100%; height: 100%; }
  .top { position: relative; display: flex; align-items: center; gap: 10px; }
  .mark { font-size: 34px; padding-top: 12px; color: var(--quote-green); }
  h1 { margin: 0; font-size: 18px; font-weight: 600; }
  .live { background: var(--quote-green); }
  .grow { flex: 1; }
  .close { width: 32px; height: 32px; border-radius: 50%; border: 0; background: var(--bg-3); color: var(--text-2); display: grid; place-items: center; }
  .status { margin: -4px 0 0; color: var(--text-3); font-size: 12.5px; }
  .options { display: flex; flex-direction: column; gap: 8px; }
  .opt { position: relative; overflow: hidden; display: flex; gap: 12px; align-items: center; padding: 10px 12px; border-radius: 12px; background: var(--bg-2); border: 1px solid var(--line); }
  .opt.mine { border-color: var(--quote-green); }
  .fill { position: absolute; inset: 0 auto 0 0; background: color-mix(in srgb, var(--quote-green) 12%, transparent); pointer-events: none; }
  .said, .tally { position: relative; }
  .said { flex: 1; min-width: 0; }
  .meta { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; margin-top: 4px; font-size: 12px; color: var(--text-3); }
  .tag { border: 1px dashed var(--bg-4); border-radius: 999px; padding: 0 6px; }
  .tally { display: flex; flex-direction: column; align-items: flex-end; gap: 6px; flex-shrink: 0; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
  .vote:disabled { opacity: .6; }
  .count { font-size: 20px; font-weight: 700; font-variant-numeric: tabular-nums; }
  .vote { height: 30px; padding: 0 12px; border-radius: 8px; border: 1px solid var(--quote-green); background: transparent; color: var(--quote-green); font-size: 12.5px; font-weight: 700; white-space: nowrap; }
  .vote.on { border-color: transparent; background: var(--quote-green); color: var(--on-quote-green); box-shadow: 0 2px 0 var(--quote-green-deep); }
  .none { margin: 0; color: var(--text-3); font-size: 13px; }
  .add { display: flex; flex-direction: column; gap: 6px; }
  .add input { height: 38px; padding: 0 12px; border-radius: 10px; border: 1px solid var(--bg-4); background: var(--bg-2); }
  .results { display: flex; flex-direction: column; max-height: 200px; overflow-y: auto; }
  .results button { display: flex; gap: 8px; align-items: baseline; padding: 7px 10px; border: 0; border-radius: 8px; background: transparent; text-align: left; }
  .results button:hover { background: var(--bg-3); }
  .r-said { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .r-by { color: var(--text-3); font-size: 12px; }
  .err { margin: 0; font-size: 13px; color: var(--danger); }
  .foot, .confirm { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .confirm p { flex: 1; margin: 0; font-size: 13.5px; }
  .foot .qoty-btn, .confirm .qoty-btn { height: 34px; }
  .qoty-btn.danger { background: var(--danger); color: var(--text); box-shadow: 0 2px 0 color-mix(in srgb, var(--danger) 55%, var(--bg-0)); }
  .ghost { height: 34px; padding: 0 12px; border-radius: 8px; border: 1px solid var(--bg-4); background: transparent; color: var(--text-2); font-size: 13px; }
  .ghost:disabled { opacity: .6; }

  /* motion (off under reduced motion: no .animated, no .popped) */
  .animated .fill { transition: width .35s steps(7, end); }
  .animated .opt.popped { animation: qoty-pop .45s steps(5, end); }
  .animated .opt.popped .count { animation: qoty-count .45s steps(4, end); }
  @keyframes qoty-pop {
    0% { box-shadow: 0 0 0 0 var(--quote-green); }
    40% { box-shadow: 0 0 0 4px var(--quote-green); }
    100% { box-shadow: 0 0 0 10px transparent; }
  }
  @keyframes qoty-count {
    0% { transform: translateY(0); }
    40% { transform: translateY(-6px); color: var(--quote-green); }
    100% { transform: translateY(0); }
  }
</style>
