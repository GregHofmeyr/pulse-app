<script lang="ts">
  // Your status (#45): opens from the status row on your own profile card (#168), above it. Online, Do Not Disturb and
  // Invisible (each with its timers in a flyout, like a channel's Mute; Invisible's since #153), each with its dot and
  // the current one ticked. The server answers with a StatusChanged to every session, which
  // ticks the new one; nothing is set here meanwhile.
  import { flushSync, onMount } from 'svelte'
  import { app, clock, ui } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { clickOutside } from '../lib/actions'
  import { closeTimer } from '../lib/hoverIntent'
  import { STATUS_FOR, statusUntil, ownStatus, type TimerChoice } from '../lib/status'
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

  // A timed status's flyout (DND or Invisible): to the right (the sidebar is on the left), level with its status. Hover
  // opens it without moving focus; ArrowRight or a click opens it with focus on its first timer; ArrowLeft or Escape
  // closes just it. One at a time.
  type Timed = 'dnd' | 'invisible'
  const isTimed = (id: Status): id is Timed => id !== 'online'
  const uid = $props.id()
  const flyoutId = (t: Timed) => `${uid}-${t}`
  const timedEls: Partial<Record<Timed, HTMLButtonElement>> = $state({})
  let sub = $state<HTMLElement>()
  let subFor = $state<Timed | null>(null)
  const subOpen = $derived(subFor !== null)
  /** The status whose flyout is open, or the one focus is on: its item. */
  const subEl = $derived(subFor ? timedEls[subFor] : undefined)
  let subTop = $state(0)
  const subItems = () => (sub ? [...sub.querySelectorAll<HTMLButtonElement>('[role=menuitem]')] : [])
  function focusSub(i: number) {
    const list = subItems()
    if (list.length) list[(i + list.length) % list.length].focus()
  }
  function openSub(t: Timed, focusFirst: boolean) {
    const at = timedEls[t]
    if (!at) return
    // its first timer beside its status (the flyout's 6px padding and 1px border)
    subTop = at.offsetTop - 7
    subFor = t
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
    if (!subFor) return
    const at = subEl
    subFor = null
    if (refocus) at?.focus()
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
  const setTimed = (t: Timed, d: TimerChoice) => set(t, statusUntil(d, new Date()))
  const FLYOUT_LABEL: Record<Timed, string> = { dnd: 'Do Not Disturb for', invisible: 'Invisible for' }

  function keydown(e: KeyboardEvent) {
    const list = items()
    const at = list.indexOf(document.activeElement as HTMLButtonElement)
    const onTimed = (['dnd', 'invisible'] as const).find((t) => timedEls[t] && document.activeElement === timedEls[t])
    if (e.key === 'ArrowRight' && onTimed) {
      e.preventDefault()
      openSub(onTimed, true)
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
    if (t && t.parentElement === el && t !== subEl && subOpen) later.schedule(() => closeSub(!!sub?.contains(document.activeElement)))
  }}>
  {#each STATUSES as s (s.id)}
    {#if isTimed(s.id)}
      {@const t = s.id}
      <button bind:this={timedEls[t]} type="button" class="menu-item has-sub" class:hi={subFor === t} role="menuitemradio" aria-checked={current === t} data-item={t}
        aria-haspopup="menu" aria-expanded={subFor === t} aria-controls={subFor === t ? flyoutId(t) : undefined}
        onpointerenter={() => { later.cancel(); openSub(t, false) }} onclick={() => openSub(t, true)}>
        <span class="dot {t}" aria-hidden="true"></span><span class="grow">{s.label}</span>
        {#if current === t}<Icon name="check" size={14} />{/if}<Icon name="chevronRight" size={14} />
      </button>
      {#if subFor === t}
        <div bind:this={sub} id={flyoutId(t)} class="flyout menu-surface" role="menu" aria-label={FLYOUT_LABEL[t]} tabindex="-1" style:top="{subTop}px" onkeydown={subKeydown} onpointerenter={later.cancel}>
          {#each STATUS_FOR as d (d.label)}
            <button type="button" class="menu-item" role="menuitem" onclick={() => setTimed(t, d)}>{d.label}</button>
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
  .status-menu { position: fixed; z-index: 60; width: max-content; min-width: 220px; max-width: 280px; gap: 2px; font-size: 13px; outline: none; --pop-origin: bottom left; }
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
