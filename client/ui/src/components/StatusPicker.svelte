<script lang="ts">
  // Your status (#45): opens from the status row on your own profile card (#168), above it. Online, Do Not Disturb (its
  // timers in a flyout, like a channel's Mute), Invisible, each with its dot and the current one ticked. The server answers with a StatusChanged to every session, which
  // ticks the new one; nothing is set here meanwhile.
  import { flushSync, onMount } from 'svelte'
  import { app, clock, ui } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { clickOutside } from '../lib/actions'
  import { closeTimer } from '../lib/hoverIntent'
  import { DND_FOR, dndUntil, ownStatus, type DndChoice } from '../lib/status'
  import Icon from './Icon.svelte'
  import type { Status } from '../lib/protocol/Status'

  let { opener, onClose }: {
    /** Your card's status row: the menu sits above it, and focus goes back to it when the menu closes from the keyboard. */
    opener: HTMLElement
    onClose: () => void
  } = $props()

  const STATUSES: { id: Status; label: string }[] = [
    { id: 'online', label: 'Online' },
    { id: 'dnd', label: 'Do Not Disturb' },
    { id: 'invisible', label: 'Invisible' },
  ]
  const current = $derived(ownStatus(app.state, clock.nowIso))

  let el = $state<HTMLElement>()
  let left = $state(8)
  let top = $state(8)
  let closed = false

  onMount(() => {
    if (!el) return
    const r = opener.getBoundingClientRect()
    left = Math.max(8, Math.min(r.left, window.innerWidth - el.offsetWidth - 8))
    top = Math.max(8, r.top - el.offsetHeight - 8)
    focusItem(0)
  })

  const items = () => (el ? [...el.querySelectorAll<HTMLButtonElement>(':scope > [data-item]')] : [])
  function focusItem(i: number) {
    const list = items()
    if (list.length) list[(i + list.length) % list.length].focus()
  }

  // The DND flyout: to the right (the sidebar is on the left), level with Do Not Disturb. Hover opens it without
  // moving focus; ArrowRight or a click opens it with focus on its first timer; ArrowLeft or Escape closes just it.
  const uid = $props.id()
  const flyoutId = `${uid}-dnd`
  let dndEl = $state<HTMLButtonElement>()
  let sub = $state<HTMLElement>()
  let subOpen = $state(false)
  let subTop = $state(0)
  const subItems = () => (sub ? [...sub.querySelectorAll<HTMLButtonElement>('[role=menuitem]')] : [])
  function focusSub(i: number) {
    const list = subItems()
    if (list.length) list[(i + list.length) % list.length].focus()
  }
  function openSub(focusFirst: boolean) {
    if (!dndEl) return
    // its first timer beside Do Not Disturb (the flyout's 6px padding and 1px border)
    subTop = dndEl.offsetTop - 7
    subOpen = true
    if (focusFirst) {
      flushSync()
      focusSub(0)
    }
  }
  // hover intent (#173): another item hovered closes the flyout after a short wait, cancelled by coming back
  const later = closeTimer()
  $effect(() => later.cancel)
  function closeSub(refocus: boolean) {
    later.cancel()
    if (!subOpen) return
    subOpen = false
    if (refocus) dndEl?.focus()
  }
  function subKeydown(e: KeyboardEvent) {
    const list = subItems()
    const at = list.indexOf(document.activeElement as HTMLButtonElement)
    switch (e.key) {
      case 'ArrowDown': focusSub(at + 1); break
      case 'ArrowUp': focusSub(at < 0 ? -1 : at - 1); break
      case 'Home': focusSub(0); break
      case 'End': focusSub(-1); break
      case 'ArrowLeft': case 'Escape': closeSub(true); break
      default: return
    }
    e.preventDefault()
    e.stopPropagation()
  }

  function close(restoreFocus: boolean) {
    if (closed) return
    closed = true
    if (restoreFocus && opener.isConnected) opener.focus()
    onClose()
  }
  /** Focus goes back to the status row, it acts, then the menu goes; a failure shows as the shell's notice. */
  function pick(action: () => unknown) {
    if (closed) return
    if (opener.isConnected) opener.focus()
    void action()
    close(false)
  }
  const set = (status: Status, until: string | null = null) =>
    pick(() => api.setStatus(status, until).catch((e: unknown) => (ui.notice = `Couldn't change your status: ${errorText(e)}`)))
  const setDnd = (d: DndChoice) => set('dnd', dndUntil(d, new Date()))

  function keydown(e: KeyboardEvent) {
    const list = items()
    const at = list.indexOf(document.activeElement as HTMLButtonElement)
    if (e.key === 'ArrowRight' && dndEl && document.activeElement === dndEl) {
      e.preventDefault()
      openSub(true)
      return
    }
    if (e.key === 'ArrowLeft') { e.preventDefault(); closeSub(false); return }
    if (e.key === 'Escape' && subOpen) { e.preventDefault(); e.stopPropagation(); closeSub(false); return }
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

<div bind:this={el} class="status-menu menu-surface pop-in" role="menu" aria-label="Your status" tabindex="-1" style:left="{left}px" style:top="{top}px"
  onkeydown={keydown} use:clickOutside={(e) => close(e.type === 'keydown')}
  onpointerover={(e) => {
    // another of the menu's own items under the pointer closes the flyout
    const t = (e.target as Element).closest?.('[data-item]')
    if (t && t.parentElement === el && t !== dndEl && subOpen) later.schedule(() => closeSub(!!sub?.contains(document.activeElement)))
  }}>
  {#each STATUSES as s (s.id)}
    {#if s.id === 'dnd'}
      <button bind:this={dndEl} type="button" class="menu-item has-sub" class:hi={subOpen} role="menuitemradio" aria-checked={current === 'dnd'} data-item="dnd"
        aria-haspopup="menu" aria-expanded={subOpen} aria-controls={subOpen ? flyoutId : undefined} onpointerenter={() => { later.cancel(); openSub(false) }} onclick={() => openSub(true)}>
        <span class="dot dnd" aria-hidden="true"></span><span class="grow">{s.label}</span>
        {#if current === 'dnd'}<Icon name="check" size={14} />{/if}<Icon name="chevronRight" size={14} />
      </button>
      {#if subOpen}
        <div bind:this={sub} id={flyoutId} class="flyout menu-surface" role="menu" aria-label="Do Not Disturb for" tabindex="-1" style:top="{subTop}px" onkeydown={subKeydown} onpointerenter={later.cancel}>
          {#each DND_FOR as d (d.label)}
            <button type="button" class="menu-item" role="menuitem" onclick={() => setDnd(d)}>{d.label}</button>
          {/each}
        </div>
      {/if}
    {:else}
      <button type="button" class="menu-item" role="menuitemradio" aria-checked={current === s.id} data-item={s.id} onclick={() => set(s.id)}>
        <span class="dot {s.id}" aria-hidden="true"></span><span class="grow">{s.label}</span>
        {#if current === s.id}<Icon name="check" size={14} />{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .status-menu { position: fixed; z-index: 60; min-width: 220px; max-width: 280px; gap: 2px; font-size: 13px; outline: none; --pop-origin: bottom left; }
  .grow { flex: 1; }
  .menu-item { gap: 10px; }
  .has-sub :global(svg:last-child) { color: var(--text-3); }
  /* the dots as Avatar draws them (Avatar.svelte .pres), on the menu's own surface */
  .dot { position: relative; flex-shrink: 0; width: 10px; height: 10px; border-radius: 50%; background: var(--ok); }
  .dot.dnd { background: var(--danger); }
  .dot.dnd::after { content: ''; position: absolute; left: 18%; right: 18%; top: 50%; height: 26%; transform: translateY(-50%); border-radius: 999px; background: var(--surface-menu); }
  .dot.invisible { background: var(--presence-off); }
  /* beside the menu, past its 1px border, with a small gap the pointer crosses without closing it */
  .flyout { position: absolute; z-index: 1; left: calc(100% + 4px + 1px); width: 210px; gap: 2px; outline: none; }
</style>
