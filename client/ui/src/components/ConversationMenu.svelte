<script lang="ts">
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { conversationName } from '../lib/conversations'
  import type { Channel } from '../lib/protocol/Channel'

  let { channel, muted, onClose, onRename, onLeft }: {
    channel: Channel
    muted: boolean
    onClose: () => void
    onRename: () => void
    /** Called after leaving or closing the conversation (the view should move away). */
    onLeft: () => void
  } = $props()

  let confirmLeave = $state(false)
  let error = $state('')
  const group = $derived(channel.kind === 'group')
  const isPrivate = $derived(channel.server_id === null)

  async function run(f: () => Promise<unknown>, after?: () => void) {
    error = ''
    try {
      await f()
      onClose()
      after?.()
    } catch (e) {
      error = errorText(e)
    }
  }
  const muteFor = (hours: number | null) =>
    run(() => api.setMute('channel', channel.id, hours === null ? null : new Date(Date.now() + hours * 3_600_000).toISOString()))
</script>

<div class="menu" role="menu">
  {#if confirmLeave}
    <p class="q">Leave {conversationName(app.state, channel.id)}? You'll lose access to its history.</p>
    <div class="row">
      <button class="danger" role="menuitem" onclick={() => run(() => api.removeMember(channel.id, app.state.me!.id), onLeft)}>Leave</button>
      <button role="menuitem" onclick={() => (confirmLeave = false)}>Cancel</button>
    </div>
  {:else}
    {#if muted}
      <button role="menuitem" onclick={() => run(() => api.clearMute('channel', channel.id))}>Unmute</button>
    {:else}
      <span class="lbl">Mute</span>
      <button role="menuitem" onclick={() => muteFor(1)}>For 1 hour</button>
      <button role="menuitem" onclick={() => muteFor(8)}>For 8 hours</button>
      <button role="menuitem" onclick={() => muteFor(null)}>Until I turn it back on</button>
    {/if}
    {#if group}
      <span class="sep"></span>
      <button role="menuitem" onclick={() => { onClose(); onRename() }}>Rename group</button>
    {/if}
    {#if isPrivate}
      <button role="menuitem" onclick={() => run(() => api.closeConversation(channel.id), onLeft)}>Close conversation</button>
    {/if}
    {#if group}
      <span class="sep"></span>
      <button role="menuitem" class="danger" onclick={() => (confirmLeave = true)}>Leave group</button>
    {/if}
  {/if}
  {#if error}<p class="err" role="alert">{error}</p>{/if}
</div>

<style>
  .menu { position: absolute; top: 46px; right: 12px; z-index: 10; width: 230px; padding: 6px; display: flex; flex-direction: column; background: var(--bg-2); border: 1px solid var(--bg-3); border-radius: 12px; box-shadow: 0 10px 30px rgba(0, 0, 0, .4); }
  button { text-align: left; padding: 8px 10px; border: 0; border-radius: 8px; background: transparent; color: var(--text-2); font: inherit; font-size: 13px; }
  button:hover { background: var(--bg-3); color: var(--text); }
  .danger { color: #f2616b; }
  .lbl { padding: 6px 10px 2px; font-size: 11px; font-weight: 600; letter-spacing: .06em; color: var(--text-3); text-transform: uppercase; }
  .sep { height: 1px; margin: 4px 6px; background: var(--bg-3); }
  .q { margin: 6px 8px; font-size: 13px; color: var(--text-2); }
  .row { display: flex; gap: 4px; }
  .err { margin: 6px 8px 2px; font-size: 12px; color: #f2616b; }
</style>
