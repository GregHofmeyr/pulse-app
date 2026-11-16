<script lang="ts" module>
  import type { LeaderboardPeriod } from '../lib/protocol/LeaderboardPeriod'
  // The period you last picked, for the rest of the session (every quotes channel shares it).
  let lastPeriod: LeaderboardPeriod = 'month'
  /** Back to the default period (tests). */
  export function resetPeriod(): void {
    lastPeriod = 'month'
  }
</script>

<script lang="ts">
  // The quotes leaderboard (spec §4.3, #410): a panel down the right of the quotes channel, under the header's trophy
  // button. Period tabs (week, month, year, all time) choose what Most reacted and Most quoted count; Quote awards,
  // the Quote of the Year winners and the Steam hours (#383) have their own fixed spans and sit below. The top three
  // people stand on a podium in the quote greens; everyone after is a ranked row with a bar for their share of the
  // leader's count. It refetches (debounced) whenever a live event could change it, and after a reconnect.
  import { onMount, untrack } from 'svelte'
  import Avatar from './Avatar.svelte'
  import Icon from './Icon.svelte'
  import Loading from './Loading.svelte'
  import Name from './Name.svelte'
  import PersonMenu from './PersonMenu.svelte'
  import VoterStack from './VoterStack.svelte'
  import { clickOutside } from '../lib/actions'
  import { hoursText } from '../lib/achievements'
  import { boardLines, boardRefreshFor, PERIODS, ranks, REFRESH_DEBOUNCE_MS, shares, tabMove, widerPeriod } from '../lib/leaderboard'
  import { messagePerson } from '../lib/messagePerson'
  import { personMenuAt, type PersonMenuAt } from '../lib/participantMenu'
  import { localDay } from '../lib/quoteFromMessage'
  import { plural, score as reactionTotal, snapshotSpeakers } from '../lib/quotes'
  import { nameOfIn } from '../lib/selectors'
  import { app, onAppEvent, onAppReady } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import type { Leaderboard } from '../lib/protocol/Leaderboard'
  import type { Message } from '../lib/protocol/Message'
  import type { QuoteSpeaker } from '../lib/protocol/QuoteSpeaker'
  import type { SpeakerCount } from '../lib/protocol/SpeakerCount'
  import type { SteamHoursEntry } from '../lib/protocol/SteamHoursEntry'

  let { serverId, onOpen, onClose }: {
    serverId: string
    /** An entry was picked: show that quote. */
    onOpen: (message: Message) => void
    onClose: () => void
  } = $props()

  const uid = $props.id()

  // --- the boards, one per period, kept while the panel is open so switching back is instant (and refetched) ---
  let period = $state<LeaderboardPeriod>(lastPeriod)
  let boards = $state<Partial<Record<LeaderboardPeriod, Leaderboard>>>({})
  let error = $state('')
  const board = $derived(boards[period] ?? null)
  // the newest request per period: an older answer that lands later is dropped
  const asked: Partial<Record<LeaderboardPeriod, number>> = {}
  let asks = 0

  function load(p: LeaderboardPeriod) {
    const id = serverId
    const n = (asked[p] = ++asks)
    api.leaderboard(id, p, localDay()).then(
      (b) => {
        if (id !== serverId || asked[p] !== n) return
        boards = { ...boards, [p]: b }
        if (p === period) error = ''
      },
      (e) => {
        // a failed refresh keeps the board you're looking at; only a board that never loaded says so
        if (id === serverId && asked[p] === n && p === period && !boards[p]) error = errorText(e)
      },
    )
  }

  $effect(() => {
    void serverId
    untrack(() => {
      boards = {}
      error = ''
      load(period)
    })
  })

  function pick(p: LeaderboardPeriod) {
    if (p === period) return
    period = lastPeriod = p
    error = ''
    load(p) // shows the kept board at once, if any, and brings it up to date
  }

  // Steam hours (#383): an older server (or one without Steam) simply has no section
  let hours = $state<SteamHoursEntry[]>([])
  function loadHours() {
    const id = serverId
    api.steamHours(id).then(
      (h) => {
        if (id === serverId) hours = h?.entries ?? []
      },
      () => {},
    )
  }
  $effect(() => {
    void serverId
    untrack(() => {
      hours = []
      loadHours()
    })
  })

  // --- live: a relevant event (a quote, a reaction on one, an award, a link, a vote ending) refetches the period
  // you're on after things go quiet for a moment; the other periods are dropped, to be fetched when picked. A
  // reconnect may have missed events, so it refetches too. ---
  function refresh() {
    const kept = boards[period]
    boards = kept ? { [period]: kept } : {}
    load(period)
  }
  onMount(() => {
    let timer: ReturnType<typeof setTimeout> | undefined
    const later = () => {
      clearTimeout(timer)
      timer = setTimeout(refresh, REFRESH_DEBOUNCE_MS)
    }
    const offEvent = onAppEvent((e) => {
      if (boardRefreshFor(e, serverId, app.state)) later()
    })
    const offReady = onAppReady(() => {
      later()
      loadHours()
    })
    return () => {
      clearTimeout(timer)
      offEvent()
      offReady()
    }
  })

  // --- tabs: a tablist, arrows / Home / End move and pick (the board fetches as you go) ---
  let tabEls = $state<HTMLButtonElement[]>([])
  function tabKey(e: KeyboardEvent, at: number) {
    const to = tabMove(e.key, at, PERIODS.length)
    if (to === null) return
    e.preventDefault()
    pick(PERIODS[to].value)
    tabEls[to]?.focus()
  }
  // focus starts on the chosen tab, and goes back to what had it (the trophy button) when the panel closes
  let panel = $state<HTMLElement>()
  onMount(() => {
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : null
    tabEls[PERIODS.findIndex((p) => p.value === period)]?.focus({ preventScroll: true })
    return () => {
      if (before?.isConnected && panel?.contains(document.activeElement)) before.focus({ preventScroll: true })
    }
  })

  // --- names, you, and the person menu on people (right-click, or ContextMenu / Shift+F10) ---
  const nameOf = $derived(nameOfIn(() => app.state, serverId))
  const meId = $derived(app.state.me?.id ?? null)
  const who = (c: SpeakerCount) => (c.user_id ? nameOf(c.user_id) : (c.name ?? ''))
  let menu = $state<PersonMenuAt | null>(null)
  function personMenu(e: MouseEvent | KeyboardEvent, userId: string | null, row: HTMLElement) {
    menu = personMenuAt(e, userId, serverId, row, { row: true }) ?? menu
  }

  // --- quotes: two lines, the rest on hover (a tip, when cut) or expanded (the chevron) ---
  let open = $state<Record<string, boolean>>({})
  let cut = $state<Record<string, boolean>>({})
  /** Notes whether the quote's words overflow their two lines. */
  const watchCut = (id: string) => (el: HTMLElement) => {
    const check = () => {
      const over = el.scrollHeight > el.clientHeight + 1
      if (cut[id] !== over) cut = { ...cut, [id]: over }
    }
    check()
    const ro = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(check)
    ro?.observe(el)
    return () => ro?.disconnect()
  }
  const fullText = (m: Message) => boardLines(app.state, serverId, m).map((l) => `“${l.said}” — ${l.by}`).join('\n')

  const within = $derived(PERIODS.find((p) => p.value === period)!.within)
  const wider = $derived(widerPeriod(period))
  const widerLabel = $derived(PERIODS.find((p) => p.value === wider)?.label.toLowerCase() ?? '')
  const empty = $derived(!!board && period === 'all' && !board.winners.length && !board.top.length
    && !board.most_quoted.length && !board.most_awarded.length)
</script>

<!-- the step under a podium place, and a ranked row's rank chip: the top three in the quote greens -->
{#snippet rank(n: number)}
  <span class="rank" class:g1={n === 1} class:g2={n === 2} class:g3={n === 3} aria-hidden="true">{n}</span>
{/snippet}

{#snippet person(userId: string | null, name: string, size: number)}
  <Avatar id={userId ?? `name:${name}`} {name} {size} serverId={userId ? serverId : null} />
{/snippet}

{#snippet quoteRow(m: Message, n: number, count: number, countLabel: string, tip: string, speakers?: QuoteSpeaker[])}
  {@const lines = boardLines(app.state, serverId, m)}
  {@const first = lines[0]}
  {@const by = speakers?.length ? snapshotSpeakers(app.state, serverId, speakers) : [...new Set(lines.map((l) => l.by))].join(' & ')}
  {@const mine = !!meId && lines.some((l) => l.userId === meId)}
  {@const expanded = !!open[m.id]}
  {@const canExpand = lines.length > 1 || !!cut[m.id]}
  <li class="qrow" class:mine class:expanded>
    {@render rank(n)}
    <button type="button" class="jump" onclick={() => onOpen(m)} data-tip={canExpand && !expanded ? fullText(m) : undefined}
      aria-label="{n}. {fullText(m)}. {countLabel}. Show in chat">
      {#if expanded}
        <span class="lines" aria-hidden="true">
          {#each lines as l, i (i)}<span class="line"><span class="said">“{l.said}”</span> <span class="by">{l.by}</span></span>{/each}
        </span>
      {:else}
        <span class="said clamp" aria-hidden="true" {@attach watchCut(m.id)}>“{first?.said ?? ''}”{#if lines.length > 1}<span class="more"> +{lines.length - 1} more</span>{/if}</span>
      {/if}
      <span class="byline" aria-hidden="true">
        {@render person(first?.userId ?? null, by, 18)}
        <span class="who">{by}</span>{#if mine}<span class="you">you</span>{/if}
      </span>
    </button>
    <span class="count" data-tip={tip}><span class="n">{count}</span></span>
    {#if canExpand}
      <button type="button" class="icon-btn icon-btn-xs expand" aria-expanded={expanded} aria-label={expanded ? 'Show less' : 'Show the whole quote'}
        onclick={() => (open = { ...open, [m.id]: !expanded })}><Icon name="chevron" size={14} /></button>
    {/if}
  </li>
{/snippet}

{#snippet personName(userId: string | null, name: string)}
  {#if userId}<Name {userId} {serverId} text={name} />{:else}<span class="name">{name}</span>{/if}
{/snippet}

<!-- a person's row: right-click (or ContextMenu / Shift+F10 on their name) for their menu, as in the member list -->
{#snippet personRow(userId: string | null, name: string, n: number, count: string, share: number, label: string)}
  {@const mine = !!meId && userId === meId}
  <!-- the menu is a pointer shortcut; from the keyboard, ContextMenu / Shift+F10 on the name button (it bubbles here) -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <li class="prow" class:mine style:--share={share}
    oncontextmenu={(e) => personMenu(e, userId, e.currentTarget)} onkeydown={(e) => personMenu(e, userId, e.currentTarget)}>
    {@render rank(n)}
    <span class="sr">{n}.</span>
    <span class="face-sm" aria-hidden="true">{@render person(userId, name, 24)}</span>
    <span class="who-col">{@render personName(userId, name)}</span>
    {#if mine}<span class="you">you</span>{/if}
    <span class="count"><span class="n" aria-hidden="true">{count}</span><span class="sr">{label}</span></span>
    <span class="bar" aria-hidden="true"></span>
  </li>
{/snippet}

<!-- Most quoted: 2 · 1 · 3 on the podium (read 1, 2, 3), then the rest as rows -->
{#snippet people(list: SpeakerCount[], one: string, many: string, podium: boolean)}
  {@const counts = list.map((c) => c.count)}
  {@const r = ranks(counts)}
  {@const sh = shares(counts)}
  {@const top = podium ? list.slice(0, 3) : []}
  {#if top.length}
    <ol class="podium" aria-label="Top {top.length}">
      {#each top as c, i (`${c.user_id ?? ''}:${c.name ?? ''}`)}
        {@const name = who(c)}
        {@const mine = !!meId && c.user_id === meId}
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <li class="place p{i + 1}" class:mine
          oncontextmenu={(e) => personMenu(e, c.user_id, e.currentTarget)} onkeydown={(e) => personMenu(e, c.user_id, e.currentTarget)}>
          <span class="sr">{r[i]}.</span>
          <span class="face" aria-hidden="true">{@render person(c.user_id, name, i === 0 ? 52 : 40)}</span>
          <span class="who-col">{@render personName(c.user_id, name)}</span>
          <span class="tally"><span class="n" aria-hidden="true">{c.count}</span><span class="sr">{plural(c.count, one, many)}</span>{#if mine}<span class="you">you</span>{/if}</span>
          <span class="step" aria-hidden="true"><span class="place-n">{r[i]}</span></span>
        </li>
      {/each}
    </ol>
  {/if}
  {#if list.length > top.length}
    <ol class="rows" start={top.length + 1}>
      {#each list.slice(top.length) as c, j (`${c.user_id ?? ''}:${c.name ?? ''}`)}
        {@const i = j + top.length}
        {@render personRow(c.user_id, who(c), r[i], String(c.count), sh[i], plural(c.count, one, many))}
      {/each}
    </ol>
  {/if}
{/snippet}

{#snippet head(title: string, unit: string, note = '', any = true)}
  <div class="shead">
    <h3>{title}</h3>
    {#if note}<span class="note">{note}</span>{/if}
    {#if any}<span class="unit" aria-hidden="true">{unit}</span>{/if}
  </div>
{/snippet}

<div class="leaderboard" role="dialog" aria-labelledby="{uid}-title" bind:this={panel} use:clickOutside={onClose}>
  <header class="top">
    <span class="trophy" aria-hidden="true"><Icon name="trophy" size={16} /></span>
    <h2 id="{uid}-title">Leaderboard</h2>
    <button type="button" class="icon-btn icon-btn-sm" aria-label="Close leaderboard" onclick={onClose}><Icon name="x" size={16} /></button>
  </header>

  <div class="tabs" role="tablist" aria-label="Count quotes posted">
    {#each PERIODS as p, i (p.value)}
      <button type="button" role="tab" id="{uid}-tab-{p.value}" aria-selected={period === p.value} aria-controls="{uid}-panel"
        tabindex={period === p.value ? 0 : -1} bind:this={tabEls[i]} onclick={() => pick(p.value)} onkeydown={(e) => tabKey(e, i)}>{p.label}</button>
    {/each}
  </div>

  <div class="scroll">
    {#if error}
      <p class="error" role="alert">{error}</p>
    {:else if !board}
      <Loading />
    {:else if empty}
      <div class="blank">
        <span class="qoty-mark mark" aria-hidden="true">“</span>
        <p><strong>No quotes yet.</strong> Post one in this channel and the board starts counting.</p>
      </div>
    {:else}
      <div id="{uid}-panel" role="tabpanel" aria-labelledby="{uid}-tab-{period}">
        <section aria-labelledby="{uid}-reacted">
          {@render head('Most reacted', 'people', '', board.top.length > 0)}
          <span id="{uid}-reacted" class="sr">Most reacted {within === 'yet' ? 'of all time' : within}</span>
          {#if board.top.length}
            {@const counts = board.top.map((t) => t.score)}
            {@const r = ranks(counts)}
            <ol class="rows">
              {#each board.top as t, i (t.message.id)}
                {@const total = reactionTotal(t.message)}
                {@const label = `${plural(t.score, 'person', 'people')} reacted`}
                {@render quoteRow(t.message, r[i], t.score, label, total > t.score ? `${label} (${plural(total, 'reaction', 'reactions')})` : label)}
              {/each}
            </ol>
          {:else}
            <p class="none">No reactions on quotes {within}.{#if wider}{' '}<button type="button" class="link" onclick={() => pick(wider)}>See this {widerLabel}</button>{/if}</p>
          {/if}
        </section>

        <section aria-labelledby="{uid}-quoted">
          {@render head('Most quoted', 'quotes', '', board.most_quoted.length > 0)}
          <span id="{uid}-quoted" class="sr">Most quoted {within === 'yet' ? 'of all time' : within}</span>
          {#if board.most_quoted.length}
            {@render people(board.most_quoted, 'quote', 'quotes', true)}
          {:else}
            <p class="none">Nobody has been quoted {within}.</p>
          {/if}
        </section>
      </div>

      <section>
        {@render head('Quote awards', 'awards', 'all time', board.most_awarded.length > 0)}
        {#if board.most_awarded.length}
          {@render people(board.most_awarded, 'award', 'awards', false)}
        {:else}
          <p class="none">No awards yet. Award a line you love from its quote.</p>
        {/if}
      </section>

      <section>
        {@render head('Quote of the Year', 'votes', '', board.winners.length > 0)}
        {#if board.winners.length}
          <ol class="wins">
            {#each board.winners as w (`${w.year}:${w.message.id}`)}
              <li class="win">
                <span class="year">{w.year}</span>
                <div class="win-body">
                  <!-- who said it when the vote ended (a later edit or link never moves a win) -->
                  {#if w.message.deleted}
                    <!-- the win stays (spec §3.1), the words don't; nothing to jump to -->
                    {@const by = snapshotSpeakers(app.state, serverId, w.speakers)}
                    <div class="gone">
                      <span class="said">This quote was deleted.</span>
                      {#if by}<span class="by">{by}</span>{/if}
                    </div>
                  {:else}
                    {@const lines = boardLines(app.state, serverId, w.message)}
                    {@const by = w.speakers.length ? snapshotSpeakers(app.state, serverId, w.speakers) : (lines[0]?.by ?? '')}
                    <button type="button" class="jump win-jump" onclick={() => onOpen(w.message)} data-tip={fullText(w.message)}
                      aria-label="{w.year} winner: {fullText(w.message)}. {plural(w.votes, 'vote', 'votes')}. Show in chat">
                      <span class="said clamp" aria-hidden="true">“{lines[0]?.said ?? ''}”{#if lines.length > 1}<span class="more"> +{lines.length - 1} more</span>{/if}</span>
                      {#if by}<span class="by" aria-hidden="true">{by}</span>{/if}
                    </button>
                  {/if}
                  <div class="votes">
                    <VoterStack userIds={w.voters} {nameOf} />
                    <span class="vn">{plural(w.votes, 'vote', 'votes')}</span>
                  </div>
                </div>
              </li>
            {/each}
          </ol>
        {:else}
          <p class="none">No winners yet. Start a Quote of the Year vote from the channel header.</p>
        {/if}
      </section>
    {/if}

    {#if hours.length && !error}
      {@const sh = shares(hours.map((h) => h.minutes))}
      {@const r = ranks(hours.map((h) => h.minutes))}
      <section data-steam-hours>
        {@render head('Most Steam hours', 'played', 'last 2 weeks')}
        <ol class="rows">
          {#each hours as h, i (h.user_id)}
            {@render personRow(h.user_id, nameOf(h.user_id), r[i], hoursText(h.minutes), sh[i], hoursText(h.minutes))}
          {/each}
        </ol>
      </section>
    {/if}
  </div>
  <!-- inside the panel (a press on it isn't outside); its Esc goes no further than the menu -->
  <PersonMenu {menu} onMessage={messagePerson} onClose={() => (menu = null)} />
</div>

<style>
  /* A panel down the right of the channel, under its header (TextChannel's .chan is the frame; 52px is its header),
     over the chat rather than squeezing it. Full width when the window is narrow. */
  .leaderboard { position: absolute; top: 52px; right: 0; bottom: 0; z-index: 30; width: min(440px, 100%); display: flex; flex-direction: column; min-height: 0;
    background: var(--bg-1); border-left: 1px solid var(--line); box-shadow: var(--shadow-3); color: var(--text);
    animation: pane-in var(--dur-slow) var(--ease); will-change: transform, opacity; }
  @keyframes pane-in { from { opacity: 0; transform: translateX(16px); } }

  .top { display: flex; align-items: center; gap: 10px; padding: 14px 12px 10px 18px; }
  .trophy { width: 28px; height: 28px; border-radius: var(--radius-sm); display: grid; place-items: center; background: var(--quote-green-soft); color: var(--quote-green-text); }
  h2 { flex: 1; margin: 0; font-size: 16px; font-weight: 700; }

  /* the period tabs: one track, the chosen tab lifted out of it */
  .tabs { display: grid; grid-template-columns: repeat(4, 1fr); gap: 2px; margin: 0 18px 6px; padding: 3px; border-radius: var(--radius-md); background: var(--bg-inset); }
  .tabs button { height: 30px; padding: 0 6px; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text-3); font: inherit; font-size: 13px; font-weight: 600; white-space: nowrap; }
  .tabs button:hover { color: var(--text); }
  .tabs button[aria-selected='true'] { background: var(--bg-3); color: var(--text); box-shadow: var(--shadow-1); }
  .tabs button:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 1px; }

  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: 6px 10px 18px; }
  section { padding: 12px 0 4px; }
  section + section, [role='tabpanel'] + section { border-top: 1px solid var(--line-soft); margin-top: 8px; }
  [role='tabpanel']:focus-visible { outline: none; }
  .shead { display: flex; align-items: baseline; gap: 8px; padding: 0 8px 8px; }
  h3 { margin: 0; font-size: 14px; font-weight: 700; color: var(--text); }
  .note { color: var(--text-3); font-size: 12px; }
  /* the counts' column heading, right over the numbers */
  .unit { margin-left: auto; padding-right: 28px; color: var(--text-3); font-size: 11px; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }

  ol { list-style: none; margin: 0; padding: 0; }
  .rows { display: flex; flex-direction: column; gap: 2px; }

  /* rank chips: tabular, one width; the top three take the quote greens (the pixel spill's three shades) */
  .rank { flex-shrink: 0; width: 22px; height: 22px; border-radius: var(--radius-xs); display: grid; place-items: center; color: var(--text-3); font-size: 12px; font-weight: 700; font-variant-numeric: tabular-nums; }
  .rank.g1 { background: var(--quote-green); color: var(--on-quote-green); }
  .rank.g2 { background: var(--quote-green-mid); color: var(--on-quote-green); }
  .rank.g3 { background: var(--quote-green-deep); color: var(--island-text); }

  /* a ranked row: rank, the thing, its count; a hairline bar along the bottom shows its share of the leader's count */
  .qrow, .prow { position: relative; display: flex; align-items: flex-start; gap: 10px; padding: 8px 8px 10px; border-radius: var(--radius-sm); }
  .prow { align-items: center; padding: 6px 8px 8px; }
  .qrow:hover, .prow:hover { background: var(--hover-tint-strong); }
  .bar { position: absolute; left: 40px; right: 8px; bottom: 3px; height: 2px; border-radius: 1px; background: var(--quote-green-line); transform: scaleX(var(--share)); transform-origin: left; pointer-events: none; }
  .count { flex-shrink: 0; min-width: 34px; text-align: right; }
  .n { font-size: 15px; font-weight: 700; font-variant-numeric: tabular-nums; color: var(--text); }
  .qrow .count { padding-top: 1px; }
  /* you: the accent's faint wash and edge, as a message that mentions you */
  .mine { background: var(--accent-faint); box-shadow: inset 2px 0 0 var(--focus-ring); }
  .you { flex-shrink: 0; padding: 0 6px; border-radius: var(--radius-sm); background: var(--accent-soft); color: var(--accent-text); font-size: 11px; font-weight: 700; line-height: 18px; }

  /* a quote: its words in the quotes channel's green-edged column, two lines, then who said it */
  .jump { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 5px; padding: 0 0 0 10px; border: 0; border-left: 2px solid var(--quote-green-deep); border-radius: 0; background: transparent; color: inherit; font: inherit; text-align: left; cursor: pointer; }
  .jump:hover .said, .jump:focus-visible .said { color: var(--text); }
  .said { color: var(--text-body); font-size: 14px; line-height: 1.45; overflow-wrap: anywhere; }
  .clamp { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; overflow: hidden; }
  .more { margin-left: 4px; color: var(--text-3); font-size: 12px; white-space: nowrap; }
  .lines { display: flex; flex-direction: column; gap: 4px; }
  .line .by { color: var(--text-3); font-size: 12px; white-space: nowrap; }
  .line .by::before { content: '— '; }
  .byline { display: flex; align-items: center; gap: 6px; min-width: 0; color: var(--text-2); font-size: 12px; }
  .who { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .expand { align-self: flex-start; margin: -2px -4px 0 -6px; color: var(--text-3); }
  .expand :global(svg) { transition: transform var(--dur-fast) var(--ease); }
  .expanded .expand :global(svg) { transform: rotate(180deg); }
  /* room for the chevron's column when a row has none, so every count lines up */
  .qrow:not(:has(.expand)) .count { margin-right: 20px; }

  .face-sm { display: grid; flex-shrink: 0; }
  .who-col { min-width: 0; display: flex; font-size: 14px; font-weight: 600; }
  .prow .who-col { flex: 1; }
  .who-col :global(.name) { border: 0; padding: 0; background: none; font: inherit; color: inherit; text-align: left; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .prow .count { margin-right: 20px; }

  /* the podium: second, first, third left to right (first raised and largest), steps in the three quote greens */
  .podium { display: grid; grid-template-columns: 1fr 1.15fr 1fr; align-items: end; gap: 6px; padding: 4px 8px 8px; }
  .place { min-width: 0; display: flex; flex-direction: column; align-items: center; gap: 4px; padding-top: 6px; border-radius: var(--radius-md) var(--radius-md) 0 0; }
  .place.p1 { grid-column: 2; grid-row: 1; }
  .place.p2 { grid-column: 1; grid-row: 1; }
  .place.p3 { grid-column: 3; grid-row: 1; }
  .place.mine { box-shadow: none; background: var(--accent-faint); }
  .face { display: grid; padding: 2px; border-radius: 50%; box-shadow: 0 0 0 2px var(--quote-green-line); }
  .p1 .face { box-shadow: 0 0 0 2px var(--quote-green); }
  .place .who-col { max-width: 100%; justify-content: center; font-size: 13px; }
  .p1 .who-col { font-size: 14px; }
  .tally { display: flex; align-items: center; gap: 6px; min-height: 20px; }
  .p1 .tally .n { font-size: 18px; }
  /* the step: its place, large, in the green of that place */
  .step { width: 100%; margin-top: 2px; display: grid; place-items: center; border-radius: var(--radius-sm) var(--radius-sm) 0 0; background: var(--quote-green-deep); color: var(--island-text); }
  .p1 .step { height: 56px; background: var(--quote-green); color: var(--on-quote-green); }
  .p2 .step { height: 40px; background: var(--quote-green-mid); color: var(--on-quote-green); }
  .p3 .step { height: 30px; }
  .place-n { font-size: 18px; font-weight: 800; line-height: 1; font-variant-numeric: tabular-nums; }
  .p1 .place-n { font-size: 24px; }

  /* Quote of the Year: the year in green, the winning words, who voted */
  .wins { display: flex; flex-direction: column; gap: 4px; }
  .win { display: flex; align-items: flex-start; gap: 10px; padding: 8px; border-radius: var(--radius-sm); }
  .year { flex-shrink: 0; padding: 2px 8px; border-radius: var(--radius-md); background: var(--quote-green); color: var(--on-quote-green); font-size: 12px; font-weight: 700; font-variant-numeric: tabular-nums; }
  .win-body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 6px; }
  .win-jump .by, .gone .by { color: var(--text-2); font-size: 12px; }
  .win-jump .by::before, .gone .by::before { content: '— '; color: var(--text-3); }
  .gone { display: flex; flex-direction: column; gap: 4px; padding-left: 12px; border-left: 2px solid var(--line); }
  .gone .said { color: var(--text-3); font-style: italic; }
  .votes { display: flex; align-items: center; gap: 8px; padding-left: 12px; }
  .vn { color: var(--text-3); font-size: 12px; font-variant-numeric: tabular-nums; }

  .none { margin: 0 8px 4px; color: var(--text-3); font-size: 13px; line-height: 1.45; }
  .link { border: 0; padding: 0; background: none; color: var(--accent-text); font: inherit; text-decoration: underline; cursor: pointer; }
  .error { margin: 8px; color: var(--danger-text); font-size: 13px; }
  .blank { display: flex; flex-direction: column; align-items: center; gap: 6px; padding: 40px 24px; text-align: center; color: var(--text-2); font-size: 14px; line-height: 1.5; }
  .blank .mark { font-size: 64px; color: var(--quote-green-text); padding-top: 18px; }
  .blank p { margin: 0; max-width: 30ch; }
  .blank strong { color: var(--text); }
</style>
