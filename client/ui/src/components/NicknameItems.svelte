<script lang="ts">
  // The nickname entries of a person's menu (#113), by the profile card's rules (lib/nicknameMenu).
  // Change / Set open the NicknameDialog; Reset / Clear rename to nothing. The menu decides how an
  // entry closes it: `pick` for one that opens the dialog, `run` for a call that can be refused.
  import { app } from '../lib/store.svelte'
  import { api } from '../lib/tauri'
  import { nickItems, nickLabel } from '../lib/nicknameMenu'
  import { openRename } from '../lib/profileCard.svelte'

  let { serverId, userId, pick, run }: {
    serverId: string | null
    userId: string
    pick: (action: () => unknown) => void
    run: (call: () => Promise<unknown>) => void
  } = $props()

  const ids = $derived(nickItems(app.state, serverId, userId))
  const hasPrivate = $derived(!!app.state.privateNicks[userId])
</script>

{#if ids.length}<span class="menu-sep" role="separator"></span>{/if}
{#each ids as id (id)}
  <button type="button" class="menu-item" role="menuitem" data-item={id} onclick={() => {
    if (id === 'nick' && serverId) pick(() => openRename({ kind: 'server', userId, serverId }))
    else if (id === 'nick-reset' && serverId) run(() => api.setNickname(serverId, userId, null))
    else if (id === 'private-nick') pick(() => openRename({ kind: 'private', userId }))
    else if (id === 'private-nick-clear') run(() => api.setPrivateNickname(userId, null))
  }}>{nickLabel(id, hasPrivate)}</button>
{/each}
