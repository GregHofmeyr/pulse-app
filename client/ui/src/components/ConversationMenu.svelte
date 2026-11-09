<script lang="ts">
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { channelLevel, conversationName } from '../lib/conversations'
  import { CHANNEL_LEVELS } from '../lib/notifyLevels'
  import { clickOutside } from '../lib/actions'
  import type { Channel } from '../lib/protocol/Channel'
  import { muteUntil } from '../lib/muteDurations'
  import { openRename } from '../lib/profileCard.svelte'
  import { onMount } from 'svelte'
  import AppearOfflineItem from './AppearOfflineItem.svelte'
  import LiveNotifyItem from './LiveNotifyItem.svelte'
  import MenuFlyout from './MenuFlyout.svelte'
  import MuteFlyout from './MuteFlyout.svelte'
  import { pickGroupIcon, removeGroupIcon } from '../lib/groupIcon.svelte'

  let { channel, muted, show, at = null, anchor = null, onClose, onRename, onLeft }: {
    channel: Channel
    muted: boolean
    /** 'mute' = the bell's popover, 'manage' = the ⋯ menu, 'all' = right-click on a conversation. */
    show: 'mute' | 'manage' | 'all'
    /** Pointer position for a context menu; omitted = anchored under the channel header. */
    at?: { x: number; y: number } | null
    /** The header button it opened from (#162): the menu opens under it, its right edge on the button's. */
    anchor?: HTMLElement | null
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
  // The bell's popover (#203, #236): the right-click menu's items, Mute … as the durations flyout (Unmute … while
  // muted) and, on a server channel, Notifications as the channel menu's levels flyout (#195) with yours ticked,
  // muted or not. DMs and groups have no levels.
  const withLevels = $derived(withMute && !at && channel.server_id !== null)
  const level = $derived(channelLevel(app.state, channel.id))
  const levelChoices = $derived(CHANNEL_LEVELS.map((l) => ({ label: l.label, checked: l.level === level })))
  const pickLevel = (i: number) => run(() => api.setNotifyLevel('channel', channel.id, CHANNEL_LEVELS[i].level))
  // what the bell's items call it: as the channel menu does on a server, else as the conversation menu does
  const noun = $derived(channel.server_id !== null ? 'channel' : 'conversation')
  const flyoutName = $derived(channel.server_id === null ? conversationName(app.state, channel.id) : channel.kind === 'voice' ? channel.name : `#${channel.name}`)
  // the mute flyout (#190; the channel menu's, #131) and the levels one
  let flyout = $state<ReturnType<typeof MuteFlyout>>()
  let levelFlyout = $state<ReturnType<typeof MenuFlyout>>()
  const flyouts = () => [flyout, levelFlyout]
  // a flyout opened by hover: the first Escape closes just it (and not, from the window, the menu)
  function keydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && flyouts().some((f) => f?.isOpen())) {
      e.preventDefault()
      e.stopPropagation()
      for (const f of flyouts()) f?.close(false)
    }
  }

  // By its button: under it, lined up on its right edge, kept 8px inside the window; above it when there's no room
  // below. The layout size (offsetWidth): the .pop-in entrance scales the box while it runs.
  const EDGE = 8
  const GAP = 4
  let el = $state<HTMLElement>()
  let pos = $state<{ left: number; top: number } | null>(null)
  onMount(() => {
    if (!anchor || !el) return
    const r = anchor.getBoundingClientRect()
    const w = el.offsetWidth
    const h = el.offsetHeight
    const left = Math.max(EDGE, Math.min(r.right - w, window.innerWidth - w - EDGE))
    const below = r.bottom + GAP
    const top = below + h > window.innerHeight - EDGE ? Math.max(EDGE, r.top - GAP - h) : below
    pos = { left, top }
  })
  const place = $derived(at ? { left: at.x, top: at.y } : pos)
</script>

<div bind:this={el} class="menu menu-surface pop-in" class:ctx={!!at} class:anchored={!at && !!anchor} role="menu" tabindex="-1" use:clickOutside={onClose}
  style:left={place ? `${place.left}px` : null} style:top={place ? `${place.top}px` : null}
  onpointerover={(e) => {
    // another of the menu's own items under the pointer closes the flyout (after the hover-intent wait)
    const t = (e.target as Element).closest?.('.menu-item')
    if (t && t.parentElement === el) for (const f of flyouts()) if (t !== f?.trigger()) f?.closeSoon()
  }} onkeydown={keydown}>
  {#if confirmLeave}
    <p class="q">Leave {conversationName(app.state, channel.id)}? You'll lose access to its history.</p>
    <div class="row">
      <button class="menu-item danger" role="menuitem" onclick={() => run(() => api.removeMember(channel.id, app.state.me!.id), onLeft)}>Leave</button>
      <button class="menu-item" role="menuitem" onclick={() => (confirmLeave = false)}>Cancel</button>
    </div>
  {:else}
    {#if withMute && at}
      <!-- right-click (#190): one item with the durations in a flyout beside it, as the channel and server menus -->
      {#if muted}
        <button class="menu-item" role="menuitem" data-item onclick={() => run(() => api.clearMute('channel', channel.id))}>Unmute conversation</button>
      {:else}
        <MuteFlyout bind:this={flyout} label="Mute conversation" flyoutLabel="Mute {conversationName(app.state, channel.id)} for" menuLeft={at.x} menuTop={at.y}
          onPick={muteFor} />
      {/if}
    {:else if withMute}
      <!-- the bell (#203, #236): the same items and flyouts as the right-click menus -->
      {#if muted}
        <button class="menu-item" role="menuitem" onclick={() => run(() => api.clearMute('channel', channel.id))}>Unmute {noun}</button>
      {:else}
        <MuteFlyout bind:this={flyout} label="Mute {noun}" flyoutLabel="Mute {flyoutName} for" menuLeft={place?.left ?? 0} menuTop={place?.top ?? 0} onPick={muteFor} />
      {/if}
      {#if withLevels}
        <MenuFlyout bind:this={levelFlyout} label="Notifications" flyoutLabel="Notifications for {flyoutName}" menuLeft={place?.left ?? 0} menuTop={place?.top ?? 0}
          choices={levelChoices} onPick={pickLevel} />
      {/if}
    {/if}
    {#if withManage}
      {#if withMute && isPrivate}<span class="menu-sep"></span>{/if}
      {#if group && onRename}
        <button class="menu-item" role="menuitem" onclick={() => { onClose(); onRename() }}>Rename group</button>
      {/if}
      {#if group}
        <!-- any member may (#56): the avatar upload and crop, then every member sees it -->
        <button class="menu-item" role="menuitem" onclick={() => { pickGroupIcon(channel.id); onClose() }}>Change icon</button>
        {#if channel.icon_hash}
          <button class="menu-item" role="menuitem" onclick={() => { void removeGroupIcon(channel.id); onClose() }}>Remove icon</button>
        {/if}
      {/if}
      {#if dmWith}
        <button class="menu-item" role="menuitem" onclick={() => { openRename({ kind: 'private', userId: dmWith }); onClose() }}>Change nickname</button>
        <LiveNotifyItem userId={dmWith} pick={(action) => { void action(); onClose() }} />
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
  .menu { position: absolute; top: 46px; right: 12px; z-index: 10; width: 230px; outline: none; }
  .menu.ctx, .menu.anchored { position: fixed; right: auto; z-index: 45; }
  /* the pop-in grows from the button's corner, or from the cursor (#173) */
  .menu.anchored { --pop-origin: top right; }
  .menu.ctx { --pop-origin: top left; }
  .q { margin: 6px 8px; font-size: 13px; color: var(--text-2); }
  .row { display: flex; gap: 4px; }
  .err { margin: 6px 8px 2px; font-size: 12px; color: var(--danger-text); }
</style>
