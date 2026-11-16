<script lang="ts">
  // "Join game" (#381): an item in every person's right-click menu while their Steam lobby or game server is public
  // and they share Join. Steam opens on it (the core builds the steam:// URL from checked parts).
  import { app, ui } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { joinOf } from '../lib/steamJoin'

  let { userId, pick }: {
    userId: string
    /** How the menu runs an item and closes. */
    pick: (action: () => unknown) => void
  } = $props()

  const join = $derived(joinOf(app.state, userId))
</script>

{#if join}
  <button type="button" class="menu-item" role="menuitem" data-item="join-game"
    onclick={() => pick(() => api.joinSteamGame(join).catch((e: unknown) => (ui.notice = errorText(e))))}>Join game</button>
{/if}
