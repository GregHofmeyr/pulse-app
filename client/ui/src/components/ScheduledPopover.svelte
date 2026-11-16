<script lang="ts">
  // This chat's scheduled messages (#299), from the line above its composer (or Send later's menu): the list, styled as
  // the pins popover, just above where it opened from. Every chat's are in the inbox's Scheduled tab.
  import { untrack } from 'svelte'
  import { clickOutside } from '../lib/actions'
  import { scheduledIn } from '../lib/scheduled.svelte'
  import ScheduledList from './ScheduledList.svelte'

  let { anchor, channelId, onClose }: { anchor: HTMLElement; channelId: string; onClose: () => void } = $props()

  const items = $derived(scheduledIn(channelId))
  const pos = untrack(() => {
    const r = anchor.getBoundingClientRect()
    return `position: fixed; left: ${Math.max(6, Math.round(r.left))}px; bottom: ${Math.round(window.innerHeight - r.top + 6)}px; z-index: 50;`
  })
  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      onClose()
    }
  }
</script>

<div class="sched-pop menu-surface pop-in" role="dialog" aria-label="Scheduled messages" tabindex="-1" style={pos} use:clickOutside={onClose} onkeydown={key}>
  <h2>Scheduled</h2>
  <div class="list"><ScheduledList {items} empty="Nothing scheduled here. Use the clock beside the message box to send one later." /></div>
</div>

<style>
  .sched-pop { width: min(380px, calc(100vw - 24px)); max-height: min(60vh, 520px); padding: 12px 6px 6px; gap: 6px; color: var(--text); --pop-origin: bottom left; }
  h2 { margin: 0 8px; font-size: 15px; }
  .list { min-height: 0; overflow-y: auto; }
</style>
