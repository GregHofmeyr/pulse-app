<script lang="ts">
  // "Do Not Disturb for them" (#310): an item in every person's right-click menu (voice rows and call tiles, member
  // list, message author, DM list, profile card). Off: a flyout of how long (or "Only in DMs" until turned off). On: a
  // small bell-off line saying how long it lasts, and "Turn off Do Not Disturb for them". Their messages still
  // arrive and count as unread; only the sound and the desktop notification are held back (their calls don't ring and
  // their going live doesn't alert either, #426). Saved on your account, so
  // every device has it; they're never told. Never for yourself. The menu closes the flyout through the exported
  // functions, as with any MenuFlyout.
  import { app, clock, ui } from '../lib/store.svelte'
  import { errorText } from '../lib/tauri'
  import { MUTE_FOR, muteUntil } from '../lib/muteDurations'
  import { personDndOf, personDndUntilLabel } from '../lib/personDnd'
  import { setPersonDnd } from '../lib/personDnd.svelte'
  import Icon from './Icon.svelte'
  import MenuFlyout from './MenuFlyout.svelte'

  let { userId, pick, menuLeft, menuTop }: {
    userId: string
    /** How the menu runs an item and closes. */
    pick: (action: () => unknown) => void
    menuLeft: number
    menuTop: number
  } = $props()

  const self = $derived(userId === app.state.me?.id)
  const on = $derived(personDndOf(app.state.personDnd, userId, clock.nowIso))
  const CHOICES = [...MUTE_FOR.map((m) => ({ label: m.label })), { label: 'Only in DMs, until I turn it back on' }]
  const report = (f: () => Promise<unknown>) => () => f().catch((e: unknown) => (ui.notice = errorText(e)))
  function chosen(i: number) {
    const dmsOnly = i >= MUTE_FOR.length
    const until = dmsOnly ? null : muteUntil(MUTE_FOR[i].minutes)
    pick(report(() => setPersonDnd(userId, { dms_only: dmsOnly, until })))
  }

  let inner = $state<ReturnType<typeof MenuFlyout>>()
  export const close = (refocus: boolean | 'if-inside') => inner?.close(refocus)
  export const closeSoon = () => inner?.closeSoon()
  export const isOpen = (): boolean => inner?.isOpen() ?? false
  export const trigger = (): HTMLButtonElement | undefined => inner?.trigger()
</script>

{#if !self}
  {#if on}
    <p class="dnd-on" role="none" data-person-dnd><Icon name="bellOff" size={12} />Do Not Disturb{on.dms_only ? ' in DMs' : ''} · {personDndUntilLabel(on.until, Date.parse(clock.nowIso)).toLowerCase()}</p>
    <button type="button" class="menu-item" role="menuitem" data-item="person-dnd" onclick={() => pick(report(() => setPersonDnd(userId, null)))}>Turn off Do Not Disturb for them</button>
  {:else}
    <MenuFlyout bind:this={inner} label="Do Not Disturb for them" flyoutLabel="Do Not Disturb for this person" {menuLeft} {menuTop} item="person-dnd"
      choices={CHOICES} onPick={chosen} />
  {/if}
{/if}

<style>
  /* the indicator: a quiet line above the item, not an item itself (arrow keys skip it) */
  .dnd-on { margin: 0; padding: 4px 10px 0; display: flex; align-items: center; gap: 6px; color: var(--text-3); font-size: 12px; }
</style>
