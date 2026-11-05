<script lang="ts">
  // "Mute …" and its durations flyout (Discord's, #131), inside a right-click menu: the channel menu and the server
  // tab's (#161). It renders the item and the flyout as the menu's own children; the menu asks it to close (another
  // item hovered, the arrow keys moving on, Escape) through the exported functions.
  // Beside the menu, to the left unless the menu is too near the left edge. Hover opens it without moving focus;
  // ArrowRight or Enter opens it with focus on its first duration; ArrowLeft or Escape closes just it, back on the item.
  import { flushSync } from 'svelte'
  import { MUTE_FOR } from '../lib/muteDurations'
  import { closeTimer } from '../lib/hoverIntent'
  import Icon from './Icon.svelte'

  let { label, flyoutLabel, menuLeft, menuTop, onPick }: {
    /** The item's text, e.g. "Mute channel". */
    label: string
    /** The flyout menu's accessible name, e.g. "Mute #general for". */
    flyoutLabel: string
    /** Where the menu sits (fixed), not its rectangle, which the entrance scales. */
    menuLeft: number
    menuTop: number
    /** A duration was picked (minutes; null: until turned back on). */
    onPick: (minutes: number | null) => void
  } = $props()

  const FLYOUT_W = 200
  const GAP = 4
  /** The durations at 32px, 2px apart, 6px padding and a 1px border. */
  const SUB_H = MUTE_FOR.length * 32 + (MUTE_FOR.length - 1) * 2 + 2 * 6 + 2
  const uid = $props.id()
  const flyoutId = `${uid}-mute`
  let muteEl = $state<HTMLButtonElement>()
  let sub = $state<HTMLElement>()
  let subOpen = $state(false)
  const subSide = $derived<'left' | 'right'>(menuLeft - GAP - FLYOUT_W < 8 ? 'right' : 'left')
  let subTop = $state(0)
  const subItems = () => (sub ? [...sub.querySelectorAll<HTMLButtonElement>('[role=menuitem]')] : [])
  function focusSub(i: number) {
    const list = subItems()
    if (list.length) list[(i + list.length) % list.length].focus()
  }
  function openSub(focusFirst: boolean) {
    if (!muteEl) return
    // level with the item (its first duration beside it: the flyout's 6px padding and 1px border), kept inside
    // the window
    const want = menuTop + muteEl.offsetTop - 7
    subTop = Math.max(8, Math.min(want, window.innerHeight - SUB_H - 8)) - menuTop
    subOpen = true
    if (focusFirst) {
      flushSync()
      focusSub(0)
    }
  }

  // hover intent (#173): another item hovered closes it after a short wait, cancelled by coming back
  const later = closeTimer()
  $effect(() => later.cancel)

  /** Closes the flyout. `refocus`: focus goes back to the item ('if-inside': only if focus was in the flyout). */
  export function close(refocus: boolean | 'if-inside') {
    later.cancel()
    if (!subOpen) return
    const back = refocus === 'if-inside' ? !!sub?.contains(document.activeElement) : refocus
    subOpen = false
    if (back) muteEl?.focus()
  }
  /** The pointer is over another of the menu's items: close after the hover-intent wait, unless it comes back. */
  export function closeSoon() {
    if (subOpen) later.schedule(() => close('if-inside'))
  }
  export function isOpen(): boolean {
    return subOpen
  }
  /** The item itself (the menu tells it apart from its other items). */
  export function trigger(): HTMLButtonElement | undefined {
    return muteEl
  }

  function triggerKeydown(e: KeyboardEvent) {
    if (e.key !== 'ArrowRight') return
    e.preventDefault()
    // the menu's own arrow handling leaves it alone
    e.stopPropagation()
    openSub(true)
  }
  function subKeydown(e: KeyboardEvent) {
    const list = subItems()
    const at = list.indexOf(document.activeElement as HTMLButtonElement)
    switch (e.key) {
      case 'ArrowDown': focusSub(at + 1); break
      case 'ArrowUp': focusSub(at < 0 ? -1 : at - 1); break
      case 'Home': focusSub(0); break
      case 'End': focusSub(-1); break
      case 'ArrowLeft': case 'Escape': close(true); break
      // Tab leaves the whole menu, as in the main one
      default: return
    }
    e.preventDefault()
    // the main menu (and its Escape-anywhere close) leaves it alone
    e.stopPropagation()
  }
</script>

<button bind:this={muteEl} type="button" class="menu-item has-sub" class:hi={subOpen} role="menuitem" data-item aria-haspopup="menu" aria-expanded={subOpen}
  aria-controls={subOpen ? flyoutId : undefined} onpointerenter={() => { later.cancel(); openSub(false) }} onclick={() => openSub(true)} onkeydown={triggerKeydown}>
  <span class="grow">{label}</span><Icon name={subSide === 'right' ? 'chevronRight' : 'chevronLeft'} size={14} />
</button>
{#if subOpen}
  <div bind:this={sub} id={flyoutId} class="flyout menu-surface" data-side={subSide} role="menu" aria-label={flyoutLabel} tabindex="-1"
    style:top="{subTop}px" onkeydown={subKeydown} onpointerenter={later.cancel}>
    {#each MUTE_FOR as d (d.label)}
      <button type="button" class="menu-item" role="menuitem" onclick={() => onPick(d.minutes)}>{d.label}</button>
    {/each}
  </div>
{/if}

<style>
  .grow { flex: 1; }
  .has-sub :global(svg) { color: var(--text-3); }
  /* beside the menu, past its 1px border, with a small gap the pointer crosses without closing it */
  .flyout { position: absolute; z-index: 1; width: 200px; gap: 2px; outline: none; }
  .flyout[data-side='left'] { right: calc(100% + 4px + 1px); }
  .flyout[data-side='right'] { left: calc(100% + 4px + 1px); }
</style>
