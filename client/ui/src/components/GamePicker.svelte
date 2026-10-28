<script lang="ts">
  // The game of a clip: a type-ahead over the server's games, with "Add 'X'" for a name it doesn't know.
  // A detected title (`guess`) selects the game it matches, or is offered as "Add 'X'" until picked.
  import { onMount } from 'svelte'
  import { matchGame } from '../lib/clips'
  import { applyEvent } from '../lib/state'
  import { collapseName, nameKey } from '../lib/quotes'
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'

  let { serverId, value, guess = null, onChange }: { serverId: string; value: string | null; guess?: string | null; onChange: (gameId: string | null) => void } = $props()

  const MAX_NAME = 40

  const games = $derived(app.state.games[serverId] ?? [])
  let query = $state('')
  let open = $state(false)
  let offered = $state(false) // a guessed new name is on offer without the input being focused
  let hi = $state(0)
  let busy = $state(false)
  let error = $state('')

  const name = $derived(collapseName(query))
  const matches = $derived(games.filter((g) => nameKey(g.name).includes(nameKey(name))))
  const canAdd = $derived(name !== '' && [...name].length <= MAX_NAME && !games.some((g) => nameKey(g.name) === nameKey(name)))
  const list = $derived(name === '' && !offered ? games : matches)
  const rows = $derived<({ game: (typeof games)[number] } | { add: string })[]>([...list.map((game) => ({ game })), ...(canAdd ? [{ add: name }] : [])])
  const shown = $derived((open || offered) && rows.length > 0)

  onMount(() => {
    const m = matchGame(guess, games)
    if (value === null && m) {
      if ('game' in m) {
        query = m.game.name
        onChange(m.game.id)
      } else {
        query = m.add
        offered = true
      }
    } else {
      query = games.find((g) => g.id === value)?.name ?? ''
    }
  })

  // The chosen game changed from outside: show its name.
  $effect(() => {
    const g = value ? games.find((x) => x.id === value) : undefined
    if (g && !open) query = g.name
  })

  function pick(id: string, label: string) {
    query = label
    open = false
    offered = false
    error = ''
    onChange(id)
  }

  async function add(newName: string) {
    if (busy) return
    busy = true
    error = ''
    try {
      const g = await api.addGame(serverId, newName)
      app.state = applyEvent(app.state, { t: 'GameAdded', d: { game: g } }, Date.now())
      pick(g.id, g.name)
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }

  function choose(r: (typeof rows)[number]) {
    if ('game' in r) pick(r.game.id, r.game.name)
    else void add(r.add)
  }

  function onInput(e: Event & { currentTarget: HTMLInputElement }) {
    query = e.currentTarget.value
    open = true
    offered = false
    hi = 0
    if (collapseName(query) === '') onChange(null)
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      open = true
      if (rows.length) hi = (hi + (e.key === 'ArrowDown' ? 1 : rows.length - 1)) % rows.length
    } else if (e.key === 'Enter' && shown && rows[hi]) {
      e.preventDefault()
      choose(rows[hi])
    } else if (e.key === 'Escape' && open) {
      e.preventDefault() // closes the list, not the dialog
      open = false
    }
  }
</script>

<div class="picker">
  <input
    type="text"
    role="combobox"
    aria-label="Game"
    aria-expanded={shown}
    aria-controls="game-list"
    aria-autocomplete="list"
    placeholder="Game (optional)"
    maxlength={MAX_NAME}
    autocomplete="off"
    value={query}
    oninput={onInput}
    onfocus={() => { open = true; offered = false }}
    onblur={() => setTimeout(() => (open = false), 120)}
    onkeydown={onKey}
  />
  {#if shown}
    <ul id="game-list" role="listbox" aria-label="Games">
      {#each rows as r, i ('game' in r ? r.game.id : 'add')}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- mousedown keeps the input focused so the click lands before blur closes the list -->
        <li role="option" aria-selected={i === hi} class:hi={i === hi} class:add={'add' in r} onmousedown={(e) => e.preventDefault()} onclick={() => choose(r)}>
          {'game' in r ? r.game.name : `Add '${r.add}'`}
        </li>
      {/each}
    </ul>
  {/if}
  {#if error}<p class="err" role="alert">{error}</p>{/if}
</div>

<style>
  .picker { position: relative; }
  input { width: 100%; box-sizing: border-box; height: 34px; padding: 0 10px; border-radius: 8px; border: 1px solid var(--bg-4); background: var(--bg-1); color: var(--text); font: inherit; }
  ul { position: absolute; z-index: 5; left: 0; right: 0; top: calc(100% + 4px); margin: 0; padding: 4px; list-style: none; max-height: 180px; overflow-y: auto; background: var(--bg-3); border: 1px solid var(--line); border-radius: 8px; }
  li { padding: 6px 8px; border-radius: 6px; cursor: pointer; font-size: 14px; }
  li.hi, li:hover { background: var(--bg-4); }
  li.add { color: var(--accent); }
  .err { margin: 4px 0 0; font-size: 12px; color: var(--warn); }
</style>
