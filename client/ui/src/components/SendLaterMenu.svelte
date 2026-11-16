<script lang="ts">
  // Send later (#299): the composer's clock button opens this above it. Quick picks (in an hour, tonight at 20:00,
  // tomorrow at 09:00), "Pick a date and time…" (the custom picker in its place), and this chat's Scheduled list when
  // it has any. Text only: with files queued (or nothing written) it says so instead of offering times.
  import { onMount, tick, untrack } from 'svelte'
  import { clickOutside } from '../lib/actions'
  import { sendLaterPicks } from '../lib/schedule'
  import { errorText } from '../lib/tauri'
  import WhenPicker from './WhenPicker.svelte'

  let { anchor, blocked = null, count = 0, onPick, onShowScheduled, onClose }: {
    /** The clock button (`data-menu-toggle`, so a press on it toggles instead). */
    anchor: HTMLElement
    /** Why nothing can be scheduled right now (shown instead of the picks). */
    blocked?: string | null
    /** How many messages are scheduled in this chat. */
    count?: number
    /** Schedule what's in the box for `at` (an instant). */
    onPick: (at: string) => Promise<unknown>
    onShowScheduled: () => void
    onClose: () => void
  } = $props()

  const picks = untrack(() => sendLaterPicks(Date.now()))
  let custom = $state(false)
  let busy = $state(false)
  let error = $state('')
  let el: HTMLElement | undefined = $state()
  // just above the button, its right edge on the composer's left side
  const pos = untrack(() => {
    const r = anchor.getBoundingClientRect()
    return `position: fixed; left: ${Math.max(6, Math.round(r.left))}px; bottom: ${Math.round(window.innerHeight - r.top + 6)}px; z-index: 50;`
  })
  onMount(() => void tick().then(() => el?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus()))

  async function pick(at: string) {
    if (busy) return
    busy = true
    error = ''
    try {
      await onPick(at)
      onClose()
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      onClose()
      anchor.focus()
    }
  }
</script>

{#if custom}
  <WhenPicker {anchor} title="Send later" confirm="Schedule" onPick={onPick} onClose={onClose} />
{:else}
  <div bind:this={el} class="later menu-surface pop-in" role="menu" aria-label="Send later" tabindex="-1" style={pos} use:clickOutside={onClose} onkeydown={key}>
    <span class="menu-label">Send later</span>
    {#if blocked}
      <p class="note">{blocked}</p>
    {:else}
      {#each picks as p (p.label)}
        <button type="button" class="menu-item" role="menuitem" disabled={busy} onclick={() => void pick(p.at)}>{p.label}</button>
      {/each}
      <button type="button" class="menu-item" role="menuitem" disabled={busy} onclick={() => (custom = true)}>Pick a date and time…</button>
    {/if}
    {#if count > 0}
      <div class="sep" role="separator"></div>
      <button type="button" class="menu-item" role="menuitem" onclick={() => { onClose(); onShowScheduled() }}>Scheduled in this chat ({count})</button>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  </div>
{/if}

<style>
  .later { width: max-content; min-width: 220px; max-width: 300px; gap: 2px; --pop-origin: bottom left; }
  .menu-label { padding: 4px 8px; }
  .note { margin: 4px 8px 6px; color: var(--text-3); font-size: 13px; }
  .sep { height: 1px; margin: 4px 6px; background: var(--line); }
  .error { margin: 4px 8px; color: var(--danger-text); font-size: 12px; }
</style>
