<script lang="ts">
  // One build's page (#618, spec §4.1): a top row with Back, the Comments toggle and the Actions menu; the build
  // (BuildView); and its Comments (a ThreadChat, the author's tagged "author") in a right column that can be hidden,
  // remembered on this device. It replaces the channel's browse view; Back returns to it.
  //
  // The layout comes from the page's own width (a ResizeObserver on its body, never the window's), #618 testing:
  // with room, the spec's media row (video left, 2×2 screenshots right, notes under); with Comments shown and too little
  // room beside them, the media stacks; with too little room for a Comments column at all, Comments go under the build.
  import BuildActions from './BuildActions.svelte'
  import BuildForm from './BuildForm.svelte'
  import BuildVideo from './BuildVideo.svelte'
  import BuildView from './BuildView.svelte'
  import Icon from './Icon.svelte'
  import ImageViewer from './ImageViewer.svelte'
  import ThreadChat from './ThreadChat.svelte'
  import { attachmentLink } from '../lib/attachmentLinks'
  import { commentsShown, setCommentsShown } from '../lib/buildPrefs'
  import { buildGameName, buildRights, buildShots, isArchived, isQuickDrop, type BuildActionItem, type BuildScreenshot } from '../lib/builds'
  import { relativeTime } from '../lib/conversations'
  import { isPinned, pinBuild, unpinBuild } from '../lib/pinnedBuild.svelte'
  import { popOutBuild } from '../lib/buildWindow.svelte'
  import { nameOfIn } from '../lib/selectors'
  import { app, clock } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
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
  // --- layout: from the body's width (see the top) ---
  /** The Comments column's width, the least room the build gets beside it, and the least for the wide media row. */
  const COMMENTS_W = 360
  const MAIN_MIN = 360
  const WIDE_MIN = 600
  let showComments = $state(commentsShown())
  let body: HTMLElement | undefined = $state()
  let bodyW = $state(0)
  $effect(() => {
    const el = body
    if (!el) return
    bodyW = el.clientWidth
    if (typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver((entries) => {
      const w = entries[entries.length - 1]?.contentRect.width
      if (w !== undefined) bodyW = w
    })
    ro.observe(el)
    return () => ro.disconnect()
  })
  /** Comments shown but no room for their column: they go under the build. */
  const stack = $derived(showComments && bodyW < COMMENTS_W + MAIN_MIN)
  const mainW = $derived(showComments && !stack ? bodyW - COMMENTS_W : bodyW)
  const layout = $derived<'wide' | 'stacked'>(!stack && mainW >= WIDE_MIN ? 'wide' : 'stacked')
  const commentsId = $derived(`build-comments-${message.id}`)
  function toggleComments() {
    showComments = !showComments
    setCommentsShown(showComments)
  }
  let enlarged = $state(false)

  let viewing = $state<number | null>(null)
  let error = $state('')
  let editing = $state(false)
  /** Pin to side (#619), or Unpin when it's the pinned one; Pop out (#620): its own window, kept current and closed on
   *  delete by lib/buildWindow.svelte.ts. */
  function pickItem(it: BuildActionItem) {
    if (it === 'popout') {
      popOutBuild(message).catch((e) => (error = errorText(e)))
      return
    }
    if (isPinned(message.id)) unpinBuild()
    else pinBuild(message.channel_id, message.id, message)
  }
</script>

<div class="build-page" data-testid="build-page">
  <div class="bar">
    <button type="button" class="back btn btn-quiet" onclick={onBack}><Icon name="chevronLeft" size={14} />Builds</button>
    {#if !message.deleted}
      <div class="tools">
        <button type="button" class="btn btn-quiet comments-toggle" aria-expanded={showComments} aria-controls={commentsId} onclick={toggleComments}>
          <Icon name="message" size={14} /><span class="label">Comments</span>{#if b.reply_count > 0}<span class="count">{b.reply_count}</span>{/if}
        </button>
        <BuildActions {message} canEdit={rights.edit} canArchive={rights.archive} canDelete={rights.delete} items={['pin', 'popout']} onItem={pickItem} onEdit={() => (editing = true)} onDeleted={onBack} />
      </div>
    {/if}
  </div>
  {#if message.deleted}
    <p class="gone">This build was deleted.</p>
  {:else}
    <div class="body" class:stack bind:this={body}>
      <div class="main">
        <BuildView {layout} videoEnlarged={enlarged} title={b.title} gameName={buildGameName(app.state, serverId, message)} guessed={b.game_guessed} authorName={nameOf(message.author_id)}
          savedAt={relativeTime(message.created_at, now)} updatedAt={b.updated_at ? relativeTime(b.updated_at, now) : null} notes={b.notes}
          {screenshots} link={b.link} archived={isArchived(message)} quickDrop={isQuickDrop(message)} onImage={(i) => (viewing = i)}>
          {#snippet video()}
            <BuildVideo link={b.link!} messageId={message.id} bind:enlarged onError={(t) => (error = t)} />
          {/snippet}
        </BuildView>
        {#if error}<p class="error" role="alert">{error}</p>{/if}
      </div>
      <!-- hidden, not removed: what's being typed survives hiding them -->
      <section class="comments" id={commentsId} aria-label="Comments" hidden={!showComments}>
        <h2>Comments</h2>
        {#key message.id}
          <ThreadChat root={message} {serverId} placeholder="Comment on this build" authorTag={message.author_id} emptyText="No comments yet." />
        {/key}
      </section>
    </div>
  {/if}
</div>

{#if editing && !message.deleted}
  <BuildForm channelId={message.channel_id} {serverId} build={message} onClose={() => (editing = false)} />
{/if}

{#if viewing !== null}
  <ImageViewer images={shots} messageId={message.id} start={viewing} onClose={() => (viewing = null)} />
{/if}

<style>
  .build-page { flex: 1; display: flex; flex-direction: column; min-height: 0; min-width: 0; }
  /* the top row, over the build and its Comments: Back left, the Comments toggle and Actions right */
  .bar { flex-shrink: 0; display: flex; align-items: center; gap: 8px; min-width: 0; min-height: 52px; padding: 8px 18px; box-sizing: border-box; border-bottom: 1px solid var(--bg-3); container-type: inline-size; }
  .back { flex-shrink: 0; padding-left: 8px; }
  .tools { margin-left: auto; flex-shrink: 0; display: flex; align-items: center; gap: 8px; }
  .comments-toggle[aria-expanded='true'] { color: var(--text); }
  .count { min-width: 16px; padding: 0 5px; box-sizing: border-box; border-radius: var(--radius-xs); background: var(--bg-4); color: var(--text-2); font-size: 11px; line-height: 16px; text-align: center; }
  /* a very narrow page: the toggle keeps its icon and count, and its name for screen readers */
  @container (max-width: 340px) {
    .label { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
  }
  .error { margin: 8px 0 0; color: var(--danger-text); font-size: 13px; }
  .gone { margin: 0; padding: 24px 18px; color: var(--text-3); font-size: 13px; }
  .body { flex: 1; display: flex; min-height: 0; min-width: 0; }
  .main { flex: 1; min-width: 0; overflow-y: auto; overflow-x: hidden; padding: 18px 22px 24px; }
  .comments { width: 360px; flex-shrink: 0; display: flex; flex-direction: column; min-height: 0; min-width: 0; border-left: 1px solid var(--bg-3); }
  .comments[hidden] { display: none; }
  .comments h2 { margin: 0; padding: 14px 18px 6px; font-size: 13px; font-weight: 700; color: var(--text-2); }
  /* no room for a Comments column: they go under the build */
  .body.stack { flex-direction: column; }
  .body.stack .main { flex: none; max-height: 55%; }
  .body.stack .comments { width: auto; flex: 1; border-left: 0; border-top: 1px solid var(--bg-3); }
</style>
