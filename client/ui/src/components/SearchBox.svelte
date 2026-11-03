<script lang="ts" module>
  let boxes = 0
</script>

<script lang="ts">
  // The channel header's search field: words plus filter chips, with a suggestion list (a combobox)
  // that offers the filters, then who / which channel / what kind for the one being typed. Enter runs
  // the search; the results show in the right-hand panel (SearchResults).
  import { tick, untrack } from 'svelte'
  import Avatar from './Avatar.svelte'
  import Icon from './Icon.svelte'
  import Name from './Name.svelte'
  import { app } from '../lib/store.svelte'
  import { displayName, namesOf, serverPeople, userOf } from '../lib/selectors'
  import { closeSearch, runSearch, search, type SearchScope } from '../lib/search.svelte'
  import { rankNames } from '../lib/mentions'
  import { activeTerm, chipText, filterRows, HAS_VALUES, hasSearchableWords, parseText, replaceTerm, type Chip, type FilterKey } from '../lib/searchQuery'

  let { serverId, channelId }: {
    /** The server searched; null in a DM or group, which is searched on its own. */
    serverId: string | null
    channelId: string
  } = $props()

  const uid = `search-${++boxes}`
  const scope = $derived<SearchScope>(serverId ? { kind: 'server', serverId } : { kind: 'conversation', channelId })
  const serverName = $derived(serverId ? (app.state.servers.find((s) => s.id === serverId)?.name ?? 'this server') : null)
  const placeholder = $derived(serverName ? `Search ${serverName}` : 'Search this conversation')

  // Who `from:` and `mentions:` can name: the server's members, or the conversation's people. Matched
  // on every name they have (username, display name, nickname, your private nickname; #47), shown by
  // the name that applies here.
  const people = $derived.by(() => {
    if (serverId) return serverPeople(app.state, serverId).map((p) => ({ ...p, alt: p.username }))
    const ids = app.state.dmMembers[channelId] ?? []
    return ids
      .filter((id) => id === app.state.me?.id || app.state.people[id])
      .map((id) => ({ id, name: displayName(app.state, null, id), alt: userOf(app.state, id)?.username, alts: namesOf(app.state, null, id) }))
  })
  const channels = $derived(
    serverId
      ? Object.values(app.state.channels)
          .filter((c) => c.server_id === serverId && (c.kind === 'text' || c.kind === 'quotes' || c.kind === 'clips'))
          .sort((a, b) => a.position - b.position)
          .map((c) => ({ id: c.id, name: c.name ?? '' }))
      : [],
  )

  let input: HTMLInputElement
  // The box is rebuilt with its channel view, and clicking a result in another channel does that:
  // while those results are open, start from what produced them, so they can be refined.
  const sameScope = (s: SearchScope | null) =>
    !!s && (s.kind === 'server' ? s.serverId === serverId : !serverId && s.channelId === channelId)
  const resume = untrack(() => search.open && sameScope(search.scope))
  let text = $state(untrack(() => (resume ? search.words : '')))
  let caret = $state(untrack(() => text.length))
  let chips = $state<Chip[]>(untrack(() => (resume ? [...search.chips] : [])))
  /** Words that can't find anything were entered: say what to type instead. */
  let hint = $state(false)
  let focused = $state(false)
  /** The row Enter picks: set by typing and the arrow keys only. The mouse hovers (CSS) and clicks,
   *  but never makes a row the one Enter picks: a pointer resting on the list would take the Enter
   *  that was meant to search. */
  let active = $state(-1)
  /** Esc closed the list: it stays shut until the text or the caret moves. */
  let dismissedAt = $state<string | null>(null)

  type Option =
    | { kind: 'filter'; key: FilterKey; syntax: string; hint: string }
    | { kind: 'person'; id: string; name: string }
    | { kind: 'channel'; id: string; name: string }
    | { kind: 'has'; value: (typeof HAS_VALUES)[number]['value']; hint: string }

  const term = $derived(activeTerm(text, caret))
  const options = $derived.by<Option[]>(() => {
    if (!term.filter) return filterRows(term.token, !!serverId).map((f) => ({ kind: 'filter' as const, ...f }))
    if (term.filter === 'from' || term.filter === 'mentions') return rankNames(people, term.value).map((p) => ({ kind: 'person' as const, id: p.id, name: p.name }))
    if (term.filter === 'in') return serverId ? rankNames(channels, term.value).map((c) => ({ kind: 'channel' as const, ...c })) : []
    const v = term.value.toLowerCase()
    return HAS_VALUES.filter((h) => h.value.startsWith(v)).map((h) => ({ kind: 'has' as const, ...h }))
  })
  const here = $derived(`${text}\u0000${caret}`)
  const open = $derived(focused && options.length > 0 && dismissedAt !== here)
  // A value list has its best match ready for Enter; the filter list waits for a choice, so Enter
  // on plain words searches.
  $effect(() => {
    void options
    active = term.filter ? 0 : -1
  })
  const optionId = (i: number) => `${uid}-opt-${i}`


  function sync() {
    caret = input.selectionStart ?? text.length
  }

  async function setText(next: { text: string; caret: number }) {
    text = next.text
    caret = next.caret
    await tick()
    // A result clicked meanwhile can rebuild the channel header, and this box with it.
    if (!input?.isConnected) return
    input.value = text
    input.setSelectionRange(caret, caret)
    input.focus()
  }

  function addChip(c: Chip) {
    // One person per from: and mentions:, one channel: a new one replaces the old.
    chips = [...chips.filter((x) => c.key === 'has' ? !(x.key === 'has' && x.value === c.value) : x.key !== c.key), c]
  }

  function choose(o: Option) {
    if (o.kind === 'filter') return void setText(replaceTerm(text, term.start, caret, `${o.key}:`))
    const key = term.filter
    if (!key) return
    if (o.kind === 'has') addChip({ key: 'has', value: o.value, label: o.value })
    else if (key === 'from' || key === 'mentions' || key === 'in') addChip({ key, id: o.id, label: o.name })
    void setText(replaceTerm(text, term.start, caret, ''))
  }

  /** Filters typed out in full become chips when they name someone or something; the rest are words. */
  function absorbTyped(): string {
    const { words, filters } = parseText(text)
    const left: string[] = []
    for (const f of filters) {
      const v = f.value.toLowerCase()
      if (f.key === 'has') {
        const h = HAS_VALUES.find((x) => x.value === v)
        if (h) addChip({ key: 'has', value: h.value, label: h.value })
        else left.push(`has:${f.value}`)
      } else if (f.key === 'in') {
        const c = channels.find((x) => x.name.toLowerCase() === v)
        if (c) addChip({ key: 'in', id: c.id, label: c.name })
        else left.push(`in:${f.value}`)
      } else {
        const p = people.find((x) => x.name.toLowerCase() === v || x.alt?.toLowerCase() === v || x.alts.some((a) => a.toLowerCase() === v))
        if (p) addChip({ key: f.key, id: p.id, label: p.name })
        else left.push(`${f.key}:${f.value}`)
      }
    }
    return [words, ...left].filter((w) => w).join(' ')
  }

  function run() {
    const words = absorbTyped()
    if (!words.trim() && !chips.length) return
    // Emoji or punctuation alone can't match anything (the index holds words): say so rather than
    // send a search the server can only refuse.
    if (!chips.length && !hasSearchableWords(words)) {
      hint = true
      return
    }
    void setText({ text: words, caret: words.length })
    dismissedAt = `${words}\u0000${words.length}`
    void runSearch(scope, words, chips)
  }

  function keydown(e: KeyboardEvent) {
    if (open && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
      e.preventDefault()
      const n = options.length
      active = e.key === 'ArrowDown' ? (active + 1) % n : active <= 0 ? n - 1 : active - 1
      return
    }
    if ((e.key === 'Enter' || e.key === 'Tab') && open && active >= 0) {
      e.preventDefault()
      choose(options[active])
      return
    }
    if (e.key === 'Enter') {
      e.preventDefault()
      run()
      return
    }
    if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      // One step back each time: the list, the results, what's typed, then the box itself.
      if (open) dismissedAt = here
      else if (search.open) closeSearch()
      else if (text || chips.length) {
        chips = []
        hint = false
        void setText({ text: '', caret: 0 })
      } else input.blur()
      return
    }
    if (e.key === 'Backspace' && input.selectionStart === 0 && input.selectionEnd === 0 && chips.length) {
      e.preventDefault()
      chips = chips.slice(0, -1)
    }
  }

  function removeChip(i: number) {
    chips = chips.filter((_, j) => j !== i)
    input.focus()
  }

  // Ctrl+F (the find key people try first) focuses the box instead of the page's own find. Not
  // while a dialog is open over it (or something else took the key): the box is behind it.
  function globalKey(e: KeyboardEvent) {
    if (e.defaultPrevented || document.querySelector('[aria-modal="true"], .modal-overlay')) return
    if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'f') {
      e.preventDefault()
      input.focus()
      input.select()
    }
  }
</script>

<svelte:window onkeydown={globalKey} />

<div class="search" class:focused>
  <Icon name="search" size={15} />
  {#each chips as c, i (chipText(c))}
    <span class="chip">{chipText(c)}<button type="button" tabindex="-1" aria-label="Remove {chipText(c)}" onclick={() => removeChip(i)}><Icon name="x" size={11} /></button></span>
  {/each}
  <input bind:this={input} class="field" type="text" placeholder={chips.length ? '' : placeholder} aria-label={placeholder} autocomplete="off" spellcheck="false"
    role="combobox" aria-autocomplete="list" aria-haspopup="listbox" aria-expanded={open} aria-controls={open ? `${uid}-list` : undefined}
    aria-activedescendant={open && active >= 0 ? optionId(active) : undefined}
    value={text}
    oninput={() => { text = input.value; hint = false; sync() }}
    onkeydown={keydown}
    onkeyup={(e) => { if (e.key.startsWith('Arrow') && !open) sync() }}
    onclick={sync}
    onfocus={() => (focused = true)}
    onblur={() => (focused = false)} />
  {#if open}
  <div id="{uid}-list" class="list menu-surface pop-in" role="listbox" aria-label={term.filter ? 'Suggestions' : 'Search filters'}>
      {#each options as o, i (i)}
        <div id={optionId(i)} class="menu-item" class:hi={i === active} role="option" tabindex="-1" aria-selected={i === active}
          onmousedown={(e) => { e.preventDefault(); choose(o) }}>
          {#if o.kind === 'filter'}
            <span class="syntax">{o.syntax}</span> <small>{o.hint}</small>
          {:else if o.kind === 'person'}
            <Avatar id={o.id} name={o.name} size={20} /> <Name userId={o.id} {serverId} text={o.name} interactive={false} surface="menu" />
          {:else if o.kind === 'channel'}
            <Icon name="hash" size={14} /> <span>{o.name}</span>
          {:else}
            <span class="syntax">{o.value}</span> <small>{o.hint}</small>
          {/if}
        </div>
      {/each}
  </div>
  {/if}
  <p class="hint" aria-live="polite">{hint ? 'Type a word or pick a filter' : ''}</p>
</div>

<style>
  /* the shared .input look, holding chips beside the text */
  .search { position: relative; width: 240px; min-width: 0; height: 32px; padding: 0 8px; display: flex; align-items: center; gap: 6px; border: 1px solid var(--bg-4); border-radius: 8px; background: var(--bg-1); color: var(--text-3); transition: width var(--dur) var(--ease); }
  .search:hover { border-color: var(--line-menu); }
  .search.focused { width: 320px; outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  .field { flex: 1; min-width: 60px; height: 100%; padding: 0; border: 0; background: none; color: var(--text); font-size: 13px; outline: none; text-overflow: ellipsis; }
  .field::placeholder { color: var(--text-3); }
  .chip { flex-shrink: 0; display: inline-flex; align-items: center; gap: 2px; height: 22px; padding: 0 2px 0 7px; border-radius: 6px; background: var(--accent-soft); color: var(--text); font-size: 12px; white-space: nowrap; }
  .chip button { display: grid; place-items: center; width: 18px; height: 18px; padding: 0; border: 0; border-radius: 4px; background: none; color: var(--text-2); }
  .chip button:hover { background: var(--bg-4); color: var(--text); }
  .list { position: absolute; top: calc(100% + 6px); right: 0; z-index: 20; width: 320px; max-height: 320px; overflow-y: auto; }
  .list .menu-item { cursor: pointer; }
  .hint { position: absolute; top: calc(100% + 4px); left: 0; z-index: 19; margin: 0; font-size: 12px; color: var(--text-3); white-space: nowrap; pointer-events: none; }
  .hint:empty { display: none; }
  .syntax { color: var(--text); font-weight: 600; }
  .list small { margin-left: auto; padding-left: 12px; color: var(--text-3); font-size: 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
</style>
