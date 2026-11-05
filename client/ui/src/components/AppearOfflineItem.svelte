<script lang="ts">
  // "Appear offline to them" / "Stop appearing offline to them" (#45): an ordinary item in every person's right-click
  // menu (member list, message author, voice row, DM list, profile card). Owner: every action between people lives in
  // the right-click menu, never a button on a card. Never for yourself. The server answers with an
  // AppearOfflineChanged, which flips the label; they're never told.
  import { app, ui } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'

  let { userId, pick }: {
    userId: string
    /** How the menu runs an item and closes. */
    pick: (action: () => unknown) => void
  } = $props()

  const self = $derived(userId === app.state.me?.id)
  const hidden = $derived(app.state.appearOfflineTo.includes(userId))
  const toggle = () => api.setAppearOffline(userId, !hidden).catch((e: unknown) => (ui.notice = errorText(e)))
</script>

{#if !self}
  <button type="button" class="menu-item" role="menuitem" data-item="appear-offline" onclick={() => pick(toggle)}>
    {hidden ? 'Stop appearing offline to them' : 'Appear offline to them'}
  </button>
{/if}
