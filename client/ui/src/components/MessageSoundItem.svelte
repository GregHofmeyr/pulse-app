<script lang="ts">
  // "Message sound" (#312): an item in every person's right-click menu (voice rows and call tiles, member list, message
  // author, DM list, profile card) with the sounds in a flyout beside it: Default (the set picked in Settings) or any
  // set's message sound, the current one ticked. On this device only. Picking one plays it once. Never for yourself.
  // The menu closes the flyout through the exported functions, as with any MenuFlyout.
  import { app } from '../lib/store.svelte'
  import { loadSoundPrefs } from '../lib/voiceui'
  import { messageSoundChoices } from '../lib/messageSounds'
  import { messageSounds, setMessageSound } from '../lib/messageSounds.svelte'
  import MenuFlyout from './MenuFlyout.svelte'

  let { userId, pick, menuLeft, menuTop }: {
    userId: string
    /** How the menu runs an item and closes. */
    pick: (action: () => unknown) => void
    menuLeft: number
    menuTop: number
  } = $props()

  const self = $derived(userId === app.state.me?.id)
  const choices = $derived(messageSoundChoices(messageSounds.people[userId] ?? null, loadSoundPrefs().set))

  let inner = $state<ReturnType<typeof MenuFlyout>>()
  export const close = (refocus: boolean | 'if-inside') => inner?.close(refocus)
  export const closeSoon = () => inner?.closeSoon()
  export const isOpen = (): boolean => inner?.isOpen() ?? false
  export const trigger = (): HTMLButtonElement | undefined => inner?.trigger()
</script>

{#if !self}
  <MenuFlyout bind:this={inner} label="Message sound" flyoutLabel="Message sound for this person" {menuLeft} {menuTop} item="message-sound" {choices}
    onPick={(i) => pick(() => setMessageSound(userId, choices[i].set))} />
{/if}
