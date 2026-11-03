<script lang="ts">
  // The nickname entry of a person's menu (#113, #137; lib/nicknameMenu): Change server nickname in a server,
  // Change nickname (your private one) outside servers. It opens the NicknameDialog, which holds Reset / Clear.
  import { app } from '../lib/store.svelte'
  import { nickItems, nickLabel } from '../lib/nicknameMenu'
  import { openRename } from '../lib/profileCard.svelte'

  let { serverId, userId, pick }: {
    /** The server the menu opens in, or null outside servers. */
    serverId: string | null
    userId: string
    /** How the menu closes for an entry that opens a dialog. */
    pick: (action: () => unknown) => void
  } = $props()

  const ids = $derived(nickItems(app.state, serverId, userId))
</script>

{#if ids.length}<span class="menu-sep" role="separator"></span>{/if}
{#each ids as id (id)}
  <button type="button" class="menu-item" role="menuitem" data-item={id} onclick={() => {
    if (id === 'nick' && serverId) pick(() => openRename({ kind: 'server', userId, serverId }))
    else if (id === 'private-nick') pick(() => openRename({ kind: 'private', userId }))
  }}>{nickLabel(id)}</button>
{/each}
