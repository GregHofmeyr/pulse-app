<script lang="ts">
  // One clip in the list view: like a message, with the player inline.
  import Avatar from './Avatar.svelte'
  import ClipActions from './ClipActions.svelte'
  import ClipCaption from './ClipCaption.svelte'
  import ClipPlayer from './ClipPlayer.svelte'
  import ClipReplies from './ClipReplies.svelte'
  import Icon from './Icon.svelte'
  import Name from './Name.svelte'
  import ReactionRow from './ReactionRow.svelte'
  import { clipRowEstimate, gameName } from '../lib/clips'
  import { downloadClip } from '../lib/clipDownload'
  import { clipKey } from '../lib/downloads.svelte'
  import { suggestedReactions } from '../lib/reactions'
  import { app, clock } from '../lib/store.svelte'
  import { InlineError } from '../lib/inlineError.svelte'
  import { livePin, pinTip } from '../lib/pins'
  import { serverNow } from '../lib/serverClock'
  import type { Message } from '../lib/protocol/Message'

  let {
    message,
    serverId,
    meId,
    nameOf,
    onOpenReplies,
    unsentReplies = 0,
    compact = false,
  }: {
    message: Message
    serverId: string
    meId: string
    nameOf: (userId: string | null) => string
    /** Absent where the replies panel pins the clip itself: no "View N replies" there. */
    onOpenReplies?: () => void
    /** Your replies to it that failed to send, shown while its panel is closed; opens it. */
    unsentReplies?: number
    /** Pinned at the top of its replies panel: a smaller player, no replies link. */
    compact?: boolean
  } = $props()

  const clip = $derived(message.clip)
  // its height before it is drawn, from the clip's own shape (#173): the 640px player (compact: about a third of a
  // wide window) capped at 70% of the window as ClipPlayer caps it, plus the row's name, caption and reactions
  const estimate = $derived(clipRowEstimate(clip, compact ? 360 : 640, window.innerHeight * 0.7, compact ? 90 : 140))
  const mine = $derived(!!meId && message.author_id === meId)
  const game = $derived(gameName(app.state, serverId, message))
  // Any number of reactions per person: offer your usual picks it has no chip for yet.
  const suggest = $derived(suggestedReactions(app.state, message.reactions))
  const when = $derived(new Date(message.created_at).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' }))

  // pinned (#257): a marker by the time, as a message's; by the server's clock, moving every 30 s (#258)
  const now = $derived((void clock.nowIso, serverNow()))
  const pin = $derived(livePin(message, now))

  const inlineError = new InlineError()
  $effect(() => inlineError.stop)
  const download = () => void downloadClip(message, inlineError.show)
  // the poster's avatar animates while the pointer is over the clip
  let over = $state(false)
</script>

{#if clip}
  <article class="clip-item" class:compact data-clip-id={message.id} data-message-id={message.id} style:contain-intrinsic-size="auto {estimate}px"
    onpointerenter={() => (over = true)} onpointerleave={() => (over = false)}>
    <Avatar id={message.author_id ?? ''} name={nameOf(message.author_id)} size={compact ? 32 : 40} interactive={!!message.author_id} serverId={serverId || null} hovered={() => over} />
    <div class="body">
      <div class="meta">
        <strong>{#if message.author_id}<Name userId={message.author_id} serverId={serverId || null} text={nameOf(message.author_id)} />{:else}{nameOf(null)}{/if}</strong>
        {#if game}<span class="game">{game}</span>{/if}
        <span class="time">{when}</span>
        {#if message.edited_at}<span class="edited">(edited)</span>{/if}
        {#if pin}<span class="pin-mark" data-tip={pinTip(nameOf(pin.pinned_by), pin.expires_at, now)}><Icon name="pin" size={14} /></span>{/if}
        <span class="grow"></span>
        {#if onOpenReplies && !compact}<button type="button" class="icon-btn icon-btn-sm" aria-label="Reply" onclick={onOpenReplies}><Icon name="reply" size={16} /></button>{/if}
        <ClipActions {message} {serverId} {mine} onDownload={download} onError={inlineError.show} />
      </div>
      <ClipCaption {message} />
      <div class="player"><ClipPlayer {message} {compact} onDownload={download} downloadKey={clipKey(message.id)} /></div>
      <ReactionRow {message} {meId} {nameOf} onError={inlineError.show} {suggest} alwaysAdd />
      {#if onOpenReplies && !compact}<ClipReplies {message} {onOpenReplies} {unsentReplies} />{/if}
      {#if inlineError.text}<p class="err" role="alert">{inlineError.text}</p>{/if}
    </div>
  </article>
{/if}

<style>
  /* drawn only near the screen; until it is, its size is the estimate set inline (the 480/260 here are fallbacks) */
  .clip-item { display: flex; gap: 14px; padding: 8px 18px; content-visibility: auto; contain-intrinsic-size: auto 480px; container-type: inline-size; }
  .clip-item:hover { background: var(--hover-tint); }
  .clip-item.compact { padding: 4px 18px; contain-intrinsic-size: auto 260px; }
  .body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 6px; align-items: flex-start; }
  .meta { align-self: stretch; display: flex; align-items: center; gap: 8px; min-height: 28px; }
  .meta strong { font-size: 14.5px; }
  .game { padding: 1px 8px; border-radius: var(--radius-md); background: var(--bg-3); color: var(--text-2); font-size: 11px; font-weight: 600; }
  /* a narrow card (small window, side panels open; #97) drops the game tag: the feed's chips still say it */
  @container (max-width: 420px) {
    .game { display: none; }
  }
  .time, .edited { font-size: 11px; color: var(--text-3); }
  .pin-mark { display: inline-flex; color: var(--text-3); }
  .grow { flex: 1; }
  .player { width: 100%; max-width: 640px; }
  .compact .player { max-width: 100%; }
  .err { margin: 0; font-size: 12px; color: var(--danger-text); }
</style>
