<script lang="ts">
  // Server mute / deafen / Disconnect (#140) as one item of a person's right-click menu: the voice participant menu,
  // and (#144) the member list, message author and profile card menus, so a mute can be undone once they've left
  // voice. One component, so every surface says and does exactly the same. A held mute or deafen says who did it.
  // A refusal (say they just left voice) is a toast: the call box's line isn't on screen from the sidebar (#147).
  import { app, ui } from '../lib/store.svelte'
  import { serverVoiceOf, serverVoiceTip } from '../lib/selectors'
  import { api, errorText } from '../lib/tauri'

  let { id, serverId, userId, pick }: {
    id: 'server-mute' | 'server-deafen' | 'disconnect'
    serverId: string
    userId: string
    /** How the menu runs an item and closes. */
    pick: (action: () => unknown) => void
  } = $props()

  const held = $derived(serverVoiceOf(app.state, serverId, userId))
  const refused = (e: unknown) => (ui.notice = errorText(e))
  const setVoice = (change: { mute?: boolean; deafen?: boolean }) => () => api.setServerVoice(serverId, userId, change).catch(refused)
  const disconnect = () => api.disconnectVoice(serverId, userId).catch(refused)
</script>

{#if id === 'server-mute'}
  <button type="button" class="menu-item" class:danger={!held.serverMutedBy} role="menuitem" data-item="server-mute"
    data-tip={serverVoiceTip(app.state, serverId, { serverMutedBy: held.serverMutedBy })}
    onclick={() => pick(setVoice({ mute: !held.serverMutedBy }))}>{held.serverMutedBy ? 'Server unmute' : 'Server mute'}</button>
{:else if id === 'server-deafen'}
  <button type="button" class="menu-item" class:danger={!held.serverDeafenedBy} role="menuitem" data-item="server-deafen"
    data-tip={serverVoiceTip(app.state, serverId, { serverDeafenedBy: held.serverDeafenedBy })}
    onclick={() => pick(setVoice({ deafen: !held.serverDeafenedBy }))}>{held.serverDeafenedBy ? 'Server undeafen' : 'Server deafen'}</button>
{:else}
  <button type="button" class="menu-item danger" role="menuitem" data-item="disconnect" onclick={() => pick(disconnect)}>Disconnect</button>
{/if}
