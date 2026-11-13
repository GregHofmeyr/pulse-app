<script lang="ts">
  // An item with a flyout of choices (Discord's, #131), inside a right-click menu: "Mute …" and its durations
  // (MuteFlyout), and "Notifications" and its levels with the current one ticked (#195), in the channel menu and the
  // server tab's (#161). It renders the item and the flyout as the menu's own children; the menu asks it to close
  // (another item hovered, the arrow keys moving on, Escape) through the exported functions.
  // Beside the menu, to the left unless the menu is too near the left edge; in a window too narrow for either side,
  // pulled back over the menu so it stays inside (#134). Hover opens it without moving focus;
  // ArrowRight or Enter opens it with focus on its first choice; ArrowLeft or Escape closes just it, back on the item.
  import { flushSync } from 'svelte'
  import { closeTimer } from '../lib/hoverIntent'
  import { placeFlyout } from '../lib/menuPlace'
  import Icon, { type IconName } from './Icon.svelte'

  let { label, flyoutLabel, menuLeft, menuTop, choices, onPick, item = '', icon }: {
    /** An icon before the label, in a menu whose items have them (a clip's menu, #257). */
    icon?: IconName
    /** The item's text, e.g. "Mute channel". */
    label: string
    /** The flyout menu's accessible name, e.g. "Mute #general for". */
    flyoutLabel: string
    /** Where the menu sits (fixed), not its rectangle, which the entrance scales. */
    menuLeft: number
    menuTop: number
    /** The flyout's choices. With `checked` set on any, they are radio items and the checked one is ticked (as the
     *  status menu ticks yours). */
    choices: readonly { label: string; checked?: boolean }[]
    /** The choice at this index was picked. */
    onPick: (index: number) => void
    /** The item's `data-item` value, for menus that name their items (the person menu's "mute"). */
    item?: string
  } = $props()

  /** The choices at 32px, 2px apart, 6px padding and a 1px border. */
  const SUB_H = $derived(choices.length * 32 + (choices.length - 1) * 2 + 2 * 6 + 2)
  const radio = $derived(choices.some((c) => c.checked !== undefined))
  const uid = $props.id()
  const flyoutId = `${uid}-sub`
  let itemEl = $state<HTMLButtonElement>()
  let sub = $state<HTMLElement>()
  let subOpen = $state(false)
  /** The menu's layout width (the item's parent is the menu), measured when the flyout opens. */
  let menuW = $state(0)
  const place = $derived(placeFlyout(menuLeft, menuW))
  const subSide = $derived(place.side)
  let subTop = $state(0)
  const subItems = () => (sub ? [...sub.querySelectorAll<HTMLButtonElement>('[role^=menuitem]')] : [])
  function focusSub(i: number) {
    const list = subItems()
    if (list.length) list[(i + list.length) % list.length].focus()
  }
  function openSub(focusFirst: boolean) {
    if (!itemEl) return
    menuW = itemEl.parentElement?.offsetWidth ?? 0
    // level with the item (its first choice beside it: the flyout's 6px padding and 1px border), kept inside
    // the window
    const want = menuTop + itemEl.offsetTop - 7
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
    if (back) itemEl?.focus()
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
    return itemEl
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

<button bind:this={itemEl} type="button" class="menu-item has-sub" class:hi={subOpen} role="menuitem" data-item={item} aria-haspopup="menu" aria-expanded={subOpen}
  aria-controls={subOpen ? flyoutId : undefined} onpointerenter={() => { later.cancel(); openSub(false) }} onclick={() => openSub(true)} onkeydown={triggerKeydown}>
  {#if icon}<Icon name={icon} size={14} />{/if}<span class="grow">{label}</span><Icon name={subSide === 'right' ? 'chevronRight' : 'chevronLeft'} size={14} />
</button>
{#if subOpen}
  <div bind:this={sub} id={flyoutId} class="flyout menu-surface" data-side={subSide} role="menu" aria-label={flyoutLabel} tabindex="-1"
    style:top="{subTop}px" style:margin-left={place.shift && subSide === 'right' ? `${-place.shift}px` : null}
    style:margin-right={place.shift && subSide === 'left' ? `${-place.shift}px` : null} onkeydown={subKeydown} onpointerenter={later.cancel}>
    {#each choices as c, i (c.label)}
      {#if radio}
        <button type="button" class="menu-item" role="menuitemradio" aria-checked={!!c.checked} onclick={() => onPick(i)}>
          <span class="grow">{c.label}</span>{#if c.checked}<Icon name="check" size={14} />{/if}
        </button>
      {:else}
        <button type="button" class="menu-item" role="menuitem" onclick={() => onPick(i)}>{c.label}</button>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .grow { flex: 1; }
  /* the chevron (a leading icon keeps the item's colour) */
  .has-sub > :global(svg:last-child) { color: var(--text-3); }
  /* beside the menu, past its 1px border, with a small gap the pointer crosses without closing it */
  .flyout { position: absolute; z-index: 1; width: 200px; gap: 2px; outline: none; }
  .flyout[data-side='left'] { right: calc(100% + 4px + 1px); }
  .flyout[data-side='right'] { left: calc(100% + 4px + 1px); }
</style>
