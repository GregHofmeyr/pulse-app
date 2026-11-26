<script lang="ts">
  // A channel's right-hand filter panel (#617, shared with Clips in #663): Games ("All games", then each game used
  // here, with counts as the server gives them; a click filters), then any further sections the channel adds, each
  // behind a rule. Builds adds Show archived and Archive old builds; Clips adds People.
  import type { Snippet } from 'svelte'
  import FilterList from './FilterList.svelte'
  import { app } from '../lib/store.svelte'
  import type { GameCount } from '../lib/protocol/GameCount'

  let { serverId, games, total, selected, onSelect, label = 'Games', class: cls = '', id, children }: {
    serverId: string
    /** Counts per game, in the server's order (most first); undefined until loaded. */
    games: GameCount[] | undefined
    /** "All games": every item, those with no game too; no number while unknown. */
    total: number | null | undefined
    /** The picked game's id (undefined: all games). */
    selected: string | undefined
    onSelect: (id: string | undefined) => void
    /** The panel's accessible name. */
    label?: string
    /** The channel's own class for the panel (`builds-panel`, `clips-panel`). */
    class?: string
    /** For the header toggle's `aria-controls`. */
    id?: string
    /** Further sections; put a `<div class="sep" role="separator"></div>` between two of your own. */
    children?: Snippet
  } = $props()

  const registry = $derived(app.state.games[serverId] ?? [])
  // a game the registry doesn't know yet has no row until it arrives
  const rows = $derived(
    (games ?? [])
      .map((g) => ({ id: g.game_id, count: g.count, name: registry.find((x) => x.id === g.game_id)?.name }))
      .filter((g): g is { id: string; count: number; name: string } => !!g.name),
  )
</script>

<aside class="filter-panel {cls}" aria-label={label} {id}>
  <FilterList heading="Games" kind="games" allLabel="All games" allCount={total} {rows} {selected} {onSelect} />
  {#if children}
    <div class="sep" role="separator"></div>
    {@render children()}
  {/if}
</aside>

<style>
  /* One left edge: section headers and rows put their text --inset in from the content box; a channel's own controls
     that fill the box use 10px padding to land on the same edge. Sections share one vertical rhythm and are split by
     the settings/sidebar rule. The panel keeps its width (the feed beside it gives way) and scrolls, never spills. */
  .filter-panel { --inset: 10px; width: 260px; flex-shrink: 0; overflow-y: auto; padding: 0 12px 4px; box-sizing: border-box; display: flex; flex-direction: column; border-left: 1px solid var(--bg-3); background: var(--bg-1); }
  .filter-panel > :global(section) { display: flex; flex-direction: column; gap: 8px; padding-block: 14px; min-width: 0; }
  .filter-panel > :global(.sep) { height: 1px; flex-shrink: 0; background: var(--line-soft); }
</style>
