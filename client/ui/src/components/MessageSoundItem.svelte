<script lang="ts">
  // "Message sound" (#312): an item in every person's right-click menu (voice rows and call tiles, member list, message
  // author, DM list, profile card) with the sounds in a flyout beside it: Default (the set picked in Settings) or any
  // set's message sound, the current one ticked. On this device only. Picking one plays it once. Never for yourself.
  // Given a `channelId` instead (#378), it's the channel's own (a server channel's or a group's menu); a person's own
  // sound still wins over it. The menu closes the flyout through the exported functions, as with any MenuFlyout.
  import { app } from '../lib/store.svelte'
  import { loadSoundPrefs } from '../lib/voiceui'
  import { messageSoundChoices } from '../lib/messageSounds'
  import { messageSounds, setChannelMessageSound, setMessageSound } from '../lib/messageSounds.svelte'
  import MenuFlyout from './MenuFlyout.svelte'

  let { userId, channelId, pick, menuLeft, menuTop }: {
    userId?: string
    /** The channel's own sound instead of a person's. */
    channelId?: string
    /** How the menu runs an item and closes. */
    pick: (action: () => unknown) => void
    menuLeft: number
    menuTop: number
  } = $props()

  const self = $derived(!channelId && userId === app.state.me?.id)
  const current = $derived(channelId ? messageSounds.channels[channelId] : userId ? messageSounds.people[userId] : undefined)
  const choices = $derived(messageSoundChoices(current ?? null, loadSoundPrefs().set))
  const choose = (i: number) => {
    const set = choices[i].set
    if (channelId) setChannelMessageSound(channelId, set)
    else if (userId) setMessageSound(userId, set)
  }

  let inner = $state<ReturnType<typeof MenuFlyout>>()
  export const close = (refocus: boolean | 'if-inside') => inner?.close(refocus)
  export const closeSoon = () => inner?.closeSoon()
  export const isOpen = (): boolean => inner?.isOpen() ?? false
  export const trigger = (): HTMLButtonElement | undefined => inner?.trigger()
</script>

{#if !self && (channelId || userId)}
  <MenuFlyout bind:this={inner} label="Message sound" flyoutLabel={channelId ? 'Message sound for this channel' : 'Message sound for this person'} {menuLeft} {menuTop}
    item={channelId ? 'channel-message-sound' : 'message-sound'} {choices} onPick={(i) => pick(() => choose(i))} />
{/if}
