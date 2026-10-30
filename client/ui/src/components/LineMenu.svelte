<script lang="ts">
  import { onMount, untrack } from 'svelte'
  import { clickOutside } from '../lib/actions'
  import { suggest, type Mentionable } from '../lib/mentions'

  let {
    x,
    y,
    opener,
    award = null,
    link = null,
    members = [],
    onAward,
    onCopy,
    onLink,
    onUnlink,
    onClose,
  }: {
    x: number
    y: number
    /** What had focus (or was clicked): focus goes back to it when the menu closes from the keyboard. */
    opener: HTMLElement
    /** The award entry: none on your own line. */
    award?: 'give' | 'take' | null
    /** The plain-name entry: none on a member's line. */
    link?: 'link' | 'unlink' | null
    /** Who a plain name can be linked to. */
    members?: Mentionable[]
    onAward: () => unknown
    /** Null where there is nothing to copy (an image without a caption). */
    onCopy: (() => unknown) | null
    onLink: (userId: string) => unknown
    onUnlink: () => unknown
    onClose: () => void
  } = $props()

  let el = $state<HTMLElement>()
  const x0 = untrack(() => x)
  const y0 = untrack(() => y)
  let left = $state(x0)
  let top = $state(y0)
  let closed = false
  let picking = $state(false)
  let query = $state('')
  const matches = $derived(suggest(members, query))

  function place() {
    if (!el) return
    const r = el.getBoundingClientRect()
    left = Math.max(8, Math.min(x0, window.innerWidth - r.width - 8))
    top = Math.max(8, Math.min(y0, window.innerHeight - r.height - 8))
  }
  onMount(() => {
    place()
    focusItem(0)
  })

  const items = () => (el ? [...el.querySelectorAll<HTMLElement>('[data-item]')] : [])
  function focusItem(i: number) {
    const list = items()
    if (list.length) list[(i + list.length) % list.length].focus()
  }
  function startPicking() {
    picking = true
    // the picker is taller than the menu: re-fit it once drawn, then type-ahead gets the focus
    queueMicrotask(() => {
      if (closed) return
      place()
      el?.querySelector<HTMLInputElement>('input')?.focus()
    })
  }

  function close(restoreFocus: boolean) {
    if (closed) return
    closed = true
    if (restoreFocus && opener.isConnected) opener.focus()
    onClose()
  }
  /** Focus goes back first, then it acts, then the menu goes (same order as the participant menu). */
  function pick(action: () => unknown) {
    if (closed) return
    if (opener.isConnected) opener.focus()
    void action()
    close(false)
  }

  function keydown(e: KeyboardEvent) {
    const list = items()
    const at = list.indexOf(document.activeElement as HTMLElement)
    switch (e.key) {
      case 'ArrowDown': e.preventDefault(); focusItem(at + 1); break
      case 'ArrowUp': e.preventDefault(); focusItem(at < 0 ? -1 : at - 1); break
      // in the search box Home/End move the caret
      case 'Home': if (!(e.target instanceof HTMLInputElement)) { e.preventDefault(); focusItem(0) } break
      case 'End': if (!(e.target instanceof HTMLInputElement)) { e.preventDefault(); focusItem(-1) } break
      case 'Escape': e.preventDefault(); close(true); break
      case 'Tab': e.preventDefault(); close(true); break
      case 'Enter': {
        const t = e.target as HTMLElement
        if (t instanceof HTMLButtonElement) {
          e.preventDefault()
          t.click()
        } else if (t instanceof HTMLInputElement && matches[0]) {
          e.preventDefault()
          pick(() => onLink(matches[0].id))
        }
        break
      }
    }
  }
</script>

<div bind:this={el} class="menu menu-surface" role="menu" aria-label="Line actions" tabindex="-1" style:position="fixed" style:left="{left}px" style:top="{top}px"
  onkeydown={keydown} use:clickOutside={(e) => close(e.type === 'keydown')}>
  {#if picking}
    <input type="text" class="input input-sm find" data-item aria-label="Find a member" placeholder="Find a member" autocomplete="off" bind:value={query} />
    <div role="group" aria-label="Members">
      {#each matches as m (m.id)}
        <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(() => onLink(m.id))}>{m.name}</button>
      {:else}
        <span class="none">No one by that name</span>
      {/each}
    </div>
  {:else}
    {#if award}
      <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(onAward)}>{award === 'give' ? 'Give quote award' : 'Take back award'}</button>
    {/if}
    {#if onCopy}<button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(onCopy)}>Copy</button>{/if}
    {#if link === 'link'}
      <button type="button" class="menu-item" role="menuitem" data-item onclick={startPicking}>Link to a member…</button>
    {:else if link === 'unlink'}
      <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(onUnlink)}>Unlink</button>
    {/if}
  {/if}
</div>

<style>
  .menu { position: fixed; z-index: 60; min-width: 200px; max-width: 280px; gap: 2px; font-size: 13px; outline: none; }
  .find { margin-bottom: 2px; }
  [role='group'] { display: flex; flex-direction: column; gap: 2px; }
  .none { padding: 6px 10px; color: var(--text-3); }
</style>
