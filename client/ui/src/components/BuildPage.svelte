<script lang="ts">
  // One build's page (#618, spec §4.1): Back, the build (BuildView, with its Actions menu), and its Comments (a
  // ThreadChat, the author's tagged "author"). It replaces the channel's browse view; Back returns to it.
  import BuildActions from './BuildActions.svelte'
  import BuildView from './BuildView.svelte'
  import Icon from './Icon.svelte'
  import ImageViewer from './ImageViewer.svelte'
  import ThreadChat from './ThreadChat.svelte'
  import { attachmentLink } from '../lib/attachmentLinks'
  import { buildGameName, buildRights, buildShots, isArchived, isQuickDrop, type BuildScreenshot } from '../lib/builds'
  import { relativeTime } from '../lib/conversations'
  import { nameOfIn } from '../lib/selectors'
  import { app, clock } from '../lib/store.svelte'
  import { api } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'

  let { message, serverId, onBack }: {
    /** The build (kept current by the caller). */
    message: Message
    serverId: string
    onBack: () => void
  } = $props()

  const b = $derived(message.build!)
  const nameOf = $derived(nameOfIn(() => app.state, serverId))
  const now = $derived(Date.parse(clock.nowIso))
  const rights = $derived(buildRights(app.state, serverId, message))

  // --- screenshots: every image's link in one request (as MessageFiles does), shown as each arrives ---
  const shots = $derived(buildShots(message))
  let urls = $state<Record<string, string>>({})
  $effect(() => {
    const id = message.id
    for (const a of shots) {
      if (urls[a.id]) continue
      attachmentLink(id, a.id, api.attachmentUrls).then((u) => (urls[a.id] = u), () => {})
    }
  })
  const screenshots = $derived<BuildScreenshot[]>(shots.map((a) => ({ id: a.id, url: urls[a.id] ?? null, w: a.width, h: a.height })))
  let viewing = $state<number | null>(null)
</script>

<div class="build-page" data-testid="build-page">
  <div class="bar">
    <button type="button" class="back btn btn-quiet" onclick={onBack}><Icon name="chevronLeft" size={14} />Builds</button>
  </div>
  {#if message.deleted}
    <p class="gone">This build was deleted.</p>
  {:else}
    <div class="body">
      <div class="main">
        <BuildView title={b.title} gameName={buildGameName(app.state, serverId, message)} guessed={b.game_guessed} authorName={nameOf(message.author_id)}
          savedAt={relativeTime(message.created_at, now)} updatedAt={b.updated_at ? relativeTime(b.updated_at, now) : null} notes={b.notes}
          {screenshots} link={b.link} archived={isArchived(message)} quickDrop={isQuickDrop(message)} onImage={(i) => (viewing = i)}>
          {#snippet actions()}
            <BuildActions {message} canEdit={rights.edit} canArchive={rights.archive} canDelete={rights.delete} items={[]} onDeleted={onBack} />
          {/snippet}
        </BuildView>
      </div>
      <section class="comments" aria-label="Comments">
        <h2>Comments</h2>
        {#key message.id}
          <ThreadChat root={message} {serverId} placeholder="Comment on this build" authorTag={message.author_id} emptyText="No comments yet." />
        {/key}
      </section>
    </div>
  {/if}
</div>

{#if viewing !== null}
  <ImageViewer images={shots} messageId={message.id} start={viewing} onClose={() => (viewing = null)} />
{/if}

<style>
  .build-page { flex: 1; display: flex; flex-direction: column; min-height: 0; }
  .bar { flex-shrink: 0; display: flex; align-items: center; min-height: 52px; padding: 8px 18px; box-sizing: border-box; border-bottom: 1px solid var(--bg-3); }
  .back { padding-left: 8px; }
  .gone { margin: 0; padding: 24px 18px; color: var(--text-3); font-size: 13px; }
  .body { flex: 1; display: flex; min-height: 0; }
  .main { flex: 1; min-width: 0; overflow-y: auto; padding: 18px 22px 24px; }
  .comments { width: 360px; flex-shrink: 0; display: flex; flex-direction: column; min-height: 0; border-left: 1px solid var(--bg-3); }
  .comments h2 { margin: 0; padding: 14px 18px 6px; font-size: 13px; font-weight: 700; color: var(--text-2); }
  @media (max-width: 899.98px) {
    .body { flex-direction: column; }
    .main { flex: none; max-height: 55%; }
    .comments { width: auto; flex: 1; border-left: 0; border-top: 1px solid var(--bg-3); }
  }
</style>
