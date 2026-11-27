<script lang="ts">
  // The right-click menu on a server tab (#161): Invite people (#80, its members), Server settings (as the sidebar header's menu opens them), Rename (#55, anyone in it), Mute
  // server as a durations flyout, the channel menu's (#131), or Unmute server, and Notifications: the server's level
  // as a flyout with yours ticked (#195), and Move left / Move right in your rail (#185: dragging's keyboard and menu
  // way), and Leave server (#371, asked first; not its owner's). Arrange channels just for me / Reset to server order
  // (#304): your own order of its channel list. My status here (#154): a flyout of your status in it alone. Keyboard as
  // in the channel menu.
  import { onMount, untrack } from 'svelte'
  import { app, clock, ui } from '../lib/store.svelte'
  import { placeMenu } from '../lib/menuPlace'
  import { api, errorText } from '../lib/tauri'
  import { clickOutside } from '../lib/actions'
  import { muteUntil } from '../lib/muteDurations'
  import { serverLevel } from '../lib/conversations'
  import { NOTIFY_LEVELS, serverLevelToSet } from '../lib/notifyLevels'
  import MenuFlyout from './MenuFlyout.svelte'
  import MuteFlyout from './MuteFlyout.svelte'
  import { isMember } from '../lib/selectors'
  import { SERVER_STATUS_CHOICES } from '../lib/serverStatus'
  import { orderOf, sidebarLayout } from '../lib/layout'
  import { hasPersonalOrder, resetPersonalOrder, savePersonalOrder } from '../lib/channelOrder.svelte'

  let { serverId, name, x, y, opener, onClose, onSettings, onRename, onCreateCategory, onMoveLeft, onMoveRight, onError }: {
    serverId: string
    name: string
    x: number
    y: number
    /** The tab it opened from: focus goes back to it when the menu closes from the keyboard. */
    opener: HTMLElement
    onClose: () => void
    onSettings: () => void
    /** Rename (#55): anyone in it (v1 trust model); the name at the top of its channel list becomes a field. */
    onRename?: () => void
    /** Create category (#54): members of a server with categories; the channel list opens the field. */
    onCreateCategory?: () => void
    /** Move it one place in your rail (#185): given only when it can go that way. */
    onMoveLeft?: () => void
    onMoveRight?: () => void
    /** An action failed after the menu had gone. */
    onError: (message: string) => void
  } = $props()

  const muted = $derived(
    app.state.mutes.some((m) => m.target_kind === 'server' && m.target_id === serverId && (m.until === null || m.until > clock.nowIso)),
  )
  // Leave server (#371): anyone in it but its owner (there's no handing a server over, so the owner can't)
  const canLeave = $derived(isMember(app.state, serverId) && app.state.servers.find((s) => s.id === serverId)?.owner_id !== app.state.me?.id)
  // an older server has no categories to add to
  const canCreateCategory = $derived(!!onCreateCategory && isMember(app.state, serverId) && !!app.state.layouts[serverId])
  // your own channel order (#304): members of a server with categories; started from the list as it is now
  const canArrange = $derived(isMember(app.state, serverId) && !!app.state.layouts[serverId])
  const ownOrder = $derived(hasPersonalOrder(serverId))
  const startOwnOrder = () => savePersonalOrder(serverId, orderOf(sidebarLayout(app.state, serverId)))

  let el = $state<HTMLElement>()
  let left = $state(untrack(() => x))
  let top = $state(untrack(() => y))
  /** where its pop-in grows from: the corner nearest the cursor (#181) */
  let origin = $state('top left')
  let closed = false
  let flyout = $state<ReturnType<typeof MuteFlyout>>()
  let levelFlyout = $state<ReturnType<typeof MenuFlyout>>()
  // My status here (#154): yours in this server only; members see it there, everyone your main status elsewhere
  let statusFlyout = $state<ReturnType<typeof MenuFlyout>>()
  const myHere = $derived(app.state.myServerStatuses.find((x) => x.server_id === serverId)?.status ?? null)
  const statusChoices = $derived(SERVER_STATUS_CHOICES.map((c) => ({ label: c.label, checked: c.status === myHere })))
  const pickStatus = (i: number) => pick(report(() => api.setServerStatus(serverId, SERVER_STATUS_CHOICES[i].status)))
  const flyouts = () => [flyout, levelFlyout, statusFlyout]
  const level = $derived(serverLevel(app.state, serverId))
  const levelChoices = $derived(NOTIFY_LEVELS.map((l) => ({ label: l.label, checked: l.level === level })))
  const pickLevel = (i: number) => pick(report(() => api.setNotifyLevel('server', serverId, serverLevelToSet(NOTIFY_LEVELS[i].level))))

  onMount(() => {
    if (!el) return
    // the layout size: the .pop-in entrance scales the box while it runs
    ;({ left, top, origin } = placeMenu(x, y, el.offsetWidth, el.offsetHeight))
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
    if (e.key === 'ArrowLeft') { e.preventDefault(); for (const f of flyouts()) f?.close(false); return }
    const open = flyouts().find((f) => f?.isOpen())
    if (e.key === 'Escape' && open) { e.preventDefault(); e.stopPropagation(); open.close(false); return }
    if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(e.key)) for (const f of flyouts()) f?.close(false)
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

<div bind:this={el} class="menu menu-surface pop-in" role="menu" aria-label="Actions for {name}" tabindex="-1" style:left="{left}px" style:top="{top}px" style:--pop-origin={origin}
  onkeydown={keydown} use:clickOutside={(e) => close(e.type === 'keydown')}
  onpointerover={(e) => {
    const t = (e.target as Element).closest?.('[data-item]')
    if (t && t.parentElement === el) for (const f of flyouts()) if (t !== f?.trigger()) f?.closeSoon()
  }}>
  {#if isMember(app.state, serverId)}
    <button type="button" class="menu-item" role="menuitem" data-item="invite" onclick={() => pick(() => (ui.inviteTo = serverId))}>Invite people</button>
  {/if}
  <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(onSettings)}>Server settings</button>
  {#if isMember(app.state, serverId) && onRename}
    <button type="button" class="menu-item" role="menuitem" data-item="rename" onclick={() => pick(onRename)}>Rename</button>
  {/if}
  {#if canCreateCategory && onCreateCategory}
    <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(onCreateCategory)}>Create category</button>
  {/if}
  {#if canArrange}
    {#if ownOrder}
      <button type="button" class="menu-item" role="menuitem" data-item="own-order" onclick={() => pick(report(() => resetPersonalOrder(serverId)))}>Reset to server order</button>
    {:else}
      <button type="button" class="menu-item" role="menuitem" data-item="own-order" onclick={() => pick(report(startOwnOrder))}>Arrange channels just for me</button>
    {/if}
  {/if}
  {#if muted}
    <button type="button" class="menu-item" role="menuitem" data-item onclick={() => pick(report(() => api.clearMute('server', serverId)))}>Unmute server</button>
  {:else}
    <MuteFlyout bind:this={flyout} label="Mute server" flyoutLabel="Mute {name} for" menuLeft={left} menuTop={top}
      onPick={(m) => pick(report(() => api.setMute('server', serverId, muteUntil(m))))} />
  {/if}
  <MenuFlyout bind:this={levelFlyout} label="Notifications" flyoutLabel="Notifications for {name}" menuLeft={left} menuTop={top}
    choices={levelChoices} onPick={pickLevel} />
  {#if isMember(app.state, serverId)}
    <MenuFlyout bind:this={statusFlyout} item="my-status" label="My status here" flyoutLabel="My status in {name}" menuLeft={left} menuTop={top}
      choices={statusChoices} onPick={pickStatus} />
  {/if}
  {#if canLeave}
    <span class="menu-sep" role="separator"></span>
    <button type="button" class="menu-item danger" role="menuitem" data-item="leave" onclick={() => pick(() => (ui.serverAction = { kind: 'leave', serverId }))}>Leave server</button>
  {/if}
  {#if onMoveLeft || onMoveRight}
    <span class="menu-sep" role="separator"></span>
    {#if onMoveLeft}<button type="button" class="menu-item" role="menuitem" data-item="move-left" onclick={() => pick(onMoveLeft)}>Move left</button>{/if}
    {#if onMoveRight}<button type="button" class="menu-item" role="menuitem" data-item="move-right" onclick={() => pick(onMoveRight)}>Move right</button>{/if}
  {/if}
</div>

<style>
  .menu { position: fixed; z-index: 60; width: max-content; min-width: 220px; max-width: 280px; gap: 2px; outline: none; --pop-origin: top left; }
</style>
