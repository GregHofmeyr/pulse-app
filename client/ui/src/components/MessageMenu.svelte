<script lang="ts">
  // A message's menu (#178): from its action bar's More, or a right-click (Shift+F10, the menu key) on the message.
  // Pin message, with its lengths as the shared hover flyout (MenuFlyout, as Mute's), or Unpin message on a pinned
  // one. Built like the channel menu: placed by placeMenu, arrows move, Escape closes a hover-opened flyout first.
  import { onMount, untrack } from 'svelte'
  import { clickOutside } from '../lib/actions'
  import { placeMenu } from '../lib/menuPlace'
  import { PIN_FOR } from '../lib/pins'
  import MenuFlyout from './MenuFlyout.svelte'
  import type { PinDuration } from '../lib/protocol/PinDuration'

  let { pinned, x, y, opener, onPin, onUnpin, onClose }: {
    /** The message is pinned: Unpin message instead of Pin message. */
    pinned: boolean
    x: number
    y: number
    /** What it opened from: focus goes back to it when the menu closes from the keyboard. */
    opener: HTMLElement
    onPin: (duration: PinDuration) => unknown
    onUnpin: () => unknown
    onClose: () => void
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

  // the menu's own items (the flyout's sit inside it, under their own menu)
  const items = () => (el ? [...el.querySelectorAll<HTMLButtonElement>(':scope > [data-item]:not(:disabled)')] : [])
  function focusItem(i: number) {
    const list = items()
    if (list.length) list[(i + list.length) % list.length].focus()
  }

  let flyout = $state<ReturnType<typeof MenuFlyout>>()
  const closeSub = (refocus: boolean | 'if-inside') => flyout?.close(refocus)

  function close(restoreFocus: boolean) {
    if (closed) return
    closed = true
    if (restoreFocus && opener.isConnected) opener.focus()
    onClose()
  }
  /** Focus goes back first, then it acts, then the menu goes (as the channel menu). */
  function pick(action: () => unknown) {
    if (closed) return
    if (opener.isConnected) opener.focus()
    void action()
    close(false)
  }

  function keydown(e: KeyboardEvent) {
    const list = items()
    const at = list.indexOf(document.activeElement as HTMLButtonElement)
    if (e.key === 'ArrowLeft') { e.preventDefault(); closeSub(false); return }
    // a flyout opened by hover: the first Escape closes just it (and not, from the window, the menu)
    if (e.key === 'Escape' && flyout?.isOpen()) { e.preventDefault(); e.stopPropagation(); closeSub(false); return }
    if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(e.key)) closeSub(false)
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

<div bind:this={el} class="menu menu-surface pop-in" role="menu" aria-label="Message actions" tabindex="-1" style:left="{left}px" style:top="{top}px" style:--pop-origin={origin}
  onkeydown={keydown} use:clickOutside={(e) => close(e.type === 'keydown')}
  onpointerover={(e) => {
    // another of the menu's own items under the pointer closes the flyout
    const t = (e.target as Element).closest?.('[data-item]')
    if (t && t.parentElement === el && t !== flyout?.trigger()) flyout?.closeSoon()
  }}>
  {#if pinned}
    <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(onUnpin)}>Unpin message</button>
  {:else}
    <MenuFlyout bind:this={flyout} label="Pin message" flyoutLabel="Pin message for" menuLeft={left} menuTop={top} item="pin"
      choices={PIN_FOR} onPick={(i) => pick(() => onPin(PIN_FOR[i].duration))} />
  {/if}
</div>

<style>
  .menu { position: fixed; z-index: 60; width: max-content; min-width: 220px; max-width: 280px; gap: 2px; outline: none; --pop-origin: top left; }
</style>
