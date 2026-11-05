<script lang="ts">
  // The right-click menu on a server tab (#161): Server settings (as the sidebar header's menu opens them), and Mute
  // server as a durations flyout, the channel menu's (#131), or Unmute server. Keyboard as in the channel menu.
  import { onMount, untrack } from 'svelte'
  import { app, clock } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { clickOutside } from '../lib/actions'
  import { muteUntil } from '../lib/muteDurations'
  import MuteFlyout from './MuteFlyout.svelte'

  let { serverId, name, x, y, opener, onClose, onSettings, onError }: {
    serverId: string
    name: string
    x: number
    y: number
    /** The tab it opened from: focus goes back to it when the menu closes from the keyboard. */
    opener: HTMLElement
    onClose: () => void
    onSettings: () => void
    /** An action failed after the menu had gone. */
    onError: (message: string) => void
  } = $props()

  const muted = $derived(
    app.state.mutes.some((m) => m.target_kind === 'server' && m.target_id === serverId && (m.until === null || m.until > clock.nowIso)),
  )

  let el = $state<HTMLElement>()
  let left = $state(untrack(() => x))
  let top = $state(untrack(() => y))
  let closed = false
  let flyout = $state<ReturnType<typeof MuteFlyout>>()

  onMount(() => {
    if (!el) return
    // the layout size: the .pop-in entrance scales the box while it runs
    left = Math.max(8, Math.min(x, window.innerWidth - el.offsetWidth - 8))
    top = Math.max(8, Math.min(y, window.innerHeight - el.offsetHeight - 8))
    focusItem(0)
  })

  const items = () => (el ? [...el.querySelectorAll<HTMLButtonElement>(':scope > [data-item]:not(:disabled)')] : [])
  function focusItem(i: number) {
    const list = items()
    if (list.length) list[(i + list.length) % list.length].focus()
  }
  function close(restoreFocus: boolean) {
    if (closed) return
    closed = true
    if (restoreFocus && opener.isConnected) opener.focus()
    onClose()
  }
  /** Focus goes back to the tab, it acts, then the menu goes; a failure lands with onError after that. */
  function pick(action: () => unknown) {
    if (closed) return
    if (opener.isConnected) opener.focus()
    void action()
    close(false)
  }
  const report = (f: () => Promise<unknown>) => () => f().catch((e) => onError(errorText(e)))

  function keydown(e: KeyboardEvent) {
    const list = items()
    const at = list.indexOf(document.activeElement as HTMLButtonElement)
    if (e.key === 'ArrowLeft') { e.preventDefault(); flyout?.close(false); return }
    if (e.key === 'Escape' && flyout?.isOpen()) { e.preventDefault(); e.stopPropagation(); flyout.close(false); return }
    if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(e.key)) flyout?.close(false)
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

<div bind:this={el} class="menu menu-surface pop-in" role="menu" aria-label="Actions for {name}" tabindex="-1" style:left="{left}px" style:top="{top}px"
  onkeydown={keydown} use:clickOutside={(e) => close(e.type === 'keydown')}
  onpointerover={(e) => {
    const t = (e.target as Element).closest?.('[data-item]')
    if (t && t.parentElement === el && t !== flyout?.trigger()) flyout?.closeSoon()
  }}>
  <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(onSettings)}>Server settings</button>
  {#if muted}
    <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(report(() => api.clearMute('server', serverId)))}>Unmute server</button>
  {:else}
    <MuteFlyout bind:this={flyout} label="Mute server" flyoutLabel="Mute {name} for" menuLeft={left} menuTop={top}
      onPick={(m) => pick(report(() => api.setMute('server', serverId, muteUntil(m))))} />
  {/if}
</div>

<style>
  .menu { position: fixed; z-index: 60; width: max-content; min-width: 220px; max-width: 280px; gap: 2px; outline: none; --pop-origin: top left; }
</style>
