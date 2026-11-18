<script lang="ts" module>
  import type { IconName } from './Icon.svelte'
  /** One row of the composer's + menu (#461). `sep`: a divider goes above it. */
  export type PlusItem = { id: string; label: string; icon: IconName; sep?: boolean; disabled?: boolean; run: () => void }
</script>

<script lang="ts">
  // The composer's + menu (#461): Attach files, then the wheel, poll, plan and game invite, then Send later…, each only
  // where it applies (the composer passes what's on offer). Above the + button, its left edge on the button's.
  // Arrow keys, Home and End move between the items; Escape or Tab closes it, back on the + button.
  import { onMount, tick, untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import { clickOutside } from '../lib/actions'

  let { anchor, items, onClose }: {
    /** The + button (`data-menu-toggle`, so a press on it toggles instead). */
    anchor: HTMLElement
    items: readonly PlusItem[]
    onClose: () => void
  } = $props()

  let el: HTMLElement | undefined = $state()
  const pos = untrack(() => {
    const r = anchor.getBoundingClientRect()
    return `position: fixed; left: ${Math.max(6, Math.round(r.left))}px; bottom: ${Math.round(window.innerHeight - r.top + 6)}px; z-index: 50;`
  })
  const buttons = () => [...(el?.querySelectorAll<HTMLButtonElement>('[role=menuitem]:not(:disabled)') ?? [])]
  function focusAt(i: number) {
    const list = buttons()
    if (list.length) list[(i + list.length) % list.length].focus()
  }
  onMount(() => void tick().then(() => focusAt(0)))

  function key(e: KeyboardEvent) {
    const list = buttons()
    const at = list.indexOf(document.activeElement as HTMLButtonElement)
    switch (e.key) {
      case 'ArrowDown': e.preventDefault(); focusAt(at + 1); break
      case 'ArrowUp': e.preventDefault(); focusAt(at < 0 ? -1 : at - 1); break
      case 'Home': e.preventDefault(); focusAt(0); break
      case 'End': e.preventDefault(); focusAt(-1); break
      case 'Escape':
      case 'Tab':
        e.preventDefault()
        e.stopPropagation()
        onClose()
        anchor.focus()
        break
    }
  }
  function choose(it: PlusItem) {
    onClose()
    it.run()
  }
</script>

<div bind:this={el} class="plus menu-surface pop-in" role="menu" aria-label="Add to your message" tabindex="-1" style={pos}
  use:clickOutside={onClose} onkeydown={key}>
  {#each items as it (it.id)}
    {#if it.sep}<div class="menu-sep" role="separator"></div>{/if}
    <button type="button" class="menu-item" role="menuitem" data-item={it.id} disabled={it.disabled} onclick={() => choose(it)}>
      <Icon name={it.icon} size={16} />{it.label}
    </button>
  {/each}
</div>

<style>
  .plus { width: max-content; min-width: 200px; gap: 2px; --pop-origin: bottom left; }
  .plus .menu-item :global(svg) { color: var(--text-2); }
</style>
