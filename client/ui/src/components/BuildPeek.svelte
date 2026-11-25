<script lang="ts">
  // Quick peek (#619, spec §4.4): a floating card beside a build's card or row with its game, title and the first lines
  // of its notes, plus Open and Pin to side. Moved out to the page (`portal`) and placed with `placePopover`; the
  // caller opens and closes it (hover, Space) and gets told when the pointer enters or leaves it. `docked` (Ctrl+K):
  // drawn where it is, at the switcher's foot, since anything outside that dialog is inert.
  import { onMount } from 'svelte'
  import Icon from './Icon.svelte'
  import { portal } from '../lib/actions'
  import { buildGameName, isArchived } from '../lib/builds'
  import { placePopover } from '../lib/menuPlace'
  import { openMessage } from '../lib/openMessage'
  import { isPinned, pinBuild, unpinBuild } from '../lib/pinnedBuild.svelte'
  import { app } from '../lib/store.svelte'
  import type { Message } from '../lib/protocol/Message'

  let { message, serverId, anchor, docked = false, focus = false, onClose, onOpen, onEnter = () => {}, onLeave = () => {}, closeOnEscape = true, id }: {
    message: Message
    serverId: string
    /** What it hangs off (the card, the row); not needed docked. */
    anchor?: HTMLElement
    docked?: boolean
    /** Opened from the keyboard: the focus moves in (the caller puts it back when it closes). */
    focus?: boolean
    onClose: () => void
    /** Open: the caller's page opener; without it the build opens where it lives (its channel's page). */
    onOpen?: (m: Message) => void
    onEnter?: () => void
    onLeave?: () => void
    /** Escape anywhere closes it (Ctrl+K handles its own Escape). */
    closeOnEscape?: boolean
    id?: string
  } = $props()

  /** How many lines of the notes it shows. */
  const NOTE_LINES = 4
  const b = $derived(message.build!)
  const game = $derived(buildGameName(app.state, serverId, message))
  const notes = $derived(b.notes?.trim() ? b.notes.trim().split('\n').slice(0, NOTE_LINES).join('\n') : '')
  const pinned = $derived(isPinned(message.id))

  let el: HTMLElement | undefined = $state()
  let place = $state('')
  /** Out to the page and placed, unless docked. */
  function float(node: HTMLElement, on: boolean) {
    return on ? portal(node) : undefined
  }
  onMount(() => {
    if (!el || docked || !anchor) return
    const p = placePopover(anchor.getBoundingClientRect(), { width: el.offsetWidth || 280, height: el.offsetHeight || 160 }, undefined, 'start', 6)
    place = `left:${p.left}px;${p.below ? `top:${p.top}px` : `bottom:${p.bottom}px`};max-height:${p.maxHeight}px`
  })
  onMount(() => {
    if (focus) el?.querySelector<HTMLElement>('button')?.focus()
  })
  // Placed once: a scroll (not its own) or a resize moves what it hangs off, so it closes instead of floating astray.
  $effect(() => {
    if (docked) return
    const scrolled = (e: Event) => {
      if (el && e.target instanceof Node && el.contains(e.target)) return
      onClose()
    }
    const resized = () => onClose()
    document.addEventListener('scroll', scrolled, true)
    window.addEventListener('resize', resized)
    return () => {
      document.removeEventListener('scroll', scrolled, true)
      window.removeEventListener('resize', resized)
    }
  })

  // the message and handlers are held first: closing may take this peek's props away
  function open() {
    const [m, go] = [message, onOpen]
    onClose()
    if (go) go(m)
    else openMessage(m)
  }
  function pin() {
    const [m, was] = [message, pinned]
    onClose()
    if (was) unpinBuild()
    else pinBuild(m.channel_id, m.id, m)
  }
  function keys(e: KeyboardEvent) {
    if (closeOnEscape && e.key === 'Escape') {
      e.preventDefault()
      onClose()
    }
  }
</script>

<svelte:window onkeydown={keys} />

<div bind:this={el} {id} class="build-peek pop-in" class:menu-surface={!docked} class:docked role="dialog" tabindex="-1" aria-label={`Peek: ${b.title}`} style={place} use:float={!docked}
  onmouseenter={onEnter} onmouseleave={onLeave}>
  {#if game}<span class="caption">{game}</span>{/if}
  <span class="title">{b.title}</span>
  {#if isArchived(message)}<span class="badge">ARCHIVED</span>{/if}
  {#if notes}<p class="notes">{notes}</p>{/if}
  <div class="actions">
    <button type="button" class="btn btn-primary" onclick={open}><span class="label">Open</span></button>
    <button type="button" class="btn btn-secondary" onclick={pin}><Icon name="pin" size={14} /><span class="label">{pinned ? 'Unpin' : 'Pin to side'}</span></button>
  </div>
</div>

<style>
  .build-peek { position: fixed; z-index: 60; width: 280px; padding: 12px; box-sizing: border-box; display: flex; flex-direction: column; align-items: flex-start; gap: 4px; overflow-y: auto; }
  .build-peek.docked { position: static; width: auto; flex-shrink: 0; max-height: 40%; border-top: 1px solid var(--line-menu); padding: 12px 18px 14px; }
  .caption { font-size: 11px; font-weight: 700; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); max-width: 100%; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .title { font-size: 15px; font-weight: 600; color: var(--text); overflow-wrap: anywhere; }
  .badge { padding: 1px 7px; border-radius: var(--radius-xs); border: 1px solid var(--bg-4); color: var(--text-2); font-size: 10px; font-weight: 700; letter-spacing: .04em; }
  .notes { margin: 4px 0 0; font-size: 13px; line-height: 1.4; color: var(--text-2); white-space: pre-line; overflow-wrap: anywhere; display: -webkit-box; -webkit-line-clamp: 4; line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; }
  .actions { display: flex; gap: 6px; margin-top: 8px; max-width: 100%; }
  .actions .btn { display: inline-flex; align-items: center; gap: 6px; min-width: 0; }
  .actions .btn :global(svg) { flex-shrink: 0; }
  .label { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
