<script lang="ts">
  // "Notify me when they go live" / "Stop notifying me when they go live" (#60): an ordinary item in every person's
  // right-click menu (member list, message author, voice row, DM list, profile card), like "Appear offline to them".
  // Owner: every action between people lives in the right-click menu, never a button on a card. Never for yourself.
  // The server answers with a LiveNotifyChanged, which flips the label; they're never told.
  import { app, ui } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'

  let { userId, pick }: {
    userId: string
    /** How the menu runs an item and closes. */
    pick: (action: () => unknown) => void
  } = $props()

  const self = $derived(userId === app.state.me?.id)
  const on = $derived(app.state.liveNotify.includes(userId))
  const toggle = () => api.setLiveNotify(userId, !on).catch((e: unknown) => (ui.notice = errorText(e)))
</script>

{#if !self}
  <button type="button" class="menu-item" role="menuitem" data-item="live-notify" onclick={() => pick(toggle)}>
    {on ? 'Stop notifying me when they go live' : 'Notify me when they go live'}
  </button>
{/if}
