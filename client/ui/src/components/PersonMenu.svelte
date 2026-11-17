<script lang="ts">
  // A person's menu wherever one opens on a name, an avatar or a row (#349): the inbox, search results, quotes, a
  // group's members and messages all draw it here, from `personMenuAt`. A new opening draws a fresh menu. Its Esc goes
  // no further than the menu, so a popover or panel around it (the inbox, search) stays open.
  import type { PersonMenuAt } from '../lib/participantMenu'
  import MemberMenu from './MemberMenu.svelte'

  let { menu, onClose, onMessage, onRemove, onAllowBack, muteChannelId = null }: {
    menu: PersonMenuAt | null
    onClose: () => void
    onMessage?: (userId: string) => void
    /** A group's member list (#335): Remove from group. */
    onRemove?: (userId: string) => void
    /** Someone blocked from rejoining a server you own (#418, Server settings › Members): Allow back. */
    onAllowBack?: (userId: string) => void
    /** A message in your DM with them: Mute conversation mutes that DM. */
    muteChannelId?: string | null
  } = $props()
</script>

{#if menu}
  {#key menu}
    <div class="person-menu" role="presentation" onkeydown={(e) => { if (e.key === 'Escape') e.stopPropagation() }}>
      <MemberMenu serverId={menu.serverId} userId={menu.userId} x={menu.x} y={menu.y} opener={menu.opener} {onMessage} {onRemove} {onAllowBack} {muteChannelId} {onClose} />
    </div>
  {/key}
{/if}

<style>
  .person-menu { display: contents; }
</style>
