<script lang="ts">
  // The right-click menu on a category heading (#54), or Shift+F10 / the menu key on it: Create channel (the list's
  // inline field, in this category), Rename category… (in the heading), Delete category (no dialog: its channels move
  // to the top of the list, nothing is lost), then Move up / Move down (the keyboard's way to drag). Members only.
  // Keyboard and focus as in the channel menu.
  import { onMount, untrack } from 'svelte'
  import { api, errorText } from '../lib/tauri'
  import { placeMenu } from '../lib/menuPlace'
  import { clickOutside } from '../lib/actions'
  import type { Category } from '../lib/protocol/Category'

  let { category, x, y, opener, canMoveUp, canMoveDown, onClose, onCreateChannel, onRename, onMoveUp, onMoveDown, onError }: {
    category: Category
    x: number
    y: number
    /** The heading it opened from: focus goes back to it when the menu closes from the keyboard. */
    opener: HTMLElement
    /** False at the top / bottom: that move is shown, disabled. */
    canMoveUp: boolean
    canMoveDown: boolean
    onClose: () => void
    onCreateChannel: () => void
    onRename: () => void
    onMoveUp: () => void
    onMoveDown: () => void
    /** An action failed after the menu had gone: the list says why. */
    onError: (message: string) => void
  } = $props()

  let el = $state<HTMLElement>()
  let left = $state(untrack(() => x))
  let top = $state(untrack(() => y))
  /** where its pop-in grows from: the corner nearest the cursor (#181) */
  let origin = $state('top left')
  let closed = false

  onMount(() => {
    if (!el) return
    // the layout size: the .pop-in entrance scales the box while it runs
    ;({ left, top, origin } = placeMenu(x, y, el.offsetWidth, el.offsetHeight))
    focusItem(0)
  })

  const items = () => (el ? [...el.querySelectorAll<HTMLButtonElement>(':scope > [data-item]:not(:disabled)')] : [])
  function focusItem(i: number) {
    const list = items()
    if (list.length) list[(i + list.length) % list.length].focus()
  }
  function close(restoreFocus: boolean) {
    if (closed) return
    closed = true
    if (restoreFocus && opener.isConnected) opener.focus()
    onClose()
  }
  /** Focus goes back to the heading first (a rename or a field then takes it from there), then it acts, then the
   *  menu goes. A failure lands with `report`, after the menu has gone. */
  function pick(action: () => unknown) {
    if (closed) return
    if (opener.isConnected) opener.focus()
    void action()
    close(false)
  }
  const report = (f: () => Promise<unknown>) => () => f().catch((e) => onError(errorText(e)))

  function keydown(e: KeyboardEvent) {
    const list = items()
    const at = list.indexOf(document.activeElement as HTMLButtonElement)
    switch (e.key) {
      case 'ArrowDown': e.preventDefault(); focusItem(at + 1); break
      case 'ArrowUp': e.preventDefault(); focusItem(at < 0 ? -1 : at - 1); break
      case 'Home': e.preventDefault(); focusItem(0); break
      case 'End': e.preventDefault(); focusItem(-1); break
      case 'Escape': e.preventDefault(); close(true); break
      case 'Tab': e.preventDefault(); close(true); break
    }
  }
</script>

<div bind:this={el} class="menu menu-surface pop-in" role="menu" aria-label="Actions for {category.name}" tabindex="-1" style:left="{left}px" style:top="{top}px" style:--pop-origin={origin}
  onkeydown={keydown} use:clickOutside={(e) => close(e.type === 'keydown')}>
  <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(onCreateChannel)}>Create channel</button>
  <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(onRename)}>Rename category…</button>
  <button type="button" class="menu-item danger" role="menuitem" data-item onclick={() => pick(report(() => api.deleteCategory(category.id)))}>Delete category</button>
  <span class="menu-sep"></span>
  <button type="button" class="menu-item" role="menuitem" data-item disabled={!canMoveUp} onclick={() => pick(onMoveUp)}>Move up</button>
  <button type="button" class="menu-item" role="menuitem" data-item disabled={!canMoveDown} onclick={() => pick(onMoveDown)}>Move down</button>
</div>

<style>
  .menu { position: fixed; z-index: 60; width: max-content; min-width: 220px; max-width: 280px; gap: 2px; outline: none; --pop-origin: top left; }
  .menu-item:disabled { color: var(--text-3); cursor: default; }
</style>
