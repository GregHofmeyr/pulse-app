<script lang="ts">
  // The right-click menu on a sidebar channel row (or Shift+F10 / the menu key on it). Text, quotes
  // and clips: mark as read and mute (a flyout of durations, #131); voice: join or leave. Anyone in the server may rename and
  // delete (v1 trust model); those two are handed back to the list, which edits in the row and asks
  // in a dialog.
  import { onMount, untrack } from 'svelte'
  import { app, clock } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { clickOutside } from '../lib/actions'
  import { muteUntil } from '../lib/muteDurations'
  import { voice, voiceApi } from '../lib/voice.svelte'
  import MuteFlyout from './MuteFlyout.svelte'
  import type { Channel } from '../lib/protocol/Channel'

  let { channel, member, x, y, opener, onClose, onRename, onDelete, onJoin, onError }: {
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
  let closed = false

  onMount(() => {
    if (!el) return
    // the layout size: the .pop-in entrance scales the box while it runs
    left = Math.max(8, Math.min(x, window.innerWidth - el.offsetWidth - 8))
    top = Math.max(8, Math.min(y, window.innerHeight - el.offsetHeight - 8))
    focusItem(0)
  })

  // the main menu's own items (the flyout's sit inside it, under their own menu)
  const items = () => (el ? [...el.querySelectorAll<HTMLButtonElement>(':scope > [data-item]:not(:disabled)')] : [])
  function focusItem(i: number) {
    const list = items()
    if (list.length) list[(i + list.length) % list.length].focus()
  }

  // The mute flyout (#131; shared with the server tab's menu, #161)
  let flyout = $state<ReturnType<typeof MuteFlyout>>()
  const closeSub = (refocus: boolean | 'if-inside') => flyout?.close(refocus)

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
    if (e.key === 'Escape' && flyout?.isOpen()) { e.preventDefault(); e.stopPropagation(); closeSub(false); return }
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

<div bind:this={el} class="menu menu-surface pop-in" role="menu" aria-label="Actions for {label}" tabindex="-1" style:left="{left}px" style:top="{top}px"
  onkeydown={keydown} use:clickOutside={(e) => close(e.type === 'keydown')}
  onpointerover={(e) => {
    // another of the menu's own items under the pointer closes the flyout
    const t = (e.target as Element).closest?.('[data-item]')
    // (focus in the flyout goes back to Mute channel rather than falling out of the menu)
    if (t && t.parentElement === el && t !== flyout?.trigger()) flyout?.closeSoon()
  }}>
  {#if isVoice}
    {#if inCall}
      <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(report(() => voiceApi.leave()))}>Leave</button>
    {:else}
      <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(onJoin)}>Join</button>
    {/if}
  {:else}
    <button type="button" class="menu-item" role="menuitem" data-item disabled={!unread} onclick={() => pick(report(markRead))}>Mark as read</button>
    {#if muted}
      <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(report(() => api.clearMute('channel', channel.id)))}>Unmute channel</button>
    {:else}
      <MuteFlyout bind:this={flyout} label="Mute channel" flyoutLabel="Mute {label} for" menuLeft={left} menuTop={top}
        onPick={(m) => pick(report(() => api.setMute('channel', channel.id, muteUntil(m))))} />
    {/if}
  {/if}
  {#if member}
    <span class="menu-sep"></span>
    <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(onRename)}>Rename channel…</button>
    <button type="button" class="menu-item danger" role="menuitem" data-item onclick={() => pick(onDelete)}>Delete channel…</button>
  {/if}
</div>

<style>
  .menu { position: fixed; z-index: 60; min-width: 220px; max-width: 280px; gap: 2px; outline: none; }
  .menu-item:disabled { color: var(--text-3); cursor: default; }
</style>
