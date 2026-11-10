<script lang="ts">
  // A person's menu (#113, #137): from a member-list row, a message author, or a right-click on their profile card
  // (where the server may be none and View profile is left out: you're on it), or a message author in a DM or group
  // (#189: no server, so no server nickname). The profile, a message, server mute / deafen (#144: as the voice
  // participant menu's), and the nickname items, Appear offline to them (#45), and in a DM muting it (the DM list's
  // flyout, #190). Keyboard and focus work as in the voice participant menu.
  import { onMount, untrack } from 'svelte'
  import { app, clock, ui } from '../lib/store.svelte'
  import { placeMenu } from '../lib/menuPlace'
  import { api, errorText } from '../lib/tauri'
  import { muteUntil } from '../lib/muteDurations'
  import { displayName, inServerVoice, serverVoiceOf } from '../lib/selectors'
  import { memberVoiceItems } from '../lib/participantMenu'
  import ServerVoiceItem from './ServerVoiceItem.svelte'
  import { clickOutside } from '../lib/actions'
  import { openCard } from '../lib/profileCard.svelte'
  import NicknameItems from './NicknameItems.svelte'
  import AppearOfflineItem from './AppearOfflineItem.svelte'
  import LiveNotifyItem from './LiveNotifyItem.svelte'
  import JoinSoundMuteItem from './JoinSoundMuteItem.svelte'
  import MuteFlyout from './MuteFlyout.svelte'

  let { serverId, userId, x, y, opener, onClose, onMessage, fromCard = false, muteChannelId = null }: {
    /** The server it opens in, or null (a card opened outside servers). */
    serverId: string | null
    userId: string
    x: number
    y: number
    /** What had focus (or was clicked): focus goes back to it when the menu closes from the keyboard. */
    opener: HTMLElement
    onClose: () => void
    onMessage?: (userId: string) => void
    /** Opened on their profile card: no View profile. */
    fromCard?: boolean
    /** Opened on a message in your DM with them: Mute conversation mutes that DM. */
    muteChannelId?: string | null
  } = $props()

  const self = $derived(userId === app.state.me?.id)
  const name = $derived(displayName(app.state, serverId, userId))
  const muted = $derived(
    !!muteChannelId && app.state.mutes.some((m) => m.target_kind === 'channel' && m.target_id === muteChannelId && (m.until === null || m.until > clock.nowIso)),
  )
  // Server mute / deafen (#144): the undo for one they still have, even out of voice; both while in this server's voice
  const voiceIds = $derived.by(() => {
    const held = serverVoiceOf(app.state, serverId, userId)
    return memberVoiceItems({
      self,
      server: !!serverId,
      inVoice: !!serverId && inServerVoice(app.state, serverId, userId),
      muted: !!held.serverMutedBy,
      deafened: !!held.serverDeafenedBy,
    })
  })
  const reportMute =(f: () => Promise<unknown>) => () => f().catch((e: unknown) => (ui.notice = errorText(e)))
  // the mute flyout (the channel menu's, #131)
  let flyout = $state<ReturnType<typeof MuteFlyout>>()
  const closeSub = (refocus: boolean | 'if-inside') => flyout?.close(refocus)

  let el = $state<HTMLElement>()
  let left = $state(untrack(() => x))
  let top = $state(untrack(() => y))
  /** where its pop-in grows from: the corner nearest the cursor (#181) */
  let origin = $state('top left')
  let closed = false

  onMount(() => {
    if (!el) return
    const r = { width: el.offsetWidth, height: el.offsetHeight }
    ;({ left, top, origin } = placeMenu(x, y, r.width, r.height))
    focusItem(0)
  })

  // the menu's own items (the flyout's sit inside it, under their own menu)
  const items = () => (el ? [...el.querySelectorAll<HTMLElement>(':scope > [data-item]')] : [])
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
  function keydown(e: KeyboardEvent) {
    const list = items()
    const at = list.indexOf(document.activeElement as HTMLElement)
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

<div bind:this={el} class="menu menu-surface pop-in" role="menu" aria-label="Actions for {name}" tabindex="-1" style:left="{left}px" style:top="{top}px" style:--pop-origin={origin}
  onkeydown={keydown} use:clickOutside={(e) => close(e.type === 'keydown')}
  onpointerover={(e) => {
    // another of the menu's own items under the pointer closes the flyout
    const t = (e.target as Element).closest?.('[data-item]')
    if (t && t.parentElement === el && t !== flyout?.trigger()) flyout?.closeSoon()
  }}>
  <!-- the card opens beside the row the menu came from -->
  {#if !fromCard}
    <button type="button" class="menu-item" role="menuitem" data-item="profile" onclick={() => pick(() => openCard(userId, serverId, opener))}>View profile</button>
  {/if}
  {#if onMessage && !self}
    <button type="button" class="menu-item" role="menuitem" data-item="message" onclick={() => pick(() => onMessage(userId))}>Message</button>
  {/if}
  {#if serverId}
    {#each voiceIds as id (id)}<ServerVoiceItem {id} {serverId} {userId} {pick} />{/each}
  {/if}
  <NicknameItems {serverId} {userId} {pick} />
  <LiveNotifyItem {userId} {pick} />
  <JoinSoundMuteItem {userId} {pick} />
  <AppearOfflineItem {userId} {pick} />
  {#if muteChannelId && !self}
    <span class="menu-sep" role="separator"></span>
    {#if muted}
      <button type="button" class="menu-item" role="menuitem" data-item="mute" onclick={() => pick(reportMute(() => api.clearMute('channel', muteChannelId)))}>Unmute conversation</button>
    {:else}
      <MuteFlyout bind:this={flyout} item="mute" label="Mute conversation" flyoutLabel="Mute {name} for" menuLeft={left} menuTop={top}
        onPick={(m) => pick(reportMute(() => api.setMute('channel', muteChannelId, muteUntil(m))))} />
    {/if}
  {/if}
</div>

<style>
  .menu { position: fixed; z-index: 60; width: max-content; min-width: 220px; max-width: 280px; gap: 2px; font-size: 13px; outline: none; --pop-origin: top left; }
  /* on a card the nickname items come first: no line above them */
  .menu > :global(.menu-sep:first-child) { display: none; }
</style>
