<script lang="ts">
  // A builds channel's right panel (#617): Games ("All games", then each game with a build here, with counts that
  // follow Show archived and the search's words, #664; a click filters the view), the Show archived checkbox, and Archive old builds. The panel
  // and its Games are the shared GameFilterPanel (#663).
  import BulkArchive from './BulkArchive.svelte'
  import GameFilterPanel from './GameFilterPanel.svelte'
  import { app } from '../lib/store.svelte'
  import type { BuildFacets } from '../lib/protocol/BuildFacets'

  let { channelId, serverId, id, facets, game, onGame, archived, onArchived }: {
    channelId: string
    /** For the header toggle's `aria-controls`. */
    id?: string
    serverId: string
    facets: BuildFacets | undefined
    game: string | undefined
    onGame: (id: string | undefined) => void
    archived: boolean
    onArchived: (on: boolean) => void
  } = $props()

  const games = $derived(app.state.games[serverId] ?? [])
</script>

<GameFilterPanel class="builds-panel" {id} {serverId} games={facets?.games} total={facets?.total} selected={game} onSelect={onGame}>
  <section>
    <label class="check">
      <input type="checkbox" role="switch" class="switch" checked={archived} onchange={(e) => onArchived(e.currentTarget.checked)} />
      <span>Show archived</span>
    </label>
  </section>
  <div class="sep" role="separator"></div>
  <BulkArchive {channelId} {games} selected={game} />
</GameFilterPanel>

<style>
  /* the app-wide on/off switch (app.css .switch), laid out like Inbox filters' "Only unread"; its text on the panel's
     --inset edge */
  .check { display: flex; align-items: center; justify-content: space-between; gap: 10px; padding-inline: var(--inset); font-size: 14px; color: var(--text-2); cursor: pointer; }
  .check span { min-width: 0; }
</style>
