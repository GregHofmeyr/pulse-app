<script lang="ts">
  // One clip on the wall: its thumbnail until played (then the player, with sound), caption, poster, game and reactions.
  import { tick } from 'svelte'
  import Avatar from './Avatar.svelte'
  import Name from './Name.svelte'
  import ClipActions from './ClipActions.svelte'
  import ClipCaption from './ClipCaption.svelte'
  import ClipPlayer from './ClipPlayer.svelte'
  import ClipReplies from './ClipReplies.svelte'
  import Icon from './Icon.svelte'
  import ReactionRow from './ReactionRow.svelte'
  import { formatDuration, gameName } from '../lib/clips'
  import { mediaUrl } from '../lib/emoji'
  import { suggestedReactions } from '../lib/reactions'
  import { app } from '../lib/store.svelte'
  import { InlineError } from '../lib/inlineError.svelte'
  import type { Message } from '../lib/protocol/Message'

  let {
    message,
    serverId,
    meId,
    nameOf,
    playing,
    onPlay,
    isNew = false,
    onOpenReplies,
    unsentReplies = 0,
  }: {
    message: Message
    serverId: string
    meId: string
    nameOf: (userId: string | null) => string
    /** This card holds the wall's one player. */
    playing: boolean
    /** The thumbnail was clicked: make this the playing card. */
    onPlay: () => void
    /** Newer than your read point when the channel was opened. */
    isNew?: boolean
    onOpenReplies?: () => void
    unsentReplies?: number
  } = $props()

  let el: HTMLElement | undefined = $state()
  const clip = $derived(message.clip)
  const mine = $derived(!!meId && message.author_id === meId)
  const game = $derived(gameName(app.state, serverId, message))
  // Any number of reactions per person: offer your usual picks it has no chip for yet.
  const suggest = $derived(suggestedReactions(app.state, message.reactions))
  const when = $derived(new Date(message.created_at).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' }))

  const inlineError = new InlineError()
  $effect(() => inlineError.stop)

  // Full screen straight from the thumbnail: the player swaps in, then goes full screen (still within the click's activation).
  async function playFullscreen() {
    onPlay()
    await tick()
    void el?.querySelector<HTMLElement>('.clip')?.requestFullscreen?.()?.catch(() => {})
  }
  // the poster's avatar animates while the pointer is over the card
  let over = $state(false)
</script>

{#if clip}
  <article class="card" data-clip-id={message.id} data-message-id={message.id} bind:this={el}
    onpointerenter={() => (over = true)} onpointerleave={() => (over = false)}>
    <div class="media" class:still={!playing}>
      {#if playing}
        <ClipPlayer {message} autoplay compact />
      {:else}
        <button type="button" class="thumb" aria-label="Play clip" onclick={onPlay}>
          <img src={mediaUrl(clip.thumb_hash)} alt="" loading="lazy" decoding="async" />
          <span class="play" aria-hidden="true"><Icon name="play" size={22} /></span>
        </button>
        <span class="duration">{formatDuration(clip.duration_ms)}</span>
        <button type="button" class="fs" aria-label="Play full screen" onclick={() => void playFullscreen()}><Icon name="fullscreen" size={15} /></button>
        {#if isNew}<span class="badge-new">new</span>{/if}
      {/if}
    </div>
    <div class="info">
      <ClipCaption {message} />
      <div class="meta">
        <Avatar id={message.author_id ?? ''} name={nameOf(message.author_id)} size={20} interactive={!!message.author_id} serverId={serverId || null} hovered={() => over} />
        <span class="who">{#if message.author_id}<Name userId={message.author_id} serverId={serverId || null} text={nameOf(message.author_id)} />{:else}{nameOf(null)}{/if}</span>
        {#if game}<span class="game">{game}</span>{/if}
        <span class="grow"></span>
        {#if onOpenReplies}<button type="button" class="icon-btn icon-btn-sm" aria-label="Reply" onclick={onOpenReplies}><Icon name="reply" size={15} /></button>{/if}
        {#if mine}<ClipActions {message} {serverId} />{/if}
      </div>
      <span class="time">{when}{#if message.edited_at} · edited{/if}</span>
      <ReactionRow {message} {meId} {nameOf} onError={inlineError.show} {suggest} alwaysAdd />
      {#if onOpenReplies}<ClipReplies {message} {onOpenReplies} {unsentReplies} />{/if}
      {#if inlineError.text}<p class="err" role="alert">{inlineError.text}</p>{/if}
    </div>
  </article>
{/if}

<style>
  /* drawn only near the screen; the size is a typical card until it is */
  .card { min-width: 0; display: flex; flex-direction: column; border-radius: 12px; background: var(--bg-2); border: 1px solid var(--bg-3); content-visibility: auto; contain-intrinsic-size: auto 340px; }
  .media { position: relative; background: #000; border-radius: 12px 12px 0 0; overflow: hidden; }
  .media.still { aspect-ratio: 16 / 9; }
  .media :global(.clip) { border-radius: 0; }
  .thumb { position: absolute; inset: 0; padding: 0; border: 0; background: #000; cursor: pointer; display: block; }
  .thumb img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .play { position: absolute; inset: 0; margin: auto; width: 46px; height: 46px; border-radius: 50%; background: rgba(0, 0, 0, .55); color: #fff; display: grid; place-items: center; opacity: .85; transition: opacity var(--dur) var(--ease), background-color var(--dur) var(--ease); }
  .thumb:hover .play, .thumb:focus-visible .play { opacity: 1; background: rgba(0, 0, 0, .75); }
  .duration { position: absolute; right: 8px; bottom: 8px; padding: 1px 6px; border-radius: 6px; background: rgba(0, 0, 0, .75); color: #fff; font-size: 11px; font-variant-numeric: tabular-nums; pointer-events: none; }
  .fs { position: absolute; right: 8px; top: 8px; width: 28px; height: 28px; border: 0; border-radius: 8px; background: rgba(0, 0, 0, .6); color: #fff; display: grid; place-items: center; opacity: 0; }
  .card:hover .fs, .fs:focus-visible { opacity: 1; }
  .badge-new { position: absolute; left: 8px; top: 8px; padding: 1px 7px; border-radius: 8px; background: var(--danger); color: var(--on-danger); font-size: 10px; font-weight: 700; text-transform: uppercase; pointer-events: none; }
  .info { padding: 8px 10px 10px; display: flex; flex-direction: column; gap: 5px; min-width: 0; }
  .meta { display: flex; align-items: center; gap: 6px; min-height: 28px; }
  .who { font-size: 13px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .game { padding: 1px 8px; border-radius: 10px; background: var(--bg-3); color: var(--text-2); font-size: 11px; font-weight: 600; white-space: nowrap; }
  .grow { flex: 1; }
  .time { font-size: 11px; color: var(--text-3); }
  .err { margin: 0; font-size: 12px; color: var(--danger); }
</style>
