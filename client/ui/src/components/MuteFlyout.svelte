<script lang="ts">
  // "Mute …" and its durations flyout (Discord's, #131): MenuFlyout with the mute durations, in the channel menu, the
  // DM list's (#190) and the server tab's (#161). The menu closes it through the same functions as any MenuFlyout.
  import { MUTE_FOR } from '../lib/muteDurations'
  import MenuFlyout from './MenuFlyout.svelte'

  let { label, flyoutLabel, menuLeft, menuTop, onPick, item = '' }: {
    /** The item's text, e.g. "Mute channel". */
    label: string
    /** The flyout menu's accessible name, e.g. "Mute #general for". */
    flyoutLabel: string
    menuLeft: number
    menuTop: number
    /** A duration was picked (minutes; null: until turned back on). */
    onPick: (minutes: number | null) => void
    /** The item's `data-item` value, for menus that name their items (the person menu's "mute"). */
    item?: string
  } = $props()

  let inner = $state<ReturnType<typeof MenuFlyout>>()
  export const close = (refocus: boolean | 'if-inside') => inner?.close(refocus)
  export const closeSoon = () => inner?.closeSoon()
  export const isOpen = (): boolean => inner?.isOpen() ?? false
  export const trigger = (): HTMLButtonElement | undefined => inner?.trigger()
</script>

<MenuFlyout bind:this={inner} {label} {flyoutLabel} {menuLeft} {menuTop} {item} choices={MUTE_FOR} onPick={(i) => onPick(MUTE_FOR[i].minutes)} />
