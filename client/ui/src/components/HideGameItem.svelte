<script lang="ts">
  // "Hide what I'm playing" / "Show what I'm playing" (#436): an ordinary item in every person's right-click menu
  // (member list, message author, voice row and tile, DM list, profile card). Hidden, they never get your game (the
  // server leaves it out for them) and see you online with no game; they're never told. Never for yourself. The
  // answer (and ActivityHiddenChanged on your other devices) flips the label.
  import { app, ui } from '../lib/store.svelte'
  import { errorText } from '../lib/tauri'
  import { hidesGameFrom, setActivityHidden } from '../lib/gameInvites.svelte'

  let { userId, pick }: {
    userId: string
    /** How the menu runs an item and closes. */
    pick: (action: () => unknown) => void
  } = $props()

  const self = $derived(userId === app.state.me?.id)
  const hidden = $derived(hidesGameFrom(app.state, userId))
  const toggle = () => setActivityHidden(userId, !hidden).catch((e: unknown) => (ui.notice = errorText(e)))
</script>

{#if !self}
  <button type="button" class="menu-item" role="menuitem" data-item="hide-game" onclick={() => pick(toggle)}>
    {hidden ? "Show what I'm playing" : "Hide what I'm playing"}
  </button>
{/if}
