<script lang="ts">
  import { app, clock } from '../lib/store.svelte'
  import { byPresence, presenceOf } from '../lib/presence'
  import Avatar from './Avatar.svelte'
  import Icon from './Icon.svelte'
  import Name from './Name.svelte'
  import ListeningLine from './ListeningLine.svelte'
  import SpotifyIcon from './SpotifyIcon.svelte'
  import { listeningOf } from '../lib/activity'
  import MemberMenu from './MemberMenu.svelte'
  import { baseName, displayName } from '../lib/selectors'
  import { menuPoint } from '../lib/participantMenu'
  import { openCard } from '../lib/profileCard.svelte'

  let { serverId, onMessage }: { serverId: string; onMessage?: (userId: string) => void } = $props()
  const members = $derived(app.state.members[serverId] ?? [])
  // Online (DND included) above Offline, as Discord does (#170): only by what you're told, so someone Invisible or
  // hiding from you is Offline. A section with nobody in it has no heading.
  const sections = $derived.by(() => {
    const { online, offline } = byPresence(app.state, members, (m) => m.user.id, clock.nowIso)
    return [{ key: 'online', label: 'Online', list: online }, { key: 'offline', label: 'Offline', list: offline }].filter((g) => g.list.length)
  })
  // Opening cheaply (#238): a server's list draws its first rows with the switch and the rest (a big server's hundreds)
  // just after that first frame, holding their height (rows are 44px) until then.
  const FIRST_ROWS = 30
  /** The server whose list is drawn whole. */
  let drawnFor = $state<string | null>(null)
  const rows = $derived(drawnFor === serverId ? Infinity : FIRST_ROWS)
  $effect(() => {
    const id = serverId
    let timer: ReturnType<typeof setTimeout> | undefined
    const frame = requestAnimationFrame(() => (timer = setTimeout(() => (drawnFor = id))))
    return () => {
      cancelAnimationFrame(frame)
      clearTimeout(timer)
    }
  })
  /** How many of each section's rows are drawn: the budget goes to the sections in order. */
  const drawnRows = $derived.by(() => {
    let left = rows
    return sections.map((g) => {
      const n = Math.min(g.list.length, left)
      left -= n
      return n
    })
  })
  // the row under the pointer: its avatar animates
  let hovered = $state<string | null>(null)

  // A row's menu (#113): right-click, or ContextMenu / Shift+F10 on its name or avatar. Focus goes
  // back to what had it (from the keyboard), else to the row's name.
  let menu = $state<{ userId: string; x: number; y: number; opener: HTMLElement } | null>(null)
  function openMenu(userId: string, e: MouseEvent | KeyboardEvent, row: HTMLElement) {
    const focused = document.activeElement instanceof HTMLElement && row.contains(document.activeElement) ? document.activeElement : null
    const opener = (e instanceof KeyboardEvent && focused) || row.querySelector<HTMLElement>('.name') || row
    menu = { userId, ...menuPoint(e, opener), opener }
  }
  function rowKey(userId: string, e: KeyboardEvent) {
    if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      e.preventDefault()
      openMenu(userId, e, e.currentTarget as HTMLElement)
    }
  }
</script>

<aside class="members" aria-label="Members">
  {#each sections as g, gi (g.key)}
  <div class="label">{g.label} — {g.list.length}</div>
  {#each drawnRows[gi] < g.list.length ? g.list.slice(0, drawnRows[gi]) : g.list as m (m.user.id)}
    {@const name = displayName(app.state, serverId, m.user.id)}
    {@const base = baseName(m.user)}
    {@const listening = listeningOf(app.state, m.user.id)}
    <!-- the context menu and a click on the row (their card, #182) are pointer shortcuts; the keyboard route is the
         row's buttons (Enter on the name, ContextMenu / Shift+F10) -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="row hover-row" class:off={g.key === 'offline'} data-section={g.key} data-card-anchor role="presentation"
      onclick={(e) => { if (!(e.target as Element).closest('button')) openCard(m.user.id, serverId, e.currentTarget) }} onpointerenter={() => (hovered = m.user.id)} onpointerleave={() => { if (hovered === m.user.id) hovered = null }}
      oncontextmenu={(e) => { e.preventDefault(); openMenu(m.user.id, e, e.currentTarget) }} onkeydown={(e) => rowKey(m.user.id, e)}>
      <Avatar id={m.user.id} {name} size={32} presence={presenceOf(app.state, m.user.id, clock.nowIso)} interactive {serverId} hovered={() => hovered === m.user.id} />
      <!-- what they're listening to (#275) takes the line under the name; Spotify's icon sits at the row's end -->
      <span class="who"><Name userId={m.user.id} {serverId} text={name} surface="side" />{#if listening}<ListeningLine {listening} icon={false} />{:else if name !== base}<small class="name-clip">{base}</small>{/if}</span>
      {#if listening}<span class="sp"><SpotifyIcon /></span>{/if}
      {#if onMessage && m.user.id !== app.state.me?.id}
        <button class="msg" aria-label="Message {name}" data-tip="Message" onclick={() => onMessage(m.user.id)}><Icon name="message" size={16} /></button>
      {/if}
    </div>
  {/each}
  {#if drawnRows[gi] < g.list.length}<div class="later" style:height="{(g.list.length - drawnRows[gi]) * 44}px"></div>{/if}
  {/each}
</aside>
{#if menu}
  {#key menu}<MemberMenu {serverId} userId={menu.userId} x={menu.x} y={menu.y} opener={menu.opener} {onMessage} onClose={() => (menu = null)} />{/key}
{/if}

<style>
  .members { width: var(--right-w, 240px); flex-shrink: 0; background: var(--bg-1); border-radius: var(--radius); padding: 8px; overflow-y: auto; }
  .label { padding: 14px 10px 6px; font-size: 11px; font-weight: 600; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); }
  .row { cursor: pointer; height: 44px; padding: 0 10px; border-radius: var(--radius-md); display: flex; align-items: center; gap: 10px; }
  /* 15 px: a styled name font (script, pixel) blurs at smaller sizes */
  .who { display: flex; flex-direction: column; font-size: 15px; font-weight: 500; min-width: 0; }
  .who small { font-size: 11px; color: var(--text-3); font-weight: 400; }
  /* a row's revealed action: the same in the group member list (#173) */
  .msg { margin-left: auto; width: 28px; height: 28px; flex-shrink: 0; border: 0; border-radius: var(--radius-sm); background: var(--bg-3); color: var(--text-2); display: grid; place-items: center; opacity: 0; }
  .msg:hover { background: var(--bg-4); color: var(--text); }
  .row:hover .msg, .msg:focus-visible { opacity: 1; }
  .sp { margin-left: auto; flex-shrink: 0; color: var(--text-3); }
  .sp + .msg { margin-left: 0; }
  .row:hover { background: var(--bg-2); --ring: var(--bg-2); }
  /* offline (#170): dimmed slightly, as on Discord; back to full while hovered */
  .row.off { opacity: .55; }
  .row.off:hover { opacity: 1; }
  /* the status dot (#169): small, cut out of the avatar in the row's own colour (--ring), as on Discord */
  .row :global(.pres) { width: 12px; height: 12px; min-width: 0; min-height: 0; right: -1px; bottom: -1px; }
</style>
