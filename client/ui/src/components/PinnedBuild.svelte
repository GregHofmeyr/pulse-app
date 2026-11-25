<script lang="ts">
  // The pinned build (#619, spec §4.4) in the right column: a bar with its title, Open (its page) and × (unpin), then
  // the build as BuildView reads it, scrolling. Gone (deleted, or out of reach) it says so, with Dismiss.
  import BuildView from './BuildView.svelte'
  import Icon from './Icon.svelte'
  import ImageViewer from './ImageViewer.svelte'
  import Loading from './Loading.svelte'
  import { attachmentLink } from '../lib/attachmentLinks'
  import { buildGameName, buildShots, isArchived, isQuickDrop, type BuildScreenshot } from '../lib/builds'
  import { relativeTime } from '../lib/conversations'
  import { openMessage } from '../lib/openMessage'
  import { pinnedBuild, unpinBuild } from '../lib/pinnedBuild.svelte'
  import { nameOfIn } from '../lib/selectors'
  import { app, clock } from '../lib/store.svelte'
  import { api } from '../lib/tauri'

  const message = $derived(pinnedBuild.gone ? null : pinnedBuild.message)
  const b = $derived(message?.build ?? null)
  const serverId = $derived((pinnedBuild.key && app.state.channels[pinnedBuild.key.channelId]?.server_id) || '')
  const nameOf = $derived(nameOfIn(() => app.state, serverId))
  const now = $derived(Date.parse(clock.nowIso))

  // every screenshot's link, shown as each arrives (as the build page does)
  const shots = $derived(message ? buildShots(message) : [])
  let urls = $state<Record<string, string>>({})
  $effect(() => {
    const id = message?.id
    if (!id) return
    for (const a of shots) {
      if (urls[a.id]) continue
      attachmentLink(id, a.id, api.attachmentUrls).then((u) => (urls[a.id] = u), () => {})
    }
  })
  const screenshots = $derived<BuildScreenshot[]>(shots.map((a) => ({ id: a.id, url: urls[a.id] ?? null, w: a.width, h: a.height })))
  let viewing = $state<number | null>(null)
</script>

<aside class="pinned-build" aria-label="Pinned build">
  <div class="bar">
    <Icon name="pin" size={14} />
    <span class="title">{b?.title ?? 'Pinned build'}</span>
    {#if message}
      <button type="button" class="open-page btn btn-quiet" onclick={() => message && openMessage(message)}>Open</button>
    {/if}
    <button type="button" class="unpin icon-btn icon-btn-xs" aria-label="Unpin" data-tip="Unpin" onclick={unpinBuild}><Icon name="x" size={14} /></button>
  </div>
  {#if pinnedBuild.gone}
    <div class="gone">
      <p>This build is gone.</p>
      <button type="button" class="dismiss btn btn-secondary" onclick={unpinBuild}>Dismiss</button>
    </div>
  {:else if message && b}
    <div class="scroll">
      <BuildView title={b.title} gameName={buildGameName(app.state, serverId, message)} guessed={b.game_guessed} authorName={nameOf(message.author_id)}
        savedAt={relativeTime(message.created_at, now)} updatedAt={b.updated_at ? relativeTime(b.updated_at, now) : null} notes={b.notes}
        {screenshots} link={b.link} archived={isArchived(message)} quickDrop={isQuickDrop(message)} onImage={(i) => (viewing = i)} />
    </div>
  {:else}
    <div class="wait"><Loading /></div>
  {/if}
</aside>

{#if viewing !== null && message}
  <ImageViewer images={shots} messageId={message.id} start={viewing} onClose={() => (viewing = null)} />
{/if}

<style>
  .pinned-build { width: var(--right-w, 320px); flex-shrink: 0; display: flex; flex-direction: column; min-height: 0; background: var(--bg-1); border-radius: var(--radius); overflow: hidden; }
  .bar { flex-shrink: 0; display: flex; align-items: center; gap: 6px; padding: 8px 8px 8px 12px; border-bottom: 1px solid var(--bg-3); color: var(--text-3); }
  .bar .btn, .bar .icon-btn { flex-shrink: 0; }
  .title { flex: 1; min-width: 0; font-size: 13px; font-weight: 600; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: 12px 14px 18px; }
  /* a narrow column: a smaller title, and screenshots one under another */
  .scroll :global(.build-view h1) { font-size: 17px; }
  .scroll :global(.build-view .tiles) { grid-template-columns: minmax(0, 1fr); }
  .gone { padding: 18px 14px; display: flex; flex-direction: column; align-items: flex-start; gap: 10px; }
  .gone p { margin: 0; color: var(--text-3); font-size: 13px; }
  .wait { padding: 6px 2px; }
</style>
