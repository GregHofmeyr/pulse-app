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
  <div class="sep" role="separator"></div>
  <section>
    <label class="check">
      <input type="checkbox" role="switch" class="switch" checked={archived} onchange={(e) => onArchived(e.currentTarget.checked)} />
      <span>Show archived</span>
    </label>
  </section>
  <div class="sep" role="separator"></div>
  <BulkArchive {channelId} {games} selected={game} />
</aside>

<style>
  /* One left edge: section headers, game rows, the switch row and the field labels put their text --inset in from
     the content box; the Archive old builds controls fill the content box, and their own padding (10px) lands their
     text on the same edge. Sections share one vertical rhythm and are split by the settings/sidebar rule. */
  .builds-panel { --inset: 10px; width: 260px; flex-shrink: 0; overflow-y: auto; padding: 0 12px 4px; box-sizing: border-box; display: flex; flex-direction: column; border-left: 1px solid var(--bg-3); background: var(--bg-1); }
  .builds-panel > :global(section) { display: flex; flex-direction: column; gap: 8px; padding-block: 14px; min-width: 0; }
  .sep { height: 1px; flex-shrink: 0; background: var(--line-soft); }
  h3 { margin: 0; padding: 0 var(--inset); }
  .games { display: flex; flex-direction: column; gap: 1px; margin-top: -2px; }
  .games button { display: flex; align-items: center; gap: 8px; min-width: 0; padding: 6px var(--inset); border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text-2); font: inherit; font-size: 14px; text-align: left; }
  .games button:hover { background: var(--bg-3); color: var(--text); }
  .games button[aria-pressed='true'] { background: var(--bg-4); color: var(--text); font-weight: 600; }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .n { flex-shrink: 0; color: var(--text-3); font-size: 12px; font-variant-numeric: tabular-nums; }
  /* the app-wide on/off switch (app.css .switch), laid out like Inbox filters' "Only unread" */
  .check { display: flex; align-items: center; justify-content: space-between; gap: 10px; padding-inline: var(--inset); font-size: 14px; color: var(--text-2); cursor: pointer; }
  .check span { min-width: 0; }
</style>
