<script lang="ts">
  // One build in the list (#617): thumbnail, game caption, title, "who · when · N comments" and a notes snippet.
  import BuildPeek from './BuildPeek.svelte'
  import BuildThumb from './BuildThumb.svelte'
  import Icon from './Icon.svelte'
  import { isMenuKey } from '../lib/participantMenu'
  import { peekHover } from '../lib/hoverIntent'
  import { buildGameName, commentCount, isArchived, isQuickDrop } from '../lib/builds'
  import { relativeTime } from '../lib/conversations'
  import { app, clock } from '../lib/store.svelte'
  import type { Message } from '../lib/protocol/Message'

  let { message, serverId, nameOf, onOpen, selecting = false, selected = false, onSelect, onMenu }: {
    message: Message
    serverId: string
    nameOf: (userId: string | null) => string
    onOpen?: (m: Message) => void
    /** Multi-select is on (#617): a click toggles it instead of opening it, and it shows its check state. */
    selecting?: boolean
    selected?: boolean
    /** A click while selecting, or a Shift+click any time (`shift`); without it there's no selecting. */
    onSelect?: (m: Message, shift: boolean) => void
    /** Its right-click menu: a right-click, or the menu key / Shift+F10 on it. */
    onMenu?: (m: Message, e: MouseEvent | KeyboardEvent, opener: HTMLElement) => void
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
<article bind:this={el} class="build-row" class:archived={isArchived(message)} class:selected={selecting && selected} data-build-id={message.id} onmouseenter={enter} onmouseleave={hover.leave} oncontextmenu={menu}>
  <svelte:element this={onOpen ? 'button' : 'div'} type={onOpen ? 'button' : undefined} class="open" role={!onOpen ? 'presentation' : selecting ? 'checkbox' : undefined}
    bind:this={opener} aria-checked={onOpen && selecting ? selected : undefined} aria-expanded={onOpen && !selecting ? peeking : undefined}
    aria-controls={onOpen && peeking && !selecting ? peekId : undefined} onclick={onOpen ? click : undefined} onmousedown={down} onkeydown={keys} onkeyup={keyUp}>
    <div class="media"><BuildThumb {message} dim={isArchived(message)} />{#if selecting}<span class="check" class:on={selected} aria-hidden="true">{#if selected}<Icon name="check" size={12} />{/if}</span>{/if}{#if b.link}<span class="tag" aria-hidden="true">▶</span><span class="sr-only">video</span>{/if}</div>
    <div class="info">
      {#if game}<span class="caption">{game}{#if b.game_guessed}<span class="guessed"> · guessed</span>{/if}</span>{/if}
      <span class="line">
        <span class="title">{b.title}</span>
        {#if isQuickDrop(message)}<span class="badge">QUICK DROP</span>{/if}
        {#if isArchived(message)}<span class="badge">ARCHIVED</span>{/if}
      </span>
      <span class="meta">{meta}</span>
    </div>
    {#if b.notes}<span class="notes">{b.notes}</span>{/if}
  </svelte:element>
  {#if peeking && el}
    <BuildPeek {message} {serverId} anchor={el} id={peekId} focus={byKey} onClose={closePeek} {onOpen} onEnter={hover.stay} onLeave={hover.leave} />
  {/if}
</article>

<style>
  .build-row { border-bottom: 1px solid var(--bg-3); }
  .open { display: grid; grid-template-columns: 96px minmax(0, 1fr) minmax(0, 1.2fr); align-items: center; gap: 14px; width: 100%; padding: 10px 18px; border: 0; background: transparent; color: inherit; font: inherit; text-align: left; }
  button.open { cursor: pointer; }
  button.open:hover { background: var(--bg-2); }
  .build-row.selected .open { background: var(--accent-faint); }
  .media { position: relative; width: 96px; aspect-ratio: 16 / 9; border-radius: var(--radius-sm); overflow: hidden; }
  .tag { position: absolute; right: 4px; bottom: 4px; padding: 0 5px; border-radius: var(--radius-xs); background: var(--scrim-strong); color: var(--tooltip-text); font-size: 10px; }
  .info { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .caption { font-size: 11px; font-weight: 700; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .guessed { text-transform: none; letter-spacing: 0; font-weight: 400; font-style: italic; }
  .line { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .title { font-size: 14px; font-weight: 600; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .badge { flex-shrink: 0; padding: 1px 6px; border-radius: var(--radius-xs); border: 1px solid var(--bg-4); color: var(--text-2); font-size: 10px; font-weight: 700; letter-spacing: .04em; }
  .meta { font-size: 12px; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .notes { font-size: 13px; color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  @media (max-width: 899.98px) {
    .open { grid-template-columns: 80px minmax(0, 1fr); }
    .media { width: 80px; }
    .notes { display: none; }
  }
  /* the check state while selecting: over the picture's corner, so nothing moves */
  .check { position: absolute; left: 4px; top: 4px; width: 20px; height: 20px; box-sizing: border-box; display: grid; place-items: center; border-radius: var(--radius-xs); border: 2px solid var(--text-2); background: var(--scrim-strong); color: var(--on-accent); pointer-events: none; }
  .check.on { border-color: var(--focus-ring); background: var(--accent); }
</style>
