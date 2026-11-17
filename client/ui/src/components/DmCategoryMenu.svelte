<script lang="ts">
  // The right-click menu on one of your own conversation categories (#305), or Shift+F10 / the menu key on its
  // heading: Rename category… (in the heading), Move up / Move down among your categories (#419; only where it can go)
  // and Delete category (no dialog: its conversations go back to the main list, pinned ones stay pinned, nothing is
  // lost). Keyboard and focus as in the server category menu.
  import { onMount, untrack } from 'svelte'
  import { errorText } from '../lib/tauri'
  import { placeMenu } from '../lib/menuPlace'
  import { clickOutside } from '../lib/actions'
  import { deleteCategory, moveCategory } from '../lib/dmLayout.svelte'
  import { app } from '../lib/store.svelte'
  import type { DmCategory } from '../lib/protocol/DmCategory'

  let { category, x, y, opener, onClose, onRename, onError }: {
    category: DmCategory
    x: number
    y: number
    /** The heading it opened from: focus goes back to it when the menu closes from the keyboard. */
    opener: HTMLElement
    onClose: () => void
    onRename: () => void
    /** An action failed after the menu had gone: the list says why. */
    onError: (message: string) => void
  } = $props()

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
  function pick(action: () => unknown) {
    if (closed) return
    if (opener.isConnected) opener.focus()
    void action()
    close(false)
  }
  const remove = () => deleteCategory(category.id).catch((e) => onError(errorText(e)))
  const index = untrack(() => app.state.dmLayout.categories.findIndex((c) => c.id === category.id))
  const count = untrack(() => app.state.dmLayout.categories.length)
  const move = (by: -1 | 1) => moveCategory(category.id, by).catch((e) => onError(errorText(e)))

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
  <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(onRename)}>Rename category…</button>
  {#if index > 0}<button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(() => move(-1))}>Move up</button>{/if}
  {#if index >= 0 && index < count - 1}<button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(() => move(1))}>Move down</button>{/if}
  <button type="button" class="menu-item danger" role="menuitem" data-item onclick={() => pick(remove)}>Delete category</button>
</div>

<style>
  .menu { position: fixed; z-index: 60; width: max-content; min-width: 200px; max-width: 280px; gap: 2px; outline: none; --pop-origin: top left; }
</style>
