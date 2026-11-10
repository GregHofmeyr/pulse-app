<script lang="ts">
  // The right-click menu on a sidebar channel row (or Shift+F10 / the menu key on it). Text, quotes
  // and clips: mark as read, mute (a flyout of durations, #131) and notifications (its level or following its server,
  // a flyout with yours ticked, #195); voice: join or leave, then the same three for its chat (#226). Anyone in the
  // server may rename and
  // delete (v1 trust model), and open a voice channel's settings (#75); those are handed back to the list, which
  // edits in the row and asks in a dialog.
  import { onMount, untrack } from 'svelte'
  import { app, clock } from '../lib/store.svelte'
  import { placeMenu } from '../lib/menuPlace'
  import { api, errorText } from '../lib/tauri'
  import { clickOutside } from '../lib/actions'
  import { muteUntil } from '../lib/muteDurations'
  import { channelLevel } from '../lib/conversations'
  import { CHANNEL_LEVELS } from '../lib/notifyLevels'
  import MenuFlyout from './MenuFlyout.svelte'
  import { voice, voiceApi } from '../lib/voice.svelte'
  import MuteFlyout from './MuteFlyout.svelte'
  import type { Channel } from '../lib/protocol/Channel'

  let { channel, member, x, y, opener, onClose, onRename, onDelete, onJoin, onError, onSettings = () => {}, onMoveUp, onMoveDown, canMoveUp = false, canMoveDown = false }: {
    channel: Channel
    /** You're in the channel's server: you may rename, delete and join. */
    member: boolean
    x: number
    y: number
    /** The row it opened from: focus goes back to it when the menu closes from the keyboard. */
    opener: HTMLElement
    onClose: () => void
    onRename: () => void
    onDelete: () => void
    onJoin: () => void
    /** An action failed after the menu had gone: the list says why. */
    onError: (message: string) => void
    /** Channel settings… (#75): a voice channel's, for members. */
    onSettings?: () => void
    /** Move up / Move down (#54), the keyboard's way to drag: given only to members on a server with categories. */
    onMoveUp?: () => void
    onMoveDown?: () => void
    /** False at the very top / bottom of the list: that move is shown, disabled. */
    canMoveUp?: boolean
    canMoveDown?: boolean
  } = $props()

  const isVoice = $derived(channel.kind === 'voice')
  const label = $derived(isVoice ? channel.name : `#${channel.name}`)
  const unread = $derived((app.state.reads[channel.id]?.unread ?? 0) > 0)
  // This channel's own mute (a muted server is unmuted from its tab).
  const muted = $derived(
    app.state.mutes.some((m) => m.target_kind === 'channel' && m.target_id === channel.id && (m.until === null || m.until > clock.nowIso)),
  )
  const inCall = $derived(voice.channelId === channel.id || voice.joining === channel.id)

  let el = $state<HTMLElement>()
  let left = $state(untrack(() => x))
  let top = $state(untrack(() => y))
  /** where its pop-in grows from: the corner nearest the cursor (#181) */
  let origin = $state('top left')
  let closed = false

  onMount(() => {
    if (!el) return
    // the layout size: the .pop-in entrance scales the box while it runs
    ;({ left, top, origin } = placeMenu(x, y, el.offsetWidth, el.offsetHeight))
    focusItem(0)
  })

  // the main menu's own items (the flyout's sit inside it, under their own menu)
  const items = () => (el ? [...el.querySelectorAll<HTMLButtonElement>(':scope > [data-item]:not(:disabled)')] : [])
  function focusItem(i: number) {
    const list = items()
    if (list.length) list[(i + list.length) % list.length].focus()
  }

  // The mute flyout (#131; shared with the server tab's menu, #161) and the notifications one (#195)
  let flyout = $state<ReturnType<typeof MuteFlyout>>()
  let levelFlyout = $state<ReturnType<typeof MenuFlyout>>()
  const flyouts = () => [flyout, levelFlyout]
  const closeSub = (refocus: boolean | 'if-inside') => { for (const f of flyouts()) f?.close(refocus) }
  const level = $derived(channelLevel(app.state, channel.id))
  const levelChoices = $derived(CHANNEL_LEVELS.map((l) => ({ label: l.label, checked: l.level === level })))
  const pickLevel = (i: number) => pick(report(() => api.setNotifyLevel('channel', channel.id, CHANNEL_LEVELS[i].level)))

  function close(restoreFocus: boolean) {
    if (closed) return
    closed = true
    if (restoreFocus && opener.isConnected) opener.focus()
    onClose()
  }
  /** Focus goes back to the row first (a rename or dialog then takes it from there), then it acts,
   *  then the menu goes. A failure lands with `report`, after the menu has gone. */
  function pick(action: () => unknown) {
    if (closed) return
    if (opener.isConnected) opener.focus()
    void action()
    close(false)
  }
  const report = (f: () => Promise<unknown>) => () => f().catch((e) => onError(errorText(e)))

  async function markRead() {
    // the latest page; its order isn't relied on (ids sort by time)
    const page = await api.listMessages(channel.id)
    const newest = page.reduce<string | null>((max, m) => (max === null || m.id > max ? m.id : max), null)
    if (newest) await api.markRead(channel.id, newest)
  }

  function keydown(e: KeyboardEvent) {
    const list = items()
    const at = list.indexOf(document.activeElement as HTMLButtonElement)
    if (e.key === 'ArrowLeft') { e.preventDefault(); closeSub(false); return }
    // a flyout opened by hover: the first Escape closes just it (and not, from the window, the menu)
    if (e.key === 'Escape' && flyouts().some((f) => f?.isOpen())) { e.preventDefault(); e.stopPropagation(); closeSub(false); return }
    if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(e.key)) closeSub(false)
    switch (e.key) {
      case 'ArrowDown': e.preventDefault(); focusItem(at + 1); break
      case 'ArrowUp': e.preventDefault(); focusItem(at < 0 ? -1 : at - 1); break
      case 'Home': e.preventDefault(); focusItem(0); break
      case 'End': e.preventDefault(); focusItem(-1); break
      case 'Escape': e.preventDefault(); close(true); break
      case 'Tab': e.preventDefault(); close(true); break
    }
  }
</script>

<div bind:this={el} class="menu menu-surface pop-in" role="menu" aria-label="Actions for {label}" tabindex="-1" style:left="{left}px" style:top="{top}px" style:--pop-origin={origin}
  onkeydown={keydown} use:clickOutside={(e) => close(e.type === 'keydown')}
  onpointerover={(e) => {
    // another of the menu's own items under the pointer closes the flyout
    const t = (e.target as Element).closest?.('[data-item]')
    // (focus in a flyout goes back to its item rather than falling out of the menu)
    if (t && t.parentElement === el) for (const f of flyouts()) if (t !== f?.trigger()) f?.closeSoon()
  }}>
  {#if isVoice}
    {#if inCall}
      <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(report(() => voiceApi.leave()))}>Leave</button>
    {:else}
      <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(onJoin)}>Join</button>
    {/if}
    {#if member}<span class="menu-sep"></span>{/if}
  {/if}
  {#if !isVoice || member}
    <button type="button" class="menu-item" role="menuitem" data-item disabled={!unread} onclick={() => pick(report(markRead))}>Mark as read</button>
    {#if muted}
      <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(report(() => api.clearMute('channel', channel.id)))}>Unmute channel</button>
    {:else}
      <MuteFlyout bind:this={flyout} label="Mute channel" flyoutLabel="Mute {label} for" menuLeft={left} menuTop={top}
        onPick={(m) => pick(report(() => api.setMute('channel', channel.id, muteUntil(m))))} />
    {/if}
    <MenuFlyout bind:this={levelFlyout} label="Notifications" flyoutLabel="Notifications for {label}" menuLeft={left} menuTop={top}
      choices={levelChoices} onPick={pickLevel} />
  {/if}
  {#if member}
    <span class="menu-sep"></span>
    {#if isVoice}<button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(onSettings)}>Channel settings…</button>{/if}
    <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(onRename)}>Rename channel…</button>
    <button type="button" class="menu-item danger" role="menuitem" data-item onclick={() => pick(onDelete)}>Delete channel…</button>
    {#if onMoveUp && onMoveDown}
      <span class="menu-sep"></span>
      <button type="button" class="menu-item" role="menuitem" data-item disabled={!canMoveUp} onclick={() => pick(onMoveUp)}>Move up</button>
      <button type="button" class="menu-item" role="menuitem" data-item disabled={!canMoveDown} onclick={() => pick(onMoveDown)}>Move down</button>
    {/if}
  {/if}
</div>

<style>
  .menu { position: fixed; z-index: 60; width: max-content; min-width: 220px; max-width: 280px; gap: 2px; outline: none; --pop-origin: top left; }
  .menu-item:disabled { color: var(--text-3); cursor: default; }
</style>
