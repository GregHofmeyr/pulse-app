<script lang="ts">
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { conversationName } from '../lib/conversations'
  import { clickOutside } from '../lib/actions'
  import type { Channel } from '../lib/protocol/Channel'
  import { MUTE_FOR, muteUntil } from '../lib/muteDurations'
  import { openRename } from '../lib/profileCard.svelte'
  import AppearOfflineItem from './AppearOfflineItem.svelte'

  let { channel, muted, show, at = null, onClose, onRename, onLeft }: {
    channel: Channel
    muted: boolean
    /** 'mute' = the bell's popover, 'manage' = the ⋯ menu, 'all' = right-click on a conversation. */
    show: 'mute' | 'manage' | 'all'
    /** Pointer position for a context menu; omitted = anchored under the channel header. */
    at?: { x: number; y: number } | null
    onClose: () => void
    onRename?: () => void
    /** Called after leaving or closing the conversation (the view should move away). */
    onLeft: () => void
  } = $props()

  let confirmLeave = $state(false)
  let error = $state('')
  const group = $derived(channel.kind === 'group')
  const isPrivate = $derived(channel.server_id === null)
  const withMute = $derived(show !== 'manage')
  const withManage = $derived(show !== 'mute')
  // Right-click a DM (#137): your private nickname for the other person (never a server one here), and appearing
  // offline to them (#45).
  const dmWith = $derived(show === 'all' && channel.kind === 'dm' ? ((app.state.dmMembers[channel.id] ?? []).find((u) => u !== app.state.me?.id) ?? null) : null)

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
  const muteFor = (minutes: number | null) => run(() => api.setMute('channel', channel.id, muteUntil(minutes)))
</script>

<div class="menu menu-surface pop-in" class:ctx={!!at} role="menu" use:clickOutside={onClose}
  style:left={at ? `${at.x}px` : null} style:top={at ? `${at.y}px` : null}>
  {#if confirmLeave}
    <p class="q">Leave {conversationName(app.state, channel.id)}? You'll lose access to its history.</p>
    <div class="row">
      <button class="menu-item danger" role="menuitem" onclick={() => run(() => api.removeMember(channel.id, app.state.me!.id), onLeft)}>Leave</button>
      <button class="menu-item" role="menuitem" onclick={() => (confirmLeave = false)}>Cancel</button>
    </div>
  {:else}
    {#if withMute}
      {#if muted}
        <button class="menu-item" role="menuitem" onclick={() => run(() => api.clearMute('channel', channel.id))}>Unmute</button>
      {:else}
        <span class="menu-label">Mute</span>
        {#each MUTE_FOR as d (d.label)}
          <button class="menu-item" role="menuitem" onclick={() => muteFor(d.minutes)}>{d.label}</button>
        {/each}
      {/if}
    {/if}
    {#if withManage}
      {#if withMute && isPrivate}<span class="menu-sep"></span>{/if}
      {#if group && onRename}
        <button class="menu-item" role="menuitem" onclick={() => { onClose(); onRename() }}>Rename group</button>
      {/if}
      {#if dmWith}
        <button class="menu-item" role="menuitem" onclick={() => { openRename({ kind: 'private', userId: dmWith }); onClose() }}>Change nickname</button>
        <AppearOfflineItem userId={dmWith} pick={(action) => { void action(); onClose() }} />
      {/if}
      {#if isPrivate}
        <button class="menu-item" role="menuitem" onclick={() => run(() => api.closeConversation(channel.id), onLeft)}>Close conversation</button>
      {/if}
      {#if group}
        <span class="menu-sep"></span>
        <button class="menu-item danger" role="menuitem" onclick={() => (confirmLeave = true)}>Leave group</button>
      {/if}
    {/if}
  {/if}
  {#if error}<p class="err" role="alert">{error}</p>{/if}
</div>

<style>
  .menu { position: absolute; top: 46px; right: 12px; z-index: 10; width: 230px; }
  .menu.ctx { position: fixed; right: auto; z-index: 45; }
  .q { margin: 6px 8px; font-size: 13px; color: var(--text-2); }
  .row { display: flex; gap: 4px; }
  .err { margin: 6px 8px 2px; font-size: 12px; color: var(--danger-text); }
</style>
