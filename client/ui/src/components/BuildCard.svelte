<script lang="ts">
  // One build in the gallery (#617): its picture, a "▶ video" tag when it has a link, the game as a caption, the title,
  // two lines of notes and "who · when · N comments". Quick drops and archived builds say so; an archived one's picture
  // is greyed.
  import BuildPeek from './BuildPeek.svelte'
  import BuildThumb from './BuildThumb.svelte'
  import { peekHover } from '../lib/hoverIntent'
  import { buildGameName, commentCount, isArchived, isQuickDrop } from '../lib/builds'
  import { relativeTime } from '../lib/conversations'
  import { app, clock } from '../lib/store.svelte'
  import type { Message } from '../lib/protocol/Message'

  let { message, serverId, nameOf, onOpen }: {
    message: Message
    serverId: string
    nameOf: (userId: string | null) => string
    /** Opens the build (its page); without it the card is only shown. */
    onOpen?: (m: Message) => void
  } = $props()

  const b = $derived(message.build!)
  const game = $derived(buildGameName(app.state, serverId, message))
  // quick peek (#619): resting on it ~600 ms, or Space while it has the focus
  let el: HTMLElement | undefined = $state()
  let opener: HTMLElement | undefined = $state()
  let peeking = $state(false)
  /** Opened with Space: the focus went into the peek, and comes back here when it closes. */
  let byKey = $state(false)
  const peekId = $derived(`build-peek-${message.id}`)
  const hover = peekHover((v) => {
    peeking = v
    if (!v) byKey = false
  })
  $effect(() => () => hover.stop())
  function closePeek() {
    hover.stop()
    peeking = false
    if (byKey) opener?.focus()
    byKey = false
  }
  function keys(e: KeyboardEvent) {
    if (e.key !== ' ') return
    e.preventDefault()
    if (e.repeat) return
    hover.stop()
    if (peeking) return closePeek()
    byKey = true
    peeking = true
  }
  /** A button presses on Space's keyup: the peek took Space instead. */
  const keyUp = (e: KeyboardEvent) => { if (e.key === ' ') e.preventDefault() }
  const meta = $derived([nameOf(message.author_id), relativeTime(message.created_at, Date.parse(clock.nowIso)), commentCount(message)].filter(Boolean).join(' · '))
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<article bind:this={el} class="build-card" class:archived={isArchived(message)} data-build-id={message.id} onmouseenter={hover.enter} onmouseleave={hover.leave}>
  <svelte:element this={onOpen ? 'button' : 'div'} type={onOpen ? 'button' : undefined} class="open" role={onOpen ? undefined : 'presentation'}
    bind:this={opener} aria-expanded={onOpen ? peeking : undefined} aria-controls={onOpen && peeking ? peekId : undefined}
    onclick={onOpen ? () => onOpen(message) : undefined} onkeydown={keys} onkeyup={keyUp}>
    <div class="media">
      <BuildThumb {message} dim={isArchived(message)} />
      {#if b.link}<span class="tag video"><span aria-hidden="true">▶</span> video</span>{/if}
      <span class="badges">
        {#if isQuickDrop(message)}<span class="badge">QUICK DROP</span>{/if}
        {#if isArchived(message)}<span class="badge">ARCHIVED</span>{/if}
      </span>
    </div>
    <div class="info">
      {#if game}<span class="caption">{game}{#if b.game_guessed}<span class="guessed"> · guessed</span>{/if}</span>{/if}
      <span class="title">{b.title}</span>
      {#if b.notes}<span class="notes">{b.notes}</span>{/if}
      <span class="meta">{meta}</span>
    </div>
  </svelte:element>
  {#if peeking && el}
    <BuildPeek {message} {serverId} anchor={el} id={peekId} focus={byKey} onClose={closePeek} {onOpen} onEnter={hover.stay} onLeave={hover.leave} />
  {/if}
</article>

<style>
  .build-card { min-width: 0; border-radius: var(--radius-lg); background: var(--bg-2); border: 1px solid var(--bg-3); overflow: hidden; content-visibility: auto; contain-intrinsic-size: auto 280px; transition: border-color var(--dur) var(--ease); }
  .build-card:hover { border-color: var(--bg-4); }
  .open { display: flex; flex-direction: column; width: 100%; padding: 0; border: 0; background: transparent; color: inherit; font: inherit; text-align: left; }
  button.open { cursor: pointer; }
  .media { position: relative; aspect-ratio: 16 / 9; }
  .tag, .badge { padding: 1px 7px; border-radius: var(--radius-xs); font-size: 10px; font-weight: 700; letter-spacing: .04em; pointer-events: none; }
  .tag { position: absolute; right: 8px; bottom: 8px; background: var(--scrim-strong); color: var(--tooltip-text); letter-spacing: 0; font-weight: 600; font-size: 11px; }
  .badges { position: absolute; left: 8px; top: 8px; display: flex; gap: 4px; }
  .badge { background: var(--bg-1); color: var(--text-2); border: 1px solid var(--bg-4); }
  .info { padding: 10px 12px 12px; display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .caption { font-size: 11px; font-weight: 700; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .guessed { text-transform: none; letter-spacing: 0; font-weight: 400; font-style: italic; }
  .title { font-size: 15px; font-weight: 600; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .notes { font-size: 13px; color: var(--text-2); line-height: 1.35; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; white-space: pre-line; }
  .meta { margin-top: 4px; font-size: 12px; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
</style>
