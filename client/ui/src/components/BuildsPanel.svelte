<script lang="ts">
  // A builds channel's right panel (#617): Games ("All games", then each game with a build here, with counts that
  // follow Show archived; a click filters the view), the Show archived checkbox, and Archive old builds.
  import BulkArchive from './BulkArchive.svelte'
  import { app } from '../lib/store.svelte'
  import type { BuildFacets } from '../lib/protocol/BuildFacets'

  let { channelId, serverId, facets, game, onGame, archived, onArchived }: {
    channelId: string
    serverId: string
    facets: BuildFacets | undefined
    game: string | undefined
    onGame: (id: string | undefined) => void
    archived: boolean
    onArchived: (on: boolean) => void
  } = $props()

  const games = $derived(app.state.games[serverId] ?? [])
  // each game with builds here, most first (as the server counts them); one the registry doesn't know yet waits for it
  const rows = $derived(
    (facets?.games ?? [])
      .map((g) => ({ id: g.game_id, count: g.count, name: games.find((x) => x.id === g.game_id)?.name }))
      .filter((g): g is { id: string; count: number; name: string } => !!g.name),
  )
</script>

<aside class="builds-panel" aria-label="Games">
  <section>
    <h3 class="menu-label">Games</h3>
    <div class="games">
      <button type="button" aria-pressed={!game} onclick={() => onGame(undefined)}>
        <span class="name">All games</span><span class="n">{facets?.total ?? ''}</span>
      </button>
      {#each rows as g (g.id)}
        <button type="button" aria-pressed={game === g.id} onclick={() => onGame(g.id)}>
          <span class="name">{g.name}</span><span class="n">{g.count}</span>
        </button>
      {/each}
    </div>
  </section>
  <label class="check">
    <input type="checkbox" checked={archived} onchange={(e) => onArchived(e.currentTarget.checked)} />
    Show archived
  </label>
  <BulkArchive {channelId} {games} selected={game} />
</aside>

<style>
  .builds-panel { width: 260px; flex-shrink: 0; overflow-y: auto; padding: 14px 14px 18px; box-sizing: border-box; display: flex; flex-direction: column; gap: 18px; border-left: 1px solid var(--bg-3); background: var(--bg-1); }
  section { display: flex; flex-direction: column; gap: 6px; }
  h3 { margin: 0; }
  .games { display: flex; flex-direction: column; gap: 1px; }
  .games button { display: flex; align-items: center; gap: 8px; padding: 6px 8px; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text-2); font: inherit; font-size: 14px; text-align: left; }
  .games button:hover { background: var(--bg-3); color: var(--text); }
  .games button[aria-pressed='true'] { background: var(--bg-4); color: var(--text); font-weight: 600; }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .n { color: var(--text-3); font-size: 12px; font-variant-numeric: tabular-nums; }
  .check { display: flex; align-items: center; gap: 8px; font-size: 14px; color: var(--text-2); cursor: pointer; }
  .check input { width: 16px; height: 16px; margin: 0; accent-color: var(--accent); }
</style>
