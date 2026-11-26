<script lang="ts">
  // A clips channel's right panel (#663), the builds channel's design: Games ("All games", then each game used here,
  // with counts), then People ("Everyone", then each person who shared one, with counts). A click filters the feed.
  // Counts are the whole channel's (as the server gives them), not narrowed by the other filter.
  import FilterList from './FilterList.svelte'
  import GameFilterPanel from './GameFilterPanel.svelte'
  import { displayName } from '../lib/selectors'
  import { app } from '../lib/store.svelte'
  import type { ClipFacets } from '../lib/protocol/ClipFacets'
  import type { ClipFilter } from '../lib/clips'
  import { clipFilter } from '../lib/clips'

  let { serverId, id, facets, filter, onFilter }: {
    serverId: string
    /** For the header toggle's `aria-controls`. */
    id: string
    facets: ClipFacets | undefined
    filter: ClipFilter
    onFilter: (f: ClipFilter) => void
  } = $props()

  // the picked person always keeps a row (count 0 once the counts say they have none), so something shows as picked
  const people = $derived.by(() => {
    const rows: { id: string; count: number | null; name: string }[] = (facets?.authors ?? []).map((a) => ({ id: a.user_id, count: a.count, name: displayName(app.state, serverId, a.user_id) }))
    const picked = filter.author
    if (!picked || rows.some((r) => r.id === picked)) return rows
    return [...rows, { id: picked, count: facets ? 0 : null, name: displayName(app.state, serverId, picked) }]
  })
</script>

<GameFilterPanel class="clips-panel" {id} label="Filter clips" {serverId} games={facets?.games} total={facets?.total} selected={filter.game}
  onSelect={(g) => onFilter(clipFilter(g, filter.author))}>
  <FilterList heading="People" kind="people" allLabel="Everyone" allCount={facets?.total} rows={people} selected={filter.author}
    onSelect={(a) => onFilter(clipFilter(filter.game, a))} />
</GameFilterPanel>
