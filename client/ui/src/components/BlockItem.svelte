<script lang="ts">
  // "Block…" / "Unblock" (#474): an ordinary item in every person's right-click menu (member list, message author,
  // voice row, DM list, profile card). Block… asks first (the dialog's choice of how they show); Unblock acts at once.
  // Never for yourself. They're never told.
  import { app, ui } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { isBlocked } from '../lib/blocks'

  let { userId, pick }: {
    userId: string
    /** How the menu runs an item and closes. */
    pick: (action: () => unknown) => void
  } = $props()

  const self = $derived(userId === app.state.me?.id)
  const blocked = $derived(isBlocked(app.state, userId))
  const unblock = () => api.setBlock(userId, null).catch((e: unknown) => (ui.notice = errorText(e)))
</script>

{#if !self}
  {#if blocked}
    <button type="button" class="menu-item" role="menuitem" data-item="unblock" onclick={() => pick(unblock)}>Unblock</button>
  {:else}
    <button type="button" class="menu-item danger" role="menuitem" data-item="block" onclick={() => pick(() => (ui.blockAsk = userId))}>Block…</button>
  {/if}
{/if}
