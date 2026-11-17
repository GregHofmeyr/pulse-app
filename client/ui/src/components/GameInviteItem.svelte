<script lang="ts">
  // "Invite to game…" (#436): an item in every person's right-click menu while you're in a game (your own "Playing").
  // Sends an invite card into your DM with them (made if needed) and goes there. Never for yourself.
  import { app } from '../lib/store.svelte'
  import { inGame, invitePerson } from '../lib/gameInvites.svelte'

  let { userId, pick }: {
    userId: string
    /** How the menu runs an item and closes. */
    pick: (action: () => unknown) => void
  } = $props()

  const show = $derived(userId !== app.state.me?.id && inGame(app.state))
</script>

{#if show}
  <button type="button" class="menu-item" role="menuitem" data-item="invite-game" onclick={() => pick(() => invitePerson(userId))}>Invite to game…</button>
{/if}
