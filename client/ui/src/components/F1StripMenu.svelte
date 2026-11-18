<script lang="ts">
  // The right-click menu on the F1 countdown strip (#456), or Shift+F10 / the menu key on it: Hide for this weekend
  // (until the next race weekend, this device only) and Turn off F1 strip (Settings › Chat › F1 countdown strip,
  // which is also the way back for both). Keyboard and focus as in the other cursor menus.
  import { onMount, untrack } from 'svelte'
  import { placeMenu } from '../lib/menuPlace'
  import { clickOutside } from '../lib/actions'

  let { x, y, opener, onClose, onHideWeekend, onTurnOff }: {
    x: number
    y: number
    /** The strip's button: focus goes back to it when the menu closes from the keyboard (if it's still there). */
    opener: HTMLElement
    onClose: () => void
    onHideWeekend: () => void
    onTurnOff: () => void
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
  function pick(action: () => void) {
    if (closed) return
    action()
    close(false)
  }
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

<div bind:this={el} class="menu menu-surface pop-in" role="menu" aria-label="F1 strip" tabindex="-1" style:left="{left}px" style:top="{top}px" style:--pop-origin={origin}
  onkeydown={keydown} use:clickOutside={(e) => close(e.type === 'keydown')}>
  <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(onHideWeekend)}>Hide for this weekend</button>
  <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(onTurnOff)}>Turn off F1 strip</button>
  <div class="menu-sep" role="separator"></div>
  <p class="note">Bring it back in Settings › Chat.</p>
</div>

<style>
  .menu { position: fixed; z-index: 60; width: max-content; min-width: 200px; max-width: 280px; gap: 2px; outline: none; --pop-origin: top left; }
  .note { margin: 0; padding: 4px 10px 2px; font-size: 11px; color: var(--text-3); }
</style>
