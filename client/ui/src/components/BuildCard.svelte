<script lang="ts">
  // One build in the gallery (#617): its picture, a "▶ video" tag when it has a link, the game as a caption, the title,
  // two lines of notes and "who · when · N comments". Quick drops and archived builds say so; an archived one's picture
  // is greyed.
  import BuildPeek from './BuildPeek.svelte'
  import BuildThumb from './BuildThumb.svelte'
  import Icon from './Icon.svelte'
  import { isMenuKey } from '../lib/participantMenu'
  import { peekHover } from '../lib/hoverIntent'
  import { buildGameName, commentCount, isArchived, isFavourite, isQuickDrop } from '../lib/builds'
  import { relativeTime } from '../lib/conversations'
  import { app, clock } from '../lib/store.svelte'
  import type { Message } from '../lib/protocol/Message'

  let { message, serverId, nameOf, onOpen, selecting = false, selected = false, onSelect, onMenu, onFavourite }: {
    message: Message
    serverId: string
    nameOf: (userId: string | null) => string
    /** Opens the build (its page); without it the card is only shown. */
    onOpen?: (m: Message) => void
    /** Multi-select is on (#617): a click toggles it instead of opening it, and it shows its check state. */
    selecting?: boolean
    selected?: boolean
    /** A click while selecting, or a Shift+click any time (`shift`); without it there's no selecting. */
    onSelect?: (m: Message, shift: boolean) => void
    /** Its right-click menu: a right-click, or the menu key / Shift+F10 on it. */
    onMenu?: (m: Message, e: MouseEvent | KeyboardEvent, opener: HTMLElement) => void
    /** Its star (your own favourite, #617): shown on hover or focus, always once starred. */
    onFavourite?: (m: Message) => void
  } = $props()
  const fav = $derived(isFavourite(message))

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
    if (onMenu && opener && isMenuKey(e)) {
      e.preventDefault()
      hover.stop()
      peeking = false
      return onMenu(message, e, opener)
    }
    // selecting, Space ticks it (a checkbox's key), never peeks
    if (e.key !== ' ' || selecting) return
    e.preventDefault()
    if (e.repeat) return
    hover.stop()
    if (peeking) return closePeek()
    byKey = true
    peeking = true
  }
  /** A button presses on Space's keyup: the peek took Space instead. */
  const keyUp = (e: KeyboardEvent) => { if (e.key === ' ' && !selecting) e.preventDefault() }
  // selecting: no peek
  $effect(() => {
    if (!selecting) return
    hover.stop()
    peeking = false
  })
  const enter = () => { if (!selecting) hover.enter() }
  function click(e: MouseEvent) {
    if (onSelect && (selecting || e.shiftKey)) return onSelect(message, e.shiftKey)
    onOpen?.(message)
  }
  /** A Shift+click selects builds, not the page's text. */
  const down = (e: MouseEvent) => { if (e.shiftKey && onSelect) e.preventDefault() }
  function menu(e: MouseEvent) {
    if (!onMenu || !opener) return
    e.preventDefault()
    hover.stop()
    peeking = false
    onMenu(message, e, opener)
  }
  const meta = $derived([nameOf(message.author_id), relativeTime(message.created_at, Date.parse(clock.nowIso)), commentCount(message)].filter(Boolean).join(' · '))
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<article bind:this={el} class="build-card" class:archived={isArchived(message)} class:selected={selecting && selected} data-build-id={message.id} onmouseenter={enter} onmouseleave={hover.leave} oncontextmenu={menu}>
  <svelte:element this={onOpen ? 'button' : 'div'} type={onOpen ? 'button' : undefined} class="open" role={!onOpen ? 'presentation' : selecting ? 'checkbox' : undefined}
    bind:this={opener} aria-checked={onOpen && selecting ? selected : undefined} aria-expanded={onOpen && !selecting ? peeking : undefined}
    aria-controls={onOpen && peeking && !selecting ? peekId : undefined} onclick={onOpen ? click : undefined} onmousedown={down} onkeydown={keys} onkeyup={keyUp}>
    <div class="media">
      <BuildThumb {message} dim={isArchived(message)} />
      {#if selecting}<span class="check" class:on={selected} aria-hidden="true">{#if selected}<Icon name="check" size={12} />{/if}</span>{/if}
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
  {#if onFavourite}
    <button type="button" class="star" class:on={fav} aria-label={fav ? 'Remove from favourites' : 'Favourite'} aria-pressed={fav}
      data-tip={fav ? 'Remove from favourites' : 'Favourite'} onclick={() => onFavourite(message)}><Icon name="star" size={14} /></button>
  {/if}
  {#if peeking && el}
    <BuildPeek {message} {serverId} anchor={el} id={peekId} focus={byKey} onClose={closePeek} {onOpen} onEnter={hover.stay} onLeave={hover.leave} />
  {/if}
</article>

<style>
  .build-card { position: relative; min-width: 0; border-radius: var(--radius-lg); background: var(--bg-2); border: 1px solid var(--bg-3); overflow: hidden; content-visibility: auto; contain-intrinsic-size: auto 280px; transition: border-color var(--dur) var(--ease); }
  .build-card:hover { border-color: var(--bg-4); }
  /* selected: the border's colour and a ring outside it (no size change) */
  .build-card.selected { border-color: var(--focus-ring); box-shadow: 0 0 0 1px var(--focus-ring); }
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
  /* the star: over the picture's top-right corner, its own button beside the one that opens the build; hidden
     (still focusable) until hovered or focused, always once starred; it never moves anything */
  .star { position: absolute; top: 6px; right: 6px; z-index: 1; width: 26px; height: 26px; padding: 0; display: grid; place-items: center; border: 0; border-radius: var(--radius-sm); background: var(--scrim-strong); color: var(--tooltip-text); cursor: pointer; opacity: 0; transition: opacity var(--dur) var(--ease); }
  .build-card:hover .star, .build-card:focus-within .star, .star:focus-visible { opacity: 1; }
  .star.on { opacity: 1; color: var(--first-gold); }
  .star.on :global(svg) { fill: currentColor; }
  /* the check state while selecting: over the picture's corner, so nothing moves */
  .check { position: absolute; left: 8px; bottom: 8px; width: 20px; height: 20px; box-sizing: border-box; display: grid; place-items: center; border-radius: var(--radius-xs); border: 2px solid var(--text-2); background: var(--scrim-strong); color: var(--on-accent); pointer-events: none; }
  .check.on { border-color: var(--focus-ring); background: var(--accent); }
</style>
