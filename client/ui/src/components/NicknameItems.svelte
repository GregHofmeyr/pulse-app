<script lang="ts">
  // The nickname entries of a person's menu (#113, #137; lib/nicknameMenu): in a server Change server nickname and
  // Set / Change private nickname, outside servers Change nickname (your private one). Each opens the NicknameDialog.
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
  }}>{nickLabel(app.state, serverId, userId, id)}</button>
{/each}
