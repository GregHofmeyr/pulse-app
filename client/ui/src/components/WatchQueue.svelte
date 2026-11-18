<script lang="ts">
  // The watch party's queue (#464): a popover under the party bar's "Queue · N" pill. Every queued video in order, with
  // its thumbnail and who added it; drag a row (yours, or any while you control the party) to reorder, right-click it
  // (or the menu key) for Play now, moves and Remove; paste a link into the field to add one. Anyone in the call may
  // add. Dropping a link anywhere on the party adds it too (the panel's).
  import { onMount, tick } from 'svelte'
  import Icon from './Icon.svelte'
  import { clickOutside } from '../lib/actions'
  import { isMediaHash, mediaUrl } from '../lib/emoji'
  import { placeMenu } from '../lib/menuPlace'
  import { errorText } from '../lib/tauri'
  import { mayControl, mayEdit } from '../lib/watchParty'
  import { enqueueLink, watchPartyAction } from '../lib/watchParty.svelte'
  import type { WatchParty } from '../lib/protocol/WatchParty'
  import type { WatchPartyAction } from '../lib/protocol/WatchPartyAction'

  let { party, me, nameOf, opener, onClose }: {
    party: WatchParty
    me: string | null
    nameOf: (id: string) => string
    /** The pill that opened it: a click there is the pill's (it toggles), not a click outside. */
    opener: HTMLElement | undefined
    onClose: () => void
  } = $props()

  const control = $derived(mayControl(party, me))
  let link = $state('')
  let error = $state('')
  let adding = $state(false)
  let input = $state<HTMLInputElement>()
  onMount(() => input?.focus())

  async function add(e?: SubmitEvent) {
    e?.preventDefault()
    if (adding || !link.trim()) return
    adding = true
    error = ''
    try {
      await enqueueLink(party.channel_id, link)
      link = ''
    } catch (err) {
      error = errorText(err)
    } finally {
      adding = false
    }
  }
  function act(a: WatchPartyAction) {
    error = ''
    watchPartyAction(party.channel_id, a).catch((err: unknown) => (error = errorText(err)))
  }

  // ---- dragging a row to reorder ----
  const DRAG_TYPE = 'application/x-pulse-queue'
  let dragFrom = $state<number | null>(null)
  let dragOver = $state<number | null>(null)
  function dragStart(e: DragEvent, i: number) {
    dragFrom = i
    e.dataTransfer?.setData(DRAG_TYPE, String(i))
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move'
  }
  function over(e: DragEvent, i: number) {
    if (dragFrom === null) return
    e.preventDefault()
    e.stopPropagation()
    dragOver = i
  }
  function drop(e: DragEvent, i: number) {
    if (dragFrom === null) return
    e.preventDefault()
    e.stopPropagation()
    const from = dragFrom
    dragFrom = dragOver = null
    if (from !== i) act({ action: 'move', index: from, to: i })
  }
  const dragEnd = () => (dragFrom = dragOver = null)

  // ---- a row's menu ----
  let menu = $state<{ index: number; x: number; y: number; row: HTMLElement } | null>(null)
  let menuEl = $state<HTMLElement>()
  let menuPos = $state({ left: 0, top: 0, origin: 'top left' })
  async function openMenu(index: number, x: number, y: number, row: HTMLElement) {
    menu = { index, x, y, row }
    await tick()
    if (!menuEl) return
    menuPos = placeMenu(x, y, menuEl.offsetWidth, menuEl.offsetHeight)
    menuEl.querySelector<HTMLElement>('[data-item]')?.focus()
  }
  function closeMenu(restore: boolean) {
    if (restore && menu?.row.isConnected) menu.row.focus()
    menu = null
  }
  function pick(a: WatchPartyAction) {
    closeMenu(true)
    act(a)
  }
  function rowKey(e: KeyboardEvent, i: number) {
    const row = e.currentTarget as HTMLElement
    if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      e.preventDefault()
      const r = row.getBoundingClientRect()
      void openMenu(i, r.left + 16, r.bottom, row)
    } else if (e.altKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown') && mayEdit(party, party.queue[i], me)) {
      e.preventDefault()
      const to = e.key === 'ArrowUp' ? i - 1 : i + 1
      if (to >= 0 && to < party.queue.length) act({ action: 'move', index: i, to })
    }
  }
  function menuKey(e: KeyboardEvent) {
    const items = menuEl ? [...menuEl.querySelectorAll<HTMLElement>('[data-item]')] : []
    const at = items.indexOf(document.activeElement as HTMLElement)
    if (e.key === 'Escape' || e.key === 'Tab') {
      e.preventDefault()
      e.stopPropagation()
      closeMenu(true)
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      const n = items.length
      if (n) items[(at + (e.key === 'ArrowDown' ? 1 : -1) + n) % n].focus()
    }
  }
  const menuItem = $derived(menu ? party.queue[menu.index] : undefined)
  // the queue changed under an open menu (someone else moved or removed it): close it
  $effect(() => {
    if (menu && !menuItem) menu = null
  })

  function keydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && !menu) {
      e.preventDefault()
      e.stopPropagation()
      opener?.focus()
      onClose()
    }
  }
  const thumbOf = (hash: string | undefined) => (hash && isMediaHash(hash) ? mediaUrl(hash) : null)
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="pop menu-surface pop-in" role="dialog" aria-label="Watch party queue" tabindex="-1" onkeydown={keydown}
  use:clickOutside={(e) => {
    if (menu) return
    if (opener && e.target instanceof Node && opener.contains(e.target)) return
    onClose()
  }}>
  <div class="head">Queue{party.queue.length ? ` · ${party.queue.length}` : ''}</div>
  {#if party.queue.length}
    <ol class="rows" aria-label="Up next">
      {#each party.queue as item, i (i + item.video_id)}
        {@const editable = mayEdit(party, item, me)}
        {@const thumb = thumbOf(item.thumb?.hash)}
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <li class="row" class:dragging={dragFrom === i} class:over={dragOver === i && dragFrom !== i} tabindex="0"
          aria-label="{item.title ?? 'A YouTube video'}, added by {nameOf(item.added_by)}"
          draggable={editable} ondragstart={(e) => dragStart(e, i)} ondragover={(e) => over(e, i)} ondrop={(e) => drop(e, i)} ondragend={dragEnd}
          oncontextmenu={(e) => { e.preventDefault(); void openMenu(i, e.clientX, e.clientY, e.currentTarget as HTMLElement) }}
          onkeydown={(e) => rowKey(e, i)}>
          <span class="n" aria-hidden="true">{i + 1}</span>
          <span class="thumb" aria-hidden="true">
            {#if thumb}<img src={thumb} alt="" loading="lazy" />{:else}<Icon name="film" size={14} />{/if}
          </span>
          <span class="text">
            <span class="t">{item.title ?? 'A YouTube video'}</span>
            <span class="by">added by {item.added_by === me ? 'you' : nameOf(item.added_by)}</span>
          </span>
        </li>
      {/each}
    </ol>
    <p class="tip">Right-click a video for more. Drag to reorder.</p>
  {:else}
    <p class="empty">Nothing queued. Paste a YouTube link, or drop one on the watch party.</p>
  {/if}
  <form class="add" onsubmit={add}>
    <input bind:this={input} class="input input-sm" type="url" placeholder="Paste a YouTube link" aria-label="YouTube link to add" bind:value={link}
      autocomplete="off" spellcheck="false" />
    <button type="submit" class="btn btn-primary sm" disabled={adding || !link.trim()}>Add</button>
  </form>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

{#if menu && menuItem}
  {@const i = menu.index}
  {@const editable = mayEdit(party, menuItem, me)}
  <div bind:this={menuEl} class="menu menu-surface pop-in" role="menu" aria-label="Queued video" tabindex="-1" style:left="{menuPos.left}px" style:top="{menuPos.top}px"
    style:--pop-origin={menuPos.origin} onkeydown={menuKey} use:clickOutside={(e) => closeMenu(e.type === 'keydown')}>
    {#if control}<button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick({ action: 'play_now', index: i })}>Play now</button>{/if}
    {#if editable && i > 0}
      <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick({ action: 'move', index: i, to: 0 })}>Move to the top</button>
      <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick({ action: 'move', index: i, to: i - 1 })}>Move up</button>
    {/if}
    {#if editable && i < party.queue.length - 1}
      <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick({ action: 'move', index: i, to: i + 1 })}>Move down</button>
    {/if}
    {#if editable}
      <button type="button" class="menu-item danger" role="menuitem" data-item onclick={() => pick({ action: 'unqueue', index: i })}>Remove from queue</button>
    {/if}
    {#if !control && !editable}<span class="none">Only {nameOf(menuItem.added_by)} or the host can change this one</span>{/if}
  </div>
{/if}

<style>
  .pop { position: absolute; top: calc(100% + 6px); right: 0; z-index: 40; width: min(360px, calc(100vw - 32px)); padding: 10px; display: flex; flex-direction: column; gap: 8px; --pop-origin: top right; }
  .head { font-size: 12px; font-weight: 600; color: var(--text-2); text-transform: uppercase; letter-spacing: .04em; }
  .rows { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 2px; max-height: 280px; overflow-y: auto; }
  .row { display: flex; align-items: center; gap: 8px; padding: 4px 6px; border-radius: var(--radius-sm); min-width: 0; cursor: default; }
  .row[draggable='true'] { cursor: grab; }
  .row:hover { background: var(--bg-3); }
  .row:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
  .row.dragging { opacity: .5; }
  .row.over { box-shadow: inset 0 2px 0 var(--focus-ring); }
  .n { width: 14px; text-align: right; font-size: 11px; color: var(--text-3); font-variant-numeric: tabular-nums; flex-shrink: 0; }
  .thumb { width: 48px; height: 27px; border-radius: var(--radius-xs); overflow: hidden; background: var(--bg-inset); display: grid; place-items: center; color: var(--text-3); flex-shrink: 0; }
  .thumb img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .text { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .t { font-size: 13px; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .by { font-size: 11px; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .tip, .empty { margin: 0; font-size: 12px; color: var(--text-3); }
  .add { display: flex; gap: 6px; }
  .add input { flex: 1; min-width: 0; }
  .sm { height: 28px; padding: 0 10px; font-size: 12px; white-space: nowrap; }
  .error { margin: 0; color: var(--danger-text); font-size: 12px; }
  .menu { position: fixed; z-index: 60; width: max-content; min-width: 180px; max-width: 260px; gap: 2px; font-size: 13px; outline: none; --pop-origin: top left; }
  .none { padding: 6px 10px; color: var(--text-3); font-size: 12px; }
</style>
