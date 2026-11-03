<script lang="ts">
  // A member-list row's menu (#113): the profile, a message, and the nickname actions the profile
  // card offers. Keyboard and focus work as in the voice participant menu.
  import { onMount, untrack } from 'svelte'
  import { app } from '../lib/store.svelte'
  import { displayName } from '../lib/selectors'
  import { clickOutside } from '../lib/actions'
  import { openCard } from '../lib/profileCard.svelte'
  import { errorText } from '../lib/tauri'
  import NicknameItems from './NicknameItems.svelte'

  let { serverId, userId, x, y, opener, onClose, onMessage }: {
    serverId: string
    userId: string
    x: number
    y: number
    /** What had focus (or was clicked): focus goes back to it when the menu closes from the keyboard. */
    opener: HTMLElement
    onClose: () => void
    onMessage?: (userId: string) => void
  } = $props()

  const self = $derived(userId === app.state.me?.id)
  const name = $derived(displayName(app.state, serverId, userId))

  let el = $state<HTMLElement>()
  let left = $state(untrack(() => x))
  let top = $state(untrack(() => y))
  let closed = false
  let error = $state<string | null>(null)
  let running = false

  onMount(() => {
    if (!el) return
    const r = { width: el.offsetWidth, height: el.offsetHeight }
    left = Math.max(8, Math.min(x, window.innerWidth - r.width - 8))
    top = Math.max(8, Math.min(y, window.innerHeight - r.height - 8))
    focusItem(0)
  })

  const items = () => (el ? [...el.querySelectorAll<HTMLElement>('[data-item]')] : [])
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
  /** Focus goes back first (so a dialog the action opens remembers the right place), then it acts, then the menu goes. */
  function pick(action: () => unknown) {
    if (closed) return
    if (opener.isConnected) opener.focus()
    void action()
    close(false)
  }
  /** A call that can be refused (a nickname reset): the menu stays open to say why. */
  async function run(call: () => Promise<unknown>) {
    if (closed || running) return
    running = true
    error = null
    try {
      await call()
      close(true)
    } catch (e) {
      error = errorText(e)
    } finally {
      running = false
    }
  }

  function keydown(e: KeyboardEvent) {
    const list = items()
    const at = list.indexOf(document.activeElement as HTMLElement)
    switch (e.key) {
      case 'ArrowDown': e.preventDefault(); focusItem(at + 1); break
      case 'ArrowUp': e.preventDefault(); focusItem(at < 0 ? -1 : at - 1); break
      case 'Home': e.preventDefault(); focusItem(0); break
      case 'End': e.preventDefault(); focusItem(-1); break
      case 'Escape': e.preventDefault(); close(true); break
      case 'Tab': e.preventDefault(); close(true); break
      case 'Enter': {
        // a button's own Enter would click it too; one path keeps it to a single activation
        const t = e.target as HTMLElement
        if (t instanceof HTMLButtonElement) {
          e.preventDefault()
          t.click()
        }
        break
      }
    }
  }
</script>

<div bind:this={el} class="menu menu-surface pop-in" role="menu" aria-label="Actions for {name}" tabindex="-1" style:left="{left}px" style:top="{top}px"
  onkeydown={keydown} use:clickOutside={(e) => close(e.type === 'keydown')}>
  <!-- the card opens beside the row the menu came from -->
  <button type="button" class="menu-item" role="menuitem" data-item="profile" onclick={() => pick(() => openCard(userId, serverId, opener))}>View profile</button>
  {#if onMessage && !self}
    <button type="button" class="menu-item" role="menuitem" data-item="message" onclick={() => pick(() => onMessage(userId))}>Message</button>
  {/if}
  <NicknameItems {serverId} {userId} {pick} {run} />
  {#if error}<p class="err" role="alert">{error}</p>{/if}
</div>

<style>
  .menu { position: fixed; z-index: 60; min-width: 200px; max-width: 280px; gap: 2px; font-size: 13px; outline: none; }
  .err { margin: 6px 8px 2px; font-size: 12px; color: var(--danger-text); }
</style>
