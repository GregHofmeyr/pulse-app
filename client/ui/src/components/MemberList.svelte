<script lang="ts">
  import { app, clock, ui } from '../lib/store.svelte'
  import { isHidden } from '../lib/blocks'
  import { byPresence, presenceOf } from '../lib/presence'
  import Avatar from './Avatar.svelte'
  import Icon from './Icon.svelte'
  import Name from './Name.svelte'
  import ListeningLine from './ListeningLine.svelte'
  import PlayerIcon from './PlayerIcon.svelte'
  import { listeningOf } from '../lib/activity'
  import PlayingLine from './PlayingLine.svelte'
  import { playingOf } from '../lib/game'
  import CustomStatusLine from './CustomStatusLine.svelte'
  import { customStatusOf } from '../lib/customStatus'
  import MemberMenu from './MemberMenu.svelte'
  import { baseName, displayName, isMember } from '../lib/selectors'
  import { isMenuKey, rowMenuAt } from '../lib/participantMenu'
  import { openCard } from '../lib/profileCard.svelte'
  import { loadDeadlockRanks, rankOf } from '../lib/deadlock.svelte'
  import { rankText } from '../lib/deadlock'
  import { thumbUrl } from '../lib/achievements'

  let { serverId, onMessage }: { serverId: string; onMessage?: (userId: string) => void } = $props()
  // someone blocked with Hide completely (#474) isn't listed or counted
  const members = $derived((app.state.members[serverId] ?? []).filter((m) => !isHidden(app.state, m.user.id)))
  // Online (DND included) above Offline, as Discord does (#170): only by what you're told, so someone Invisible or
  // hiding from you is Offline. A section with nobody in it has no heading.
  const sections = $derived.by(() => {
    const { online, offline } = byPresence(app.state, members, (m) => m.user.id, clock.nowIso, serverId)
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
  // Deadlock ranks (#390): a small badge after the name of members who show theirs
  $effect(() => {
    void loadDeadlockRanks(serverId)
  })
  // the row under the pointer: its avatar animates
  let hovered = $state<string | null>(null)

  // A row's menu (#113): right-click, or ContextMenu / Shift+F10 on its name or avatar. Focus goes
  // back to what had it (from the keyboard), else to the row's name.
  let menu = $state<{ userId: string; x: number; y: number; opener: HTMLElement } | null>(null)
  function openMenu(userId: string, e: MouseEvent | KeyboardEvent, row: HTMLElement) {
    menu = { userId, ...rowMenuAt(e, row) }
  }
  function rowKey(userId: string, e: KeyboardEvent) {
    if (isMenuKey(e)) {
      e.preventDefault()
      openMenu(userId, e, e.currentTarget as HTMLElement)
    }
  }
</script>

<aside class="members" aria-label="Members">
  <!-- Invite people (#80): any member, from the top of the list -->
  {#if isMember(app.state, serverId)}
    <button type="button" class="invite" onclick={() => (ui.inviteTo = serverId)}><Icon name="plus" size={14} /> Invite</button>
  {/if}
  {#each sections as g, gi (g.key)}
  <div class="label">{g.label} — {g.list.length}</div>
  {#each drawnRows[gi] < g.list.length ? g.list.slice(0, drawnRows[gi]) : g.list as m (m.user.id)}
    {@const name = displayName(app.state, serverId, m.user.id)}
    {@const base = baseName(m.user)}
    {@const custom = customStatusOf(app.state, m.user.id, clock.nowIso)}
    {@const playing = custom ? null : playingOf(app.state, m.user.id)}
    {@const listening = custom || playing ? null : listeningOf(app.state, m.user.id)}
    {@const rank = rankOf(serverId, m.user.id)}
    {@const badge = rank ? thumbUrl(rank.image) : null}
    <!-- the context menu and a click on the row (their card, #182) are pointer shortcuts; the keyboard route is the
         row's buttons (Enter on the name, ContextMenu / Shift+F10) -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="row hover-row" class:off={g.key === 'offline'} data-section={g.key} data-card-anchor role="presentation"
      onclick={(e) => { if (!(e.target as Element).closest('button')) openCard(m.user.id, serverId, e.currentTarget) }} onpointerenter={() => (hovered = m.user.id)} onpointerleave={() => { if (hovered === m.user.id) hovered = null }}
      oncontextmenu={(e) => { e.preventDefault(); openMenu(m.user.id, e, e.currentTarget) }} onkeydown={(e) => rowKey(m.user.id, e)}>
      <Avatar id={m.user.id} {name} size={32} presence={presenceOf(app.state, m.user.id, clock.nowIso, serverId)} interactive {serverId} hovered={() => hovered === m.user.id} />
      <!-- their custom status (#152), else the game they're playing (#392), else what they're listening to (#275),
           takes the line under the name; Spotify's icon sits at the row's end, only with its line -->
      <span class="who"><Name userId={m.user.id} {serverId} text={name} surface="side" />{#if custom}<CustomStatusLine status={custom} />{:else if playing}<PlayingLine {playing} />{:else if listening}<ListeningLine {listening} icon={false} />{:else if name !== base}<small class="name-clip">{base}</small>{/if}</span>
      {#if rank && badge}<img class="rank" src={badge} alt="Deadlock rank {rankText(rank)}" data-tip="Deadlock: {rankText(rank)}" data-deadlock-rank />{/if}
      {#if listening}<span class="sp"><PlayerIcon {listening} /></span>{/if}
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
  .invite { width: 100%; height: 32px; margin-bottom: 2px; padding: 0 10px; border: 0; border-radius: var(--radius-md); background: transparent; color: var(--text-2); font: inherit; font-size: 13px; font-weight: 600; display: flex; align-items: center; gap: 8px; cursor: pointer; }
  .invite:hover { background: var(--bg-2); color: var(--text); }
  .label { padding: 14px 10px 6px; font-size: 11px; font-weight: 600; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); }
  .row { cursor: pointer; min-height: 44px; padding: 0 10px; border-radius: var(--radius-md); display: flex; align-items: center; gap: 10px; }
  /* 15 px: a styled name font (script, pixel) blurs at smaller sizes */
  .who { display: flex; flex-direction: column; font-size: 15px; font-weight: 500; min-width: 0; }
  .who small { font-size: 11px; color: var(--text-3); font-weight: 400; }
  /* a row's revealed action: the same in the group member list (#173) */
  .msg { margin-left: auto; width: 28px; height: 28px; flex-shrink: 0; border: 0; border-radius: var(--radius-sm); background: var(--bg-3); color: var(--text-2); display: grid; place-items: center; opacity: 0; }
  .msg:hover { background: var(--bg-4); color: var(--text); }
  .row:hover .msg, .msg:focus-visible { opacity: 1; }
  .sp { margin-left: auto; flex-shrink: 0; color: var(--text-3); }
  .rank { margin-left: auto; width: 22px; height: 22px; flex-shrink: 0; object-fit: contain; }
  .rank + .sp, .rank ~ .msg { margin-left: 0; }
  .sp + .msg { margin-left: 0; }
  .row:hover { background: var(--bg-2); --ring: var(--bg-2); }
  /* offline (#170): dimmed slightly, as on Discord; back to full while hovered */
  .row.off { opacity: .55; }
  .row.off:hover { opacity: 1; }
  /* the status dot (#169): small, cut out of the avatar in the row's own colour (--ring), as on Discord */
  .row :global(.pres) { width: 12px; height: 12px; min-width: 0; min-height: 0; right: -1px; bottom: -1px; }
</style>
