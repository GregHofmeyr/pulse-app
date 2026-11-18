<script lang="ts">
  // "Invite to server ›" (#80): an item in every person's right-click menu (member list, messages, DMs, voice), a
  // flyout of the servers you're in, each with its chip; the ones they're already in greyed out. Picking one sends them
  // a direct invite (a card in your DM with them) and says "Invite sent". Never for yourself, nor with no server to
  // invite them to. The menu closes the flyout through the exported functions, as with any MenuFlyout.
  import { app } from '../lib/store.svelte'
  import { isMember } from '../lib/selectors'
  import { invitePersonTo } from '../lib/inviteActions.svelte'
  import MenuFlyout from './MenuFlyout.svelte'
  import ServerChip from './ServerChip.svelte'

  let { userId, pick, menuLeft, menuTop }: {
    userId: string
    /** How the menu runs an item and closes. */
    pick: (action: () => unknown) => void
    menuLeft: number
    menuTop: number
  } = $props()

  const self = $derived(userId === app.state.me?.id)
  const servers = $derived(app.state.servers.filter((s) => isMember(app.state, s.id)))
  const choices = $derived(
    servers.map((s) => {
      const theyreIn = (app.state.members[s.id] ?? []).some((m) => m.user.id === userId)
      return { label: s.name, disabled: theyreIn, tip: theyreIn ? 'Already in it' : undefined }
    }),
  )

  let inner = $state<ReturnType<typeof MenuFlyout>>()
  export const close = (refocus: boolean | 'if-inside') => inner?.close(refocus)
  export const closeSoon = () => inner?.closeSoon()
  export const isOpen = (): boolean => inner?.isOpen() ?? false
  export const trigger = (): HTMLButtonElement | undefined => inner?.trigger()
</script>

{#snippet chip(i: number)}<ServerChip id={servers[i].id} name={servers[i].name} iconHash={servers[i].icon_hash} size={18} />{/snippet}

{#if !self && servers.length}
  <MenuFlyout bind:this={inner} label="Invite to server" flyoutLabel="Invite to a server" {menuLeft} {menuTop} item="invite-server"
    {choices} lead={chip} onPick={(i) => { const id = servers[i].id; pick(() => invitePersonTo(id, userId)) }} />
{/if}
