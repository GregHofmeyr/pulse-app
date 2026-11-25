<script lang="ts">
  // A build's right-click menu in the browse view (#617; or the menu key / Shift+F10 on its card or row): Open,
  // Favourite (or Unfavourite: your own star), Edit, Archive (or Restore), Delete… and Select. Edit, Archive and Delete as the build page's Actions menu allows
  // (`buildRights`); the list acts on the pick (Delete asks there first). Placed and driven as the channel menu is.
  import { onMount, untrack } from 'svelte'
  import Icon, { type IconName } from './Icon.svelte'
  import { clickOutside } from '../lib/actions'
  import { buildRights, isArchived, isFavourite } from '../lib/builds'
  import { buildMenuItems, type BuildMenuItem } from '../lib/buildSelect'
  import { placeMenu } from '../lib/menuPlace'
  import { app } from '../lib/store.svelte'
  import type { Message } from '../lib/protocol/Message'

  let { message, serverId, x, y, opener, onPick, onClose }: {
    message: Message
    serverId: string
    x: number
    y: number
    /** What it opened from: the focus goes back there when it closes from the keyboard. */
    opener: HTMLElement
    onPick: (item: BuildMenuItem, m: Message) => void
    onClose: () => void
  } = $props()

  const ITEM: Record<BuildMenuItem, { label: string; icon: IconName }> = {
    open: { label: 'Open', icon: 'expand' },
    favourite: { label: 'Favourite', icon: 'star' },
    unfavourite: { label: 'Unfavourite', icon: 'star' },
    edit: { label: 'Edit', icon: 'pencil' },
    archive: { label: 'Archive', icon: 'inbox' },
    restore: { label: 'Restore', icon: 'inbox' },
    delete: { label: 'Delete…', icon: 'trash' },
    select: { label: 'Select', icon: 'check' },
  }
  const list = $derived(buildMenuItems(buildRights(app.state, serverId, message), isArchived(message), isFavourite(message)))

  let el = $state<HTMLElement>()
  let left = $state(untrack(() => x))
  let top = $state(untrack(() => y))
  let origin = $state('top left')
  let closed = false
  onMount(() => {
    if (!el) return
    ;({ left, top, origin } = placeMenu(x, y, el.offsetWidth, el.offsetHeight))
    focusItem(0)
  })
  const items = () => (el ? [...el.querySelectorAll<HTMLButtonElement>('[role=menuitem]')] : [])
  function focusItem(i: number) {
    const l = items()
    if (l.length) l[(i + l.length) % l.length].focus()
  }
  function close(restoreFocus: boolean) {
    if (closed) return
    closed = true
    if (restoreFocus && opener.isConnected) opener.focus()
    onClose()
  }
  function pick(it: BuildMenuItem) {
    if (closed) return
    if (opener.isConnected) opener.focus()
    const m = message
    close(false)
    onPick(it, m)
  }
  function keydown(e: KeyboardEvent) {
    const l = items()
    const at = l.indexOf(document.activeElement as HTMLButtonElement)
    switch (e.key) {
      case 'ArrowDown': e.preventDefault(); focusItem(at + 1); break
      case 'ArrowUp': e.preventDefault(); focusItem(at < 0 ? -1 : at - 1); break
      case 'Home': e.preventDefault(); focusItem(0); break
      case 'End': e.preventDefault(); focusItem(-1); break
      // handled here: the browse view's Escape (leaving multi-select) leaves it alone
      case 'Escape': e.preventDefault(); e.stopPropagation(); close(true); break
      case 'Tab': e.preventDefault(); close(true); break
    }
  }
</script>

<div bind:this={el} class="build-menu menu menu-surface pop-in" role="menu" aria-label="Actions for {message.build?.title ?? 'build'}" tabindex="-1"
  style:left="{left}px" style:top="{top}px" style:--pop-origin={origin} onkeydown={keydown} use:clickOutside={(e) => close(e.type === 'keydown')}>
  {#each list as it (it)}
    {#if it === 'select'}<span class="menu-sep"></span>{/if}
    <button type="button" class="menu-item" class:danger={it === 'delete'} role="menuitem" data-item={it} onclick={() => pick(it)}><Icon name={ITEM[it].icon} size={14} />{ITEM[it].label}</button>
  {/each}
</div>

<style>
  .menu { position: fixed; z-index: 60; width: max-content; min-width: 180px; max-width: 280px; gap: 2px; outline: none; --pop-origin: top left; }
</style>
