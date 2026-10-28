<script lang="ts">
  // A clips channel's header: its name, the filter chips (All, each game used here, By person ▾), the
  // list/wall toggle and "Share a clip".
  import Icon from './Icon.svelte'
  import { clickOutside } from '../lib/actions'
  import { clipFilter, type ClipFilter } from '../lib/clips'
  import { displayName } from '../lib/selectors'
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import type { ClipView } from '../lib/protocol/ClipView'

  let { channelId, serverId, filter, onFilter, view, onView = () => {}, onShare }: {
    channelId: string
    serverId: string
    filter: ClipFilter
    onFilter: (f: ClipFilter) => void
    /** The view shown (the caller reads it from state; the toggle updates state itself). */
    view: ClipView
    /** Told after the toggle switched the view. */
    onView?: (v: ClipView) => void
    onShare: () => void
  } = $props()

  const channel = $derived(app.state.channels[channelId])
  const facets = $derived(app.state.clipFacets[channelId])
  const games = $derived(app.state.games[serverId] ?? [])
  // Each game used here, by name (a game the registry doesn't know yet has no chip until it arrives).
  const gameChips = $derived(
    (facets?.games ?? [])
      .map((g) => ({ id: g.game_id, name: games.find((x) => x.id === g.game_id)?.name }))
      .filter((g): g is { id: string; name: string } => !!g.name)
      .sort((a, b) => a.name.localeCompare(b.name)),
  )
  const authors = $derived((facets?.authors ?? []).map((a) => ({ id: a.user_id, count: a.count, name: displayName(app.state, serverId, a.user_id) })))
  const personLabel = $derived(filter.author ? `${displayName(app.state, serverId, filter.author)} ▾` : 'By person ▾')

  let people = $state(false)
  let error = $state('')

  function pickGame(id: string) {
    onFilter(clipFilter(filter.game === id ? null : id, filter.author))
  }
  function pickAuthor(id: string | null) {
    people = false
    onFilter(clipFilter(filter.game, id))
  }

  // Optimistic: the view switches now, and goes back if the server refuses.
  function setView(v: ClipView) {
    if (v === view) return
    const was = view
    error = ''
    app.state = { ...app.state, clipViews: { ...app.state.clipViews, [channelId]: v } }
    onView(v)
    api.setClipView(channelId, v).catch((e) => {
      if (app.state.clipViews[channelId] === v) app.state = { ...app.state, clipViews: { ...app.state.clipViews, [channelId]: was } }
      error = errorText(e)
    })
  }
</script>

<div class="head">
  <span class="ico"><Icon name="film" size={18} /></span>
  <span class="title">{channel?.name ?? ''}</span>
  <div class="chips" role="group" aria-label="Filter clips">
    <button type="button" class="chip" aria-pressed={!filter.game && !filter.author} onclick={() => onFilter({})}>All</button>
    {#each gameChips as g (g.id)}
      <button type="button" class="chip" aria-pressed={filter.game === g.id} onclick={() => pickGame(g.id)}>{g.name}</button>
    {/each}
    <div class="person">
      <button type="button" class="chip" aria-pressed={!!filter.author} aria-haspopup="menu" aria-expanded={people} data-menu-toggle
        onclick={() => (people = !people)}>{personLabel}</button>
      {#if people}
        <div class="menu" role="menu" aria-label="By person" use:clickOutside={() => (people = false)}>
          <button type="button" role="menuitemradio" aria-checked={!filter.author} onclick={() => pickAuthor(null)}>Everyone</button>
          {#each authors as a (a.id)}
            <button type="button" role="menuitemradio" aria-checked={filter.author === a.id} onclick={() => pickAuthor(a.id)}>
              <span class="name">{a.name}</span> <span class="count">{a.count}</span>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  </div>
  <span class="grow"></span>
  {#if error}<span class="err" role="alert">{error}</span>{/if}
  <div class="toggle" role="group" aria-label="View">
    <button type="button" class="tbtn" aria-label="List view" data-tip="List" aria-pressed={view === 'list'} onclick={() => setView('list')}><Icon name="list" size={16} /></button>
    <button type="button" class="tbtn" aria-label="Wall view" data-tip="Wall" aria-pressed={view === 'wall'} onclick={() => setView('wall')}><Icon name="grid" size={16} /></button>
  </div>
  <button type="button" class="share primary" onclick={onShare}>Share a clip</button>
</div>

<style>
  .head { min-height: 52px; flex-shrink: 0; padding: 8px 18px; box-sizing: border-box; display: flex; align-items: center; flex-wrap: wrap; gap: 10px; border-bottom: 1px solid var(--bg-3); color: var(--text-3); }
  .ico { display: flex; }
  .title { color: var(--text); font-size: 15px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 220px; }
  .chips { display: flex; align-items: center; flex-wrap: wrap; gap: 6px; margin-left: 6px; }
  .chip { height: 28px; padding: 0 11px; border-radius: 14px; border: 1px solid var(--bg-4); background: transparent; color: var(--text-2); font-size: 12.5px; font-weight: 600; white-space: nowrap; }
  .chip:hover { background: var(--bg-3); color: var(--text); }
  .chip[aria-pressed='true'] { background: var(--accent); border-color: var(--accent); color: var(--on-accent); }
  .person { position: relative; }
  .menu { position: absolute; z-index: 6; left: 0; top: calc(100% + 4px); min-width: 180px; max-height: 300px; overflow-y: auto; padding: 4px; display: flex; flex-direction: column; background: var(--bg-3); border: 1px solid var(--line); border-radius: 10px; box-shadow: 0 8px 24px rgba(0, 0, 0, .35); }
  .menu button { height: 32px; padding: 0 10px; border: 0; border-radius: 7px; background: none; color: var(--text); display: flex; align-items: center; gap: 8px; font-size: 13.5px; text-align: left; }
  .menu button:hover, .menu button:focus-visible { background: var(--bg-4); }
  .menu button[aria-checked='true'] { color: var(--accent); }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .count { color: var(--text-3); font-size: 12px; font-variant-numeric: tabular-nums; }
  .grow { flex: 1; }
  .err { color: var(--danger); font-size: 12.5px; }
  .toggle { display: flex; padding: 2px; border-radius: 9px; background: var(--bg-2); }
  .tbtn { width: 30px; height: 28px; border: 0; border-radius: 7px; background: transparent; color: var(--text-3); display: grid; place-items: center; }
  .tbtn:hover { color: var(--text); }
  .tbtn[aria-pressed='true'] { background: var(--bg-4); color: var(--text); }
  .share { height: 32px; padding: 0 14px; border-radius: 9px; font-size: 13px; font-weight: 600; white-space: nowrap; }
</style>
