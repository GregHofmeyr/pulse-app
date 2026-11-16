<script lang="ts">
  // "Make watch party host" (#384): in the right-click menu on a person in the call, for the watch party's host (every
  // action between people lives in the right-click menu). The server answers with a WatchPartyChanged naming the new
  // host, whose player opens (or, with watch parties off on their device, passes it on).
  import { app, ui } from '../lib/store.svelte'
  import { errorText } from '../lib/tauri'
  import { voice } from '../lib/voice.svelte'
  import { partyIn, watchPartyAction, watchPrefs } from '../lib/watchParty.svelte'

  let { channelId, userId, pick }: {
    channelId: string
    userId: string
    /** How the menu runs an item and closes. */
    pick: (action: () => unknown) => void
  } = $props()

  const party = $derived(watchPrefs.on ? partyIn(channelId) : null)
  const me = $derived(app.state.me?.id ?? null)
  const shown = $derived(!!party && party.host_id === me && userId !== me && voice.channelId === channelId && voice.connection === 'connected')
  const handOff = () =>
    watchPartyAction(channelId, { action: 'hand_off', user_id: userId }).catch((e: unknown) => (ui.notice = errorText(e)))
</script>

{#if shown}
  <button type="button" class="menu-item" role="menuitem" data-item="watch-host" onclick={() => pick(handOff)}>Make watch party host</button>
{/if}
